//! The one pass over a note every whole-note extractor reads — see
//! `2026-10-06-single-note-walk-design.md`. `walk` parses the note as written
//! once and returns a `NoteWalk` (text lines, links, tags, frontmatter); every
//! whole-note extractor is a view of it. All offsets are byte offsets into
//! the note as given. The line builder (`TextLine`/`TextLines`) lives here too.

use std::ops::Range;

use log::debug;
use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

use super::content_extractor::{
    frontmatter_bounds, frontmatter_delimiter, label_matches_inner, md_link_matches, split_bom,
    split_link_fragment, target_looks_like_image, wikilink_parts,
};
use super::ContentChunk;
use crate::nfs::VaultPath;

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
    /// `TextBlocks::end`). Rendered as a link only when it links somewhere.
    Found,
    /// `[label][ref]`, `[label][]`, `[label]` resolved against a definition.
    Reference,
    /// `<https://…>` or `<mail@…>`.
    Autolink,
    /// `![alt](dest)` and its reference forms, and one pulldown did not
    /// read found with the editor's pattern (see `TextBlocks::end`).
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
    /// of a markdown link or image, an autolink's address.
    pub label: String,
    /// Source bytes of the whole link, in the note.
    pub range: Range<usize>,
    /// An inline link written with a title or a `<…>` destination: the
    /// rendered markdown leaves it as written (neither survives being
    /// re-emitted).
    pub as_written: bool,
}

