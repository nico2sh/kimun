//! The one pass over a note every whole-note extractor reads — see
//! `2026-10-06-single-note-walk-design.md`. `walk` parses the note as written
//! once and returns a `NoteWalk` (text lines, links, tags, frontmatter); every
//! whole-note extractor is a view of it. All offsets are byte offsets into
//! the note as given. Wikilinks are Kimün's own, recognised with the
//! editor's pattern over a plain CommonMark parse (see `TextBlocks::scan`).
//! A rename rewrites links from the same walk (`retarget_links`), so it
//! rewrites exactly the links the index records. The line builder
//! (`TextLine`/`TextLines`) lives here too.

use std::ops::Range;

use log::debug;
use pulldown_cmark::{CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

use super::content_extractor::{
    frontmatter_bounds, frontmatter_delimiter, label_matches_inner, md_link_matches, split_bom,
    split_link_fragment, target_looks_like_image, wikilink_matches, wikilink_parts,
};
use super::ContentChunk;
use crate::nfs::{VaultPath, PATH_SEPARATOR};

use super::content_extractor::is_remote_url;
use super::{LinkType as NoteLinkType, NoteHeading, NoteLink};

/// What kind of link a [`WalkLink`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::note) enum WalkLinkKind {
    /// `[[target|text]]`.
    Wiki,
    /// `![[target|text]]`.
    WikiEmbed,
    /// `[label](dest)`.
    Inline,
    /// `[label](dest)` pulldown did not read as a link — in practice a
    /// destination with spaces — found with the editor's pattern (see
    /// `TextBlocks::scan`). Rendered as a link only when it links somewhere.
    Found,
    /// `[label][ref]`, `[label][]`, `[label]` resolved against a definition.
    Reference,
    /// `<https://…>` or `<mail@…>`.
    Autolink,
    /// `![alt](dest)` and its reference forms, and one pulldown did not
    /// read found with the editor's pattern (see `TextBlocks::scan`).
    Image,
}

/// A link the walk found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::note) struct WalkLink {
    pub kind: WalkLinkKind,
    /// A wikilink's target per `wikilink_parts`; otherwise the destination
    /// as pulldown resolved it.
    pub target: String,
    /// The link's text as written: a wikilink's display part, the `[label]`
    /// of a markdown link or image, an autolink's address. Source bytes, a
    /// CRLF kept: the rendered markdown splices it back into the note; the
    /// listed link text (`NoteLink::text`) is it with LF line endings.
    pub label: String,
    /// Source bytes of the whole link, in the note.
    pub range: Range<usize>,
    /// The rendered markdown leaves it as written (still listed): an inline
    /// link written with a title or a `<…>` destination (neither survives
    /// being re-emitted), a wikilink in an indented code block (code).
    pub as_written: bool,
}

impl WalkLink {
    /// Whether this is a link at all — the one predicate every view lists
    /// links by: a wikilink (or embed) when the note it points to (see
    /// [`Self::wiki_note`]) is a valid vault path — `[[#tag]]` reads as a
    /// wikilink but links nowhere — and any other when its destination
    /// links somewhere ([`md_vault_path`]: a URL or a valid vault path), so
    /// `[t]()`, `[t]([[b]])` and `[x](a|b.md)` are no links.
    /// [`NoteWalk::link_targets`] (the CLI, `NoteMetadata::links`) lists
    /// every link that passes, its target as written.
    fn is_link(&self) -> bool {
        match self.kind {
            WalkLinkKind::Wiki | WalkLinkKind::WikiEmbed => {
                let (note, _) = self.wiki_note();
                !note.is_empty() && VaultPath::is_valid(note)
            }
            _ => is_remote_url(&self.target) || md_vault_path(&self.target).is_some(),
        }
    }

    /// A wikilink's target split into the note it points to and the
    /// `#section` / `^block` inside it (`split_link_fragment`).
    fn wiki_note(&self) -> (&str, &str) {
        split_link_fragment(&self.target)
    }

    /// How an inline link writes `dest`, its resolved destination, back as
    /// `[label](…)` (see [`markdown_destination`]); `None` leaves it as
    /// written: so does one [`Self::as_written`]. Padding and escapes in the
    /// destination are no reason to keep it.
    fn rewritten_destination(&self, dest: String) -> Option<String> {
        if self.as_written {
            None
        } else {
            markdown_destination(dest)
        }
    }
}

/// A hashtag in prose: its name without `#`, and the source bytes of
/// `#name` in the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::note) struct WalkTag {
    pub name: String,
    pub range: Range<usize>,
}

/// Everything one pass over a note yields; every whole-note extractor is a
/// view of it. All offsets index the note as given.
pub(in crate::note) struct NoteWalk {
    pub lines: Vec<TextLine>,
    pub links: Vec<WalkLink>,
    pub tags: Vec<WalkTag>,
    /// The frontmatter text: the lines between its fences without their
    /// endings, joined with `\n`; empty without a closed block.
    pub frontmatter: String,
}

/// Byte of `note` where the body starts — after a closed frontmatter block,
/// after only the first line of an unclosed fence (an unclosed fence is not
/// frontmatter), or after a byte-order mark — and, for a closed block, the
/// bytes between its fences.
fn body_start(note: &str) -> (usize, Option<Range<usize>>) {
    if let Some(bounds) = frontmatter_bounds(note) {
        return (bounds.end, Some(bounds.inner));
    }
    let start = match frontmatter_delimiter(note) {
        Some((_, after_first_line)) => after_first_line,
        None => split_bom(note).0.len(),
    };
    (start, None)
}

/// A frontmatter block's text: its lines without their endings, joined
/// with `\n`.
fn frontmatter_text(block: &str) -> String {
    let mut out = String::with_capacity(block.len());
    for (i, line) in block.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

/// A wikilink's record — `[[…]]` or `![[…]]` at `range` of the walked
/// `body`, which starts at byte `body_start` of the note — and the range of
/// its display text in `body`: target and text by `wikilink_parts`. `None`
/// without a target (`[[|b]]`): not a wikilink, its text stays as written.
fn wikilink(
    body: &str,
    range: Range<usize>,
    body_start: usize,
) -> Option<(WalkLink, Range<usize>)> {
    let (kind, opener) = if body[range.clone()].starts_with('!') {
        (WalkLinkKind::WikiEmbed, "![[".len())
    } else {
        (WalkLinkKind::Wiki, "[[".len())
    };
    let inner = &body[range.start + opener..range.end - "]]".len()];
    let (target, label) = wikilink_parts(inner);
    if target.is_empty() {
        return None;
    }
    // `label` is a slice of `body`: its place there.
    let display = label.as_ptr() as usize - body.as_ptr() as usize;
    let link = WalkLink {
        kind,
        target: lf(target).into_owned(),
        label: label.to_string(),
        range: body_start + range.start..body_start + range.end,
        as_written: false,
    };
    Some((link, display..display + label.len()))
}

/// A markdown link or image a parse is inside: the record whose label its
/// text sets (none for an autolink, labelled by its address, or an email
/// autolink, not recorded), the byte of the parsed text its label starts at
/// (just past the opening `[` / `![`, so a leading escape like `\[` stays in
/// it), and the end of the text events seen so far.
struct OpenLink {
    record: Option<usize>,
    label_start: usize,
    label_end: Option<usize>,
}

impl OpenLink {
    /// One more event of the link's text.
    fn extend(&mut self, event: &Range<usize>) {
        self.label_end = Some(self.label_end.map_or(event.end, |end| end.max(event.end)));
    }

    /// The link ended: its label is the source of its text, as written,
    /// and an inline link whose destination is written in `<…>` (the text
    /// after the `](` that follows its label) is kept as written.
    fn close(self, links: &mut [WalkLink], src: &str) {
        let Some(record) = self.record else {
            return;
        };
        let link = &mut links[record];
        let label_end = self.label_end.unwrap_or(self.label_start);
        link.label = src[self.label_start..label_end].to_string();
        if link.kind == WalkLinkKind::Inline {
            let after = &src[label_end..];
            link.as_written |= after
                .find("](")
                .is_some_and(|at| after[at + 2..].trim_start().starts_with('<'));
        }
    }
}

/// Records the markdown links and images of the walk's parse, with
/// pulldown's destination and their label as written. HTML is opaque:
/// pulldown hands it over unparsed, and nothing inside it is a link.
struct LinkRecorder<'s> {
    /// The parsed text as written: labels come from it.
    src: &'s str,
    /// Byte of the note `src` starts at.
    offset: usize,
    /// Open links and images: text inside them is link text, and its source
    /// is their label.
    open: Vec<OpenLink>,
}

impl<'s> LinkRecorder<'s> {
    fn new(src: &'s str, offset: usize) -> Self {
        Self {
            src,
            offset,
            open: Vec::new(),
        }
    }

    /// Takes one event (`range` in `src`), recording any link it starts or
    /// labels; whether the event is link or image text.
    fn step(&mut self, event: &Event, range: &Range<usize>, links: &mut Vec<WalkLink>) -> bool {
        if matches!(event, Event::End(TagEnd::Link | TagEnd::Image)) {
            if let Some(open) = self.open.pop() {
                open.close(links, self.src);
            }
        }
        for open in &mut self.open {
            open.extend(range);
        }
        if let Event::Start(Tag::Link { .. } | Tag::Image { .. }) = event {
            let in_note = self.offset + range.start..self.offset + range.end;
            let found = md_link(event, in_note);
            let labelled = found
                .as_ref()
                .is_some_and(|l| l.kind != WalkLinkKind::Autolink);
            links.extend(found);
            let opener = if matches!(event, Event::Start(Tag::Image { .. })) {
                "!["
            } else {
                "["
            };
            self.open.push(OpenLink {
                record: labelled.then(|| links.len() - 1),
                label_start: range.start + opener.len(),
                label_end: None,
            });
        }
        !self.open.is_empty()
    }
}

/// A markdown link or image's record (none for an email autolink), its
/// label left for its [`OpenLink`] to fill.
// Out of line: most events start no link, and inlining the link builders
// into `LinkRecorder::step` measurably slowed the walk (criterion, medium).
#[inline(never)]
fn md_link(event: &Event, range: Range<usize>) -> Option<WalkLink> {
    let (kind, target, label, titled) = match event {
        // An email address is not a vault or web link: its text stays in
        // the line, nothing is recorded.
        Event::Start(Tag::Link {
            link_type: LinkType::Email,
            ..
        }) => return None,
        Event::Start(Tag::Link {
            link_type: LinkType::Autolink,
            dest_url,
            ..
        }) => (
            WalkLinkKind::Autolink,
            dest_url,
            dest_url.to_string(),
            false,
        ),
        Event::Start(Tag::Link {
            link_type: LinkType::Inline,
            dest_url,
            title,
            ..
        }) => (
            WalkLinkKind::Inline,
            dest_url,
            String::new(),
            !title.is_empty(),
        ),
        Event::Start(Tag::Link { dest_url, .. }) => {
            (WalkLinkKind::Reference, dest_url, String::new(), false)
        }
        Event::Start(Tag::Image { dest_url, .. }) => {
            (WalkLinkKind::Image, dest_url, String::new(), false)
        }
        _ => return None,
    };
    Some(WalkLink {
        kind,
        target: target.to_string(),
        label,
        range,
        // The `<…>` form is read from the source when the link closes.
        as_written: titled,
    })
}

/// A prose text event's line text. Hashtag candidates lying wholly inside
/// it become tags and lose their `#`, unless it is link text (kept as
/// written, not tags). When the source and decoded text differ in length
/// (HTML entities) the tags are recorded but the `#` stays.
fn prose_text(
    decoded: &str,
    range: Range<usize>,
    body: &str,
    body_start: usize,
    candidates: &[(Range<usize>, &str)],
    in_link: bool,
    tags: &mut Vec<WalkTag>,
) -> String {
    let first = candidates.partition_point(|(r, _)| r.start < range.start);
    let inside: Vec<&(Range<usize>, &str)> = candidates[first..]
        .iter()
        .take_while(|(r, _)| r.end <= range.end)
        .collect();
    if inside.is_empty() || in_link {
        return decoded.to_string();
    }
    for (r, name) in &inside {
        tags.push(WalkTag {
            name: name.to_string(),
            range: body_start + r.start..body_start + r.end,
        });
    }
    let source = &body[range.clone()];
    if source.len() != decoded.len() {
        return decoded.to_string();
    }
    let mut out = String::with_capacity(source.len());
    let mut last = range.start;
    for (r, _) in &inside {
        out.push_str(&body[last..r.start]);
        out.push_str(&body[r.start + 1..r.end]);
        last = r.end;
    }
    out.push_str(&body[last..range.end]);
    out
}

/// Whether `r` overlaps one of `ranges`, which are disjoint and in order.
fn overlaps_any(ranges: &[Range<usize>], r: &Range<usize>) -> bool {
    let i = ranges.partition_point(|x| x.end <= r.start);
    ranges.get(i).is_some_and(|x| x.start < r.end)
}

/// Whether the byte after `before` is escaped: an odd run of backslashes
/// ends `before` (`\\[` is a literal backslash followed by a real `[`).
fn escaped(before: &str) -> bool {
    (before.len() - before.trim_end_matches('\\').len()) % 2 == 1
}

/// A wikilink the walk found: its source in the walked body (an embed's `!`
/// included), its display text's, and its record.
struct Wiki {
    range: Range<usize>,
    display: Range<usize>,
    link: WalkLink,
}

/// A paragraph, heading, list item or indented code block the walk is
/// inside — a block of text, where wikilinks, and markdown links pulldown
/// did not read, are looked for when it ends (see [`TextBlocks::end`]).
/// Ranges are bytes of the walked body, each list in document order.
struct TextBlock {
    range: Range<usize>,
    /// An indented code block — in practice a Logseq-style outline indented
    /// after a heading, which the editor shows as text: only wikilinks are
    /// looked for, and they are left as written (see [`TextBlocks::scan`]).
    code: bool,
    /// Its child blocks (a nested list, a paragraph of a loose item, code,
    /// HTML…): not its own text; each is read as its own block.
    children: Vec<Range<usize>>,
    /// The links and images pulldown read in it, outermost only (an image
    /// inside a link is in the link): disjoint.
    links: Vec<Range<usize>>,
    /// Its code spans and inline HTML.
    opaque: Vec<Range<usize>>,
}

/// What pulldown does not read, found per text block: wikilinks (the
/// editor's pattern, [`wikilink_matches`]) and markdown links whose
/// destination has spaces (the editor's link pattern, [`md_link_matches`]).
#[derive(Default)]
struct TextBlocks {
    /// The text blocks the walk is inside, innermost last.
    open: Vec<TextBlock>,
    /// How many links and images the walk is inside.
    link_depth: u32,
    /// The wikilinks found, in the order their blocks ended.
    wikis: Vec<Wiki>,
    /// The links found, likewise.
    found: Vec<WalkLink>,
    /// Bytes of the body read for them: each block reads its own text only.
    #[cfg(test)]
    scanned: usize,
}

