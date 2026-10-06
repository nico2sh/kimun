//! The one pass over a note every whole-note extractor reads — see
//! `2026-10-06-single-note-walk-design.md`. `walk` parses the note as written
//! once and returns a `NoteWalk` (text lines, links, tags, frontmatter); every
//! whole-note extractor is a view of it. All offsets are byte offsets into
//! the note as given. The line builder (`TextLine`/`TextLines`) lives here too.

use std::ops::Range;

use log::debug;
use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

use super::content_extractor::{
    frontmatter_bounds, frontmatter_delimiter, label_matches_inner, split_bom,
    target_looks_like_image, wikilink_parts, MD_LINK_RX, WIKILINK_RX,
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
    /// `[label][ref]`, `[label][]`, `[label]` resolved against a definition.
    Reference,
    /// `<https://…>` or `<mail@…>`.
    Autolink,
    /// `![alt](dest)` and its reference forms.
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
    /// Found in HTML (a block or an inline tag), which pulldown does not
    /// parse: still a link, but a wikilink's text is left as written.
    pub in_html: bool,
}

impl WalkLink {
    /// Whether every view treats this as a link: a wikilink (or embed) only
    /// when its target is a valid vault path — `[[#tag]]` and `[[a#sec]]`
    /// parse as wikilinks but link nowhere.
    fn is_link(&self) -> bool {
        match self.kind {
            WalkLinkKind::Wiki | WalkLinkKind::WikiEmbed => VaultPath::is_valid(&self.target),
            _ => true,
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
    if let Some((inner, end)) = frontmatter_bounds(note) {
        return (end, Some(inner));
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

/// Whether `src` alone parses as one wikilink, as pulldown reads one
/// (`[[|b]]` and `[[]]` do not; `[[#tag]]` does).
fn is_wikilink(src: &str) -> bool {
    Parser::new_ext(src, Options::ENABLE_WIKILINKS)
        .into_offset_iter()
        .any(|(event, range)| range == (0..src.len()) && wikilink_kind(&event).is_some())
}

/// The links in a slice of HTML starting at byte `offset` of the note, in
/// document order. Pulldown hands HTML over unparsed, so they are found in
/// its source: wikilinks with the editor's wikilink pattern, kept only when
/// pulldown would read the same text as a wikilink, and markdown links (not
/// images) with the markdown-link pattern.
fn html_links(src: &str, offset: usize, links: &mut Vec<WalkLink>) {
    let first = links.len();
    html_wikilinks(src, offset, links);
    let wikilinks = first..links.len();
    for caps in MD_LINK_RX.captures_iter(src) {
        let Some(whole) = caps.get(0) else { continue };
        let range = offset + whole.start()..offset + whole.end();
        let overlaps_a_wikilink = links[wikilinks.clone()]
            .iter()
            .any(|l| l.range.start < range.end && range.start < l.range.end);
        if !caps["bang"].is_empty() || overlaps_a_wikilink {
            continue;
        }
        links.push(WalkLink {
            kind: WalkLinkKind::Inline,
            target: caps["link"].trim().to_string(),
            label: caps["text"].to_string(),
            range,
            in_html: true,
        });
    }
    links[first..].sort_by_key(|l| l.range.start);
}

/// The wikilinks of [`html_links`].
fn html_wikilinks(src: &str, offset: usize, links: &mut Vec<WalkLink>) {
    for m in WIKILINK_RX.find_iter(src) {
        if !is_wikilink(m.as_str()) {
            continue;
        }
        let inner = &src[m.start() + 2..m.end() - 2];
        let (target, label) = wikilink_parts(inner);
        let embed = src[..m.start()].ends_with('!');
        let (kind, start) = if embed {
            (WalkLinkKind::WikiEmbed, m.start() - 1)
        } else {
            (WalkLinkKind::Wiki, m.start())
        };
        links.push(WalkLink {
            kind,
            target: target.to_string(),
            label: label.to_string(),
            range: offset + start..offset + m.end(),
            in_html: true,
        });
    }
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

/// A markdown link or image the walk is inside: the record whose label its
/// text sets (none for an autolink, labelled by its address, or an email
/// autolink, not recorded), the body byte its label starts at (just past the
/// opening `[` / `![`, so a leading escape like `\[` stays in it), and the
/// end of the text events seen so far.
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

    /// The link ended: its label is the source of its text, as written.
    fn close(self, links: &mut [WalkLink], body: &str) {
        if let Some(record) = self.record {
            links[record].label = self
                .label_end
                .map_or_else(String::new, |end| body[self.label_start..end].to_string());
        }
    }
}

/// A markdown link or image's record (none for an email autolink), its
/// label left for its [`OpenLink`] to fill.
fn md_link(event: &Event, range: Range<usize>) -> Option<WalkLink> {
    let (kind, target, label) = match event {
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
        }) => (WalkLinkKind::Autolink, dest_url, dest_url.to_string()),
        Event::Start(Tag::Link {
            link_type: LinkType::Inline,
            dest_url,
            ..
        }) => (WalkLinkKind::Inline, dest_url, String::new()),
        Event::Start(Tag::Link { dest_url, .. }) => {
            (WalkLinkKind::Reference, dest_url, String::new())
        }
        Event::Start(Tag::Image { dest_url, .. }) => (WalkLinkKind::Image, dest_url, String::new()),
        _ => return None,
    };
    Some(WalkLink {
        kind,
        target: target.to_string(),
        label,
        range,
        in_html: false,
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
    // Open non-wiki links and images: text inside them is link text, and
    // its source is their label.
    let mut open_links: Vec<OpenLink> = Vec::new();
    // Inside a wikilink, pulldown's own events are skipped up to its end:
    // the display text comes from `wikilink_parts` on the source.
    let mut wikilink_depth = 0u32;

    for (event, range) in Parser::new_ext(body, Options::ENABLE_WIKILINKS).into_offset_iter() {
        let start = body_start + range.start;
        if wikilink_depth > 0 {
            match event {
                Event::Start(_) => wikilink_depth += 1,
                Event::End(_) => {
                    wikilink_depth -= 1;
                    if wikilink_depth == 0 {
                        lines.push(event, start, str::to_string);
                    }
                }
                _ => {}
            }
            continue;
        }
        if matches!(event, Event::End(TagEnd::Link | TagEnd::Image)) {
            if let Some(open) = open_links.pop() {
                open.close(&mut links, body);
            }
        }
        for open in &mut open_links {
            open.extend(&range);
        }
        if let Some(kind) = wikilink_kind(&event) {
            let src = &body[range.clone()];
            let inner = src.strip_prefix('!').unwrap_or(src);
            let inner = inner
                .strip_prefix("[[")
                .and_then(|s| s.strip_suffix("]]"))
                .unwrap_or(inner);
            let (target, label) = wikilink_parts(inner);
            links.push(WalkLink {
                kind,
                target: target.to_string(),
                label: label.to_string(),
                range: start..body_start + range.end,
                in_html: false,
            });
            let display = label.replace("\r\n", "\n");
            lines.push(event, start, str::to_string);
            lines.push(Event::Text(CowStr::from(display)), start, str::to_string);
            wikilink_depth = 1;
            continue;
        }
        match &event {
            Event::Start(Tag::CodeBlock(_)) => code_depth += 1,
            Event::End(TagEnd::CodeBlock) => code_depth = code_depth.saturating_sub(1),
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => {
                let found = md_link(&event, start..body_start + range.end);
                let labelled = found
                    .as_ref()
                    .is_some_and(|l| l.kind != WalkLinkKind::Autolink);
                links.extend(found);
                let opener = if matches!(event, Event::Start(Tag::Image { .. })) {
                    "!["
                } else {
                    "["
                };
                open_links.push(OpenLink {
                    record: labelled.then(|| links.len() - 1),
                    label_start: range.start + opener.len(),
                    label_end: None,
                });
            }
            // A whole HTML block at once, so a link across its lines is
            // found; HTML never sits inside a code block.
            Event::Start(Tag::HtmlBlock) | Event::InlineHtml(_) => {
                html_links(&body[range.clone()], start, &mut links);
            }
            _ => {}
        }
        let in_code = code_depth > 0;
        let in_link = !open_links.is_empty();
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
/// folder), any other its note path, unresolved.
fn embed_image(link: &WalkLink) -> String {
    let dest = if target_looks_like_image(&link.target) {
        link.target.clone()
    } else {
        VaultPath::note_path_from(&link.target).to_string()
    };
    format!("![{}]({dest})", link.label)
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

    /// The first non-empty line, trimmed: a heading's text, a paragraph's,
    /// or a list item's first line.
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
            .map(|text| text.trim().to_owned())
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
    /// to valid vault paths, markdown and image destinations, autolinks.
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
                    let path = VaultPath::note_path_from(&link.target);
                    found.push((link.range.start, NoteLink::note(&path, &link.label)));
                }
                WalkLinkKind::Inline | WalkLinkKind::Reference | WalkLinkKind::Autolink => {
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
    /// become `[text](note path)`, a path target resolved against the note's
    /// folder as the editor follows it; an embed becomes an image left to the
    /// image pipeline (see `embed_image`), not listed; inline links get their
    /// destination resolved; hashtags become `[#tag](#tag)`. Reference links,
    /// autolinks, inline links not in the plain `[label](dest)` form (`<…>`
    /// destination, title) and wikilinks inside HTML are listed but left as
    /// written. Links come in document order, hashtags after them.
    /// Everything outside a recorded range — frontmatter, code, images — is
    /// copied verbatim.
    pub(in crate::note) fn render_markdown(
        &self,
        note: &str,
        ref_path: &VaultPath,
    ) -> (String, Vec<NoteLink>) {
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut links = Vec::new();
        for link in self.listed_links() {
            match link.kind {
                // HTML is left as the renderer will read it.
                WalkLinkKind::WikiEmbed if !link.in_html => {
                    edits.push((link.range.clone(), embed_image(link)));
                }
                WalkLinkKind::WikiEmbed => {}
                WalkLinkKind::Wiki => {
                    let path =
                        VaultPath::note_path_from(&link.target).resolve_against_note(ref_path);
                    links.push(NoteLink::note(&path, &link.label));
                    if !link.in_html {
                        edits.push((link.range.clone(), format!("[{}]({path})", link.label)));
                    }
                }
                WalkLinkKind::Inline => {
                    let (dest, found) = resolve_md_link(&link.target, &link.label, ref_path);
                    links.extend(found);
                    // Only the plain `[label](dest)` form is rewritten: a
                    // `<…>` destination, a title, padding or escapes would
                    // be lost (or break the link) if spliced back decoded.
                    let plain = format!("[{}]({})", link.label, link.target);
                    if note[link.range.clone()] == plain {
                        edits.push((link.range.clone(), format!("[{}]({dest})", link.label)));
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

    pub(in crate::note) fn trim(&self) -> Self {
        match self {
            TextLine::Empty => TextLine::Empty,
            TextLine::Header(level, text, start) => {
                TextLine::Header(*level, text.trim().to_string(), *start)
            }
            TextLine::Text(text) => TextLine::Text(text.trim().to_string()),
            TextLine::ListItem(level, text) => TextLine::ListItem(*level, text.trim().to_string()),
        }
    }
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
    }

    pub(in crate::note) fn finish(self) -> Vec<TextLine> {
        self.lines
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
            vec![current_line.trim(), TextLine::Text("```".to_string())]
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
        TagEnd::Heading(_) => {
            vec![current_line, TextLine::Empty]
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

    // Item 1 (final review): a wikilink inside an HTML block is still a link.
    #[test]
    fn a_wikilink_inside_an_html_block_is_still_a_link() {
        let note = "<details>\n<summary>More</summary>\nSee [[hidden]]\n</details>\n";
        let hidden = VaultPath::note_path_from("hidden");
        let at = VaultPath::new("n.md");
        let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
        assert_eq!(raw_links(&index), [hidden.to_string()]);
        let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
        assert_eq!(md, note, "HTML stays as written");
        assert_eq!(raw_links(&listed), [hidden.to_string()]);
        assert_eq!(crate::note::note_link_targets(note), ["hidden"]);
        let w = walk(note);
        assert_eq!(&note[w.links[0].range.clone()], "[[hidden]]");
        assert!(text(&w).contains("See [[hidden]]"), "{:?}", text(&w));
    }

    #[test]
    fn a_wikilink_in_html_follows_the_wikilink_rules() {
        // Across lines inside the block, inside an inline-HTML attribute,
        // an embed; degenerate forms stay text, as outside HTML.
        let note =
            "<div>\n[[a\n|b]] ![[e]] [[|x]] [[]]\n</div>\n\nsee <span title=\"[[q]]\">z</span>\n";
        let w = walk(note);
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Wiki, "a\n", "b"),
                (WalkLinkKind::WikiEmbed, "e", "e"),
                (WalkLinkKind::Wiki, "q", "q"),
            ]
        );
        assert_eq!(&note[w.links[1].range.clone()], "![[e]]");
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

    // Review 2, item 1: a markdown link inside HTML is a link for every
    // consumer — listed, rewritten in plain form, a CLI target, indexed.
    #[test]
    fn a_markdown_link_inside_an_html_block_is_a_link_everywhere() {
        let at = VaultPath::new("/dir/n.md");
        for (note, target) in [
            ("<details>\nSee [doc](doc.md)\n</details>\n", "doc.md"),
            ("<div>[x](y.md)</div>\n", "y.md"),
        ] {
            let path = VaultPath::new(target);
            let (md, listed) = crate::note::content_extractor::get_markdown_and_links(&at, note);
            assert_eq!(md, note, "a bare note name is already its own path");
            assert_eq!(raw_links(&listed), [path.to_string()], "{note:?}");
            assert_eq!(crate::note::note_link_targets(note), [target], "{note:?}");
            let (_, index) = crate::note::content_extractor::get_chunks_and_links(&at, note);
            assert_eq!(raw_links(&index), [path.to_string()], "{note:?}");
        }
    }

    #[test]
    fn a_markdown_link_inside_html_is_rewritten_in_plain_form_and_images_skipped() {
        let note = "<div>[x](sub/y.md) ![i](p.png) [[w]]</div>\n";
        let at = VaultPath::new("/dir/n.md");
        let (md, listed) = walk(note).render_markdown(note, &at);
        let y = VaultPath::new("/dir/sub/y.md");
        assert_eq!(md, format!("<div>[x]({y}) ![i](p.png) [[w]]</div>\n"));
        let w = VaultPath::note_path_from("w");
        assert_eq!(raw_links(&listed), [y.to_string(), w.to_string()]);
        assert_eq!(crate::note::note_link_targets(note), ["sub/y.md", "w"]);
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
            ["ok", "e"]
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
    fn a_leading_escape_in_a_link_label_is_part_of_the_label() {
        let note = "[\\[a](sub/x.md) ![\\[b](i.png)";
        let w = walk(note);
        assert_eq!(links(&w)[0], (WalkLinkKind::Inline, "sub/x.md", "\\[a"));
        assert_eq!(links(&w)[1], (WalkLinkKind::Image, "i.png", "\\[b"));
        // The label matches the source again, so the plain form is rewritten.
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
        let note = "---\nrel: [[fm]]\n---\n[[a]] [b](b.md) [[#bad]] `[[c]]`";
        assert_eq!(crate::note::NoteMetadata::of(note).links, ["a", "b.md"]);
    }
}