impl WalkLink {
    /// Whether every view treats this as a link: a wikilink (or embed) only
    /// when the note it points to (see [`Self::wiki_note`]) is a valid vault
    /// path — `[[#tag]]` parses as a wikilink but links nowhere — and any
    /// other only with a destination that is not blank (`[t]()`).
    fn is_link(&self) -> bool {
        match self.kind {
            WalkLinkKind::Wiki | WalkLinkKind::WikiEmbed => {
                let (note, _) = self.wiki_note();
                !note.is_empty() && VaultPath::is_valid(note)
            }
            _ => !self.target.trim().is_empty(),
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

/// A wikilink's record from its source (`[[…]]` or `![[…]]`) starting at
/// byte `start` of the note, and its display text: target and text by
/// `wikilink_parts`. `None` without a target (`[[|b]]`), which pulldown
/// does not read as a wikilink either.
// Out of line: most events start no link, and inlining the link builders
// into `LinkRecorder::step` measurably slowed the walk (criterion, medium).
#[inline(never)]
fn wikilink(src: &str, start: usize) -> Option<(WalkLink, &str)> {
    let (kind, inner) = match src.strip_prefix('!') {
        Some(rest) => (WalkLinkKind::WikiEmbed, rest),
        None => (WalkLinkKind::Wiki, src),
    };
    let inner = inner.strip_prefix("[[")?.strip_suffix("]]")?;
    let (target, label) = wikilink_parts(inner);
    if target.is_empty() {
        return None;
    }
    let link = WalkLink {
        kind,
        target: target.to_string(),
        label: label.to_string(),
        range: start..start + src.len(),
        as_written: false,
    };
    Some((link, label))
}

/// A wikilink's display text as written, line by line, as pulldown reads
/// text over lines: each line after the first without its container prefix
/// (indentation, blockquote `>` markers), no line with its `\r`. The walk
/// puts a line break between them, so a heading ends at the first.
fn display_lines(display: &str) -> impl Iterator<Item = &str> {
    display.split('\n').enumerate().map(|(i, line)| {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if i == 0 {
            line
        } else {
            line.trim_start_matches([' ', '\t', '>'])
        }
    })
}

fn wikilink_kind(event: &Event) -> Option<WalkLinkKind> {
    match event {
        Event::Start(Tag::Link {
            link_type: LinkType::WikiLink { .. },
            ..
        }) => Some(WalkLinkKind::Wiki),
        Event::Start(Tag::Image {
            link_type: LinkType::WikiLink { .. },
            ..
        }) => Some(WalkLinkKind::WikiEmbed),
        _ => None,
    }
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

/// What one event is to the line builder, as a [`LinkRecorder`] saw it.
enum Step<'s> {
    /// Pulldown's own event inside a wikilink: not for the lines.
    InWikilink,
    /// A wikilink's end.
    WikilinkEnd,
    /// A wikilink's start; its display text, as written, follows it.
    Wikilink(&'s str),
    /// Any other event; `in_link` when it is link or image text.
    Other { in_link: bool },
}

/// Records the links of the walk's parse: wikilinks from their source
/// through `wikilink_parts`, markdown links and images with pulldown's
/// destination and their label as written. HTML is opaque: pulldown hands
/// it over unparsed, and nothing inside it is a link.
struct LinkRecorder<'s> {
    /// The parsed text as written: labels and wikilink parts come from it.
    src: &'s str,
    /// Byte of the note `src` starts at.
    offset: usize,
    /// Open non-wiki links and images: text inside them is link text, and
    /// its source is their label.
    open: Vec<OpenLink>,
    /// Inside a wikilink, pulldown's own events are skipped up to its end:
    /// the display text comes from `wikilink_parts` on the source.
    wikilink_depth: u32,
}

impl<'s> LinkRecorder<'s> {
    fn new(src: &'s str, offset: usize) -> Self {
        Self {
            src,
            offset,
            open: Vec::new(),
            wikilink_depth: 0,
        }
    }

    /// Takes one event (`range` in `src`), recording any link it starts or
    /// labels.
    fn step(&mut self, event: &Event, range: &Range<usize>, links: &mut Vec<WalkLink>) -> Step<'s> {
        if self.wikilink_depth > 0 {
            match event {
                Event::Start(_) => self.wikilink_depth += 1,
                Event::End(_) => {
                    self.wikilink_depth -= 1;
                    if self.wikilink_depth == 0 {
                        return Step::WikilinkEnd;
                    }
                }
                _ => {}
            }
            return Step::InWikilink;
        }
        if matches!(event, Event::End(TagEnd::Link | TagEnd::Image)) {
            if let Some(open) = self.open.pop() {
                open.close(links, self.src);
            }
        }
        for open in &mut self.open {
            open.extend(range);
        }
        let in_note = self.offset + range.start..self.offset + range.end;
        if wikilink_kind(event).is_some() {
            let src: &'s str = &self.src[range.clone()];
            self.wikilink_depth = 1;
            return match wikilink(src, in_note.start) {
                Some((link, label)) => {
                    links.push(link);
                    Step::Wikilink(label)
                }
                // Defensive: pulldown reads no wikilink without a target.
                None => Step::Wikilink(""),
            };
        }
        if let Event::Start(Tag::Link { .. } | Tag::Image { .. }) = event {
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
        Step::Other {
            in_link: !self.open.is_empty(),
        }
    }
}

/// A markdown link or image's record (none for an email autolink), its
/// label left for its [`OpenLink`] to fill.
// Out of line, as `wikilink`.
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
        return decoded.replace("\r\n", "\n");
    }
    for (r, name) in &inside {
        tags.push(WalkTag {
            name: name.to_string(),
            range: body_start + r.start..body_start + r.end,
        });
    }
    let source = &body[range.clone()];
    if source.len() != decoded.len() {
        return decoded.replace("\r\n", "\n");
    }
    let mut out = String::with_capacity(source.len());
    let mut last = range.start;
    for (r, _) in &inside {
        out.push_str(&body[last..r.start]);
        out.push_str(&body[r.start + 1..r.end]);
        last = r.end;
    }
    out.push_str(&body[last..range.end]);
    out.replace("\r\n", "\n")
}

/// A paragraph, heading or list item the walk is inside — a block of text,
/// where a markdown link or image pulldown did not read is looked for when
/// it ends (see [`TextBlocks::end`]). Ranges are bytes of the walked body.
struct TextBlock {
    range: Range<usize>,
    /// What pulldown read inside it — links, images, wikilinks, nested
    /// blocks: a found link overlaps none.
    taken: Vec<Range<usize>>,
    /// Its code spans and inline HTML: a found link holds one only in its
    /// text.
    opaque: Vec<Range<usize>>,
}

/// The text blocks the walk is inside, innermost last; `None` for one
/// whose source holds no `](` — no link to find there.
#[derive(Default)]
struct TextBlocks {
    open: Vec<Option<TextBlock>>,
    /// Whether a link was found, so the links need sorting.
    found: bool,
}

impl TextBlocks {
    /// Takes one event (`range` in `body`, the walked text, which starts at
    /// byte `body_start` of the note); a block it ends is looked through
    /// (see [`Self::end`]).
    #[inline]
    fn step(
        &mut self,
        event: &Event,
        range: &Range<usize>,
        body: &str,
        body_start: usize,
        links: &mut Vec<WalkLink>,
        tags: &mut Vec<WalkTag>,
    ) {
        match event {
            Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough) => {}
            Event::Start(tag) => {
                if let Some(Some(block)) = self.open.last_mut() {
                    block.taken.push(range.clone());
                }
                if matches!(tag, Tag::Paragraph | Tag::Heading { .. } | Tag::Item) {
                    // A block's start spans the whole block.
                    let block = body[range.clone()].contains("](").then(|| TextBlock {
                        range: range.clone(),
                        taken: Vec::new(),
                        opaque: Vec::new(),
                    });
                    self.open.push(block);
                }
            }
            Event::Code(_) | Event::InlineHtml(_) => {
                if let Some(Some(block)) = self.open.last_mut() {
                    block.opaque.push(range.clone());
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => {
                if let Some(Some(block)) = self.open.pop() {
                    self.end(block, body, body_start, links, tags);
                }
            }
            _ => {}
        }
    }

    /// A text block ended: the markdown links and images pulldown did not
    /// read in it — the editor's pattern, [`md_link_matches`], line by line
    /// as the editor scans; in practice a destination with spaces,
    /// `[David H](../People/David H.md)`, `![shot](my shot.png)` — are
    /// recorded. A match is kept when it overlaps nothing pulldown read,
    /// holds a code span or inline HTML only in its text, and is not
    /// escaped (`\[x](a b.md)`). A hashtag inside one is link text, not a
    /// tag.
    #[inline(never)]
    fn end(
        &mut self,
        block: TextBlock,
        body: &str,
        body_start: usize,
        links: &mut Vec<WalkLink>,
        tags: &mut Vec<WalkTag>,
    ) {
        let source = &body[block.range.clone()];
        let mut line_start = block.range.start;
        for line in source.split_inclusive('\n') {
            let at_line = line_start;
            line_start += line.len();
            // Only a `](` outside everything pulldown read can end a link
            // to find (a parsed link's own `](` is the common case).
            let open = |(i, _): (usize, &str)| {
                let at = at_line + i;
                !block.taken.iter().any(|r| r.start <= at && at < r.end)
            };
            if !line.match_indices("](").any(open) {
                continue;
            }
            for m in md_link_matches(line) {
                let at = at_line + m.range.start..at_line + m.range.end;
                let label = at_line + m.label.start..at_line + m.label.end;
                let overlaps = |r: &Range<usize>| r.start < at.end && at.start < r.end;
                // Escaped by an odd run of backslashes (`\\[` is a literal
                // backslash followed by a real link).
                let backslashes = line[..m.range.start].len()
                    - line[..m.range.start].trim_end_matches('\\').len();
                if backslashes % 2 == 1
                    || block.taken.iter().any(overlaps)
                    || block
                        .opaque
                        .iter()
                        .any(|r| overlaps(r) && !(label.start <= r.start && r.end <= label.end))
                {
                    continue;
                }
                let in_note = body_start + at.start..body_start + at.end;
                tags.retain(|t| !(in_note.start <= t.range.start && t.range.end <= in_note.end));
                links.push(WalkLink {
                    kind: if m.image {
                        WalkLinkKind::Image
                    } else {
                        WalkLinkKind::Found
                    },
                    target: m.target.to_string(),
                    label: body[label].to_string(),
                    range: in_note,
                    as_written: false,
                });
                self.found = true;
            }
        }
    }
}

/// One pass over `note` as written — see the module docs above.
pub(in crate::note) fn walk(note: &str) -> NoteWalk {
    let (body_start, frontmatter) = body_start(note);
    let body = &note[body_start..];
    let candidates: Vec<(Range<usize>, &str)> = label_matches_inner(body)
        .map(|m| (m.byte_start..m.byte_end, m.name))
        .collect();

    let mut lines = TextLines::default();
    let mut links = Vec::new();
    let mut tags = Vec::new();
    let mut code_depth = 0u32;
    let mut recorder = LinkRecorder::new(body, body_start);
    // No `](` in the body, no link pulldown did not read.
    let mut blocks = body.contains("](").then(TextBlocks::default);

    for (event, range) in Parser::new_ext(body, Options::ENABLE_WIKILINKS).into_offset_iter() {
        let start = body_start + range.start;
        if let Some(blocks) = &mut blocks {
            blocks.step(&event, &range, body, body_start, &mut links, &mut tags);
        }
        let in_link = match recorder.step(&event, &range, &mut links) {
            Step::InWikilink => continue,
            Step::WikilinkEnd => {
                lines.push(event, start, str::to_string);
                continue;
            }
            Step::Wikilink(display) => {
                lines.push(event, start, str::to_string);
                for (i, line) in display_lines(display).enumerate() {
                    if i > 0 {
                        lines.push(Event::SoftBreak, start, str::to_string);
                    }
                    lines.push(Event::Text(CowStr::Borrowed(line)), start, str::to_string);
                }
                continue;
            }
            Step::Other { in_link } => in_link,
        };
        match &event {
            Event::Start(Tag::CodeBlock(_)) => code_depth += 1,
            Event::End(TagEnd::CodeBlock) => code_depth = code_depth.saturating_sub(1),
            _ => {}
        }
        let in_code = code_depth > 0;
        lines.push(event, start, |decoded| {
            if in_code {
                decoded.replace("\r\n", "\n")
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
    // A found link is recorded when its block ends, after the links read
    // inside the block: back in document order.
    if blocks.is_some_and(|b| b.found) {
        links.sort_by_key(|l| l.range.start);
    }

    NoteWalk {
        lines: lines.finish(),
        links,
        tags,
        frontmatter: frontmatter.map_or_else(String::new, |r| frontmatter_text(&note[r])),
    }
}

/// Where a markdown link destination points, as the rendered markdown
/// writes it, plus the link it records: a URL, a note, or another vault
/// path (relative destinations resolve against `ref_path`'s directory).
/// `None` for a destination that is neither a URL nor a valid vault path.
pub(in crate::note) fn resolve_md_link(
    dest: &str,
    label: &str,
    ref_path: &VaultPath,
) -> (String, Option<NoteLink>) {
    if is_remote_url(dest) {
        return (dest.to_string(), Some(NoteLink::url(dest, label)));
    }
    if !VaultPath::is_valid(dest) {
        return (dest.to_string(), None);
    }
    let path = VaultPath::new(dest).resolve_against_note(ref_path);
    (path.to_string(), Some(NoteLink::vault_path(&path, label)))
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
/// link (`[[a|b]c]]` renders `[b\]c](a.md)`).
fn link_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
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
    /// destinations, autolinks. Strings, not resolved paths.
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
                    found.push((link.range.start, NoteLink::note(&path, &link.label)));
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
    /// `WalkLink::rewritten_destination`), and links whose destination
    /// cannot be written back are listed but left as written. A link found
    /// with the editor's pattern (`WalkLinkKind::Found`) is rewritten and
    /// listed like an inline link when it resolves to a link; one that
    /// resolves nowhere (`[x](a b.md "T")`) is left as written and not
    /// listed. Links come in document order, hashtags after them. Everything outside a recorded
    /// range — frontmatter, code, HTML, images — is copied verbatim.
    pub(in crate::note) fn render_markdown(
        &self,
        note: &str,
        ref_path: &VaultPath,
    ) -> (String, Vec<NoteLink>) {
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut links = Vec::new();
        for link in self.listed_links() {
            match link.kind {
                WalkLinkKind::WikiEmbed => {
                    edits.push((link.range.clone(), embed_image(link)));
                }
                WalkLinkKind::Wiki => {
                    let (target, fragment) = link.wiki_note();
                    let path = VaultPath::note_path_from(target).resolve_against_note(ref_path);
                    links.push(NoteLink::note(&path, &link.label));
                    let dest = markdown_destination(format!("{path}{}", url_fragment(fragment)));
                    if let Some(dest) = dest {
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
                WalkLinkKind::Inline => {
                    let (dest, found) = resolve_md_link(&link.target, &link.label, ref_path);
                    links.extend(found);
                    if let Some(dest) = link.rewritten_destination(dest) {
                        edits.push((link.range.clone(), format!("[{}]({dest})", link.label)));
                    }
                }
                WalkLinkKind::Found => {
                    // Written back only as the link it was read as: a
                    // destination that links nowhere (`[x](a b.md "T")`, a
                    // URL with a space) stays as written.
                    let (dest, found) = resolve_md_link(&link.target, &link.label, ref_path);
                    if let Some(found) = found {
                        links.push(found);
                        if let Some(dest) = markdown_destination(dest) {
                            edits.push((link.range.clone(), format!("[{}]({dest})", link.label)));
                        }
                    }
                }
                WalkLinkKind::Reference | WalkLinkKind::Autolink => {
                    links.extend(resolve_md_link(&link.target, &link.label, ref_path).1);
                }
                WalkLinkKind::Image => {}
            }
        }
        for tag in &self.tags {
            links.push(NoteLink::hashtag(&tag.name));
            edits.push((tag.range.clone(), format!("[#{0}](#{0})", tag.name)));
        }
        edits.sort_by_key(|(range, _)| range.start);
        let mut out = String::with_capacity(note.len());
        let mut last = 0;
        for (range, replacement) in edits {
            // Defensive: pulldown never nests a recorded range in another
            // (a link cannot contain a link), but never slice backwards.
            if range.start < last {
                continue;
            }
            out.push_str(&note[last..range.start]);
            out.push_str(&replacement);
            last = range.end;
        }
        out.push_str(&note[last..]);
        (out, links)
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
    pub(in crate::note) fn append_text(&self, text: String) -> TextLine {
        match self {
            TextLine::Empty => TextLine::Text(text),
            TextLine::Header(level, header_text, start) => {
                TextLine::Header(*level, format!("{}{}", header_text, text), *start)
            }
            TextLine::Text(line_text) => TextLine::Text(format!("{}{}", line_text, text)),
            TextLine::ListItem(level, item_text) => {
                TextLine::ListItem(*level, format!("{}{}", item_text, text))
            }
        }
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
    /// text, in place: what a line's text is once it ends. Prose lines trim
    /// what pulldown trims ([`pulldown_whitespace`]); a code block's text
    /// any whitespace.
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
                    other => other.append_text(cow_str.replace("\r\n", "\n")),
                });
            }
            Event::InlineMath(cow_str)
            | Event::DisplayMath(cow_str)
            | Event::Html(cow_str)
            | Event::FootnoteReference(cow_str) => {
                self.lines
                    .push(TextLine::Text(cow_str.replace("\r\n", "\n")));
            }
            // A line break inside a list item continues the item — its text
            // and its nesting level stay together (rendered as one line, the
            // first line alone naming a note); elsewhere it ends the line.
            Event::SoftBreak | Event::HardBreak
                if matches!(self.lines.last(), Some(TextLine::ListItem(..))) =>
            {
                let item = self.lines.pop().unwrap_or_default();
                self.lines
                    .push(item.trim(pulldown_whitespace).append_text("\n".to_string()));
            }
            Event::SoftBreak => {
                self.end_line();
                self.lines.push(TextLine::Empty);
            }
            Event::HardBreak => {
                self.end_line();
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
    }

    pub(in crate::note) fn finish(self) -> Vec<TextLine> {
        self.lines
    }

    /// The last line ends at a line break: its text is trimmed.
    fn end_line(&mut self) {
        if let Some(line) = self.lines.pop() {
            self.lines.push(line.trim(pulldown_whitespace));
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
        // A line ends with its paragraph, heading or item: its text is
        // trimmed, as pulldown trims it (a wikilink's text at its edge can
        // leave whitespace there).
        TagEnd::Paragraph => {
            vec![current_line.trim(pulldown_whitespace), TextLine::Empty]
        }
        // A heading's text ends with it: what follows in the same block (a
        // heading inside a list item) starts a new line, not the heading's.
        TagEnd::Heading(_) => {
            vec![current_line.trim(pulldown_whitespace), TextLine::Empty]
        }
        // An item's own line, not an HTML or code line inside it.
        TagEnd::Item => match current_line {
            TextLine::ListItem(..) => vec![current_line.trim(pulldown_whitespace)],
            other => vec![other],
        },
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
    fn nested_events_inside_a_wikilink_do_not_reach_the_line_builder() {
        // Defensive: pulldown 0.13.4 never nests a wikilink inside link or
        // image text (the outer link then does not form), so the fix to end
        // a wikilink on its own End, not the first nested one, cannot be
        // observed through `link_depth`. This pins what is observable: no
        // stray End reaches `TextLines`, nested content is not emitted twice,
        // and the lines after the wikilink parse normally.
        let w = walk("# [[a|![b](c)]] head\n\n![alt [[a|![b](c)]] more #t](i.png)\n\n#u\n");
        assert_eq!(headers(&w), [(1, "![b](c) head".to_string(), 0)]);
        assert_eq!(
            w.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["t", "u"]
        );
        assert_eq!(w.links.len(), 2);
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
    fn a_wikilink_inside_a_markdown_link_label_is_the_only_link() {
        // spec: a wikilink inside a markdown link's label — the outer
        // `[…](x.md)` is text, so only `a` is recorded and rewritten.
        let note = "[see [[a]]](x.md)";
        let a = VaultPath::note_path_from("a");
        let (md, links) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, format!("[see [a]({a})](x.md)"));
        let raw: Vec<String> = links.into_iter().map(|l| l.raw_link).collect();
        assert_eq!(raw, [a.to_string()]);
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
    fn a_wikilink_in_image_alt_text_degrades_the_image_to_text() {
        let note = "![x [[a]] #t](p.png)";
        let w = walk(note);
        assert_eq!(links(&w), [(WalkLinkKind::Wiki, "a", "a")]);
        assert_eq!(tag_list(&w), ["t"]);
        let (md, _) = w.render_markdown(note, &VaultPath::new("n.md"));
        // As the rendered markdown was before the single walk.
        let a = VaultPath::note_path_from("a");
        assert_eq!(md, format!("![x [a]({a}) [#t](#t)](p.png)"));
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
        assert_eq!(crate::note::note_link_targets(note), ["#anchor"]);
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

        let note = "a [[[tri]]] b";
        let tri = VaultPath::note_path_from("tri");
        assert_eq!(crate::note::note_link_targets(note), ["tri"]);
        let (md, _) = walk(note).render_markdown(note, &root);
        assert_eq!(md, format!("a [[tri]({tri})] b"));

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
        for (note, expected) in [
            ("[[a|b]c]]", format!("[b\\]c]({a})")),
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
    // left as written.
    #[test]
    fn a_spaced_destination_that_resolves_nowhere_is_left_as_written() {
        let at = VaultPath::new("n.md");
        for (note, written) in [
            ("[x](a b.md \"T\")", "a b.md \"T\""),
            ("[a](https://x.y/a b)", "https://x.y/a b"),
        ] {
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(md, note);
            assert!(listed.is_empty(), "{note:?}: {listed:?}");
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert!(index.is_empty(), "{note:?}: {index:?}");
            assert_eq!(crate::note::note_link_targets(note), [written]);
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
            (
                "[ **a\n[y](e f.md)**",
                vec!["e f.md"],
                vec!["y"],
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
}