impl TextBlocks {
    /// Takes one event (`range` in `body`, the walked text, which starts at
    /// byte `body_start` of the note); a block it ends is looked through.
    fn step(&mut self, event: &Event, range: &Range<usize>, body: &str, body_start: usize) {
        match event {
            Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough) => {}
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => {
                if self.link_depth == 0 {
                    if let Some(block) = self.open.last_mut() {
                        block.links.push(range.clone());
                    }
                }
                self.link_depth += 1;
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                self.link_depth = self.link_depth.saturating_sub(1);
            }
            Event::Start(tag) => {
                if let Some(block) = self.open.last_mut() {
                    block.children.push(range.clone());
                }
                let code = matches!(tag, Tag::CodeBlock(CodeBlockKind::Indented));
                if code || matches!(tag, Tag::Paragraph | Tag::Heading { .. } | Tag::Item) {
                    // A block's start spans the whole block.
                    self.open.push(TextBlock {
                        range: range.clone(),
                        code,
                        children: Vec::new(),
                        links: Vec::new(),
                        opaque: Vec::new(),
                    });
                }
            }
            Event::Code(_) | Event::InlineHtml(_) => {
                if let Some(block) = self.open.last_mut() {
                    block.opaque.push(range.clone());
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => {
                if let Some(block) = self.open.pop() {
                    self.end(&block, body, body_start);
                }
            }
            // Only an indented one was opened.
            Event::End(TagEnd::CodeBlock) if self.open.last().is_some_and(|b| b.code) => {
                if let Some(block) = self.open.pop() {
                    self.end(&block, body, body_start);
                }
            }
            _ => {}
        }
    }

    /// A text block ended: its own text — the stretches between its child
    /// blocks — is looked through (see [`Self::scan`]).
    fn end(&mut self, block: &TextBlock, body: &str, body_start: usize) {
        let mut at = block.range.start;
        let end = block.range.end..block.range.end;
        // A child's descendants lie inside it; pulldown's ranges may also
        // overlap a little (a tab-indented list starts inside the line
        // before it): what any child covers is not the block's own text.
        for child in block.children.iter().chain([&end]) {
            if at < child.start {
                self.scan(at..child.start, block, body, body_start);
            }
            at = at.max(child.end);
        }
    }

    /// One stretch of a text block's own text, `body[own]`: its wikilinks,
    /// then the markdown links and images pulldown did not read in it.
    ///
    /// A wikilink is a match of the editor's pattern whose brackets are
    /// prose: not escaped (`\[[a]]`), and neither its `[[` nor its `]]`
    /// inside a link, image, code span or inline HTML pulldown read — a
    /// wikilink in a link's text is that link's text, as in the editor; one
    /// in code or HTML is code or HTML. What pulldown read between its
    /// brackets (``[[a|b `c` d]]``, an autolink) is part of its text, not a
    /// link (see [`walk`]). `![[` is an embed unless its `!` is escaped or
    /// not prose.
    /// A match across lines whose last line holds a wikilink in prose is that
    /// wikilink, as the editor highlights it (`Type [[ then pick.⏎See
    /// [[Note]]`); otherwise the match is judged on its own
    /// (``[[a|see the⏎`[[` syntax]]`` links `a`).
    ///
    /// A found link — in practice a destination with spaces,
    /// `[David H](../People/David H.md)`, `![shot](my shot.png)` — is kept
    /// when it overlaps no link, image or wikilink, has its destination on
    /// one line and without a `(` (the editor's pattern, which ends a
    /// destination at its first `)`, cannot read one reliably), holds a
    /// code span or inline HTML only in its text, and is not escaped
    /// (`\[x](a b.md)`). A hashtag inside one is link text, not a tag (see
    /// [`walk`]).
    ///
    /// In an indented code block only wikilinks are looked for: they are
    /// links, but the block is still code, so each is recorded as written
    /// ([`WalkLink::as_written`]) — its source stays in the lines and in the
    /// rendered markdown.
    fn scan(&mut self, own: Range<usize>, block: &TextBlock, body: &str, body_start: usize) {
        let source = &body[own.clone()];
        #[cfg(test)]
        {
            self.scanned += source.len();
        }
        let wikis_from = self.wikis.len();
        // Something pulldown read: a link, an image, a code span, HTML.
        let read =
            |r: &Range<usize>| overlaps_any(&block.links, r) || overlaps_any(&block.opaque, r);
        // Whether a match's brackets are prose. A link pulldown read as
        // exactly the match's inner `[…]` — a shortcut reference link,
        // `[[docs]]` with `[docs]: …` defined — does not count: the wikilink
        // wins, and that link is its text.
        let prose = |m: &Range<usize>| {
            let open = own.start + m.start..own.start + m.start + "[[".len();
            let close = own.start + m.end - "]]".len()..own.start + m.end;
            let inner = open.start + 1..close.end - 1;
            let inner_link = block.links.binary_search_by_key(&inner.start, |l| l.start);
            let inner_link = inner_link.is_ok_and(|i| block.links[i] == inner);
            !escaped(&source[..m.start]) && (inner_link || !read(&open) && !read(&close))
        };
        if source.contains("[[") {
            for m in wikilink_matches(source) {
                // The editor reads line by line: the last line of a match
                // across lines may hold a wikilink of its own.
                let last_line = source[m.clone()].rfind('\n').map(|at| m.start + at + 1);
                let own_line = last_line.and_then(|line| {
                    let inner = wikilink_matches(&source[line..m.end]).next()?;
                    Some(line + inner.start..line + inner.end)
                });
                let m = own_line.filter(prose).unwrap_or(m);
                if !prose(&m) {
                    continue;
                }
                let open = own.start + m.start..own.start + m.start + "[[".len();
                let close = own.start + m.end - "]]".len()..own.start + m.end;
                let embed = source[..m.start].ends_with('!')
                    && !escaped(&source[..m.start - 1])
                    && !read(&(open.start - 1..open.start));
                let range = open.start - usize::from(embed)..close.end;
                if let Some((mut link, display)) = wikilink(body, range.clone(), body_start) {
                    link.as_written = block.code;
                    self.wikis.push(Wiki {
                        range,
                        display,
                        link,
                    });
                }
            }
        }
        if block.code || !source.contains("](") {
            return;
        }
        let wikis: Vec<Range<usize>> = (self.wikis[wikis_from..].iter())
            .map(|w| w.range.clone())
            .collect();
        // Only a `](` outside every parsed link can end a link to find (a
        // parsed link's own `](` is the common case).
        let open = |(i, _): (usize, &str)| {
            let at = own.start + i;
            !overlaps_any(&block.links, &(at..at + 1))
        };
        if !source.match_indices("](").any(open) {
            return;
        }
        for m in md_link_matches(source) {
            let at = own.start + m.range.start..own.start + m.range.end;
            let label = own.start + m.label.start..own.start + m.label.end;
            let first = block.opaque.partition_point(|o| o.end <= at.start);
            let code_outside_label = block.opaque[first..]
                .iter()
                .take_while(|o| o.start < at.end)
                .any(|o| !(label.start <= o.start && o.end <= label.end));
            // A label may wrap; a destination may not (nor in pulldown). A
            // spaced destination holding a `(` — `[n](My Notes (draft).md)`,
            // `[n](a\(b c.md)` — the pattern cannot read reliably.
            if escaped(&source[..m.range.start])
                || m.target.contains(['\n', '('])
                || overlaps_any(&block.links, &at)
                || overlaps_any(&wikis, &at)
                || code_outside_label
            {
                continue;
            }
            self.found.push(WalkLink {
                kind: if m.image {
                    WalkLinkKind::Image
                } else {
                    WalkLinkKind::Found
                },
                target: m.target.to_string(),
                label: body[label].to_string(),
                range: body_start + at.start..body_start + at.end,
                as_written: false,
            });
        }
    }
}

/// A code span's text as pulldown gives it for the same span in LF, from
/// its text `code` in a CRLF note and its source `src` (backticks
/// included); `None` when there is nothing to change. Pulldown turns a
/// line ending in a code span into a space and skips the next line's
/// container prefix (indentation, `>` markers), but reads a CRLF as two
/// line endings: two spaces. Its text is rebuilt line by line from the
/// source: each line after the first is the end of its source line, past
/// the prefix pulldown skipped — the `[ \t>]` run its text starts with
/// tells how much of the line's own run is left. Then one space is cut at
/// each edge, as pulldown does, when both edges are spaces.
fn crlf_code_span(code: &str, src: &str) -> Option<String> {
    let ticks = src.len() - src.trim_start_matches('`').len();
    let inner = src.get(ticks..src.len().checked_sub(ticks)?)?;
    if !inner.contains("\r\n") {
        return None;
    }
    let is_space = |b: Option<u8>| matches!(b, Some(b' ' | b'\r' | b'\n'));
    // Whether pulldown cut a space from each edge, as the source's edges
    // tell — unless its last line is only a container prefix pulldown
    // skipped (`>`): then the other reading is tried too, and only one that
    // rebuilds every line from its source is kept.
    let all_spaces = code.bytes().all(|b| b == b' ');
    let cut = is_space(inner.bytes().next()) && is_space(inner.bytes().last()) && !all_spaces;
    rebuild_code_span(code, inner, cut)
        .or_else(|| rebuild_code_span(code, inner, !cut && !all_spaces))
}

/// [`crlf_code_span`]'s rebuild of `code` (pulldown's text for a code span
/// whose source between its backticks is `inner`), given whether pulldown
/// cut a space from each edge; `None` when a line's text is not the end of
/// its source line.
fn rebuild_code_span(code: &str, inner: &str, cut: bool) -> Option<String> {
    let prefix_run = |s: &str| s.len() - s.trim_start_matches([' ', '\t', '>']).len();
    // The text before pulldown cut a space from each edge.
    let spaced = if cut {
        format!(" {code} ")
    } else {
        code.to_string()
    };
    let mut out = String::with_capacity(spaced.len());
    let mut at = 0;
    let lines: Vec<&str> = inner.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let crlf = line.ends_with('\r');
        let text = line.strip_suffix('\r').unwrap_or(line);
        let len = if i == 0 {
            text.len()
        } else if i + 1 == lines.len() {
            spaced.len().checked_sub(at)?
        } else {
            // What is left of the line's own `[ \t>]` run, then the rest.
            let left = prefix_run(spaced.get(at..)?).min(prefix_run(text));
            text.len() - prefix_run(text) + left
        };
        let kept = spaced.get(at..at + len)?;
        if !text.ends_with(kept) {
            return None;
        }
        out.push_str(kept);
        at += len;
        if i + 1 < lines.len() {
            // Its line ending: one space, two for a CRLF.
            let ending = if crlf { "  " } else { " " };
            if spaced.get(at..at + ending.len())? != ending {
                return None;
            }
            out.push(' ');
            at += ending.len();
        }
    }
    if out.len() > 1
        && out.starts_with(' ')
        && out.ends_with(' ')
        && !out.bytes().all(|b| b == b' ')
    {
        out = out[1..out.len() - 1].to_string();
    }
    Some(out)
}

/// `text` with its CRLF line endings as LF: text read from the note as
/// written reads the same in a CRLF note.
fn lf(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        text.replace("\r\n", "\n").into()
    } else {
        text.into()
    }
}

/// The parse of `body`, the one place the walk reads it: plain CommonMark.
/// Pulldown-cmark's wikilink extension is deliberately not used (in 0.13.4
/// it panics on some inputs, re-emits events exponentially after `[[a|]]`
/// and garbles linked embeds): wikilinks are Kimün's own, found per text
/// block with the editor's pattern (see [`TextBlocks::scan`]).
///
/// A CRLF note gives every event the same note in LF gives. Pulldown drops a
/// CRLF's `\r` almost everywhere; what is left: it splits an HTML line from
/// its line ending (`Html("<div>")`, `Html("\n")`, joined back here) and
/// keeps it in inline HTML over lines (and in the odd text event); a code
/// span over a CRLF it reads as two line endings (see [`crlf_code_span`]).
fn events(body: &str) -> impl Iterator<Item = (Event<'_>, Range<usize>)> {
    let mut parser = Parser::new_ext(body, Options::empty())
        .into_offset_iter()
        .peekable();
    let crlf = body.contains('\r');
    std::iter::from_fn(move || {
        let (event, range) = parser.next()?;
        Some(match event {
            Event::Html(html) if !html.ends_with('\n') => {
                match parser.next_if(|(e, _)| matches!(e, Event::Html(t) if t.as_ref() == "\n")) {
                    Some((_, end)) => (
                        Event::Html(format!("{html}\n").into()),
                        range.start..end.end,
                    ),
                    None => (Event::Html(html), range),
                }
            }
            Event::InlineHtml(html) if crlf && html.contains('\r') => {
                (Event::InlineHtml(lf(&html).into_owned().into()), range)
            }
            Event::Code(code) if crlf && body[range.clone()].contains('\r') => {
                let code = crlf_code_span(&code, &body[range.clone()]).map_or(code, Into::into);
                (Event::Code(code), range)
            }
            Event::Text(text) if crlf && text.contains('\r') => {
                (Event::Text(lf(&text).into_owned().into()), range)
            }
            other => (other, range),
        })
    })
}

/// `body[range]` as pulldown gives text over lines: a text event per line,
/// each after the first without its container prefix (indentation,
/// blockquote `>` markers) and none with its `\r`, a soft break between (so
/// a heading ends at the first): how the walk shows a wikilink's display
/// text, as written.
fn source_text(body: &str, range: Range<usize>) -> Vec<(Event<'_>, Range<usize>)> {
    let mut out = Vec::new();
    let mut at = range.start;
    for (i, line) in body[range].split('\n').enumerate() {
        let line_end = at + line.len();
        let text = line.strip_suffix('\r').unwrap_or(line);
        let end = at + text.len();
        let start = match i {
            0 => at,
            _ => end - text.trim_start_matches([' ', '\t', '>']).len(),
        };
        if i > 0 {
            out.push((Event::SoftBreak, at - 1..at));
        }
        if start < end {
            out.push((Event::Text(body[start..end].into()), start..end));
        }
        at = line_end + 1;
    }
    out
}

/// One pass over `note` as written — see the module docs above.
///
/// Pulldown's events are read twice: first per text block for what it does
/// not read (wikilinks, links with spaces in their destination — see
/// [`TextBlocks`]), then into the lines, where a wikilink's source is
/// replaced by its display text: pulldown splits text at every bracket, so
/// a wikilink's source is whole events — its text, and code, HTML or line
/// breaks inside it — dropped, its display text shown once in their place
/// (a text event straddling one is cut at its edges); the starts and ends
/// of emphasis or an autolink inside it pass, so the lines stay well formed.
pub(in crate::note) fn walk(note: &str) -> NoteWalk {
    let (body_start, frontmatter) = body_start(note);
    let body = &note[body_start..];
    let candidates: Vec<(Range<usize>, &str)> = label_matches_inner(body)
        .map(|m| (m.byte_start..m.byte_end, m.name))
        .collect();
    let events: Vec<(Event, Range<usize>)> = events(body).collect();

    let mut blocks = TextBlocks::default();
    // No `[[` or `](` in the body: nothing for the blocks to find.
    if body.contains("[[") || body.contains("](") {
        for (event, range) in &events {
            blocks.step(event, range, body, body_start);
        }
    }
    let TextBlocks {
        mut wikis,
        mut found,
        ..
    } = blocks;
    // In document order (a nested block ends before the one around it),
    // each found once.
    wikis.sort_by_key(|w| w.range.start);
    wikis.dedup_by(|w, before| w.range.start < before.range.end);
    found.sort_by_key(|l| l.range.start);
    found.dedup_by(|l, before| l.range.start < before.range.end);
    // A hashtag in a found link's text is link text (see `TextBlocks::scan`).
    let found_ranges: Vec<Range<usize>> = found.iter().map(|l| l.range.clone()).collect();
    let sort_links = !wikis.is_empty() || !found.is_empty();
    let mut links: Vec<WalkLink> = wikis.iter().map(|w| w.link.clone()).collect();
    links.append(&mut found);
    // A wikilink in an indented code block is a link, but its text stays
    // code, as written (see `TextBlocks::scan`).
    wikis.retain(|w| !w.link.as_written);

    let mut lines = TextLines::default();
    let mut tags = Vec::new();
    let mut code_depth = 0u32;
    let mut recorder = LinkRecorder::new(body, body_start);
    // The first wikilink not passed yet, and how many have been shown.
    let (mut next, mut shown) = (0, 0);
    for (event, range) in events {
        let in_link = recorder.step(&event, &range, &mut links);
        let is_text = matches!(event, Event::Text(_));
        let is_leaf = matches!(
            event,
            Event::Code(_) | Event::InlineHtml(_) | Event::SoftBreak | Event::HardBreak
        );
        if is_text || is_leaf {
            while wikis.get(next).is_some_and(|w| w.range.end <= range.start) {
                next += 1;
            }
            if wikis.get(next).is_some_and(|w| w.range.start < range.end) {
                let mut at = range.start;
                let mut prose = |piece: Range<usize>, lines: &mut TextLines| {
                    let text: CowStr = lf(&body[piece.clone()]).into_owned().into();
                    lines.push(Event::Text(text), body_start + piece.start, |decoded| {
                        prose_text(
                            decoded,
                            piece,
                            body,
                            body_start,
                            &candidates,
                            in_link,
                            &mut tags,
                        )
                    });
                };
                while let Some(w) = wikis.get(next).filter(|w| w.range.start < range.end) {
                    if is_text && at < w.range.start {
                        prose(at..w.range.start, &mut lines);
                    }
                    if shown <= next {
                        lines.display("");
                        for (text, _) in source_text(body, w.display.clone()) {
                            match text {
                                Event::Text(text) => lines.display(&text),
                                other => lines.push(other, 0, str::to_string),
                            }
                        }
                        shown = next + 1;
                    }
                    at = at.max(w.range.end);
                    if w.range.end > range.end {
                        break;
                    }
                    next += 1;
                }
                if is_text && at < range.end {
                    prose(at..range.end, &mut lines);
                }
                continue;
            }
        }
        match &event {
            Event::Start(Tag::CodeBlock(_)) => code_depth += 1,
            Event::End(TagEnd::CodeBlock) => code_depth = code_depth.saturating_sub(1),
            _ => {}
        }
        let in_code = code_depth > 0;
        lines.push(event, body_start + range.start, |decoded| {
            if in_code {
                decoded.to_string()
            } else {
                prose_text(
                    decoded,
                    range.clone(),
                    body,
                    body_start,
                    &candidates,
                    in_link,
                    &mut tags,
                )
            }
        });
    }
    if !found_ranges.is_empty() {
        tags.retain(|t| {
            let i = found_ranges.partition_point(|r| r.end <= t.range.start);
            !found_ranges
                .get(i)
                .is_some_and(|r| r.start <= t.range.start && t.range.end <= r.end)
        });
    }
    // What pulldown read inside a wikilink — an autolink — is its text, not
    // a link (see `TextBlocks::scan`).
    if !wikis.is_empty() {
        links.retain(|l| {
            let i = wikis.partition_point(|w| w.link.range.end <= l.range.start);
            matches!(l.kind, WalkLinkKind::Wiki | WalkLinkKind::WikiEmbed)
                || !wikis
                    .get(i)
                    .is_some_and(|w| w.link.range.start <= l.range.start)
        });
    }
    // The blocks' links come first: back in document order.
    if sort_links {
        links.sort_by_key(|l| l.range.start);
    }

    NoteWalk {
        lines: lines.finish(),
        links,
        tags,
        frontmatter: frontmatter.map_or_else(String::new, |r| frontmatter_text(&note[r])),
    }
}

/// Bytes of `note`'s body its text blocks read for what pulldown does not
/// read.
#[cfg(test)]
fn scanned_bytes(note: &str) -> usize {
    let body = &note[body_start(note).0..];
    let mut blocks = TextBlocks::default();
    for (event, range) in events(body) {
        blocks.step(&event, &range, body, 0);
    }
    blocks.scanned
}

/// `note` with its links to `from` pointed at `to`, for a rename: the links
/// the walk lists — the ones the index records — read as the index reads
/// them, each rewritten in place, the rest of the note copied verbatim.
///
/// - A wikilink or embed whose note is named like `from` gets `to`'s name;
///   its padding, `#section`/`^block` and display text stay as written.
/// - A markdown link (and a reference definition) whose destination, its
///   `#section` cut, resolves to `from` — against `at`, where the note was
///   when it was indexed; a bare name by name — gets `to` written the same
///   way: a bare name as `to`'s name, a path in `from`'s folder with just
///   its name changed, an absolute path as `to`, any other relative to where
///   the note lives after the rename; its `#section` kept. Wrapped in `<…>`
///   when it would not read back as a link otherwise.
///
/// Left alone: images (not note links), a destination not written as
/// pulldown read it (escapes, entities), and anything that is no link in
/// the walk — code, HTML, the frontmatter.
pub(in crate::note) fn retarget_links(
    note: &str,
    at: &VaultPath,
    from: &VaultPath,
    to: &VaultPath,
) -> String {
    let walked = walk(note);
    let old_name = from.get_name();
    let from = from.canonical();
    // A self-link is written from where the note now lives.
    let home = if at.canonical() == from { to } else { at };
    // A markdown destination written as `dest` (its bytes at `span` of
    // `note`) that resolves to `from`: its edit. `spaced` — whether a bare
    // destination with whitespace still reads as the link.
    let retarget = |dest: &str, span: Range<usize>, spaced: bool| {
        if is_remote_url(dest) {
            return None;
        }
        let (written, fragment) = md_vault_path(dest)?;
        let path = VaultPath::new(written).resolve_against_note(at);
        let points_at_from = if path.is_note_file() {
            path.get_name() == old_name
        } else {
            path.canonical() == from
        };
        if !points_at_from {
            return None;
        }
        let new = if VaultPath::new(written).is_note_file() {
            to.get_name()
        } else if from.get_parent_path().0 == to.canonical().get_parent_path().0 {
            let folder = written.rfind(PATH_SEPARATOR).map_or(0, |at| at + 1);
            format!("{}{}", &written[..folder], to.get_name())
        } else if written.starts_with(PATH_SEPARATOR) {
            to.canonical().to_string()
        } else {
            to.relative_link_from_note(home).to_string()
        };
        let wrapped = note[..span.start].ends_with('<');
        let unreadable = |c: char| c == '(' || c == ')' || !spaced && c.is_whitespace();
        if !wrapped && new.contains(unreadable) {
            return Some((span, format!("<{new}{fragment}>")));
        }
        let start = span.start + (written.as_ptr() as usize - dest.as_ptr() as usize);
        Some((start..start + written.len(), new))
    };
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for link in walked.listed_links() {
        match link.kind {
            WalkLinkKind::Wiki | WalkLinkKind::WikiEmbed => {
                let (name, _) = link.wiki_note();
                if VaultPath::note_path_from(name).get_name() != old_name {
                    continue;
                }
                let opener = if link.kind == WalkLinkKind::WikiEmbed {
                    "![["
                } else {
                    "[["
                };
                let lead = link.target.len() - link.target.trim_start_matches([' ', '\t']).len();
                let start = link.range.start + opener.len() + lead;
                edits.push((start..start + name.len(), to.get_clean_name()));
            }
            WalkLinkKind::Inline | WalkLinkKind::Found => {
                // The destination as written: the target after the label's `](`.
                let after_label = link.range.start + "[".len() + link.label.len();
                let Some(src) = note.get(after_label..link.range.end) else {
                    continue;
                };
                let Some(open) = src.find("](").map(|at| at + "](".len()) else {
                    continue;
                };
                let Some(at) = src[open..].find(link.target.as_str()) else {
                    continue;
                };
                let start = after_label + open + at;
                // A title (`as_written` without `<…>`) ends a spaced one.
                let span = start..start + link.target.len();
                edits.extend(retarget(&link.target, span, !link.as_written));
            }
            WalkLinkKind::Reference | WalkLinkKind::Autolink | WalkLinkKind::Image => {}
        }
    }
    // A reference link's destination is its definition's.
    let body_start = body_start(note).0;
    let parser = Parser::new_ext(&note[body_start..], Options::empty());
    for (_, def) in parser.reference_definitions().iter() {
        let span = body_start + def.span.start..body_start + def.span.end;
        let Some(colon) = note[span.clone()].find("]:") else {
            continue;
        };
        let from_colon = span.start + colon + "]:".len();
        let Some(at) = note[from_colon..span.end].find(def.dest.as_ref()) else {
            continue;
        };
        let start = from_colon + at;
        edits.extend(retarget(&def.dest, start..start + def.dest.len(), false));
    }
    splice(note, edits)
}

/// `note` with each `(range, replacement)` edit applied, in order of range.
/// Defensive: the walk never nests a recorded range in another (a link
/// cannot contain a link), but an overlapping edit is skipped rather than
/// slicing backwards.
fn splice(note: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(note.len());
    let mut last = 0;
    for (range, replacement) in edits {
        if range.start < last {
            continue;
        }
        out.push_str(&note[last..range.start]);
        out.push_str(&replacement);
        last = range.end;
    }
    out.push_str(&note[last..]);
    out
}

/// Where a markdown link destination points, as the rendered markdown
/// writes it, plus the link it records: a URL, a note, or another vault
/// path (relative destinations resolve against `ref_path`'s directory). A
/// vault path's `#section` is cut for resolving (`split_link_fragment`, as
/// the editor follows it) and written back after the resolved path. `None`
/// for a destination that is neither a URL nor a valid vault path.
pub(in crate::note) fn resolve_md_link(
    dest: &str,
    label: &str,
    ref_path: &VaultPath,
) -> (String, Option<NoteLink>) {
    if is_remote_url(dest) {
        return (dest.to_string(), Some(NoteLink::url(dest, label)));
    }
    let Some((written, fragment)) = md_vault_path(dest) else {
        return (dest.to_string(), None);
    };
    let path = VaultPath::new(written).resolve_against_note(ref_path);
    (
        format!("{path}{fragment}"),
        Some(NoteLink::vault_path(&path, label)),
    )
}

/// A markdown destination that is not a URL, split into the vault path it
/// points to and its `#section` (`split_link_fragment`, as the editor follows
/// it); `None` when that path is empty or not a valid vault path.
fn md_vault_path(dest: &str) -> Option<(&str, &str)> {
    let (path, fragment) = split_link_fragment(dest);
    (!path.is_empty() && VaultPath::is_valid(path)).then_some((path, fragment))
}

/// An embed as a markdown image for the image pipeline: a target that looks
/// like an image as written (the pipeline resolves it against the note's
/// folder), any other its note path, unresolved; a fragment is dropped (an
/// image shows the whole file). Left bare, not `<…>`-wrapped: the image
/// pipeline reads the destination as written, and a vault path holds no
/// `<`, `>` or line break.
fn embed_image(link: &WalkLink) -> String {
    let (target, _) = link.wiki_note();
    let dest = if target_looks_like_image(target) {
        target.to_string()
    } else {
        VaultPath::note_path_from(target).to_string()
    };
    format!("![{}]({dest})", link_text(&link.label))
}

/// A wikilink's text, written as markdown link text that reads back as
/// itself: `\\`, `[` and `]` escaped, so a bracket cannot end or open the
/// link (`[[a|b[c]]` renders `[b\[c](a.md)`), and so is the `<` of an
/// autolink in it (`[[a|<https://x.y>]]` renders `[\<https://x.y>](a.md)`):
/// a link cannot hold a link, and the wikilink's text is no link.
fn link_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    if out.contains('<') {
        // Read as the link text it becomes: code keeps its `<`.
        let src = format!("[{out}]()");
        let autolinks: Vec<usize> = Parser::new_ext(&src, Options::empty())
            .into_offset_iter()
            .filter_map(|(event, range)| match event {
                Event::Start(Tag::Link {
                    link_type: LinkType::Autolink | LinkType::Email,
                    ..
                }) => Some(range.start - "[".len()),
                _ => None,
            })
            .collect();
        for at in autolinks.into_iter().rev() {
            out.insert(at, '\\');
        }
    }
    out
}

/// A wikilink's `#section` / `^block` as a URL fragment: a section as
/// written, a block `^id` as `#^id`.
fn url_fragment(fragment: &str) -> String {
    if fragment.starts_with('^') {
        format!("#{fragment}")
    } else {
        fragment.to_string()
    }
}

/// `dest` as a markdown link destination that reads back as itself: bare
/// without whitespace, `<`, `>` or parentheses; wrapped in `<…>` when it
/// holds whitespace or parentheses (`plan.md#My Goals`, `note (draft).md`)
/// but no `<`, `>` or line break; otherwise `None` — no form reads back as
/// `dest`, so the link is left as written (still listed). A backslash is
/// doubled in either form, so it is not read as an escape.
fn markdown_destination(dest: String) -> Option<String> {
    let needs_wrap = |c: char| c.is_whitespace() || c == '(' || c == ')';
    let escaped = dest.replace('\\', "\\\\");
    if !dest.contains(['<', '>']) && !dest.contains(needs_wrap) {
        Some(escaped)
    } else if !dest.contains(['<', '>', '\n', '\r']) {
        Some(format!("<{escaped}>"))
    } else {
        None
    }
}

impl NoteWalk {
    /// Heading-chunked content, plus the `FrontMatter` chunk. Consumes the
    /// walk so its lines are not copied; take any other view first.
    pub(in crate::note) fn into_chunks(self) -> Vec<ContentChunk> {
        let mut chunks = chunks_from_text_lines(self.lines);
        if !self.frontmatter.is_empty() {
            chunks.push(ContentChunk {
                breadcrumb: "FrontMatter".to_string(),
                text: self.frontmatter,
            });
        }
        chunks
    }

    /// The first non-empty line, trimmed as a line's text is (see
    /// [`pulldown_whitespace`]): a heading's text, a paragraph's, or a list
    /// item's first line.
    pub(in crate::note) fn title(&self) -> String {
        self.lines
            .iter()
            .find_map(|line| match line {
                TextLine::Empty => None,
                TextLine::Header(_, text, _) => Some(text.as_str()),
                TextLine::Text(text) => Some(text.as_str()),
                // A wrapped item names the note by its first line only.
                TextLine::ListItem(_, text) => text.lines().next(),
            })
            .map(|text| text.trim_matches(pulldown_whitespace).to_owned())
            .unwrap_or_default()
    }

    /// Every non-empty heading, with the row of `note` (the text this walk
    /// read) it starts on.
    pub(in crate::note) fn headings(&self, note: &str) -> Vec<NoteHeading> {
        let breaks: Vec<usize> = note.match_indices('\n').map(|(at, _)| at).collect();
        self.lines
            .iter()
            .filter_map(|line| match line {
                TextLine::Header(level, text, start) if !text.is_empty() => Some(NoteHeading {
                    level: *level,
                    text: text.clone(),
                    line: breaks.partition_point(|&at| at < *start),
                }),
                _ => None,
            })
            .collect()
    }

    /// The links every view lists, in document order.
    fn listed_links(&self) -> impl Iterator<Item = &WalkLink> {
        self.links.iter().filter(|link| link.is_link())
    }

    /// Every link target in document order, as written — wikilink targets
    /// whose note is a valid vault path (fragment kept), markdown and image
    /// destinations that are URLs or valid vault paths, autolinks (see
    /// [`WalkLink::is_link`]). Strings, not resolved paths.
    pub(in crate::note) fn link_targets(&self) -> Vec<String> {
        self.listed_links().map(|l| l.target.clone()).collect()
    }

    /// The links the index records, in document order: wikilinks to valid
    /// vault paths, markdown links that resolve to a note, and hashtags.
    pub(in crate::note) fn index_links(&self, ref_path: &VaultPath) -> Vec<NoteLink> {
        let mut found: Vec<(usize, NoteLink)> = Vec::new();
        for link in self.listed_links() {
            match link.kind {
                WalkLinkKind::Wiki | WalkLinkKind::WikiEmbed => {
                    let path = VaultPath::note_path_from(link.wiki_note().0);
                    found.push((link.range.start, NoteLink::note(&path, lf(&link.label))));
                }
                WalkLinkKind::Inline
                | WalkLinkKind::Found
                | WalkLinkKind::Reference
                | WalkLinkKind::Autolink => {
                    // The index stores no link text; skip the allocation.
                    if let (_, Some(note)) = resolve_md_link(&link.target, "", ref_path) {
                        if matches!(note.ltype, NoteLinkType::Note(_)) {
                            found.push((link.range.start, note));
                        }
                    }
                }
                WalkLinkKind::Image => {}
            }
        }
        found.extend(
            self.tags
                .iter()
                .map(|tag| (tag.range.start, NoteLink::hashtag(&tag.name))),
        );
        found.sort_by_key(|(at, _)| *at);
        found.into_iter().map(|(_, link)| link).collect()
    }

    /// The note rewritten for a renderer, plus its links: valid wikilinks
    /// become `[text](note path#fragment)`, a path target resolved against
    /// the note's folder as the editor follows it, a `#section` kept after it
    /// (`^block` as `#^block`), an empty display part shown as the target as
    /// written; an embed becomes an image left to the image pipeline
    /// (see `embed_image`), not listed; inline links get their destination
    /// resolved; hashtags become `[#tag](#tag)`. Every rewritten destination
    /// is written by `markdown_destination`. Reference links, autolinks,
    /// inline links written with a `<…>` destination or a title (see
    /// `WalkLink::rewritten_destination`), wikilinks in an indented code
    /// block (code, see `WalkLink::as_written`), and links whose destination
    /// cannot be written back are listed but left as written. An inline
    /// link, or one found with the editor's pattern (`WalkLinkKind::Found`),
    /// whose destination resolves nowhere (`[x](a b.md "T")`) is left as
    /// written and not listed. Links come in document order, hashtags after
    /// them. Everything outside a recorded range — frontmatter, code, HTML,
    /// images — is copied verbatim.
    pub(in crate::note) fn render_markdown(
        &self,
        note: &str,
        ref_path: &VaultPath,
    ) -> (String, Vec<NoteLink>) {
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut links = Vec::new();
        for link in self.listed_links() {
            match link.kind {
                WalkLinkKind::WikiEmbed if link.as_written => {}
                WalkLinkKind::WikiEmbed => {
                    edits.push((link.range.clone(), embed_image(link)));
                }
                WalkLinkKind::Wiki => {
                    let (target, fragment) = link.wiki_note();
                    let path = VaultPath::note_path_from(target).resolve_against_note(ref_path);
                    links.push(NoteLink::note(&path, lf(&link.label)));
                    let dest = markdown_destination(format!("{path}{}", url_fragment(fragment)));
                    if let Some(dest) = dest.filter(|_| !link.as_written) {
                        // An empty display part would render an invisible link.
                        let text = if link.label.is_empty() {
                            &link.target
                        } else {
                            &link.label
                        };
                        let text = link_text(text);
                        edits.push((link.range.clone(), format!("[{text}]({dest})")));
                    }
                }
                WalkLinkKind::Inline | WalkLinkKind::Found => {
                    // Written back only as the link it was read as: a
                    // destination that links nowhere (`[x](a b.md "T")`,
                    // `[t]([u](u))`, a URL with a space) stays as written.
                    let (dest, found) = resolve_md_link(&link.target, &lf(&link.label), ref_path);
                    if let Some(found) = found {
                        links.push(found);
                        if let Some(dest) = link.rewritten_destination(dest) {
                            edits.push((link.range.clone(), format!("[{}]({dest})", link.label)));
                        }
                    }
                }
                WalkLinkKind::Reference | WalkLinkKind::Autolink => {
                    links.extend(resolve_md_link(&link.target, &lf(&link.label), ref_path).1);
                }
                WalkLinkKind::Image => {}
            }
        }
        for tag in &self.tags {
            links.push(NoteLink::hashtag(&tag.name));
            edits.push((tag.range.clone(), format!("[#{0}](#{0})", tag.name)));
        }
        (splice(note, edits), links)
    }

    /// Hashtag names, lowercased, sorted and distinct.
    pub(in crate::note) fn tag_names(&self) -> Vec<String> {
        let names: std::collections::BTreeSet<String> =
            self.tags.iter().map(|t| t.name.to_lowercase()).collect();
        names.into_iter().collect()
    }
}

#[derive(Debug, Default, Clone)]
pub(in crate::note) enum TextLine {
    #[default]
    Empty,
    /// Level, text, and the byte the heading starts at in the walked text —
    /// kept on the line itself so a heading can never part from its place.
    Header(u8, String, usize),
    Text(String),
    ListItem(u8, String),
}

impl TextLine {
    /// The line with `text` added to its end, in place: only `text` is
    /// copied, so a long line grows in linear time.
    pub(in crate::note) fn append_text(mut self, text: String) -> TextLine {
        match &mut self {
            TextLine::Empty => return TextLine::Text(text),
            TextLine::Header(_, line, _) | TextLine::Text(line) | TextLine::ListItem(_, line) => {
                line.push_str(&text)
            }
        }
        self
    }

    pub(in crate::note) fn to_text(&self) -> String {
        match self {
            TextLine::Empty => String::new(),
            TextLine::Header(level, text, _) => {
                format!("{} {}", "#".repeat(*level as usize), text)
            }
            TextLine::Text(text) => text.to_owned(),
            TextLine::ListItem(level, text) => {
                let text = text.replace('\n', " ");
                format!("{}* {}", " ".repeat((*level as usize) * 4), text)
            }
        }
    }

    /// The line with whitespace (`is_whitespace`) cut from both ends of its
    /// text, in place: a heading's text once it ends, a code block's.
    pub(in crate::note) fn trim(mut self, is_whitespace: fn(char) -> bool) -> Self {
        if let TextLine::Header(_, text, _) | TextLine::Text(text) | TextLine::ListItem(_, text) =
            &mut self
        {
            let end = text.trim_end_matches(is_whitespace).len();
            text.truncate(end);
            let start = end - text.trim_start_matches(is_whitespace).len();
            text.drain(..start);
        }
        self
    }

    /// The line with the whitespace pulldown trims ([`pulldown_whitespace`])
    /// cut from both ends of its last line of text (a list item holds one
    /// per line of its text).
    fn trim_last_line(mut self) -> Self {
        if let TextLine::Header(_, text, _) | TextLine::Text(text) | TextLine::ListItem(_, text) =
            &mut self
        {
            let from = text.rfind('\n').map_or(0, |at| at + 1);
            let end = text.trim_end_matches(pulldown_whitespace).len().max(from);
            text.truncate(end);
            let lead = end - from - text[from..].trim_start_matches(pulldown_whitespace).len();
            text.drain(from..from + lead);
        }
        self
    }
}

/// The whitespace pulldown trims from a line of prose: ASCII only, so a
/// no-break space at a line's edge stays.
fn pulldown_whitespace(c: char) -> bool {
    c.is_ascii_whitespace()
}

/// Builds the [`TextLine`] sequence from markdown events, fed by [`walk`]:
/// the lines behind titles, headings and chunks.
#[derive(Default)]
pub(in crate::note) struct TextLines<'a> {
    lines: Vec<TextLine>,
    tag_stack: Vec<Tag<'a>>,
    /// The line a wikilink's display text was added to, until it ends.
    display: Option<usize>,
}

impl<'a> TextLines<'a> {
    /// Feeds one event, which starts at byte `start` of the walked text. A
    /// text event's content is first passed through `text` (the walk strips
    /// hashtags there).
    pub(in crate::note) fn push(
        &mut self,
        event: Event<'a>,
        start: usize,
        text: impl FnOnce(&str) -> String,
    ) {
        match event {
            Event::Start(tag) => {
                let current_line = self.lines.pop().unwrap_or_default();
                self.lines.extend(parse_tag(&tag, current_line, start));
                self.tag_stack.push(tag);
            }
            Event::End(tag_end) => {
                let Some(start_tag) = self.tag_stack.pop() else {
                    debug!("Non matching tag end (empty stack): {:?}", tag_end);
                    return;
                };
                if tag_end != start_tag.to_end() {
                    debug!(
                        "Non matching tags: expected {:?}, got {:?}",
                        start_tag.to_end(),
                        tag_end
                    );
                    self.tag_stack.push(start_tag);
                    return;
                }
                if tag_end == TagEnd::Item {
                    self.end_display_line();
                }
                let current_line = self.lines.pop().unwrap_or_default();
                self.lines.extend(parse_tag_end(&tag_end, current_line));
            }
            Event::Text(cow_str) => {
                let last_text = self.lines.pop().unwrap_or_default();
                self.lines.push(last_text.append_text(text(&cow_str)));
            }
            Event::Code(cow_str) => {
                let current_line = self.lines.pop().unwrap_or_default();
                self.lines
                    .push(current_line.append_text(format!("`{}`", cow_str)));
            }
            Event::InlineHtml(cow_str) => {
                // Inline HTML continues the line it sits in. A heading keeps
                // only its text (`# Release <kbd>v2</kbd>` is "Release v2"),
                // so the tags are dropped there — a `<br>` as a space.
                let current_line = self.lines.pop().unwrap_or_default();
                let is_break = cow_str
                    .get(..3)
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("<br"));
                self.lines.push(match current_line {
                    TextLine::Header(..) if is_break => current_line.append_text(" ".to_string()),
                    TextLine::Header(..) => current_line,
                    other => other.append_text(cow_str.to_string()),
                });
            }
            Event::InlineMath(cow_str)
            | Event::DisplayMath(cow_str)
            | Event::Html(cow_str)
            | Event::FootnoteReference(cow_str) => {
                self.lines.push(TextLine::Text(cow_str.to_string()));
            }
            // A line break inside a list item continues the item — its text
            // and its nesting level stay together (rendered as one line, the
            // first line alone naming a note); elsewhere it ends the line.
            Event::SoftBreak | Event::HardBreak
                if matches!(self.lines.last(), Some(TextLine::ListItem(..))) =>
            {
                self.end_display_line();
                let item = self.lines.pop().unwrap_or_default();
                self.lines.push(item.append_text("\n".to_string()));
            }
            Event::SoftBreak => {
                self.lines.push(TextLine::Empty);
            }
            Event::HardBreak => {
                self.lines.push(TextLine::Empty);
                self.lines.push(TextLine::Empty);
            }
            Event::Rule => {
                self.lines.push(TextLine::Empty);
            }
            Event::TaskListMarker(result) => {
                self.lines.push(TextLine::Text(result.to_string()));
            }
        }
        // A line holding a display text ended when another follows it.
        if self.display.is_some_and(|line| line + 1 < self.lines.len()) {
            self.end_display_line();
        }
    }

    /// Adds a wikilink's display text, as written, to the current line —
    /// an empty one too: its line still ends as one holding display text.
    pub(in crate::note) fn display(&mut self, text: &str) {
        let line = self.lines.pop().unwrap_or_default();
        self.lines.push(if text.is_empty() {
            line
        } else {
            line.append_text(text.to_string())
        });
        self.display = Some(self.lines.len() - 1);
    }

    pub(in crate::note) fn finish(mut self) -> Vec<TextLine> {
        self.end_display_line();
        self.lines
    }

    /// The line holding a wikilink's display text ends: whitespace at its
    /// edges is trimmed, as pulldown trimmed it when wikilinks were collapsed
    /// into their text before the parse (`see [[a|]]` is "see"). A line
    /// without one keeps its edges as pulldown gave them.
    fn end_display_line(&mut self) {
        if let Some(line) = self.display.take() {
            if let Some(line) = self.lines.get_mut(line) {
                *line = std::mem::take(line).trim_last_line();
            }
        }
    }
}

fn parse_tag(tag: &Tag, current_line: TextLine, start: usize) -> Vec<TextLine> {
    match tag {
        Tag::Heading { level, .. } => {
            let level = match level {
                pulldown_cmark::HeadingLevel::H1 => 1,
                pulldown_cmark::HeadingLevel::H2 => 2,
                pulldown_cmark::HeadingLevel::H3 => 3,
                pulldown_cmark::HeadingLevel::H4 => 4,
                pulldown_cmark::HeadingLevel::H5 => 5,
                pulldown_cmark::HeadingLevel::H6 => 6,
            };
            vec![current_line, TextLine::Header(level, String::new(), start)]
        }
        Tag::Link { .. } => {
            // Link text arrives via Event::Text; nothing to prepend here.
            vec![current_line]
        }
        Tag::Image { .. } => {
            // Alt text arrives via Event::Text; nothing to prepend here.
            vec![current_line]
        }
        Tag::CodeBlock(kind) => {
            let open = match kind {
                pulldown_cmark::CodeBlockKind::Indented => "```".to_string(),
                pulldown_cmark::CodeBlockKind::Fenced(lang) => format!("```{}", lang),
            };
            // Keep the line before the fence — a heading directly above a
            // code block used to be dropped here, losing its breadcrumb.
            vec![current_line, TextLine::Text(open), TextLine::Empty]
        }
        Tag::List(_) => {
            let line = if let TextLine::ListItem(lvl, _) = current_line {
                TextLine::ListItem(lvl + 1, String::new())
            } else {
                TextLine::ListItem(0, String::new())
            };
            vec![current_line, line]
        }
        Tag::Item => match &current_line {
            TextLine::ListItem(lvl, text) => {
                let lvl = *lvl;
                if text.is_empty() {
                    vec![current_line]
                } else {
                    vec![current_line, TextLine::ListItem(lvl, String::new())]
                }
            }
            // Keep whatever line came before (it is not an item: dropping it
            // lost the end of a wrapped item).
            _ => vec![current_line, TextLine::ListItem(0, String::new())],
        },
        Tag::Paragraph => {
            vec![current_line, TextLine::Empty]
        }
        Tag::Strong | Tag::Emphasis | Tag::Strikethrough | Tag::Subscript | Tag::Superscript => {
            vec![current_line]
        }
        Tag::BlockQuote(_) => {
            vec![current_line]
        }
        _ => {
            vec![current_line]
        }
    }
}

fn parse_tag_end(tag_end: &TagEnd, current_line: TextLine) -> Vec<TextLine> {
    match tag_end {
        TagEnd::CodeBlock => {
            vec![
                current_line.trim(char::is_whitespace),
                TextLine::Text("```".to_string()),
            ]
        }
        TagEnd::List(_) => {
            if let TextLine::ListItem(lvl, text) = &current_line {
                let last_line = if *lvl > 0 {
                    TextLine::ListItem(lvl - 1, String::new())
                } else {
                    TextLine::Empty
                };

                if text.is_empty() {
                    vec![last_line]
                } else {
                    vec![current_line, last_line]
                }
            } else {
                vec![current_line]
            }
        }
        TagEnd::Paragraph => {
            vec![current_line, TextLine::Empty]
        }
        // A heading's text ends with it: what follows in the same block (a
        // heading inside a list item) starts a new line, not the heading's.
        // It is trimmed, like a title (`# a <kbd>` is "a").
        TagEnd::Heading(_) => {
            vec![current_line.trim(pulldown_whitespace), TextLine::Empty]
        }
        _ => {
            vec![current_line]
        }
    }
}

/// Converts a sequence of [`TextLine`] events into [`ContentChunk`]s.
/// Behind [`NoteWalk::into_chunks`], for both the plain and the indexing path.
pub(in crate::note) fn chunks_from_text_lines(lines: Vec<TextLine>) -> Vec<ContentChunk> {
    let mut content_chunks = vec![];
    let mut current_breadcrumb: Vec<(u8, String)> = vec![];
    let mut current_content = vec![];

    for text_line in lines {
        match text_line {
            TextLine::Header(level, text, _) => {
                if !current_breadcrumb.is_empty() || !current_content.is_empty() {
                    let content =
                        crate::note::diacritics::remove_diacritics(&current_content.join("\n"));
                    if !content.trim().is_empty() {
                        content_chunks.push(ContentChunk {
                            breadcrumb: join_breadcrumb(&current_breadcrumb),
                            text: content,
                        });
                    }
                }

                current_breadcrumb.retain(|(lvl, _)| *lvl < level);
                current_breadcrumb.push((level, text));
                current_content.clear();
            }
            TextLine::Empty => {}
            _ => {
                current_content.push(text_line.to_text());
            }
        }
    }

    if !current_breadcrumb.is_empty() || !current_content.is_empty() {
        let content = crate::note::diacritics::remove_diacritics(&current_content.join("\n"));
        if !content.trim().is_empty() {
            content_chunks.push(ContentChunk {
                breadcrumb: join_breadcrumb(&current_breadcrumb),
                text: content,
            });
        }
    }

    content_chunks
}

fn join_breadcrumb(stack: &[(u8, String)]) -> String {
    let mut out = String::new();
    for (i, (_, t)) in stack.iter().enumerate() {
        if i > 0 {
            out.push_str(crate::note::BREADCRUMB_SEP);
        }
        out.push_str(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nfs::VaultPath;

    fn headers(w: &NoteWalk) -> Vec<(u8, String, usize)> {
        w.lines
            .iter()
            .filter_map(|l| match l {
                TextLine::Header(level, text, start) => Some((*level, text.clone(), *start)),
                _ => None,
            })
            .collect()
    }

    fn text(w: &NoteWalk) -> String {
        w.lines
            .iter()
            .map(TextLine::to_text)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn links(w: &NoteWalk) -> Vec<(WalkLinkKind, &str, &str)> {
        w.links
            .iter()
            .map(|l| (l.kind, l.target.as_str(), l.label.as_str()))
            .collect()
    }

    #[test]
    fn offsets_are_in_the_note_as_given() {
        let w = walk("---\nt: x\n---\n# A\n");
        assert_eq!(body_start("---\nt: x\n---\n# A\n").0, 13);
        assert_eq!(headers(&w), [(1, "A".to_string(), 13)]);
        assert_eq!(w.frontmatter, "t: x");
    }

    #[test]
    fn an_unclosed_fence_drops_only_its_first_line() {
        let w = walk("---\n# A\n");
        assert_eq!(body_start("---\n# A\n").0, 4);
        assert_eq!(headers(&w), [(1, "A".to_string(), 4)]);
        assert_eq!(w.frontmatter, "");
    }

    #[test]
    fn a_byte_order_mark_is_not_body() {
        let w = walk("\u{feff}# A\n");
        assert_eq!(body_start("\u{feff}# A\n").0, 3);
        assert_eq!(headers(&w), [(1, "A".to_string(), 3)]);
    }

    #[test]
    fn crlf_is_normalized_and_offsets_stay_exact() {
        let note = "+++\r\na = 1\r\n+++\r\n# A\r\nline one\r\n```\r\ncode\r\n```\r\n[[t|Sh\r\nown]]\r\n<div>\r\na\r\n</div>\r\n";
        let w = walk(note);
        let start = body_start(note).0;
        assert_eq!(&note[start..start + 3], "# A");
        assert_eq!(headers(&w), [(1, "A".to_string(), start)]);
        assert!(!text(&w).contains('\r'), "{:?}", text(&w));
        assert!(text(&w).contains("Sh\nown"), "{:?}", text(&w));
    }

    #[test]
    fn an_html_block_in_a_crlf_note_has_no_carriage_return() {
        let w = walk("<div>\r\na\r\n</div>\r\n");
        assert!(!text(&w).contains('\r'), "{:?}", text(&w));
        assert!(text(&w).contains('a'));
    }

    #[test]
    fn markup_around_or_inside_a_wikilink_keeps_the_lines_well_formed() {
        // Pulldown reads emphasis without knowing the wikilink: one inside
        // it, or straddling its edge, keeps its start and end (no stray End
        // reaches `TextLines`); the wikilink shows its display text once and
        // the lines after it parse normally.
        let note = "# [[a|*b* c]] head\n\n*x [[a* b]] y\n\n#u\n";
        let w = walk(note);
        assert_eq!(headers(&w), [(1, "*b* c head".to_string(), 0)]);
        assert_eq!(text(&w), "# *b* c head\nx a* b y\nu");
        assert_eq!(tag_list(&w), ["u"]);
        // (`a* b` is no vault path: recorded, not listed.)
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Wiki, "a", "*b* c"),
                (WalkLinkKind::Wiki, "a* b", "a* b")
            ]
        );
    }

    #[test]
    fn degenerate_notes_walk_to_nothing() {
        for note in [
            "",
            "\u{feff}",
            "---",
            "---\n",
            "---\na: 1\n---\n",
            "+++\n+++",
        ] {
            let w = walk(note);
            assert!(headers(&w).is_empty(), "{note:?}");
            assert!(w.links.is_empty() && w.tags.is_empty(), "{note:?}");
        }
    }

    #[test]
    fn a_wikilink_shows_its_display_text_and_records_its_parts() {
        let w = walk("see [[proj|Project|extra]] now");
        assert_eq!(text(&w), "see Project now");
        assert_eq!(links(&w), [(WalkLinkKind::Wiki, "proj", "Project")]);
        assert_eq!(w.links[0].range, 4..26);
    }

    #[test]
    fn a_wikilink_in_code_is_not_a_link() {
        let w = walk("`[[x]]` and\n\n```\n[[y]]\n```\n");
        assert!(w.links.is_empty());
        assert!(text(&w).contains("`[[x]]`"));
        assert!(text(&w).contains("[[y]]"));
    }

    #[test]
    fn an_embed_is_a_wikilink_showing_its_display_text() {
        let w = walk("![[pic.png|Picture]]");
        assert_eq!(links(&w), [(WalkLinkKind::WikiEmbed, "pic.png", "Picture")]);
        assert_eq!(text(&w), "Picture");
    }

    #[test]
    fn degenerate_wikilinks_are_text() {
        let w = walk("[[|b]] and [[]]");
        assert!(w.links.is_empty());
        assert_eq!(text(&w), "[[|b]] and [[]]");
    }

    #[test]
    fn an_empty_display_part_shows_nothing() {
        let w = walk("a [[t|]] b");
        assert_eq!(links(&w), [(WalkLinkKind::Wiki, "t", "")]);
        assert_eq!(text(&w), "a  b");
    }

    #[test]
    fn a_wikilink_across_lines_keeps_later_offsets_exact() {
        let note = "# A\n[[target\n|Shown]]\n# B\n";
        let w = walk(note);
        let b = headers(&w)[1].2;
        assert_eq!(&note[b..b + 3], "# B");
    }

    #[test]
    fn markdown_links_keep_their_label_as_written() {
        let note = "[**bold** [x]](a.md) [ref][r] <https://e.x> ![alt](i.png)\n\n[r]: b.md\n";
        let w = walk(note);
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Inline, "a.md", "**bold** [x]"),
                (WalkLinkKind::Reference, "b.md", "ref"),
                (WalkLinkKind::Autolink, "https://e.x", "https://e.x"),
                (WalkLinkKind::Image, "i.png", "alt"),
            ]
        );
        assert_eq!(&note[w.links[0].range.clone()], "[**bold** [x]](a.md)");
    }

    #[test]
    fn a_hashtag_in_prose_is_a_tag_and_loses_its_hash() {
        let w = walk("a #one b");
        assert_eq!(text(&w), "a one b");
        assert_eq!(
            w.tags,
            [WalkTag {
                name: "one".to_string(),
                range: 2..6
            }]
        );
    }

    #[test]
    fn a_hashtag_in_link_text_or_code_is_not_a_tag() {
        let w = walk("[x #a](y.md) [[t|#b]] `#c`\n\n```\n#d\n```\n");
        assert!(w.tags.is_empty(), "{:?}", w.tags);
        let t = text(&w);
        for kept in ["x #a", "#b", "`#c`", "#d"] {
            assert!(t.contains(kept), "{kept} in {t:?}");
        }
    }

    #[test]
    fn a_hashtag_that_looks_like_a_list_marker_keeps_its_setext_heading() {
        let w = walk("#1. Intro\n---\n");
        assert_eq!(headers(&w), [(2, "1. Intro".to_string(), 0)]);
        assert_eq!(w.tags[0].name, "1");
    }

    #[test]
    fn multibyte_text_around_links_and_tags() {
        let note = "café [[ü|ö]] #a ñ [é](x.md)";
        let w = walk(note);
        assert_eq!(text(&w), "café ö a ñ é");
        for link in &w.links {
            let _ = &note[link.range.clone()];
        }
        assert_eq!(&note[w.tags[0].range.clone()], "#a");
        assert_eq!(w.links[0].range, 6..15);
    }

    #[test]
    fn an_image_inside_a_link_is_link_text() {
        let w = walk("[![a #t](i.png)](x.md) #out");
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Inline, "x.md", "![a #t](i.png)"),
                (WalkLinkKind::Image, "i.png", "a #t"),
            ]
        );
        assert_eq!(
            w.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["out"]
        );
    }

    #[test]
    fn headings_report_rows_in_a_crlf_note() {
        let note = "---\r\nt: x\r\n---\r\n# A\r\nbody\r\n\r\n## B\r\n";
        let rows: Vec<(String, usize)> = walk(note)
            .headings(note)
            .into_iter()
            .map(|h| (h.text, h.line))
            .collect();
        assert_eq!(rows, [("A".to_string(), 3), ("B".to_string(), 6)]);
    }

    #[test]
    fn headings_rows_survive_a_wikilink_across_lines() {
        let note = "# A\n[[target\n|Shown]]\n# B\n";
        let rows: Vec<usize> = walk(note).headings(note).iter().map(|h| h.line).collect();
        assert_eq!(rows, [0, 3]);
    }

    #[test]
    fn the_title_renders_like_the_heading() {
        assert_eq!(walk("# Sprint #42\nbody").title(), "Sprint 42");
        assert_eq!(walk("# See [[proj|Project]]\n").title(), "See Project");
    }

    #[test]
    fn index_links_are_in_document_order() {
        let note = "#first [[a]] then [b](https://x.y) and [c](c.md) #last";
        let got: Vec<String> = walk(note)
            .index_links(&VaultPath::new("n.md"))
            .into_iter()
            .map(|l| l.raw_link)
            .collect();
        assert_eq!(
            got,
            [
                "#first".to_string(),
                VaultPath::note_path_from("a").to_string(),
                VaultPath::new("c.md").to_string(),
                "#last".to_string(),
            ]
        );
    }

    #[test]
    fn the_markdown_rewrite_splices_links_and_tags_in_place() {
        let note = "---\nrel: [[fm]]\n---\nsee [[p|P]] and [t](https://x.y) #tag `[[c]]` `[z](z.md)` ![[e]]\n";
        let (md, links) = walk(note).render_markdown(note, &VaultPath::new("dir/n.md"));
        let p = VaultPath::note_path_from("p");
        let e = VaultPath::note_path_from("e");
        assert_eq!(
            md,
            format!(
                "---\nrel: [[fm]]\n---\nsee [P]({p}) and [t](https://x.y) [#tag](#tag) `[[c]]` `[z](z.md)` ![e]({e})\n"
            )
        );
        let got: Vec<(String, String)> = links.into_iter().map(|l| (l.raw_link, l.text)).collect();
        assert_eq!(
            got,
            [
                (p.to_string(), "P".to_string()),
                ("https://x.y".to_string(), "t".to_string()),
                ("#tag".to_string(), "tag".to_string()),
            ]
        );
    }

    #[test]
    fn reference_links_and_autolinks_are_recorded_but_not_rewritten() {
        let note = "[r][d] <https://a.b>\n\n[d]: https://c.d\n";
        let (md, links) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, note);
        let raw: Vec<String> = links.into_iter().map(|l| l.raw_link).collect();
        assert_eq!(raw, ["https://c.d", "https://a.b"]);
    }

    #[test]
    fn tag_names_are_lowercased_sorted_and_distinct() {
        assert_eq!(walk("#B #a #b `#c` [x #d](y.md)").tag_names(), ["a", "b"]);
    }

    #[test]
    fn a_wikilink_inside_a_markdown_link_label_is_link_text() {
        // spec row (review 8): per CommonMark `[see [[a]]](x.md)` is a link
        // to `x.md`, and a wikilink inside a link's text is that text (the
        // editor does not highlight it either).
        let note = "[see [[a]]](x.md)";
        let w = walk(note);
        assert_eq!(links(&w), [(WalkLinkKind::Inline, "x.md", "see [[a]]")]);
        assert_eq!(text(&w), "see [[a]]");
        let (md, links) = w.render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, note);
        let raw: Vec<String> = links.into_iter().map(|l| l.raw_link).collect();
        assert_eq!(raw, [VaultPath::new("x.md").to_string()]);
    }

    #[test]
    fn a_wikilink_path_resolves_against_the_note_folder_as_the_editor_follows_it() {
        let note = "[[schemas/event]] and [[event]]";
        let bare = VaultPath::note_path_from("event");
        // An absolute note path, as the vault passes it: exactly the editor's
        // `resolve_link_in_note`.
        let at = VaultPath::new("/folder/note.md");
        let target = VaultPath::note_path_from("schemas/event").resolve_link_in_note(&at);
        let (md, links) = walk(note).render_markdown(note, &at);
        assert_eq!(md, format!("[schemas/event]({target}) and [event]({bare})"));
        let raw: Vec<String> = links.into_iter().map(|l| l.raw_link).collect();
        assert_eq!(raw, [target.to_string(), bare.to_string()]);
        // A relative note path keeps today's relative form (the snapshot's
        // `folder/note.md`): `folder/schemas/event.md`, no leading `/`.
        let (md, _) = walk(note).render_markdown(note, &VaultPath::new("folder/note.md"));
        let relative = VaultPath::new("folder/schemas/event.md");
        assert_eq!(
            md,
            format!("[schemas/event]({relative}) and [event]({bare})")
        );
    }

    #[test]
    fn only_a_plain_inline_link_is_rewritten_but_every_form_is_listed() {
        let at = VaultPath::new("/dir/n.md");
        for note in ["[x](<my note.md>)", "[t](x.md \"Title\")"] {
            let (md, links) = walk(note).render_markdown(note, &at);
            assert_eq!(md, note);
            assert_eq!(links.len(), 1, "{note:?}: {links:?}");
        }
        let (md, links) = walk("[x](<my note.md>)").render_markdown("[x](<my note.md>)", &at);
        assert_eq!(md, "[x](<my note.md>)");
        assert_eq!(links[0].raw_link, VaultPath::new("my note.md").to_string());
        let note = "[t](sub/x.md)";
        let (md, links) = walk(note).render_markdown(note, &at);
        let resolved = VaultPath::new("/dir/sub/x.md");
        assert_eq!(md, format!("[t]({resolved})"));
        assert_eq!(links[0].raw_link, resolved.to_string());
    }

    #[test]
    fn an_email_autolink_is_not_recorded() {
        let note = "mail <me@x.com>";
        let w = walk(note);
        assert!(w.links.is_empty(), "{:?}", w.links);
        assert!(text(&w).contains("me@x.com"), "{:?}", text(&w));
        let (md, links) = w.render_markdown(note, &VaultPath::new("/dir/n.md"));
        assert_eq!(md, note);
        assert!(links.is_empty(), "{links:?}");
        assert!(crate::note::note_link_targets(note).is_empty());
    }

    fn tag_list(w: &NoteWalk) -> Vec<&str> {
        w.tags.iter().map(|t| t.name.as_str()).collect()
    }

    fn raw_links(links: &[NoteLink]) -> Vec<String> {
        links.iter().map(|l| l.raw_link.clone()).collect()
    }

    /// Asserts no view reads a link in `note`: the walk, the index, the
    /// rendered markdown (left as written) and its link list, the CLI
    /// targets and `NoteMetadata`.
    fn assert_no_link_anywhere(note: &str) {
        let at = VaultPath::new("folder/n.md");
        let w = walk(note);
        assert!(w.links.is_empty(), "{note:?}: {:?}", w.links);
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert!(index.is_empty(), "{note:?}: {index:?}");
        let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(md, note, "HTML stays as written");
        assert!(listed.is_empty(), "{note:?}: {listed:?}");
        assert!(crate::note::note_link_targets(note).is_empty(), "{note:?}");
        assert!(
            crate::note::NoteMetadata::of(note).links.is_empty(),
            "{note:?}"
        );
    }

    // HTML is opaque (spec row): nothing inside an HTML block is a link —
    // a wikilink, an embed, one with a section, one across lines, one in
    // content indented four spaces.
    #[test]
    fn a_wikilink_inside_an_html_block_is_not_a_link_anywhere() {
        let note = "<details>\n<summary>More</summary>\nSee [[hidden]]\n</details>\n";
        assert_no_link_anywhere(note);
        assert!(text(&walk(note)).contains("See [[hidden]]"));
        for note in [
            "<div>\n[[a\n|b]] ![[e]] [[|x]] [[]]\n</div>\n",
            "<div>\n[[note#sec|S]]\n</div>\n",
            "<div>\n    and [[w]]\n</div>\n",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // Spec row: pulldown splits text at a flanking `_`, and a hashtag cut
    // in two is no tag anywhere (the index never had one).
    #[test]
    fn a_hashtag_split_at_an_underscore_is_not_a_tag_anywhere() {
        let at = VaultPath::new("n.md");
        for note in ["#my_tag_ x", "a #_x"] {
            let w = walk(note);
            assert!(w.tags.is_empty(), "{note:?}: {:?}", w.tags);
            assert!(crate::note::extract_labels(note).is_empty(), "{note:?}");
            let (_, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert!(
                !listed
                    .iter()
                    .any(|l| matches!(l.ltype, NoteLinkType::Hashtag)),
                "{note:?}: {listed:?}"
            );
        }
    }

    #[test]
    fn an_escaped_hashtag_is_a_tag_and_loses_its_hash() {
        // Per event: pulldown's text event starts after the `\`, its source
        // and decoded text match, so the `#` goes as for any tag.
        let w = walk("x \\#tag y");
        assert_eq!(tag_list(&w), ["tag"]);
        assert_eq!(text(&w), "x tag y");
    }

    // Item 3: one test per Behaviour-changes row added in the final review.
    #[test]
    fn markup_and_entities_in_a_wikilink_alias_are_shown_as_written() {
        let w = walk("[[a|*Emph* name]] and [[c|a &amp; b]]");
        assert_eq!(text(&w), "*Emph* name and a &amp; b");
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Wiki, "a", "*Emph* name"),
                (WalkLinkKind::Wiki, "c", "a &amp; b"),
            ]
        );
    }

    #[test]
    fn a_hashtag_touching_a_wikilink_is_a_tag() {
        let note = "[[c]]#t2 and #t3[[d]]";
        let w = walk(note);
        assert_eq!(tag_list(&w), ["t2", "t3"]);
        let (_, index) =
            crate::note::content_extractor::get_chunks_and_links(&VaultPath::new("n.md"), note);
        assert_eq!(
            raw_links(&index),
            [
                VaultPath::note_path_from("c").to_string(),
                "#t2".to_string(),
                "#t3".to_string(),
                VaultPath::note_path_from("d").to_string(),
            ]
        );
    }

    #[test]
    fn a_wikilink_in_image_alt_text_is_alt_text() {
        // spec row (review 8): per CommonMark `![x [[a]] #t](p.png)` is an
        // image; its alt text, wikilink and hashtag included, is image text.
        let note = "![x [[a]] #t](p.png)";
        let w = walk(note);
        assert_eq!(links(&w), [(WalkLinkKind::Image, "p.png", "x [[a]] #t")]);
        assert!(w.tags.is_empty(), "{:?}", w.tags);
        let (md, _) = w.render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, note);
    }

    #[test]
    fn a_url_fragment_in_an_autolink_is_not_a_tag() {
        let note = "<https://x.com/#frag>";
        assert!(crate::note::extract_labels(note).is_empty());
        let (md, listed) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, note);
        assert_eq!(raw_links(&listed), ["https://x.com/#frag"]);
    }

    // Item 5.
    #[test]
    fn a_paragraph_title_renders_like_its_line() {
        assert_eq!(walk("#inbox call [[bob|Bob]]\n").title(), "inbox call Bob");
    }

    #[test]
    fn the_markdown_rewrite_leaves_a_link_in_the_frontmatter_alone() {
        let note = "---\nrel: [x](y.md) [[w]] #t\n---\nbody\n";
        let (md, listed) =
            crate::note::content_extractor::get_markdown_and_links(&VaultPath::new("n.md"), note);
        assert_eq!(md, note);
        assert!(listed.is_empty(), "{listed:?}");
    }

    #[test]
    fn the_markdown_rewrite_leaves_a_link_in_code_alone() {
        let note = "a `[x](y.md)` b\n\n```\n[x](y.md)\n```\n";
        let (md, listed) =
            crate::note::content_extractor::get_markdown_and_links(&VaultPath::new("n.md"), note);
        assert_eq!(md, note);
        assert!(listed.is_empty(), "{listed:?}");
    }

    // Item 7.
    #[test]
    fn a_title_is_trimmed() {
        assert_eq!(walk("[[ a ]] and more\n").title(), "a  and more");
    }

    // Item 8: splices that touch each other.
    #[test]
    fn adjacent_links_and_tags_all_splice() {
        let note = "[a](b.md)#tag [[c]]#t2 #t3[[d]]";
        let at = VaultPath::new("/dir/n.md");
        let (md, listed) = walk(note).render_markdown(note, &at);
        // A bare `b.md` is a note path at the root, as before.
        let b = VaultPath::new("b.md");
        let c = VaultPath::note_path_from("c");
        let d = VaultPath::note_path_from("d");
        assert_eq!(
            md,
            format!("[a]({b})[#tag](#tag) [c]({c})[#t2](#t2) [#t3](#t3)[d]({d})")
        );
        assert_eq!(
            raw_links(&listed),
            [
                b.to_string(),
                c.to_string(),
                d.to_string(),
                "#tag".to_string(),
                "#t2".to_string(),
                "#t3".to_string(),
            ]
        );
    }

    // HTML is opaque (spec row): a markdown link inside an HTML block is no
    // link anywhere and is not rewritten — padded, titled, `<…>`-wrapped,
    // indented four spaces, or next to an image.
    #[test]
    fn a_markdown_link_inside_an_html_block_is_not_a_link_anywhere() {
        for note in [
            "<details>\nSee [doc](doc.md)\n</details>\n",
            "<div>[x](y.md)</div>\n",
            "<div>[x](sub/y.md) ![i](p.png) [[w]]</div>\n",
            "<div>\n[p]( sub/c.md )\n</div>\n",
            "<div>[t](x.md \"T\") [u](<a b.md>)</div>\n",
            "<div>\n    [x](sub/y.md)\n</div>\n",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // Review 2, item 2 (spec rows; pinned).
    #[test]
    fn an_escaped_wikilink_is_not_a_link_anywhere() {
        let note = "\\[[a]] x";
        let w = walk(note);
        assert!(w.links.is_empty(), "{:?}", w.links);
        assert_eq!(text(&w), "[[a]] x");
        let at = VaultPath::new("n.md");
        let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(md, note);
        assert!(listed.is_empty(), "{listed:?}");
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert!(index.is_empty(), "{index:?}");
        assert!(crate::note::note_link_targets(note).is_empty());
    }

    #[test]
    fn markup_inside_a_plain_wikilink_is_shown_as_written() {
        let chunks = crate::note::content_extractor::get_content_chunks("[[a *b* c]]");
        assert_eq!(chunks[0].text, "a *b* c");
    }

    // Review 2, item 3: the CLI's link targets drop the wikilinks every
    // other consumer drops.
    #[test]
    fn link_targets_skip_wikilinks_to_invalid_paths() {
        // `[[#tag]]` parses as a wikilink — its target is no vault path.
        assert_eq!(
            links(&walk("[[#tag]] x")),
            [(WalkLinkKind::Wiki, "#tag", "#tag")]
        );
        assert!(crate::note::note_link_targets("[[#tag]] x").is_empty());
        assert_eq!(
            crate::note::note_link_targets("[[a#sec|S]] [[ok]] ![[e]]"),
            ["a#sec", "ok", "e"]
        );
    }

    // Review 2, item 4: an embed whose target looks like an image keeps its
    // target as written for the image pipeline; any other renders an
    // unresolved note path. Embeds stay unlisted.
    #[test]
    fn an_image_embed_keeps_its_target_as_written() {
        let at = VaultPath::new("/dir/n.md");
        for (note, expected) in [
            ("![[pic.png]]", "![pic.png](pic.png)"),
            ("![[sub/pic.png]]", "![sub/pic.png](sub/pic.png)"),
            ("![[pic.png|Pic]]", "![Pic](pic.png)"),
        ] {
            let (md, listed) = walk(note).render_markdown(note, &at);
            assert_eq!(md, expected);
            assert!(listed.is_empty(), "{listed:?}");
        }
    }

    // Ruling on concern 3: only an image target is kept as written; a
    // dotted note name or another file renders a note path, as before.
    #[test]
    fn an_embed_that_is_not_an_image_renders_a_note_path() {
        let at = VaultPath::new("/dir/n.md");
        for target in ["v1.2", "doc.pdf"] {
            let note = format!("![[{target}]]");
            let path = VaultPath::note_path_from(target);
            let (md, listed) = walk(&note).render_markdown(&note, &at);
            assert_eq!(md, format!("![{target}]({path})"));
            assert!(listed.is_empty(), "{listed:?}");
        }
    }

    #[test]
    fn an_embed_without_an_extension_renders_an_unresolved_note_path() {
        let note = "![[sub/e]]";
        let at = VaultPath::new("/dir/n.md");
        let e = VaultPath::note_path_from("sub/e");
        let (md, listed) = walk(note).render_markdown(note, &at);
        assert_eq!(md, format!("![sub/e]({e})"));
        assert!(listed.is_empty(), "{listed:?}");
        assert_eq!(raw_links(&walk(note).index_links(&at)), [e.to_string()]);
    }

    // Review 2, item 6: the frontmatter is read from the offsets the body
    // start was found from, exactly as the line-based reading returned it.
    #[test]
    fn the_frontmatter_is_the_text_between_its_fences() {
        for (note, expected) in [
            ("---\na: 1\nb: 2\n---\nbody", "a: 1\nb: 2"),
            ("+++\na = 1\n+++\nbody", "a = 1"),
            ("---\r\na: 1\r\nb: 2\r\n---\r\nbody", "a: 1\nb: 2"),
            ("\u{feff}---\na: 1\n---\nbody", "a: 1"),
            ("---\n---\nbody", ""),
            ("---\na\n\n---\n", "a\n"),
            ("---\na: 1\n---", "a: 1"),
            ("---\na: 1\nbody", ""),
            ("plain", ""),
            // A closing fence ending in a lone `\r` at the end of the note
            // closes the block, as for the body start (the old line-based
            // reading disagreed with itself there and lost the block).
            ("---\r\na\r\n---\r", "a"),
        ] {
            assert_eq!(walk(note).frontmatter, expected, "{note:?}");
        }
    }

    // Review 2, item 9: a link's label is the source of its text events, so
    // a code span holding `]` is part of it.
    #[test]
    fn an_escaped_backslash_before_a_spaced_link_keeps_it_a_link() {
        assert_eq!(walk("\\\\[x](a b.md)").link_targets(), ["a b.md"]);
        assert!(walk("\\[x](a b.md)").link_targets().is_empty());
    }

    #[test]
    fn a_leading_escape_in_a_link_label_is_part_of_the_label() {
        let note = "[\\[a](sub/x.md) ![\\[b](i.png)";
        let w = walk(note);
        assert_eq!(links(&w)[0], (WalkLinkKind::Inline, "sub/x.md", "\\[a"));
        assert_eq!(links(&w)[1], (WalkLinkKind::Image, "i.png", "\\[b"));
        // A plain inline link is rewritten, whatever its label holds.
        let (md, _) = w.render_markdown(note, &VaultPath::new("/dir/n.md"));
        assert!(!md.starts_with("[\\[a](sub/x.md)"), "{md}");
    }

    #[test]
    fn a_code_span_in_a_link_label_is_part_of_the_label() {
        let note = "[a `]` b](sub/x.md) and [](e.md)";
        let w = walk(note);
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Inline, "sub/x.md", "a `]` b"),
                (WalkLinkKind::Inline, "e.md", ""),
            ]
        );
        let (md, _) = w.render_markdown(note, &VaultPath::new("/dir/n.md"));
        let x = VaultPath::new("/dir/sub/x.md");
        assert_eq!(md, format!("[a `]` b]({x}) and [](e.md)"));
    }

    // Review 2, item 7: the metadata carries the CLI's link targets from the
    // same walk.
    #[test]
    fn note_metadata_lists_the_link_targets() {
        let note = "---\nrel: [[fm]]\n---\n[[a]] [b](b.md) [[#bad]] `[[c]]` <https://c.d>";
        assert_eq!(
            crate::note::NoteMetadata::of(note).links,
            ["a", "b.md", "https://c.d"]
        );
    }

    // Moved from the CLI's removed `metadata_extractor` (review 5, item 7):
    // the tags the CLI reports are the index's labels plus frontmatter tags.
    #[test]
    fn note_metadata_tags_come_from_either_frontmatter_format_and_the_body() {
        let yaml = "---\ntags:\n  - Project\n  - urgent\ntitle: Test\n---\nbody";
        assert_eq!(
            crate::note::NoteMetadata::of(yaml).tags,
            ["project", "urgent"]
        );
        let toml = "+++\ntags = [\"meeting\"]\n+++\nbody #notes";
        assert_eq!(
            crate::note::NoteMetadata::of(toml).tags,
            ["meeting", "notes"]
        );
    }

    #[test]
    fn note_metadata_tags_follow_the_label_rules() {
        let note =
            "---\ntags: [yaml_tag]\n---\nplain #body and #tag-with-dash\n```\n#code_tag\n```";
        // A dash ends a label; a tag in code is none.
        assert_eq!(
            crate::note::NoteMetadata::of(note).tags,
            ["body", "tag", "yaml_tag"]
        );
    }

    // Review 3, item 1 (spec row): a wikilink to a section or a block, or
    // padded, links to the note — indexed and listed with the fragment and
    // padding stripped, rendered keeping the fragment, a CLI target as
    // written.
    #[test]
    fn a_section_block_or_padded_wikilink_is_a_link_everywhere() {
        let at = VaultPath::new("n.md");
        for (note, target, label, fragment, written) in [
            (
                "[[note#section]]",
                "note",
                "note#section",
                "#section",
                "note#section",
            ),
            (
                "[[Plan#Goals|goals]]",
                "Plan",
                "goals",
                "#Goals",
                "Plan#Goals",
            ),
            ("[[a^blk]]", "a", "a^blk", "#^blk", "a^blk"),
            ("[[ spaced ]]", "spaced", " spaced ", "", " spaced "),
        ] {
            let path = VaultPath::note_path_from(target);
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), [path.to_string()], "{note:?}");
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(md, format!("[{label}]({path}{fragment})"), "{note:?}");
            assert_eq!(raw_links(&listed), [path.to_string()], "{note:?}");
            assert_eq!(crate::note::note_link_targets(note), [written], "{note:?}");
        }
    }

    #[test]
    fn a_wikilink_still_invalid_after_stripping_its_fragment_is_no_link() {
        let at = VaultPath::new("n.md");
        for note in ["[[#tag]]", "[[^blk]]", "[[ #x]]"] {
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(md, note);
            assert!(listed.is_empty(), "{note:?}: {listed:?}");
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert!(index.is_empty(), "{note:?}: {index:?}");
            assert!(crate::note::note_link_targets(note).is_empty(), "{note:?}");
        }
    }

    // Review 3, item 2: every inline link is rewritten but a `<…>`
    // destination or one with a title; padding and escapes are no reason
    // to leave one as written.
    #[test]
    fn a_padded_or_escaped_inline_destination_is_rewritten() {
        let at = VaultPath::new("folder/note.md");
        for (note, label, resolved) in [
            ("[u]( sub/c.md )", "u", "folder/sub/c.md"),
            ("[v](sub/a&amp;b.md)", "v", "folder/sub/a&b.md"),
            ("[w](sub/x.md\t)", "w", "folder/sub/x.md"),
        ] {
            let resolved = VaultPath::new(resolved);
            let (md, listed) = walk(note).render_markdown(note, &at);
            assert_eq!(md, format!("[{label}]({resolved})"), "{note:?}");
            assert_eq!(raw_links(&listed), [resolved.to_string()], "{note:?}");
        }
    }

    #[test]
    fn a_destination_that_cannot_be_written_back_bare_is_wrapped() {
        // `a\(b.md` decodes to `a(b.md`: written back bare, its `(` would
        // end the link early, so it is `<…>`-wrapped (review 4, item 1).
        let note = "[t](a\\(b.md)";
        let (md, listed) = walk(note).render_markdown(note, &VaultPath::new("folder/note.md"));
        assert_eq!(md, "[t](<a(b.md>)");
        assert_eq!(listed.len(), 1, "{listed:?}");
    }

    // Review 3, items 4 and 5 (spec rows; pinned).
    #[test]
    fn a_reference_definition_to_an_anchor_is_no_tag() {
        let note = "[top]: #anchor\n\nUse [top].";
        let w = walk(note);
        assert!(w.tags.is_empty(), "{:?}", w.tags);
        assert!(crate::note::note_tags(note).is_empty());
        assert_eq!(links(&w), [(WalkLinkKind::Reference, "#anchor", "top")]);
        let (md, _) = w.render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, note);
        // An anchor links to no note, URL or vault path: not listed by any
        // view (review 10, item 6).
        assert!(crate::note::note_link_targets(note).is_empty());
    }

    #[test]
    fn link_syntax_reads_as_commonmark_reads_it() {
        let root = VaultPath::new("n.md");
        assert_eq!(crate::note::note_link_targets("[t](a(b).md)"), ["a(b).md"]);
        let (_, listed) = walk("[t](a(b).md)").render_markdown("[t](a(b).md)", &root);
        assert_eq!(raw_links(&listed), [VaultPath::new("a(b).md").to_string()]);

        let note = "[[Note (draft)]]";
        let draft = VaultPath::note_path_from("Note (draft)");
        assert_eq!(draft.to_string(), "note (draft).md");
        let (md, listed) = walk(note).render_markdown(note, &VaultPath::new("folder/note.md"));
        assert_eq!(md, format!("[Note (draft)](<{draft}>)"));
        assert_eq!(raw_links(&listed), [draft.to_string()]);

        // spec row (review 8): the editor's pattern reads `[[[tri]]` — its
        // target `[tri` is no vault path — as 50dcb026 did: no link.
        let note = "a [[[tri]]] b";
        assert!(crate::note::note_link_targets(note).is_empty());
        let w = walk(note);
        assert_eq!(text(&w), "a [tri] b");
        assert_eq!(w.render_markdown(note, &root).0, note);

        let note = "[v](d\\_e.md)";
        assert_eq!(crate::note::note_link_targets(note), ["d_e.md"]);
        let (md, listed) = walk(note).render_markdown(note, &root);
        assert_eq!(md, "[v](d_e.md)");
        assert_eq!(raw_links(&listed), ["d_e.md"]);
    }

    // HTML is opaque (spec row): a link inside an inline tag — in an
    // attribute, across the lines of one tag, across two adjacent tags — is
    // no link anywhere.
    #[test]
    fn a_link_inside_a_multi_line_inline_tag_is_not_a_link_anywhere() {
        for note in [
            "a <span title=\"[[x|one\ntwo]]\">z</span> b",
            "see <span title=\"[[q]]\">z</span>\n",
            "a <i title=\"[[t|one\"></i>\n<b title=\"two]]\"> b",
            "a <a title=\"[x](\ny.md)\">z</a> b",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // HTML is opaque (spec row): an HTML comment is HTML.
    #[test]
    fn a_link_inside_an_html_comment_is_not_a_link_anywhere() {
        for note in [
            "<!-- [[hidden]] and [x](y.md) -->\n",
            "text <!-- [[hidden]]\n[x](y.md) --> more\n",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // Formatting tags inside a line are inline HTML only for the tags: the
    // text between them is markdown, so its links are links everywhere.
    #[test]
    fn a_link_between_inline_formatting_tags_is_a_link_everywhere() {
        let note = "see <b>[[note]]</b>, <mark>[x](y.md)</mark> and \
                    <span style=\"color:red\">[[other]]</span> text\n";
        let at = VaultPath::new("n.md");
        let n = VaultPath::note_path_from("note");
        let y = VaultPath::new("y.md");
        let o = VaultPath::note_path_from("other");
        let w = walk(note);
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Wiki, "note", "note"),
                (WalkLinkKind::Inline, "y.md", "x"),
                (WalkLinkKind::Wiki, "other", "other"),
            ]
        );
        let expected = [n.to_string(), y.to_string(), o.to_string()];
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert_eq!(raw_links(&index), expected);
        let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(raw_links(&listed), expected);
        assert_eq!(
            md,
            format!(
                "see <b>[note]({n})</b>, <mark>[x]({y})</mark> and \
                 <span style=\"color:red\">[other]({o})</span> text\n"
            )
        );
        assert_eq!(
            crate::note::note_link_targets(note),
            ["note", "y.md", "other"]
        );
        assert_eq!(
            crate::note::NoteMetadata::of(note).links,
            ["note", "y.md", "other"]
        );
    }

    // Rulings round, 1: only spaces and tabs pad a wikilink target; one that
    // ends at a line break stays no link, as before the single walk.
    #[test]
    fn a_wikilink_target_ending_at_a_line_break_is_no_link() {
        let note = "a [[target\n|Shown]] b";
        let at = VaultPath::new("n.md");
        let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(md, note);
        assert!(listed.is_empty(), "{listed:?}");
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert!(index.is_empty(), "{index:?}");
        assert!(crate::note::note_link_targets(note).is_empty());
        let tab = "[[\tt\t]]";
        let t = VaultPath::note_path_from("t");
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, tab);
        assert_eq!(raw_links(&index), [t.to_string()]);
    }

    // Rulings round, 3: the rendered destination is a valid markdown one.
    #[test]
    fn a_block_fragment_renders_as_a_url_fragment() {
        let note = "[[a^blk]] ![[e^b]]";
        let a = VaultPath::note_path_from("a");
        let e = VaultPath::note_path_from("e");
        let (md, _) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        // An embed drops its fragment (review 4, item 2).
        assert_eq!(md, format!("[a^blk]({a}#^blk) ![e^b]({e})"));
    }

    #[test]
    fn a_destination_with_whitespace_is_wrapped_in_angle_brackets() {
        let note = "[[Plan#My Goals|g]]";
        let plan = VaultPath::note_path_from("Plan");
        let (md, listed) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, format!("[g](<{plan}#My Goals>)"));
        assert_eq!(raw_links(&listed), [plan.to_string()]);
        // The renderer reads it back as one link to that destination.
        let dests: Vec<String> = Parser::new(&md)
            .filter_map(|e| match e {
                Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(dests, [format!("{plan}#My Goals")]);
    }

    /// The destinations of the links a renderer reads in `md`.
    fn rendered_dests(md: &str) -> Vec<String> {
        Parser::new(md)
            .filter_map(|e| match e {
                Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.to_string()),
                _ => None,
            })
            .collect()
    }

    // Review 4, item 1: a destination is written bare, `<…>`-wrapped, or —
    // when neither reads back as itself — not spliced; the link stays listed.
    #[test]
    fn a_destination_is_written_bare_wrapped_or_left_as_written() {
        let at = VaultPath::new("n.md");
        let plan = VaultPath::note_path_from("Plan");
        for (note, expected) in [
            ("[[Plan]]", format!("[Plan]({plan})")),
            ("[[Plan#My Goals|g]]", format!("[g](<{plan}#My Goals>)")),
            ("[[Plan#a)]]", format!("[Plan#a)](<{plan}#a)>)")),
            ("[[Plan#a(b]]", format!("[Plan#a(b](<{plan}#a(b>)")),
            ("[[Plan#a > b|g]]", "[[Plan#a > b|g]]".to_string()),
        ] {
            let (md, listed) = walk(note).render_markdown(note, &at);
            assert_eq!(md, expected, "{note:?}");
            assert_eq!(raw_links(&listed), [plan.to_string()], "{note:?}");
            if md != note {
                let (_, fragment) = split_link_fragment(note.trim_matches(['[', ']']));
                let fragment = fragment.split('|').next().unwrap_or_default();
                assert_eq!(
                    rendered_dests(&md),
                    [format!("{plan}{fragment}")],
                    "{note:?}"
                );
            }
        }
    }

    #[test]
    fn a_backslash_in_a_destination_is_doubled_so_it_reads_back() {
        let at = VaultPath::new("n.md");
        let plan = VaultPath::note_path_from("Plan");
        for (note, dest) in [
            ("[[Plan#a\\.b|g]]", format!("{plan}#a\\.b")),
            ("[[Plan#a b\\.c|g]]", format!("{plan}#a b\\.c")),
        ] {
            let (md, _) = walk(note).render_markdown(note, &at);
            assert_eq!(rendered_dests(&md), [dest], "{note:?} -> {md}");
        }
    }

    // Review 4, item 1: the vault's image pass leaves wrapped link
    // destinations exactly as rendered.
    #[test]
    fn the_image_pass_keeps_wrapped_destinations() {
        let note = "[[Plan#a(b]] [[Plan#My Goals|g]] [[Note (draft)]] ![[pic.png#x]]";
        let (md, _) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        let (after, images) = crate::note::process_image_links(&md, |_, raw| {
            (format!("/abs/{raw}"), NoteLink::url(raw, ""))
        });
        assert_eq!(after, md.replace("(pic.png)", "(/abs/pic.png)"));
        assert_eq!(images.len(), 1);
    }

    // Review 4, item 1: the same rule for an inline link's resolved
    // destination.
    #[test]
    fn an_inline_destination_with_parentheses_is_wrapped() {
        let note = "[t](a\\(b.md) [u](a&#32;b.md)";
        let (md, _) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, "[t](<a(b.md>) [u](<a b.md>)");
        assert_eq!(rendered_dests(&md), ["a(b.md", "a b.md"]);
    }

    // Review 4, item 2: an embed renders without its fragment.
    #[test]
    fn an_embed_renders_without_its_fragment() {
        let at = VaultPath::new("/dir/n.md");
        let e = VaultPath::note_path_from("e");
        for (note, expected) in [
            ("![[pic.png#x]]", "![pic.png#x](pic.png)".to_string()),
            ("![[pic.png#x|P]]", "![P](pic.png)".to_string()),
            ("![[e^b]]", format!("![e^b]({e})")),
            ("![[e#s]]", format!("![e#s]({e})")),
        ] {
            let w = walk(note);
            let (md, listed) = w.render_markdown(note, &at);
            assert_eq!(md, expected, "{note:?}");
            assert!(listed.is_empty(), "{listed:?}");
        }
        assert_eq!(
            raw_links(&walk("![[e^b]]").index_links(&at)),
            [e.to_string()]
        );
    }

    // Review 4, item 3: an empty or blank destination is no link anywhere.
    #[test]
    fn an_empty_destination_is_no_link() {
        let at = VaultPath::new("folder/n.md");
        for note in ["[t]()", "[t]( )", "[t](<>)", "[[a#s|]] [[ |x]]"] {
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            let (md, listed) = walk(note).render_markdown(note, &at);
            let targets = crate::note::note_link_targets(note);
            if note.starts_with("[[") {
                let a = VaultPath::note_path_from("a");
                assert_eq!(raw_links(&index), [a.to_string()], "{note:?}");
                assert_eq!(targets, ["a#s"], "{note:?}");
                assert!(md.ends_with(" [[ |x]]"), "{md:?}");
            } else {
                assert!(index.is_empty(), "{note:?}: {index:?}");
                assert!(listed.is_empty(), "{note:?}: {listed:?}");
                assert!(targets.is_empty(), "{note:?}: {targets:?}");
                assert_eq!(md, note);
            }
        }
    }

    // Review 4, item 4: an empty display part renders the target as the
    // link text; the walked text stays empty.
    #[test]
    fn an_empty_display_part_renders_the_target_as_link_text() {
        let note = "[[a#s|]]";
        let a = VaultPath::note_path_from("a");
        let w = walk(note);
        assert_eq!(text(&w), "");
        let (md, _) = w.render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, format!("[a#s]({a}#s)"));
    }

    // Review 5, item 1 (spec row): a markdown link whose destination holds
    // unencoded spaces is a link everywhere, found with the editor's pattern.
    #[test]
    fn a_markdown_link_with_spaces_in_its_destination_is_a_link_everywhere() {
        let at = VaultPath::new("/journal/2024-10-21.md");
        let david = VaultPath::new("/work/people/david h.md");
        let written = "[David H](../Work/People/David H.md)";
        for note in [
            format!("### {written}\n"),
            format!("see {written} here\n"),
            format!("- item {written} x\n"),
        ] {
            let (chunks, index) = crate::note::content_extractor::get_chunks_and_links(&at, &note);
            assert_eq!(raw_links(&index), [david.to_string()], "{note:?}");
            // The text stays as written: a heading's, or its chunk's.
            let headings = walk(&note).headings(&note);
            assert!(
                chunks.iter().any(|c| c.text.contains(written))
                    || headings.first().is_some_and(|h| h.text == written),
                "{note:?}: {chunks:?} {headings:?}"
            );
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, &note);
            assert_eq!(raw_links(&listed), [david.to_string()], "{note:?}");
            assert!(md.contains(&format!("[David H](<{david}>)")), "{md:?}");
            assert_eq!(rendered_dests(&md), [david.to_string()], "{md:?}");
            assert_eq!(
                crate::note::note_link_targets(&note),
                ["../Work/People/David H.md"],
                "{note:?}"
            );
        }
    }

    #[test]
    fn a_spaced_destination_link_is_recorded_once_and_an_image_as_an_image() {
        let note = "[t](x.md) and [u](a b.md) ![img](p q.png) [Dave #x](d e.md) #y";
        let w = walk(note);
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Inline, "x.md", "t"),
                (WalkLinkKind::Found, "a b.md", "u"),
                (WalkLinkKind::Image, "p q.png", "img"),
                (WalkLinkKind::Found, "d e.md", "Dave #x"),
            ]
        );
        assert_eq!(&note[w.links[1].range.clone()], "[u](a b.md)");
        // A hashtag in its text is link text, not a tag in any view; the
        // walked text, built before the block ends, shows it without `#`
        // (as HEAD did).
        assert_eq!(tag_list(&w), ["y"]);
        assert_eq!(
            text(&w),
            "t and [u](a b.md) ![img](p q.png) [Dave x](d e.md) y"
        );
        let (md, _) = w.render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(
            md,
            "[t](x.md) and [u](<a b.md>) ![img](p q.png) [Dave #x](<d e.md>) [#y](#y)"
        );
    }

    #[test]
    fn a_spaced_destination_link_in_code_or_html_is_no_link() {
        for note in [
            "`[u](a b.md)`\n",
            "```\n[u](a b.md)\n```\n",
            "<div>\n[u](a b.md)\n</div>\n",
            "a <span title=\"[u](a b.md)\">z</span>\n",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // Review 5, item 2: a wikilink alias wrapped over lines reads as
    // pulldown would read its text: the next line's container prefix goes.
    #[test]
    fn a_wrapped_wikilink_alias_drops_the_next_lines_container_prefix() {
        let note = "> see [[x|multi\n> line]] end";
        let chunks = crate::note::content_extractor::get_content_chunks(note);
        assert_eq!(chunks[0].text, "see multi\nline end");
        assert_eq!(walk(note).title(), "see multi");
        let note = "- item [[x|first\n  second]] tail";
        let chunks = crate::note::content_extractor::get_content_chunks(note);
        assert_eq!(chunks[0].text, "* item first second tail");
        assert_eq!(walk(note).title(), "item first");
        let note = "> Quoted [[x|setext\n> head]]\n> ---";
        let w = walk(note);
        assert_eq!(headers(&w), [(2, "Quoted setext".to_string(), 2)]);
        let chunks = w.into_chunks();
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert_eq!(
            (chunks[0].breadcrumb.as_str(), chunks[0].text.as_str()),
            ("Quoted setext", "head")
        );
    }

    // Review 5, item 3: line ends and headings are trimmed as pulldown
    // trims them; a heading of only whitespace is not listed.
    #[test]
    fn whitespace_left_by_a_wikilink_alias_is_trimmed() {
        let note = "# Heading [[a|]]\n";
        assert_eq!(walk(note).headings(note)[0].text, "Heading");
        assert!(crate::note::scan::heading_section_range(note, "Heading").is_some());
        let chunks = crate::note::content_extractor::get_content_chunks("see [[a|]]");
        assert_eq!(chunks[0].text, "see");
        let note = "# [[a| ]]\n";
        assert!(walk(note).headings(note).is_empty());
    }

    // Review 5, item 4: a fence alone, without a newline, is a fence.
    #[test]
    fn a_lone_fence_without_a_newline_is_no_body() {
        for note in ["+++", "---", "\u{feff}+++", "\u{feff}---"] {
            let w = walk(note);
            assert!(w.lines.iter().all(|l| l.to_text().is_empty()), "{note:?}");
            assert_eq!(w.title(), "", "{note:?}");
            assert!(w.into_chunks().is_empty(), "{note:?}");
        }
    }

    // Review 5, item 5: brackets in a wikilink's text are escaped when it is
    // rendered as a markdown link.
    #[test]
    fn brackets_in_a_wikilink_alias_are_escaped_when_rendered() {
        let at = VaultPath::new("folder/note.md");
        let a = VaultPath::note_path_from("a");
        // (`[[a|b]c]]` is text — review 7, item 2.)
        for (note, expected) in [
            ("[[a|b[c]]", format!("[b\\[c]({a})")),
            ("[[a|b\\c]]", format!("[b\\\\c]({a})")),
        ] {
            let (md, listed) = walk(note).render_markdown(note, &at);
            assert_eq!(md, expected, "{note:?}");
            assert_eq!(raw_links(&listed), [a.to_string()], "{note:?}");
            assert_eq!(rendered_dests(&md), [a.to_string()], "{note:?}");
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), [a.to_string()], "{note:?}");
        }
    }

    // Review 5, item 6: whether an inline link keeps its written form comes
    // from pulldown's title and the `<…>` after `](`, not a rebuilt label.
    #[test]
    fn a_label_across_lines_does_not_stop_the_rewrite() {
        let at = VaultPath::new("/dir/n.md");
        let note = "[a\n  ](sub/x.md)";
        let (md, _) = walk(note).render_markdown(note, &at);
        let x = VaultPath::new("/dir/sub/x.md");
        assert_eq!(rendered_dests(&md), [x.to_string()], "{md:?}");
        for note in [
            "[t](x.md \"Title\")",
            "[x](<my note.md>)",
            "[x]( <my note.md>)",
            "[a\n  ](<b c.md>)",
        ] {
            assert_eq!(walk(note).render_markdown(note, &at).0, note);
        }
    }

    // Review 5 fix round 1, item 1: inline markup in the label does not stop
    // the editor's pattern (as 50dcb026 listed them).
    #[test]
    fn a_spaced_destination_link_with_markup_in_its_label_is_a_link_everywhere() {
        let at = VaultPath::new("n.md");
        for (note, dest) in [
            ("see [**David** H](a b.md) x", "a b.md"),
            ("see [*x*](a b.md) x", "a b.md"),
            ("see [a `c`](d e.md) x", "d e.md"),
            ("see [a <b>x</b>](c d.md) x", "c d.md"),
        ] {
            let path = VaultPath::new(dest);
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), [path.to_string()], "{note:?}");
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(raw_links(&listed), [path.to_string()], "{note:?}");
            assert_eq!(rendered_dests(&md), [path.to_string()], "{note:?}: {md}");
            assert_eq!(crate::note::note_link_targets(note), [dest], "{note:?}");
        }
    }

    // Pinned: the pattern inside a code span is code; next to inline HTML
    // tags (not inside one) it is prose.
    #[test]
    fn a_spaced_destination_link_in_a_code_span_or_between_html_tags() {
        assert_no_link_anywhere("a `[x](a b.md)` b\n");
        assert_no_link_anywhere("a `x [y](a b.md) z` b\n");
        let note = "<b>[x](a b.md)</b>\n";
        let at = VaultPath::new("n.md");
        let ab = VaultPath::new("a b.md");
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert_eq!(raw_links(&index), [ab.to_string()]);
        let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(raw_links(&listed), [ab.to_string()]);
        assert_eq!(md, format!("<b>[x](<{ab}>)</b>\n"));
        assert_eq!(crate::note::note_link_targets(note), ["a b.md"]);
    }

    // Review 5 fix round 1, item 2: a found pattern that links nowhere is
    // left as written — and, since review 10 item 6, listed by no view.
    #[test]
    fn a_spaced_destination_that_resolves_nowhere_is_left_as_written() {
        let at = VaultPath::new("n.md");
        for note in ["[x](a b.md \"T\")", "[a](https://x.y/a b)"] {
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(md, note);
            assert!(listed.is_empty(), "{note:?}: {listed:?}");
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert!(index.is_empty(), "{note:?}: {index:?}");
            assert!(crate::note::note_link_targets(note).is_empty());
        }
    }

    // Review 5 fix round 1, item 3: a code block's text is trimmed of any
    // whitespace, as before.
    #[test]
    fn a_code_block_is_trimmed_of_unicode_whitespace() {
        for ws in ['\u{a0}', '\u{3000}'] {
            let note = format!("```\n{ws}x{ws}\n```\n");
            let chunks = crate::note::content_extractor::get_content_chunks(&note);
            assert_eq!(chunks[0].text, "```\nx\n```", "{ws:?}");
        }
    }

    // Review 5 fix round 2: the editor's pattern finds a link only where
    // pulldown read none — never a second record of a parsed link, never
    // across lines, in document order.
    #[test]
    fn a_found_link_never_repeats_or_reorders_a_parsed_one() {
        let at = VaultPath::new("n.md");
        let note_path = |t: &str| VaultPath::note_path_from(t).to_string();
        let path = |t: &str| VaultPath::new(t).to_string();
        for (note, targets, labels, listed) in [
            (
                "- [ ] **Call [Bob](bob.md)** today",
                vec!["bob.md"],
                vec!["Bob"],
                vec![path("bob.md")],
            ),
            (
                "[a] **b [c](d.md) z**",
                vec!["d.md"],
                vec!["c"],
                vec![path("d.md")],
            ),
            (
                "[a] *see [[w]] and [c](d.md)*",
                vec!["w", "d.md"],
                vec!["w", "c"],
                vec![note_path("w"), path("d.md")],
            ),
            (
                "See [ref] *a\n[Doc](My Doc.md)*",
                vec!["My Doc.md"],
                vec!["Doc"],
                vec![path("My Doc.md")],
            ),
            // The label from the first `[`, as before the walk (review 7,
            // item 3: the editor's pattern over the whole block).
            (
                "[ **a\n[y](e f.md)**",
                vec!["e f.md"],
                vec![" **a\n[y"],
                vec![path("e f.md")],
            ),
        ] {
            let w = walk(note);
            let got: Vec<&str> = w.links.iter().map(|l| l.label.as_str()).collect();
            assert_eq!(got, labels, "{note:?}");
            assert_eq!(crate::note::note_link_targets(note), targets, "{note:?}");
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), listed, "{note:?}");
            let (_, md_links) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(raw_links(&md_links), listed, "{note:?}");
        }
    }

    #[test]
    fn a_found_pattern_inside_a_code_span_or_escaped_is_no_link() {
        assert_no_link_anywhere("[a] **b\n`[x](y z.md)`**\n");
        assert_no_link_anywhere("\\[x](a b.md)\n");
    }

    // Review 6, item 1: an image the editor's pattern finds where pulldown
    // read none (a destination with spaces) is an image like any other:
    // listed by the CLI and `NoteMetadata`, never indexed, left as written
    // in the rendered markdown and not in its link list; a hashtag in its
    // alt text is not a tag. Found links' exclusions hold.
    #[test]
    fn an_image_with_spaces_in_its_destination_is_listed_like_any_image() {
        let at = VaultPath::new("n.md");
        for (note, targets) in [
            (
                "see ![shot](assets/my image.png) and ![p](p.png)",
                vec!["assets/my image.png", "p.png"],
            ),
            ("![a #t](my pic.png) end", vec!["my pic.png"]),
            ("![z `k l`](e f.png)", vec!["e f.png"]),
        ] {
            assert_eq!(crate::note::note_link_targets(note), targets, "{note:?}");
            assert_eq!(crate::note::NoteMetadata::of(note).links, targets);
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert!(index.is_empty(), "{note:?}: {index:?}");
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(md, note);
            assert!(listed.is_empty(), "{note:?}: {listed:?}");
            assert!(crate::note::note_tags(note).is_empty(), "{note:?}");
        }
        for note in [
            "`![y](c d.png)`\n",
            "\\![x](a b.png)\n",
            "<div>\n![x](a b.png)\n</div>\n",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // Review 6, item 4: the item-end trim is for the item's own prose line;
    // an HTML line inside a tight item keeps its line break.
    #[test]
    fn html_in_a_tight_list_item_keeps_its_line_break() {
        let chunks = crate::note::content_extractor::get_content_chunks(
            "- a\n- <Screen to the terminal>\n- b",
        );
        assert_eq!(chunks[0].text, "* a\n* \n<Screen to the terminal>\n\n* b");
    }

    // Review 6, item 6: a title is trimmed as its line is — ASCII
    // whitespace only, so a no-break space at its end stays, as before.
    #[test]
    fn a_title_is_trimmed_like_its_line() {
        for (note, title) in [
            ("# Title\u{a0}\nbody", "Title\u{a0}"),
            ("Title\u{a0}\nbody", "Title\u{a0}"),
            ("#   Title  \nbody", "Title"),
            ("  Title  \nbody", "Title"),
        ] {
            let w = walk(note);
            assert_eq!(w.title(), title, "{note:?}");
            if let Some(heading) = w.headings(note).first() {
                assert_eq!(heading.text, title, "{note:?}");
            }
        }
    }

    // Review 6, item 8 (spec row): a `#` right after `&`, as in an HTML
    // entity, is no tag anywhere; other hashtags are untouched.
    #[test]
    fn a_hash_after_an_ampersand_is_not_a_tag() {
        let at = VaultPath::new("n.md");
        let note = "it&#39;s Title&#32; a #tag &x #tag2";
        assert_eq!(tag_list(&walk(note)), ["tag", "tag2"]);
        assert_eq!(crate::note::note_tags(note), ["tag", "tag2"]);
        assert_eq!(crate::note::extract_labels(note), ["tag", "tag2"]);
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert_eq!(raw_links(&index), ["#tag", "#tag2"]);
        let (md, _) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(md, "it&#39;s Title&#32; a [#tag](#tag) &x [#tag2](#tag2)");
    }

    // Review 6, item 2 (spec row): a wikilink inside inline HTML or as a
    // reference definition's destination is no link anywhere.
    #[test]
    fn a_wikilink_in_inline_html_or_a_definition_destination_is_not_a_link() {
        for note in [
            "a <!-- [[x]] --> b\n",
            "<a title=\"[[x]]\">t</a> b\n",
            "[r]: [[x]]\n",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // Review 6, item 3 (spec row): a wikilink to a name with spaces renders
    // as a link a CommonMark previewer reads.
    #[test]
    fn a_wikilink_to_a_name_with_spaces_renders_as_a_real_link() {
        let note = "[[DEM Platform]]";
        let (md, _) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, "[DEM Platform](<dem platform.md>)");
    }

    // Review 6, item 7 (spec row): an escaped hashtag loses its backslash
    // in the plain chunk text and is still a tag.
    #[test]
    fn an_escaped_hashtag_is_plain_text_and_still_a_tag() {
        let note = "\\#esc #ok";
        let chunks = crate::note::content_extractor::get_content_chunks(note);
        assert_eq!(chunks[0].text, "esc ok");
        assert_eq!(crate::note::note_tags(note), ["esc", "ok"]);
    }

    // Review 7, crash: pulldown-cmark 0.13.4's wikilink extension panicked
    // on these (an `![[` opener completed as an ordinary image or link before
    // a later `]]`). The extension is no longer used (review 8): each is
    // plain CommonMark — an image whose text holds a link, then text — with
    // the chunk text 50dcb026 gave.
    const PULLDOWN_CRASH_INPUTS: [&str; 4] = [
        "![[a](b) c](d)]]",
        "See ![[img](a.png)](b.md)]] here",
        "![[!](x)](x)]]",
        "![[]<](x)]]",
    ];

    #[test]
    fn inputs_that_crashed_pulldown_wikilinks_read_as_commonmark() {
        let expected: [(&str, &[&str]); 4] = [
            ("a c]]", &["d", "b"]),
            ("See img]] here", &["b.md", "a.png"]),
            ("!]]", &["x", "x"]),
            ("[]<]]", &["x"]),
        ];
        for (note, (chunk_text, targets)) in PULLDOWN_CRASH_INPUTS.into_iter().zip(expected) {
            let w = walk(note);
            assert_eq!(
                (text(&w).as_str(), w.link_targets()),
                (chunk_text, targets.iter().map(|t| t.to_string()).collect()),
                "{note:?}"
            );
            let _ = w.render_markdown(note, &VaultPath::new("n.md"));
            let heading =
                crate::note::content_extractor::heading_display_text(&format!("# {note}"));
            assert_eq!(heading.as_deref(), Some(chunk_text), "{note:?}");
        }
    }

    /// An embed as written is a wikilink; an `![[` that is no wikilink, in
    /// prose or code, keeps the note's text.
    #[test]
    fn embeds_and_broken_embeds_keep_the_notes_text() {
        let note = "![[pic.png|200]] `![[a` ![[b](c)\n\n```\n![[x]y\n```\n";
        let w = walk(note);
        assert_eq!(links(&w)[0], (WalkLinkKind::WikiEmbed, "pic.png", "200"));
        assert_eq!(text(&w), "200 `![[a` ![b\n```\n![[x]y\n```");
    }

    /// Walks `n` random notes per seed over a markup alphabet through every
    /// view and `heading_display_text`: no panic, no input over the time
    /// cap, and the same note in CRLF gives the same output.
    fn fuzz_walk(seeds: &[u64], n: usize) {
        const ALPHABET: [&str; 21] = [
            "[", "]", "(", ")", "!", "|", "#", "<", ">", "\\", "`", "a", " ", "\n", "[[", "]]",
            "![[", "](", "](x)", "|]]", "- ",
        ];
        let cap = std::time::Duration::from_millis(50);
        for &seed in seeds {
            let mut next = rng(seed);
            for _ in 0..n {
                let note: String = (0..1 + next() % 24)
                    .map(|_| ALPHABET[next() % ALPHABET.len()])
                    .collect();
                walk_every_view_within(&note, cap);
                let _ = crate::note::content_extractor::heading_display_text(&format!("#{note}"));
                let crlf = note.replace('\n', "\r\n");
                assert_eq!(every_view(&crlf), every_view(&note), "{note:?}");
            }
        }
    }

    #[test]
    fn random_markup_walks_without_panicking() {
        fuzz_walk(&[0x9E37_79B9_7F4A_7C15], 8_000);
    }

    /// The proof run: `cargo test -p kimun_core --release --lib
    /// random_markup_walks_at_length -- --ignored`.
    #[test]
    #[ignore = "slow: 300k notes per seed"]
    fn random_markup_walks_at_length() {
        fuzz_walk(&[0x9E37_79B9_7F4A_7C15, 7, 11, 13], 300_000);
    }

    /// Everything a note's extractors give, as text: for comparing the same
    /// note in LF and CRLF. The rendered markdown keeps the note's own line
    /// endings outside what it rewrites, so it is compared with them as LF.
    fn every_view(note: &str) -> String {
        use crate::note::{NoteDetails, NoteMetadata};
        let details = NoteDetails::new(&VaultPath::new("d/n.md"), note);
        let (md, md_links) = details.get_markdown_and_links();
        format!(
            "{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}",
            details.get_title(),
            details.get_content_chunks(),
            details.get_chunks_and_links(),
            NoteMetadata::of(note),
            crate::note::note_tags(note),
            crate::note::extract_labels(note),
            md.replace("\r\n", "\n"),
            md_links,
        )
    }

    // Review 7, item 1: CRLF notes read exactly as LF notes, with and
    // without frontmatter.
    #[test]
    fn a_crlf_note_reads_exactly_as_the_same_note_in_lf() {
        let body = "# Title [[a|x\ny]]\n\nx `code\nspan` y [[a|dis\nplay]] #tag\n\n\
                    <div>\nhtml\n</div>\n\n<pre>\na\n\nb\n</pre>\n\n\
                    [l\nm](n.md) a <b\nc=\"d\">x [p\nq](a b.md)\n\n```\nc\r\n```\n\n    i\n    j\n\n## End\n";
        for note in [body.to_string(), format!("---\ntitle: x\n---\n{body}")] {
            let note = note.replace("\r\n", "\n");
            let crlf = note.replace('\n', "\r\n");
            assert_eq!(every_view(&crlf), every_view(&note), "{note:?}");
        }
    }

    // Review 8, fuzz: a code span over lines whose last line is only a
    // container prefix (`>`, skipped by pulldown) reads the same in CRLF.
    #[test]
    fn a_code_span_ending_on_a_quote_marker_reads_the_same_in_crlf() {
        for note in [">(\n]`\na b\n>`|", "> x `\n> a\n> `y"] {
            let crlf = note.replace('\n', "\r\n");
            assert_eq!(every_view(&crlf), every_view(note), "{note:?}");
        }
    }

    // Review 7, item 2: a `]` between `[[` and `]]` makes it text, as the
    // editor reads it — what pulldown reads without wikilinks.
    #[test]
    fn a_wikilink_holding_a_closing_bracket_is_text() {
        for (note, chunk_text) in [
            ("[[a|b]c]] x", "[[a|b]c]] x"),
            ("[[a]b]] x", "[[a]b]] x"),
            ("![[a]b]] x", "![[a]b]] x"),
            ("[[a]|]] x", "[[a]|]] x"),
        ] {
            assert_no_link_anywhere(note);
            assert_eq!(text(&walk(note)), chunk_text, "{note:?}");
        }
        // Plain text to CommonMark too: its markup renders (as 50dcb026
        // gave); a hashtag in it is a tag, as in any text.
        let note = "[[a|*b* #t]c]]";
        let w = walk(note);
        assert_eq!(text(&w), "[[a|b t]c]]");
        assert!(w.links.is_empty());
        assert_eq!(w.tag_names(), ["t"]);
    }

    // Review 7, item 3: a spaced-destination link whose label wraps is
    // found (the editor's pattern over the whole block, as before the walk);
    // a `]` and `(` split over two lines is no link, nor is one spanning a
    // nested list item.
    #[test]
    fn a_spaced_destination_link_with_a_wrapped_label_is_a_link() {
        let at = VaultPath::new("n.md");
        for (note, label) in [
            ("p [x\ny](a b.md) q", "x\ny"),
            ("> p [x\n> y](a b.md) q", "x\n> y"),
        ] {
            let w = walk(note);
            assert_eq!(
                links(&w),
                [(WalkLinkKind::Found, "a b.md", label)],
                "{note:?}"
            );
            assert_eq!(w.link_targets(), ["a b.md"], "{note:?}");
            let (md, _) = w.render_markdown(note, &at);
            assert!(md.contains("](<a b.md>)"), "{md:?}");
        }
        for note in [
            "p [a]\n(b c.md) q",
            "- [a\n  - b](c d.md)",
            "p [x](a\nb c.md) q",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // Review 7, item 4: an inline link whose destination links nowhere is
    // left as written, like a found one (real note: Jira Reference.md).
    #[test]
    fn an_inline_link_that_resolves_nowhere_is_left_as_written() {
        let note = "[401 Guide]([https://x.y/a](https://x.y/a)) and [ok](b.md)";
        let (md, listed) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(
            md,
            "[401 Guide]([https://x.y/a](https://x.y/a)) and [ok](b.md)"
        );
        assert_eq!(raw_links(&listed), ["b.md"]);
    }

    // Review 8: wikilinks are Kimün's own (the editor's pattern over a plain
    // CommonMark parse), not pulldown-cmark's extension.

    /// Every view of `note`, timed: panics past `limit`.
    fn walk_every_view_within(note: &str, limit: std::time::Duration) {
        let started = std::time::Instant::now();
        let path = VaultPath::new("n.md");
        let w = walk(note);
        let _ = (w.headings(note), w.link_targets(), w.index_links(&path));
        let _ = (w.render_markdown(note, &path), w.title(), w.into_chunks());
        let elapsed = started.elapsed();
        assert!(elapsed < limit, "{elapsed:?} for {note:?}");
    }

    // Finding 1: pulldown 0.13.4 re-emitted events ~2^N after N `[[a|]]`.
    #[test]
    fn many_empty_display_wikilinks_in_one_paragraph_walk_fast() {
        let limit = std::time::Duration::from_millis(100);
        let note = "[[a|]] ".repeat(30);
        walk_every_view_within(&note, limit);
        assert_eq!(walk(&note).link_targets().len(), 30);
        for unit in ["[x ![[a]]](y.md) ", "[![[p.png]]](u.md) "] {
            walk_every_view_within(&unit.repeat(20), limit);
        }
    }

    // Finding 2: links after an empty display part were lost and the text
    // after it repeated.
    #[test]
    fn links_after_an_empty_display_part_are_kept() {
        for (note, targets, chunk) in [
            (
                "see [[a|]] and [[b|]] and [[c]]",
                vec!["a", "b", "c"],
                "see  and  and c",
            ),
            ("> [[a|]] q\n> b [[c]]", vec!["a", "c"], "q\nb c"),
            ("*em [[a|]]* tail [[f]]", vec!["a", "f"], "em  tail f"),
            // A wikilink inside a link's text is that link's text (the
            // editor's rule): the link is `y.md`.
            (
                "[x [[a|]]](y.md) #t [[z]]",
                vec!["y.md", "z"],
                "x [[a|]] t z",
            ),
        ] {
            let w = walk(note);
            assert_eq!(w.link_targets(), targets, "{note:?}");
            assert_eq!(text(&w), chunk, "{note:?}");
        }
    }

    // Finding 3: an embed inside a link's text is that link's text, as
    // CommonMark (and the editor) reads it.
    #[test]
    fn an_embed_inside_a_link_is_link_text() {
        let note = "See [x ![[a]]](y.md) and [![[pic.png|200]]](https://x.com) end";
        let w = walk(note);
        assert_eq!(w.link_targets(), ["y.md", "https://x.com"]);
        assert_eq!(text(&w), "See x ![[a]] and ![[pic.png|200]] end");
        let (md, _) = w.render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(
            md,
            "See [x ![[a]]](y.md) and [![[pic.png|200]]](https://x.com) end"
        );
    }

    // Finding 4: an image whose alt text starts with `[` is an image.
    #[test]
    fn an_image_whose_alt_starts_with_a_bracket_is_an_image() {
        let w = walk("See ![[1] diagram](img.png) here");
        assert_eq!(links(&w), [(WalkLinkKind::Image, "img.png", "[1] diagram")]);
        assert_eq!(text(&w), "See [1] diagram here");
    }

    // Finding 6: a label spliced into the rendered markdown keeps the
    // note's bytes; the listed link text is LF.
    #[test]
    fn a_crlf_label_is_spliced_as_written_and_listed_as_lf() {
        let note = "[b\r\nc](sub/a.md) and [[a|d\r\ne]]\r\n";
        let (md, listed) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        let a = VaultPath::note_path_from("a");
        let sub = VaultPath::new("sub/a.md");
        assert_eq!(md, format!("[b\r\nc]({sub}) and [d\r\ne]({a})\r\n"));
        let texts: Vec<&str> = listed.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["b\nc", "d\ne"]);
    }

    // Finding 7: a line keeps its edges as pulldown reads them; only the
    // whitespace a wikilink's display text leaves at a line's edge goes, as
    // when wikilinks were collapsed before parsing. Headings and titles are
    // trimmed (spec rows).
    #[test]
    fn only_whitespace_left_by_a_wikilink_is_trimmed_from_a_line() {
        let w = walk("![](img.png) [t](x.md \"T\") (...)");
        assert_eq!(text(&w), " t (...)");
        assert_eq!(w.title(), "t (...)");
        for (note, chunk) in [
            ("see [[a|]]", "see"),
            ("[[a| b]] c", "b c"),
            ("x [[a|b ]]\ny", "x b\ny"),
            ("- [[a|]] x\n  y [[b| ]]", "* x y"),
        ] {
            assert_eq!(text(&walk(note)), chunk, "{note:?}");
        }
    }

    // Finding 9: each text block reads only its own text, so a nested list
    // is not read again for every level around it.
    #[test]
    fn each_text_block_scans_only_its_own_text() {
        let mut note = String::new();
        for depth in 0..40 {
            note.push_str(&format!(
                "{}- [[n{depth}]] [x](a b.md)\n",
                "  ".repeat(depth)
            ));
        }
        assert!(
            scanned_bytes(&note) <= note.len(),
            "{}",
            scanned_bytes(&note)
        );
        let w = walk(&note);
        let wikis = w
            .links
            .iter()
            .filter(|l| l.kind == WalkLinkKind::Wiki)
            .count();
        assert_eq!(wikis, 40);
        let starts: Vec<usize> = w.links.iter().map(|l| l.range.start).collect();
        assert!(starts.windows(2).all(|p| p[0] < p[1]), "{starts:?}");
    }

    // A wikilink's brackets must be prose; what pulldown reads between
    // them is its text, shown as written (as 50dcb026 and the editor read
    // it).
    #[test]
    fn code_html_or_an_autolink_between_a_wikilinks_brackets_is_its_text() {
        let a = VaultPath::note_path_from("a");
        for (note, chunk, targets, md) in [
            (
                "[[a|b `c` d]] x",
                "b `c` d x",
                vec!["a"],
                format!("[b `c` d]({a}) x"),
            ),
            (
                "[[a|<b>x</b>]]",
                "<b>x</b>",
                vec!["a"],
                format!("[<b>x</b>]({a})"),
            ),
            // Review 9, item 3: an autolink is the wikilink's text too — not
            // recorded, and escaped when rendered so the result is one link.
            (
                "[[a|see <https://e.f>]]",
                "see <https://e.f>",
                vec!["a"],
                format!("[see \\<https://e.f>]({a})"),
            ),
            (
                "[[a|<https://x.y>]] [[a|<m@x.y> `<https://c.d>`]]",
                "<https://x.y> <m@x.y> `<https://c.d>`",
                vec!["a", "a"],
                format!("[\\<https://x.y>]({a}) [\\<m@x.y> `<https://c.d>`]({a})"),
            ),
        ] {
            let w = walk(note);
            assert_eq!(text(&w), chunk, "{note:?}");
            assert_eq!(w.link_targets(), targets, "{note:?}");
            assert_eq!(w.render_markdown(note, &VaultPath::new("n.md")).0, md);
        }
        for note in [
            "`[[a]]`",
            "a `b [[c` d]]",
            "<span title=\"[[y|z\">a</span>]]",
        ] {
            assert_no_link_anywhere(note);
        }
    }

    // Review 8, vault probe: pulldown's ranges for a tab-indented list
    // overlap (the list starts inside the heading line before it, sibling
    // items share a byte); every link is still read once.
    #[test]
    fn a_tab_indented_nested_list_is_read_once() {
        let note = "- # Team\n\t- [[A]]\n\t- [[T]]\n\t- [[K]] [n](https://x.y)\n";
        let w = walk(note);
        assert_eq!(w.link_targets(), ["A", "T", "K", "https://x.y"]);
        assert_eq!(text(&w), "* \n# Team\n* A\n* T\n* K n");
    }

    /// The walk's wikilinks on each line of `note` equal what the editor's
    /// pattern highlights on that line (it reads line by line), less what
    /// is not prose: a `[[` outside a text event (a reference definition),
    /// one whose `[[` or `]]` lies in a link, image, code span, fenced code
    /// block or HTML pulldown read — not a link that is exactly its inner
    /// `[…]` (`[[docs]]` with `[docs]: …` defined) — an escaped `[[`, and a
    /// wikilink without a target (`[[|b]]`). An indented code block is read
    /// (the editor highlights wikilinks there; fenced code it shows raw). A
    /// wikilink across lines is the walk's alone (spec row).
    fn assert_editor_agreement(note: &str) {
        let parse: Vec<(Event, Range<usize>)> = Parser::new_ext(note, Options::empty())
            .into_offset_iter()
            .collect();
        let opaque: Vec<&Range<usize>> = parse
            .iter()
            .filter(|(e, _)| {
                matches!(
                    e,
                    Event::Start(
                        Tag::Link { .. }
                            | Tag::Image { .. }
                            | Tag::CodeBlock(CodeBlockKind::Fenced(_))
                            | Tag::HtmlBlock
                    ) | Event::Code(_)
                        | Event::InlineHtml(_)
                        | Event::Html(_)
                )
            })
            .map(|(_, r)| r)
            .collect();
        let in_text = |at: usize| {
            parse
                .iter()
                .any(|(e, r)| matches!(e, Event::Text(_)) && r.start <= at && at < r.end)
        };
        // The editor highlights line by line.
        let mut at = 0;
        let mut highlighted = Vec::new();
        for row in note.split('\n') {
            let spans = crate::note::scan::wikilink_char_spans(row).into_iter();
            // ASCII alphabet: chars are bytes.
            highlighted.extend(spans.map(|s| at + s.start..at + s.end));
            at += row.len() + 1;
        }
        let expected: Vec<Range<usize>> = highlighted
            .into_iter()
            .filter(|s| {
                let before = &note[..s.start];
                let escaped = (before.len() - before.trim_end_matches('\\').len()) % 2 == 1;
                let target = wikilink_parts(&note[s.start + 2..s.end - 2]).0;
                let read =
                    |r: Range<usize>| opaque.iter().any(|o| o.start < r.end && r.start < o.end);
                let inner_link = opaque.iter().any(|o| **o == (s.start + 1..s.end - 1));
                in_text(s.start)
                    && !escaped
                    && !target.is_empty()
                    && (inner_link || !read(s.start..s.start + 2) && !read(s.end - 2..s.end))
            })
            .collect();
        let walked: Vec<Range<usize>> = walk(note)
            .links
            .iter()
            .filter_map(|l| match l.kind {
                WalkLinkKind::Wiki => Some(l.range.clone()),
                WalkLinkKind::WikiEmbed => Some(l.range.start + 1..l.range.end),
                _ => None,
            })
            .filter(|r| !note[r.clone()].contains('\n'))
            .collect();
        assert_eq!(walked, expected, "{note:?}");
    }

    #[test]
    fn the_walk_reads_a_wikilink_where_the_editor_highlights_one() {
        for line in [
            "a [[b]] c",
            "![[e]] and \\![[f]] and \\[[g]]",
            "[[a|]] [[|b]] [[]] [[a|b]c]]",
            "`[[c]]` <b>[[d]]</b> <!-- [[x]] --> [[a|b `c` <i>d</i> <http://e.f>]]",
            "[x [[a]]](y.md) [[a]](x.md) [[[tri]]]",
            "    [[code]]",
            "```\n[[fenced]]\n```",
            "See [[docs]]\n\n[docs]: https://d.e",
            "# [[h]] #",
            "- [[i]]",
            "[r]: [[x]]",
            "a [[b\nc [[d]] e",
        ] {
            assert_editor_agreement(line);
        }
    }

    // Review 9, item 1: a line grows in place — an append copies only the
    // appended text — so one long line builds in linear time. It was
    // quadratic: a 1.1 MB line of `[[a]](b) ` took 2.6 s to index in release.
    #[test]
    fn a_long_line_grows_in_linear_time() {
        let started = std::time::Instant::now();
        let mut lines = TextLines::default();
        for _ in 0..400_000 {
            lines.push(Event::Text("ab".into()), 0, str::to_string);
        }
        let lines = lines.finish();
        let elapsed = started.elapsed();
        assert!(matches!(&lines[..], [TextLine::Text(t)] if t.len() == 800_000));
        assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
    }

    // Review 9, item 4: a `[[` that opens no wikilink on its own line — in
    // code, escaped, or stray — does not swallow one on a later line: the
    // walk reads the wikilink the editor highlights there.
    #[test]
    fn a_stray_double_bracket_does_not_swallow_a_later_lines_wikilink() {
        for (note, chunk) in [
            (
                "Use `[[` to start a link.\nSee [[Note]].",
                "Use `[[` to start a link.\nSee Note.",
            ),
            (
                "Type [[ then pick.\nSee [[Note]].",
                "Type [[ then pick.\nSee Note.",
            ),
            (
                "Escaped \\[[ here\nSee [[Note]].",
                "Escaped [[ here\nSee Note.",
            ),
            (
                "Escaped \\\\[[ here\nSee [[Note]].",
                "Escaped \\[[ here\nSee Note.",
            ),
            ("[[a|x\ny [[Note]] z", "[[a|x\ny Note z"),
        ] {
            let w = walk(note);
            assert_eq!(w.link_targets(), ["Note"], "{note:?}");
            assert_eq!(text(&w), chunk, "{note:?}");
            assert_editor_agreement(note);
        }
        // A wikilink across lines is still one (spec row) — also when its
        // last line's `[[` opens none in prose (code, escaped): then the
        // outer match is judged on its own, as 50dcb026 read it.
        let a = VaultPath::note_path_from("a");
        let at = VaultPath::new("n.md");
        for note in [
            "[[a|x\ny]] z",
            "[[a|see the\n`[[` syntax]]",
            "[[a|x\ny \\[[ z]]",
        ] {
            let w = walk(note);
            assert_eq!(w.link_targets(), ["a"], "{note:?}");
            let (md, listed) = w.render_markdown(note, &at);
            assert!(md.contains(&format!("]({a})")), "{md:?}");
            assert_eq!(raw_links(&listed), [a.to_string()], "{note:?}");
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), [a.to_string()], "{note:?}");
            assert_editor_agreement(note);
        }
        assert_eq!(walk("[[a|x\ny [[b]]").link_targets(), ["b"]);
    }

    // Review 9, item 5 (spec row): the editor's pattern ends a destination
    // at its first `)`, so it cannot read a spaced destination holding a
    // `(` reliably: no link.
    #[test]
    fn a_spaced_destination_with_a_parenthesis_is_no_link() {
        for note in ["[notes](My Notes (draft).md)", "![s](my (1).png) x"] {
            assert_no_link_anywhere(note);
            assert_eq!(text(&walk(note)), note);
        }
    }

    // Review 9, pin (spec row): an escaped bang before a wikilink is a
    // literal `!`; the wikilink is a link everywhere.
    #[test]
    fn a_wikilink_after_an_escaped_bang_is_a_link() {
        let note = "x \\![[a]] y";
        let at = VaultPath::new("n.md");
        let w = walk(note);
        assert_eq!(links(&w), [(WalkLinkKind::Wiki, "a", "a")]);
        assert_eq!(text(&w), "x !a y");
        let a = VaultPath::note_path_from("a");
        let (md, listed) = w.render_markdown(note, &at);
        assert_eq!(md, format!("x \\![a]({a}) y"));
        assert_eq!(raw_links(&listed), [a.to_string()]);
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert_eq!(raw_links(&index), [a.to_string()]);
        assert_eq!(crate::note::note_link_targets(note), ["a"]);
    }

    // Review 9, pin (spec row): a hashtag inside reference-link text is link
    // text — no tag anywhere.
    #[test]
    fn a_hashtag_in_reference_link_text_is_no_tag() {
        let at = VaultPath::new("n.md");
        for note in ["see [about #tag][r]\n\n[r]: x.md", "[#tag]\n\n[#tag]: x.md"] {
            let w = walk(note);
            assert!(w.tags.is_empty(), "{note:?}");
            assert_eq!(w.link_targets(), ["x.md"], "{note:?}");
            let (md, listed) = w.render_markdown(note, &at);
            assert_eq!(md, note);
            assert_eq!(raw_links(&listed), ["x.md"], "{note:?}");
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), ["x.md"], "{note:?}");
            assert!(crate::note::note_tags(note).is_empty(), "{note:?}");
            assert!(crate::note::extract_labels(note).is_empty(), "{note:?}");
            assert!(
                crate::note::NoteMetadata::of(note).tags.is_empty(),
                "{note:?}"
            );
        }
    }

    /// A seeded xorshift stream.
    fn rng(seed: u64) -> impl FnMut() -> usize {
        let mut x = seed;
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as usize
        }
    }

    /// `n` random one-line notes per seed through the editor-agreement
    /// property.
    fn fuzz_editor_agreement(seeds: &[u64], n: usize) {
        const ALPHABET: [&str; 23] = [
            "[", "]", "(", ")", "!", "|", "#", "<", ">", "\\", "`", "a", " ", "[[", "]]", "![[",
            "](", "](x)", "|]]", "*", "\n", "`[[`", "\\[[",
        ];
        for &seed in seeds {
            let mut next = rng(seed);
            for _ in 0..n {
                let line: String = (0..1 + next() % 24)
                    .map(|_| ALPHABET[next() % ALPHABET.len()])
                    .collect();
                assert_editor_agreement(&line);
            }
        }
    }

    #[test]
    fn random_lines_agree_with_the_editor() {
        fuzz_editor_agreement(&[1, 2], 4_000);
    }

    /// `cargo test -p kimun_core --release --lib
    /// random_lines_agree_with_the_editor_at_length -- --ignored`.
    #[test]
    #[ignore = "slow: 300k lines per seed"]
    fn random_lines_agree_with_the_editor_at_length() {
        fuzz_editor_agreement(&[3, 4, 5], 300_000);
    }

    // Review 10, item 1 (spec row): a wikilink whose name matches a
    // reference definition — pulldown reads `[` + the shortcut reference
    // link `[docs]` + `]` — is still the wikilink, and that inner link is
    // not recorded.
    #[test]
    fn a_wikilink_whose_name_matches_a_reference_definition_is_a_link() {
        let at = VaultPath::new("n.md");
        for (note, name, chunk) in [
            (
                "See [[docs]] for details.\n\n[docs]: https://docs.example.com\n",
                "docs",
                "See docs for details.",
            ),
            (
                "See [[Project Plan]] now.\n\n[project plan]: https://x.y\n",
                "Project Plan",
                "See Project Plan now.",
            ),
        ] {
            let path = VaultPath::note_path_from(name);
            let (chunks, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), [path.to_string()], "{note:?}");
            assert_eq!(chunks[0].text, chunk, "{note:?}");
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            let dest = markdown_destination(path.to_string()).unwrap();
            assert!(md.starts_with(&format!("See [{name}]({dest})")), "{md:?}");
            assert_eq!(raw_links(&listed), [path.to_string()], "{note:?}");
            assert_eq!(crate::note::note_link_targets(note), [name], "{note:?}");
        }
    }

    // Review 10, item 1: what must not change — a wikilink inside a real
    // link's text stays link text, and one as an inline link's destination
    // is no link.
    #[test]
    fn a_link_around_a_wikilink_still_wins() {
        let at = VaultPath::new("n.md");
        let note = "[see [[a]]](x.md)";
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert_eq!(raw_links(&index), [VaultPath::new("x.md").to_string()]);
        let note = "[t]([[a]])";
        assert!(walk(note)
            .links
            .iter()
            .all(|l| !matches!(l.kind, WalkLinkKind::Wiki)));
    }

    // Review 10, item 2 (spec row): an indented code block — in practice a
    // Logseq-style outline indented after a heading — is read for
    // wikilinks: they are links everywhere (index, rendered link list,
    // CLI), but the block stays code: its text is kept as written in the
    // chunks and in the rendered markdown, and markdown links and hashtags
    // in it are neither links nor tags. Fenced code is still no link.
    #[test]
    fn a_wikilink_in_an_indented_block_is_a_link_but_stays_code() {
        let at = VaultPath::new("n.md");
        let note = "# Team\n\t- [[Pep]] and [[Ann|A]] ![[pic.png]]\n\t- [x](y.md) #tag\n";
        let pep = VaultPath::note_path_from("Pep");
        let ann = VaultPath::note_path_from("Ann");
        let pic = VaultPath::note_path_from("pic.png");
        let (chunks, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert_eq!(
            raw_links(&index),
            [pep.to_string(), ann.to_string(), pic.to_string()]
        );
        assert_eq!(
            chunks[0].text,
            "```\n- [[Pep]] and [[Ann|A]] ![[pic.png]]\n- [x](y.md) #tag\n```"
        );
        let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(md, note, "code is left as written");
        assert_eq!(raw_links(&listed), [pep.to_string(), ann.to_string()]);
        assert_eq!(
            crate::note::note_link_targets(note),
            ["Pep", "Ann", "pic.png"]
        );
        assert!(walk(note).tags.is_empty());
        assert_no_link_anywhere("# Team\n\n```\n[[Pep]] [x](y.md) #tag\n```\n");
    }

    // Review 10, item 2: an indented block inside a list item is read once,
    // as its own block.
    #[test]
    fn a_wikilink_in_an_indented_block_inside_an_item_is_read_once() {
        let note = "- item [[a]]\n\n          [[b]] [[a]]\n";
        let w = walk(note);
        let targets: Vec<&str> = w.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["a", "b", "a"]);
    }

    // Review 10, item 4 (spec row): a markdown link to a section links to
    // the note — the fragment is stripped for resolving, as the editor
    // follows it — and the rendered markdown keeps it after the resolved
    // path.
    #[test]
    fn a_markdown_link_to_a_section_is_a_link_to_the_note() {
        let at = VaultPath::new("/dir/n.md");
        for (note, path, rendered, written) in [
            (
                "[x](plan.md#goals)",
                "plan.md",
                "[x](plan.md#goals)",
                "plan.md#goals",
            ),
            (
                "[y](my note.md#sec)",
                "my note.md",
                "[y](<my note.md#sec>)",
                "my note.md#sec",
            ),
            (
                "[z](sub/plan.md#My Goals)",
                "/dir/sub/plan.md",
                "[z](</dir/sub/plan.md#My Goals>)",
                "sub/plan.md#My Goals",
            ),
        ] {
            let path = VaultPath::new(path).to_string();
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), std::slice::from_ref(&path), "{note:?}");
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(md, rendered, "{note:?}");
            assert_eq!(raw_links(&listed), [path], "{note:?}");
            assert_eq!(crate::note::note_link_targets(note), [written], "{note:?}");
        }
        // A URL keeps its fragment as part of the URL.
        let note = "[u](https://x.y/a#b)";
        let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(md, note);
        assert_eq!(raw_links(&listed), ["https://x.y/a#b"]);
    }

    // Review 10, item 5 (spec row): a URL with parentheses is rendered
    // whole, wrapped in `<…>`, and reads back as the same URL.
    #[test]
    fn a_url_with_parentheses_is_rendered_whole_and_wrapped() {
        let url = "https://en.wikipedia.org/wiki/Rust_(programming_language)";
        let note = format!("[Rust]({url})");
        let (md, listed) = walk(&note).render_markdown(&note, &VaultPath::new("n.md"));
        assert_eq!(md, format!("[Rust](<{url}>)"));
        assert_eq!(rendered_dests(&md), [url]);
        assert_eq!(raw_links(&listed), [url]);
    }

    // Review 10, item 6 (spec row): the CLI / `NoteMetadata.links` list, as
    // written, exactly the links every other view lists — not a destination
    // that links nowhere.
    #[test]
    fn link_targets_list_only_what_links_somewhere() {
        for junk in ["[t]([[b]])", "[a b](c d.md \"Title\")", "[x](a|b.md)"] {
            assert!(
                crate::note::note_link_targets(junk).is_empty(),
                "{junk:?}: {:?}",
                crate::note::note_link_targets(junk)
            );
            assert!(crate::note::NoteMetadata::of(junk).links.is_empty());
        }
        let note = "[a](a.md) [b](b c.md) [[c#s]] <https://d.e> [f](g.md#h) ![i](j k.png)";
        assert_eq!(
            crate::note::note_link_targets(note),
            ["a.md", "b c.md", "c#s", "https://d.e", "g.md#h", "j k.png"]
        );
    }
}
