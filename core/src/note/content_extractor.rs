use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use regex::{Captures, Regex};
use std::ops::Range;
use std::sync::LazyLock;
use url::Url;

use crate::{
    nfs::{self, VaultPath},
    note::{ContentChunk, NoteContentData},
};

use super::walk::{retarget_links, walk, TextLine};
use super::NoteLink;

const _MAX_TITLE_LENGTH: usize = 40;

// Compile regexes once at startup
static WIKILINK_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:\[\[(?P<link_text>[^\]]+)\]\])"#).unwrap());

pub(crate) static HASHTAG_RX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"#(?P<ht_text>[A-Za-z0-9_]+)"#).unwrap());

static MD_LINK_RX: LazyLock<Regex> = LazyLock::new(|| {
    // `text` accepts an empty match so empty-alt image links like `![](path)`
    // — which the editor generates on image paste — are still recognised.
    Regex::new(r#"(?P<bang>!?)(?:\[(?P<text>[^\]]*)\])\((?P<link>[^\)]+?)\)"#).unwrap()
});

/// If `s` (after trimming) parses as a URL whose scheme is one of `allowed`,
/// returns the trimmed slice. Otherwise returns `None`.
///
/// `Url::parse` accepts more schemes than most callers want (e.g. `file://`,
/// `javascript:`), so the scheme list is caller-supplied. `Url::parse` is also
/// lenient about embedded whitespace — internal whitespace is rejected up
/// front so an accidental newline does not classify a malformed string as a
/// URL.
pub fn url_with_allowed_scheme<'a>(s: &'a str, allowed: &[&str]) -> Option<&'a str> {
    let trimmed = s.trim();
    if trimmed.contains(char::is_whitespace) {
        return None;
    }
    let url = Url::parse(trimmed).ok()?;
    if allowed.contains(&url.scheme()) {
        Some(trimmed)
    } else {
        None
    }
}

/// Returns `true` if `s` parses as an absolute http(s) URL.
///
/// Replaces the previous hand-rolled `URL_RX` and shares whitespace/parse
/// semantics with [`url_with_allowed_scheme`].
pub fn is_remote_url(s: &str) -> bool {
    url_with_allowed_scheme(s, &["http", "https"]).is_some()
}

/// Discriminates the type of an inline link found by [`link_char_spans`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSpanKind {
    /// A `[[page]]` or `[[page|display]]` wikilink.
    WikiLink,
    /// A plain `[text](url)` markdown link.
    Markdown,
    /// A `![alt](url)` markdown image embed.
    Image,
}

/// A resolved inline link span within a text string.
///
/// `start` and `end` are char-index offsets covering the full token
/// (including delimiters such as `[[`/`]]` or `[`/`)`).
/// `target` holds the link destination — the wiki page name for
/// [`LinkSpanKind::WikiLink`] (before any `|` separator), or the URL/path
/// for [`LinkSpanKind::Markdown`].
#[derive(Debug, Clone)]
pub struct LinkSpan {
    /// Char-index offset of the token's first character (inclusive),
    /// including the opening delimiter (`[[`, or `[`/`!`).
    pub start: usize,
    /// Char-index offset just past the token's last character (exclusive),
    /// including the closing delimiter (`]]` or `)`).
    pub end: usize,
    /// Which inline-link syntax this span matched.
    pub kind: LinkSpanKind,
    /// The link destination: the wiki page name (before any `|` separator)
    /// for [`LinkSpanKind::WikiLink`], or the URL/path for the markdown and
    /// image kinds.
    pub target: String,
}

/// Returns only `[[wikilink]]` spans from `text`, sorted by document order.
///
/// Cheaper than `link_char_spans` when markdown links are not needed (e.g. the
/// per-frame editor render path).
pub fn wikilink_char_spans(text: &str) -> Vec<LinkSpan> {
    let mut cursor = ByteToCharCursor::new(text);
    WIKILINK_RX
        .captures_iter(text)
        .map(|caps| {
            let m = caps.get(0).unwrap();
            let start = cursor.advance_to(m.start());
            let end = cursor.advance_to(m.end());
            let inner = &caps["link_text"];
            let target = wikilink_parts(inner).0.to_string();
            LinkSpan {
                start,
                end,
                kind: LinkSpanKind::WikiLink,
                target,
            }
        })
        .collect()
}

/// Returns every inline link span in `text`, covering both `[[wikilinks]]`
/// and `[markdown](links)`, sorted by document order.
///
/// Suitable for syntax highlighting, editor decoration, and lightweight
/// link extraction without full vault-path resolution.
pub fn link_char_spans(text: &str) -> Vec<LinkSpan> {
    // Collect each iterator's matches as (byte_start, byte_end, kind, target),
    // sort by byte_start, then walk a single byte→char cursor over the sorted
    // list — total O(N + K log K) instead of O(K · N) char-counting per match.
    let mut raw: Vec<(usize, usize, LinkSpanKind, String)> = Vec::new();

    for caps in WIKILINK_RX.captures_iter(text) {
        let m = caps.get(0).unwrap();
        let inner = &caps["link_text"];
        let target = wikilink_parts(inner).0.to_string();
        raw.push((m.start(), m.end(), LinkSpanKind::WikiLink, target));
    }
    for caps in MD_LINK_RX.captures_iter(text) {
        let m = caps.get(0).unwrap();
        let target = caps["link"].trim().to_string();
        let kind = if caps["bang"].is_empty() {
            LinkSpanKind::Markdown
        } else {
            LinkSpanKind::Image
        };
        raw.push((m.start(), m.end(), kind, target));
    }
    raw.sort_by_key(|r| r.0);

    let mut cursor = ByteToCharCursor::new(text);
    raw.into_iter()
        .map(|(byte_start, byte_end, kind, target)| {
            let start = cursor.advance_to(byte_start);
            let end = cursor.advance_to(byte_end);
            LinkSpan {
                start,
                end,
                kind,
                target,
            }
        })
        .collect()
}

/// A `[text](link)` or `![text](link)` the editor's link pattern finds —
/// what [`link_char_spans`] highlights as [`LinkSpanKind::Markdown`] or
/// [`LinkSpanKind::Image`]: its bytes (an image's `!` included) and its
/// text's bytes in the scanned text, and its destination as written
/// (trimmed, as `link_char_spans` reports it).
pub(in crate::note) struct MdLinkMatch<'t> {
    pub range: Range<usize>,
    pub label: Range<usize>,
    pub target: &'t str,
    pub image: bool,
}

/// The bytes of every `[[…]]` the editor's wikilink pattern finds in `text`
/// — exactly what [`wikilink_char_spans`] highlights — in order: the one
/// recognition rule for a wikilink, shared by the editor and the walk.
pub(in crate::note) fn wikilink_matches(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    WIKILINK_RX.find_iter(text).map(|m| m.range())
}

/// The markdown links and images the editor's link pattern finds in
/// `text`, in order.
pub(in crate::note) fn md_link_matches(text: &str) -> impl Iterator<Item = MdLinkMatch<'_>> {
    MD_LINK_RX.captures_iter(text).filter_map(|caps| {
        Some(MdLinkMatch {
            range: caps.get(0)?.range(),
            label: caps.name("text")?.range(),
            target: caps.name("link")?.as_str().trim(),
            image: !caps["bang"].is_empty(),
        })
    })
}

/// Recognised image extensions, lowercase. Used by [`target_looks_like_image`].
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tiff", "tif", "ico", "avif",
];

/// Returns `true` if `target` looks like a path or URL referencing an image
/// file, based on extension. Case-insensitive. Strips `#fragment` and
/// `?query` first so query-string parameters don't fool the check.
pub fn target_looks_like_image(target: &str) -> bool {
    let name = link_target_filename(target);
    let ext = match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => ext,
        _ => return false,
    };
    IMAGE_EXTENSIONS
        .iter()
        .any(|allowed| ext.eq_ignore_ascii_case(allowed))
}

/// Extracts a display-friendly filename from a link target.
///
/// Strips any URL fragment (`#...`) and query string (`?...`), then returns
/// the last `/`-separated segment. Useful for rendering image-link
/// placeholders like `[image_xxx.png]` in editors.
pub fn link_target_filename(target: &str) -> &str {
    let without_fragment = target.split('#').next().unwrap_or(target);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let trimmed = without_query.trim_end_matches(crate::nfs::PATH_SEPARATOR);
    match trimmed.rsplit_once(crate::nfs::PATH_SEPARATOR) {
        Some((_, name)) if !name.is_empty() => name,
        _ => trimmed,
    }
}

/// Streaming byte-offset → char-offset converter.
///
/// Callers MUST pass monotonically non-decreasing byte offsets to
/// `advance_to`; violating this contract produces stale char offsets in
/// release builds (debug builds panic via `debug_assert!`). Total work
/// over a full scan of `text` is O(text.len()), regardless of the number
/// of `advance_to` calls.
struct ByteToCharCursor<'a> {
    text: &'a str,
    byte_pos: usize,
    char_pos: usize,
}

impl<'a> ByteToCharCursor<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            byte_pos: 0,
            char_pos: 0,
        }
    }

    fn advance_to(&mut self, byte_target: usize) -> usize {
        debug_assert!(byte_target >= self.byte_pos);
        if byte_target > self.byte_pos {
            self.char_pos += self.text[self.byte_pos..byte_target].chars().count();
            self.byte_pos = byte_target;
        }
        self.char_pos
    }
}

/// Chunks and the links the index records (note links and hashtags), from
/// one walk over the note — see `walk.rs`.
pub fn get_chunks_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (Vec<ContentChunk>, Vec<super::NoteLink>) {
    let walked = walk(md_text.as_ref());
    let links = walked.index_links(reference_path);
    (walked.into_chunks(), links)
}

/// A wikilink's `(target, display text)` from what sits between `[[` and
/// `]]`: `target|text`, or the target alone shown as itself. Extra pipes
/// are dropped. The one reading of a wikilink's parts every walker uses.
pub(in crate::note) fn wikilink_parts(inner: &str) -> (&str, &str) {
    let mut parts = inner.split('|');
    let link = parts.next().unwrap_or(inner);
    let text = parts.next().unwrap_or(link);
    (link, text)
}

/// A link target split into the note it points to and the part inside that
/// note: `note#section` and `note^block` are `("note", "#section")` and
/// `("note", "^block")`. Spaces and tabs around the note part are trimmed
/// (`[[ spaced ]]` links to `spaced`), not line breaks: a target that ends
/// at a line break (`[[target⏎|Shown]]`) stays invalid. The fragment is kept
/// as written. `#` and `^` are not valid in
/// a vault path, so nothing a path could hold is cut. The one reading of a
/// fragment: the walk indexes and renders wikilinks by it, the editor
/// follows links by it.
///
/// ```
/// use kimun_core::note::scan::split_link_fragment;
/// assert_eq!(split_link_fragment("Plan#Goals"), ("Plan", "#Goals"));
/// assert_eq!(split_link_fragment("a^blk"), ("a", "^blk"));
/// assert_eq!(split_link_fragment(" spaced "), ("spaced", ""));
/// assert_eq!(split_link_fragment("#tag"), ("", "#tag"));
/// assert_eq!(split_link_fragment("target\n"), ("target\n", ""));
/// ```
pub fn split_link_fragment(target: &str) -> (&str, &str) {
    let at = target.find(['#', '^']).unwrap_or(target.len());
    (target[..at].trim_matches([' ', '\t']), &target[at..])
}

/// Content data, chunks and index links from one walk over the note.
pub fn get_index_data<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (NoteContentData, Vec<ContentChunk>, Vec<super::NoteLink>) {
    let text = md_text.as_ref();
    let walked = walk(text);
    let data = NoteContentData {
        title: walked.title(),
        hash: nfs::hash_text(text),
    };
    let links = walked.index_links(reference_path);
    (data, walked.into_chunks(), links)
}

pub fn get_content_data<S: AsRef<str>>(md_text: S) -> NoteContentData {
    let hash = nfs::hash_text(md_text.as_ref());
    let title = extract_title(md_text);

    NoteContentData { title, hash }
}

pub fn get_content_chunks<S: AsRef<str>>(md_text: S) -> Vec<ContentChunk> {
    walk(md_text.as_ref()).into_chunks()
}

/// Returns byte-offset ranges (start, end) within `md_text` covering every
/// inline code span and fenced/indented code block. Used to exclude these
/// regions from hashtag extraction so `#tag` inside code is not promoted to
/// a label.
pub(crate) fn code_char_ranges(md_text: &str) -> Vec<(usize, usize)> {
    let parser = Parser::new(md_text).into_offset_iter();
    let mut ranges = Vec::new();
    let mut depth = 0u32;
    let mut current_start: Option<usize> = None;
    for (event, range) in parser {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                if depth == 0 {
                    current_start = Some(range.start);
                }
                depth += 1;
            }
            Event::End(TagEnd::CodeBlock) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(start) = current_start.take() {
                        ranges.push((start, range.end));
                    }
                }
            }
            Event::Code(_) => {
                ranges.push((range.start, range.end));
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                ranges.push((range.start, range.end));
            }
            _ => {}
        }
    }
    ranges
}

/// Returns byte-offset ranges (start, end) within `md_text` covering every
/// markdown link `[text](href)` (full span including the `[]` brackets and
/// the `()` around the href). Used to exclude hashtag extraction inside
/// link bodies — particularly URL fragments like `https://example.com#section`.
pub(crate) fn md_link_char_ranges(md_text: &str) -> Vec<(usize, usize)> {
    MD_LINK_RX
        .find_iter(md_text)
        .map(|m| (m.start(), m.end()))
        .collect()
}

/// Returns byte-offset ranges (start, end) within `md_text` covering every
/// `[[wikilink]]` form, including invalid wikilinks (those that were not
/// rewritten to standard markdown links earlier in the pipeline, e.g.
/// `[[#tag]]`). Used to exclude hashtag extraction inside wikilink spans so
/// that `[[#wiki_tag]]` is not indexed as a label.
pub(crate) fn md_wikilink_char_ranges(md_text: &str) -> Vec<(usize, usize)> {
    WIKILINK_RX
        .find_iter(md_text)
        .map(|m| (m.start(), m.end()))
        .collect()
}

/// Returns `true` if `byte_offset` (a cursor position) falls inside a region
/// where hashtag/wikilink autocomplete should be suppressed:
///
/// - the YAML/TOML frontmatter block,
/// - an inline code span or fenced/indented code block,
/// - the body of a standard markdown link `[text](href)`,
/// - the body of an already-closed `[[wikilink]]`.
///
/// Used by the TUI autocomplete to decide whether a `#` or `[[` keystroke
/// should open the suggestion popup, so the same hashtag-exclusion truth
/// that the indexer applies (see `index_note_chunks_and_links`) governs the
/// editing-time popup too.
///
/// Containment uses **strict** bounds (`> start && < end`): a cursor sitting
/// exactly at the start or end of a zone is treated as outside, so the user
/// can place the caret right before or right after `` `code` `` or `[[foo]]`
/// and still trigger autocomplete from that position.
///
/// Note: open / unterminated wikilinks (e.g. mid-typing `[[foo`) are not
/// matched by the wikilink regex and therefore do not suppress the popup —
/// which is exactly what the editor needs.
pub fn is_inside_exclusion_zone(text: &str, byte_offset: usize) -> bool {
    ExclusionZones::from_text(text).contains(byte_offset)
}

/// Like `is_inside_exclusion_zone` but does NOT include already-closed
/// `[[wikilink]]` spans. Wikilink autocomplete uses this so the user
/// can reopen the popup with the cursor inside a closed wikilink's
/// target to edit it. Code spans, frontmatter, and standard markdown
/// link bodies still suppress.
pub fn is_inside_code_link_or_frontmatter(text: &str, byte_offset: usize) -> bool {
    ExclusionZones::from_text(text).contains_code_link_or_frontmatter(byte_offset)
}

/// Locates the section belonging to the ATX heading whose text equals
/// `heading`, for callers that only have a heading string to go on (no
/// [`ContentChunk`] available) — e.g. the Ask workspace's source reader
/// (`tui::ask::locate::section_range`), matching a server-returned section
/// title against the note's raw text when the retrieved chunk text is no
/// longer a verbatim substring.
///
/// Comparison strips diacritics and folds ASCII case on *both* sides:
/// `heading` may already be core-normalized (e.g. round-tripped through the
/// server, which indexes diacritics-stripped section titles), while `text`
/// is the note's raw source — so a note heading of "Café" matches a
/// `heading` argument of "cafe".
///
/// Returns the byte range from the matching heading line's start through to
/// the start of the next heading line (any level), or the end of `text` if
/// it's the last section. `None` when no heading line matches.
pub fn heading_section_range(text: &str, heading: &str) -> Option<Range<usize>> {
    let needle = crate::note::diacritics::remove_diacritics(heading);
    let mut offset = 0usize;
    let mut start = None;
    for line in text.split_inclusive('\n') {
        let stripped = line.strip_suffix('\n').unwrap_or(line);
        if start.is_none() {
            // Matched by the text the chunker gives it, so a heading holding
            // inline HTML, links or tags, or one inside a list item, is found.
            if let Some(title) = heading_display_text(stripped) {
                let normalized = crate::note::diacritics::remove_diacritics(&title);
                if normalized.eq_ignore_ascii_case(&needle) {
                    start = Some(offset);
                }
            }
        } else if atx_heading_text(stripped).is_some() {
            return start.map(|s| s..offset);
        }
        offset += line.len();
    }
    start.map(|s| s..text.len())
}

/// The text `line` has as a heading when it is an ATX one — rendered exactly
/// as `get_content_chunks` renders a chunk's breadcrumb (wikilinks and links
/// collapsed to their text, hashtag markers dropped, emphasis and the ATX
/// markers gone) — or `None` for any other line.
///
/// One line at a time, so a caller can match a heading title against raw text
/// without re-chunking the note ([`heading_section_range`]); a caller holding
/// the whole note wants [`crate::note::note_headings`], which has line numbers
/// and none of the limits below. Rendering a line
/// alone has limits the whole-note chunker does not, all of them fail-safe: a
/// setext heading (`Title` over `=====`) has no `#` and renders to `None`; a
/// reference-style link renders as written, its definition being on another
/// line; and a `#` line inside a fenced block or frontmatter renders as a
/// heading the chunker never lists, so it can only shadow a real heading with
/// the same text. Leading indentation and list or quote markers are dropped on
/// purpose: the chunker lists a heading nested in a list item or a quote
/// (`- # Setup`, `> # Note`), whose own line is not a top-level heading. A
/// `#`-run not followed by whitespace is a hashtag, not a heading, and renders
/// to `None`.
pub fn heading_display_text(line: &str) -> Option<String> {
    let line = strip_block_markers(line);
    if !line.starts_with('#') {
        return None;
    }
    walk(line)
        .lines
        .into_iter()
        .find_map(|text_line| match text_line {
            TextLine::Header(_, text, _) => Some(text),
            _ => None,
        })
}

/// `line` past its indentation and any list (`-`, `*`, `+`, `1.`, `1)`) or
/// quote (`>`) markers in front of its content.
fn strip_block_markers(line: &str) -> &str {
    let mut line = line.trim_start();
    loop {
        let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let marker = match line.as_bytes() {
            [b'>', ..] => 1,
            [b'-' | b'*' | b'+', b' ' | b'\t', ..] => 1,
            bytes
                if (1..=9).contains(&digits)
                    && matches!(
                        bytes.get(digits..digits + 2),
                        Some([b'.' | b')', b' ' | b'\t'])
                    ) =>
            {
                digits + 1
            }
            _ => return line,
        };
        line = line[marker..].trim_start();
    }
}

/// Recognizes one ATX heading line — 1 to 6 leading `#` characters, then a
/// space/tab (or end of line), then the heading text — and returns its text
/// with leading/trailing whitespace and an optional closing `#` run (e.g.
/// `## Title ##`) trimmed off.
///
/// A `#`-run *not* followed by whitespace (`#projects`) is a hashtag, not a
/// heading, and returns `None` — hashtags are a first-class construct here
/// ([`label_matches_inner`]), never conflated with headings.
fn atx_heading_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let after_hashes = trimmed.trim_start_matches('#');
    let hash_count = trimmed.len() - after_hashes.len();
    if !(1..=6).contains(&hash_count) {
        return None;
    }
    if !after_hashes.is_empty() && !after_hashes.starts_with([' ', '\t']) {
        return None;
    }
    let text = after_hashes.trim();
    let text = match text.trim_end_matches('#').strip_suffix([' ', '\t']) {
        // A space/tab-preceded trailing `#` run is a closing sequence — drop it.
        Some(stripped) => stripped.trim_end(),
        // No preceding space: either there's no trailing `#` run at all
        // (keep `text` as-is), or the whole heading text is nothing but a
        // closing run with no title (e.g. `### ###`) — clear it.
        None if !text.is_empty() && text.bytes().all(|b| b == b'#') => "",
        None => text,
    };
    Some(text)
}

/// Cached set of byte ranges that suppress autocomplete: frontmatter,
/// fenced/inline code, markdown link bodies, closed wikilink spans.
///
/// Builds once via `from_text` (the expensive step — runs a pulldown-cmark
/// parse plus two regex scans over the full buffer), then answers any
/// number of `contains*` point queries in O(ranges) time.
///
/// The TUI autocomplete caches one instance per buffer revision so that
/// cursor moves never repay the full-buffer parse.
#[derive(Debug, Clone, Default)]
pub struct ExclusionZones {
    /// Byte length of the source `text` the zones were built from. Used
    /// only as a sanity check in `contains*` so a stale cache that has
    /// outlived its source text fails closed rather than returning a
    /// random answer for an out-of-range query.
    text_len: usize,
    frontmatter_end: usize,
    code: Vec<(usize, usize)>,
    md_links: Vec<(usize, usize)>,
    wikilinks: Vec<(usize, usize)>,
}

impl ExclusionZones {
    /// Computes every zone for `text` in a single sweep. Mirrors
    /// `is_inside_exclusion_zone` / `is_inside_code_link_or_frontmatter`
    /// internally so the contains-checks stay in lock-step with the
    /// indexer's exclusion rules.
    ///
    /// Callers must query `contains*` only with byte offsets into the
    /// same `text` (or a longer text whose prefix matches it byte-for-byte
    /// up to that offset). Cached zones held past a text mutation are
    /// invalid; consult the caller's revision before reuse.
    pub fn from_text(text: &str) -> Self {
        Self {
            text_len: text.len(),
            frontmatter_end: frontmatter_end_byte(text),
            code: code_char_ranges(text),
            md_links: md_link_char_ranges(text),
            wikilinks: md_wikilink_char_ranges(text),
        }
    }

    /// True when `byte_offset` falls inside any zone (frontmatter, code,
    /// markdown link body, or closed wikilink). Matches the boolean of
    /// `is_inside_exclusion_zone(text, byte_offset)` for the same `text`.
    /// Returns `false` for offsets past the source text length (defensive
    /// guard against using a stale cache against a shortened buffer).
    pub fn contains(&self, byte_offset: usize) -> bool {
        if byte_offset > self.text_len {
            return false;
        }
        if byte_offset < self.frontmatter_end {
            return true;
        }
        Self::in_any(byte_offset, &self.code)
            || Self::in_any(byte_offset, &self.md_links)
            || Self::in_any(byte_offset, &self.wikilinks)
    }

    /// True when `byte_offset` falls inside frontmatter, code, or a
    /// markdown link body — but NOT inside a closed wikilink span. Matches
    /// `is_inside_code_link_or_frontmatter(text, byte_offset)`. Returns
    /// `false` for offsets past the source text length.
    pub fn contains_code_link_or_frontmatter(&self, byte_offset: usize) -> bool {
        if byte_offset > self.text_len {
            return false;
        }
        if byte_offset < self.frontmatter_end {
            return true;
        }
        Self::in_any(byte_offset, &self.code) || Self::in_any(byte_offset, &self.md_links)
    }

    fn in_any(byte_offset: usize, ranges: &[(usize, usize)]) -> bool {
        ranges
            .iter()
            .any(|(s, e)| byte_offset > *s && byte_offset < *e)
    }
}

/// Internal label iterator that powers [`crate::note::label_matches`].
///
/// Encapsulates the regex match + the word-boundary guard. A `#tag` is
/// rejected when the preceding or following character is alphanumeric, `_`,
/// or another `#` — so mid-word (`hello#tag`), stacked-hash (`##tag`,
/// Markdown header territory), and adjacent-hash (`#tag#more`) cases are
/// all skipped — or when it follows an `&`, as in an HTML entity
/// (`it&#39;s`). Code-span / HTML / link overlap suppression is left to the
/// caller because those checks are context-specific.
pub(crate) fn label_matches_inner(
    text: &str,
) -> impl Iterator<Item = crate::note::scan::LabelMatch<'_>> + '_ {
    HASHTAG_RX.captures_iter(text).filter_map(move |caps| {
        let m = caps.get(0)?;
        let preceding_blocks_label = m.start() != 0
            && text[..m.start()]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric() || c == '_' || c == '#' || c == '&')
                .unwrap_or(false);
        if preceding_blocks_label {
            return None;
        }
        let following_blocks_label = text[m.end()..]
            .chars()
            .next()
            .map(|c| c.is_alphanumeric() || c == '_' || c == '#')
            .unwrap_or(false);
        if following_blocks_label {
            return None;
        }
        let name = caps.name("ht_text")?.as_str();
        Some(crate::note::scan::LabelMatch {
            byte_start: m.start(),
            byte_end: m.end(),
            name,
        })
    })
}

/// The note rewritten for a Markdown renderer, plus the links it holds:
/// wikilinks become Markdown links, inline-link destinations are resolved
/// to vault paths, hashtags become `[#tag](#tag)` links. One walk over the
/// note, rewritten only at the links it recorded — see
/// `NoteWalk::render_markdown` for the exact rules.
pub(crate) fn get_markdown_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (String, Vec<NoteLink>) {
    let note = md_text.as_ref();
    walk(note).render_markdown(note, reference_path)
}

/// Rewrites every link in `md_text` that points at `old_path` so it points
/// at `new_path` — exactly the links the index records as pointing at it,
/// read by the same walk (see `walk::retarget_links`): wikilinks and embeds
/// with a `#section`, `^block` or padding, markdown links with spaces in
/// their destination or a `#section`, reference definitions. `note_path` is
/// where the note was when its links were indexed (the renamed note's own
/// self-links: `old_path`).
///
/// Returns `(updated_text, changed)` where `changed` is true when at least one replacement was made.
pub(crate) fn replace_note_links(
    md_text: &str,
    note_path: &VaultPath,
    old_path: &VaultPath,
    new_path: &VaultPath,
) -> (String, bool) {
    let result = retarget_links(md_text, note_path, old_path, new_path);
    let changed = result != md_text;
    (result, changed)
}

/// Process image links in already-converted markdown, calling `resolver` for each image.
///
/// `resolver(alt_text, raw_path) -> (resolved_path_in_markdown, NoteLink)`:
/// - receives the alt text and the raw path/URL from the markdown
/// - returns the path string to embed in the output markdown, and the link to record
///
/// Non-image links pass through unchanged.
pub(crate) fn process_image_links<F>(
    md_text: &str,
    mut resolver: F,
) -> (String, Vec<super::NoteLink>)
where
    F: FnMut(&str, &str) -> (String, super::NoteLink),
{
    let mut image_links = vec![];
    let result = MD_LINK_RX.replace_all(md_text, |caps: &Captures| {
        let bang = &caps["bang"];
        let text = &caps["text"];
        let link = caps["link"].trim();

        if bang.is_empty() {
            // Not an image — pass through unchanged
            return format!("[{}]({})", text, link);
        }

        let (resolved_path, note_link) = resolver(text, link);
        image_links.push(note_link);
        format!("![{}]({})", text, resolved_path)
    });
    (result.to_string(), image_links)
}

/// The note's title: the text of its first non-empty line (frontmatter
/// skipped), rendered like the rest of the walk — see `NoteWalk::title`.
pub fn extract_title<S: AsRef<str>>(md_text: S) -> String {
    walk(md_text.as_ref()).title()
}

/// Returns the byte offset immediately after the opening delimiter line —
/// past its `\n`, or the end of `text` when that line is all there is — or
/// `None` if the text does not start with a valid frontmatter delimiter
/// (`---` or `+++`).  Strips a trailing `\r` so CRLF files work the same as
/// LF files, and ignores a leading UTF-8 byte-order mark (Windows editors
/// write one); the offset still counts it, so it indexes `text`.
pub(in crate::note) fn frontmatter_delimiter(text: &str) -> Option<(&str, usize)> {
    let (bom, rest) = split_bom(text);
    let line_end = bom.len() + rest.find('\n').unwrap_or(rest.len());
    let first_line = text[bom.len()..line_end].trim_end_matches('\r');
    if first_line != "---" && first_line != "+++" {
        return None;
    }
    // Return the canonical delimiter (without \r) and the byte offset just
    // after the opening line.
    Some((first_line, (line_end + 1).min(text.len())))
}

/// Returns the byte offset of the first character after the closing delimiter
/// of a YAML/TOML frontmatter block (`---` or `+++`), or `0` if no valid
/// frontmatter is present. Tolerates both LF and CRLF line endings.
fn frontmatter_end_byte(text: &str) -> usize {
    frontmatter_bounds(text).map_or(0, |bounds| bounds.end)
}

/// Where a closed YAML/TOML frontmatter block sits in a note.
pub(in crate::note) struct FrontmatterBounds<'t> {
    /// The fence, `---` or `+++` (without `\r`).
    pub delimiter: &'t str,
    /// The bytes between the fences: just after the opening line's `\n` up
    /// to the start of the closing line.
    pub inner: Range<usize>,
    /// The byte just after the closing line.
    pub end: usize,
}

/// A closed YAML/TOML frontmatter block of `text`; `None` when there is no
/// block or it is never closed. Tolerates both LF and CRLF.
pub(in crate::note) fn frontmatter_bounds(text: &str) -> Option<FrontmatterBounds<'_>> {
    let (delimiter, start) = frontmatter_delimiter(text)?;
    let mut offset = start;
    for line in text[start..].split('\n') {
        if line.trim_end_matches('\r') == delimiter {
            // Past the closing delimiter line, and its '\n' if present.
            let mut end = offset + line.len();
            if text.as_bytes().get(end) == Some(&b'\n') {
                end += 1;
            }
            return Some(FrontmatterBounds {
                delimiter,
                inner: start..offset,
                end,
            });
        }
        offset += line.len() + 1; // +1 for '\n'
    }
    None
}

/// Splits a leading UTF-8 byte-order mark off `text`: `("\u{feff}", rest)`, or
/// `("", text)` when there is none.
pub(in crate::note) fn split_bom(text: &str) -> (&str, &str) {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", text),
    }
}

#[cfg(test)]
mod test {
    use log::debug;

    use crate::{
        nfs::VaultPath,
        note::{
            content_extractor::{get_content_chunks, get_content_data},
            LinkType,
        },
    };

    use super::{
        get_markdown_and_links, is_remote_url, link_char_spans, link_target_filename,
        replace_note_links, target_looks_like_image, wikilink_char_spans, LinkSpanKind,
    };

    #[test]
    fn a_wikilink_in_code_is_not_an_indexed_link() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (chunks, links) = super::get_chunks_and_links(&path, "see `[[x]]` and [[y]]");
        let notes: Vec<String> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Note(p) => Some(p.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(
            notes,
            [crate::nfs::VaultPath::note_path_from("y").to_string()]
        );
        assert!(chunks[0].text.contains("`[[x]]`"), "{:?}", chunks[0].text);
    }

    #[test]
    fn a_wikilink_inside_a_markdown_link_label_is_link_text() {
        // `[see [[a]]](x.md)` is a link to `x.md` whose
        // text holds `[[a]]` — as CommonMark and the editor read it.
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (_, links) = super::get_chunks_and_links(&path, "[see [[a]]](x.md)");
        let raw: Vec<String> = links.into_iter().map(|l| l.raw_link).collect();
        assert_eq!(raw, [crate::nfs::VaultPath::new("x.md").to_string()]);
    }

    #[test]
    fn both_chunk_paths_render_a_tag_in_a_wikilink_alias_alike() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let plain = super::get_content_chunks("see [[a|#b]]");
        let (indexed, _) = super::get_chunks_and_links(&path, "see [[a|#b]]");
        assert_eq!(plain, indexed);
        assert_eq!(plain[0].text, "see #b");
    }

    #[test]
    fn a_hashtag_that_looks_like_a_list_marker_keeps_its_setext_heading() {
        let headings = crate::note::note_headings("#1. Intro\n---\n\nbody\n");
        assert_eq!(headings.len(), 1);
        assert_eq!(headings[0].text, "1. Intro");
        assert_eq!(headings[0].level, 2);
    }

    fn extract_outline(text: &str) -> Vec<(u8, String, usize)> {
        crate::note::note_headings(text)
            .into_iter()
            .map(|h| (h.level, h.text, h.line))
            .collect()
    }

    // ---- ByteToCharCursor / span tests on multi-byte input ----

    #[test]
    fn wikilink_char_spans_after_emoji() {
        // "👋 hello " = 8 chars (emoji = 1 char even though 4 bytes).
        let text = "👋 hello [[target]] world";
        let spans = wikilink_char_spans(text);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, 8);
        // "👋 hello [[target]]" = 8 + len("[[target]]")=10 = 18 chars.
        assert_eq!(spans[0].end, 18);
        assert_eq!(spans[0].target, "target");
    }

    #[test]
    fn link_char_spans_mixed_after_multibyte() {
        // 4-byte emoji + 2-byte é — verify byte→char conversion stays correct.
        let text = "café 🎯 [[wiki]] then [link](http://x) end";
        let spans = link_char_spans(text);
        assert_eq!(spans.len(), 2);
        let wiki = &spans[0];
        let md = &spans[1];
        // "café 🎯 " char count: c-a-f-é-space-🎯-space = 7 chars.
        assert_eq!(wiki.start, 7);
        // "[[wiki]]" = 8 chars; ends at 7+8=15.
        assert_eq!(wiki.end, 15);
        // " then " = 6 chars; md starts at 15+6=21; "[link](http://x)" = 16 chars.
        assert_eq!(md.start, 21);
        assert_eq!(md.end, 37);
    }

    #[test]
    fn link_char_spans_distinguishes_image_from_markdown() {
        let text = "see ![alt](img.png) and [click](http://x)";
        let spans = link_char_spans(text);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].kind, LinkSpanKind::Image);
        assert_eq!(spans[0].target, "img.png");
        assert_eq!(spans[1].kind, LinkSpanKind::Markdown);
        assert_eq!(spans[1].target, "http://x");
    }

    #[test]
    fn is_remote_url_accepts_http_and_https() {
        assert!(is_remote_url("http://example.com"));
        assert!(is_remote_url("https://example.com/path?q=1#frag"));
        assert!(is_remote_url("https://example.com:8080/x"));
        assert!(is_remote_url("https://user:pass@example.com/"));
        assert!(is_remote_url("http://localhost"));
        assert!(is_remote_url("http://127.0.0.1:3000"));
        assert!(is_remote_url("http://[::1]/"));
        assert!(is_remote_url("  https://example.com  "));
    }

    #[test]
    fn is_remote_url_rejects_other_schemes_and_garbage() {
        assert!(!is_remote_url("ftp://example.com"));
        assert!(!is_remote_url("file:///etc/passwd"));
        assert!(!is_remote_url("mailto:a@b.com"));
        assert!(!is_remote_url("javascript:alert(1)"));
        assert!(!is_remote_url("example.com"));
        assert!(!is_remote_url("/notes/x.md"));
        assert!(!is_remote_url(""));
        assert!(!is_remote_url("https://example.com\nmore"));
    }

    #[test]
    fn target_looks_like_image_extension_check() {
        assert!(target_looks_like_image("img.png"));
        assert!(target_looks_like_image("foo/bar.JPG"));
        assert!(target_looks_like_image("/assets/image_123.gif"));
        assert!(target_looks_like_image("https://example.com/x.webp?v=1"));
        assert!(target_looks_like_image("a.svg#frag"));
        assert!(!target_looks_like_image("note.md"));
        assert!(!target_looks_like_image("plain"));
        assert!(!target_looks_like_image("https://example.com"));
    }

    #[test]
    fn link_target_filename_returns_last_segment() {
        assert_eq!(link_target_filename("img.png"), "img.png");
        assert_eq!(
            link_target_filename("../../assets/image_123.png"),
            "image_123.png"
        );
        assert_eq!(
            link_target_filename("/assets/image_123.png"),
            "image_123.png"
        );
        assert_eq!(
            link_target_filename("https://example.com/path/img.png"),
            "img.png"
        );
        assert_eq!(
            link_target_filename("https://example.com/img.png?v=1"),
            "img.png"
        );
        assert_eq!(
            link_target_filename("https://example.com/img.png#frag"),
            "img.png"
        );
        assert_eq!(link_target_filename(""), "");
        assert_eq!(link_target_filename("/"), "");
    }

    #[test]
    fn wikilink_char_spans_back_to_back_after_multibyte() {
        // 🌍 = 1 char (4 bytes). Adjacent wikilinks must keep monotonic spans.
        let text = "🌍[[a]][[b]]";
        let spans = wikilink_char_spans(text);
        assert_eq!(spans.len(), 2);
        // "[[a]]" = 5 chars; first wiki at [1, 6).
        assert_eq!(spans[0].start, 1);
        assert_eq!(spans[0].end, 6);
        // Second wiki immediately after.
        assert_eq!(spans[1].start, 6);
        assert_eq!(spans[1].end, 11);
    }

    // ---- replace_note_links tests ----

    /// The note whose links are rewritten.
    fn victim() -> VaultPath {
        VaultPath::new("/notes/victim.md")
    }

    #[test]
    fn replace_wikilink_no_display() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("See [[old-note]].", &victim(), &old, &new);
        assert!(changed);
        assert_eq!(result, "See [[new-note]].");
    }

    #[test]
    fn replace_wikilink_with_display_text() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) =
            replace_note_links("See [[old-note|my note]].", &victim(), &old, &new);
        assert!(changed);
        assert_eq!(result, "See [[new-note|my note]].");
    }

    #[test]
    fn replace_markdown_link_full_path() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) =
            replace_note_links("[click](/notes/old-note.md)", &victim(), &old, &new);
        assert!(changed);
        assert_eq!(result, "[click](/notes/new-note.md)");
    }

    #[test]
    fn replace_markdown_link_filename_only() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("[click](old-note.md)", &victim(), &old, &new);
        assert!(changed);
        assert_eq!(result, "[click](new-note.md)");
    }

    #[test]
    fn replace_does_not_touch_unrelated_links() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let text = "[[other-note]] [x](/notes/unrelated.md) [y](unrelated.md)";
        let (result, changed) = replace_note_links(text, &victim(), &old, &new);
        assert!(!changed);
        assert_eq!(result, text);
    }

    #[test]
    fn replace_does_not_touch_images() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        // Images that happen to match the name must not be touched
        let text = "![old-note.md](old-note.md)";
        let (result, changed) = replace_note_links(text, &victim(), &old, &new);
        assert!(!changed);
        assert_eq!(result, text);
    }

    #[test]
    fn replace_mixed_content() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/archive/new-note.md");
        let text = "[[old-note]] and [[old-note|read this]] plus [link](/notes/old-note.md) end.";
        let (result, changed) = replace_note_links(text, &victim(), &old, &new);
        assert!(changed);
        assert_eq!(
            result,
            "[[new-note]] and [[new-note|read this]] plus [link](/archive/new-note.md) end."
        );
    }

    // A new name that would not read back as the same
    // link bare is written in `<…>`; one that does stays bare.
    #[test]
    fn replace_wraps_a_destination_that_would_not_read_back() {
        let old = VaultPath::new("/notes/plan.md");
        for (new, text, expected) in [
            ("/notes/my plan.md", "[x](plan.md)", "[x](my plan.md)"),
            (
                "/notes/my plan.md",
                "[x](plan.md \"T\")",
                "[x](<my plan.md> \"T\")",
            ),
            ("/notes/my plan.md", "[x](<plan.md>)", "[x](<my plan.md>)"),
            (
                "/notes/my plan.md",
                "[r]\n\n[r]: plan.md\n",
                "[r]\n\n[r]: <my plan.md>\n",
            ),
            ("/notes/p (1).md", "[x](plan.md#s)", "[x](<p (1).md#s>)"),
            ("/notes/p (1).md", "[[plan]]", "[[p (1)]]"),
        ] {
            let new = VaultPath::new(new);
            let (result, changed) = replace_note_links(text, &victim(), &old, &new);
            assert!(changed, "{text:?}");
            assert_eq!(result, expected);
        }
    }

    // A moved note's links are written for where they
    // live — a relative path from the linking note's folder — and a link
    // in code is code, left alone.
    #[test]
    fn replace_writes_a_moved_note_relative_to_the_linking_note() {
        let old = VaultPath::new("/notes/plan.md");
        let new = VaultPath::new("/archive/old/plan.md");
        let text = "[a](../notes/plan.md) [b](/notes/plan.md) [c](plan.md) `[d](plan.md)`";
        let at = VaultPath::new("/journal/x.md");
        let (result, _) = replace_note_links(text, &at, &old, &new);
        assert_eq!(
            result,
            "[a](../archive/old/plan.md) [b](/archive/old/plan.md) [c](plan.md) `[d](plan.md)`"
        );
    }

    #[test]
    fn replace_returns_unchanged_false_when_no_match() {
        let old = VaultPath::new("/notes/missing.md");
        let new = VaultPath::new("/notes/also-missing.md");
        let text = "No references here at all.";
        let (result, changed) = replace_note_links(text, &victim(), &old, &new);
        assert!(!changed);
        assert_eq!(result, text);
    }

    #[test]
    fn convert_wiki_link() {
        let markdown = r#"Here is a [[Wikilink|text with link]]"#;

        let (md, _) = get_markdown_and_links(&VaultPath::root(), markdown);

        assert_eq!(md, "Here is a [text with link](wikilink.md)");
    }

    #[test]
    fn convert_many_wiki_links() {
        let markdown = r#"Here is a [[Wikilink|text with link]], and another [[Link]] this time without text.

    And a [[https://example.com|url link]]"#;

        let (md, _) = get_markdown_and_links(&VaultPath::root(), markdown);

        assert_eq!(
            md,
            r#"Here is a [text with link](wikilink.md), and another [Link](link.md) this time without text.

    And a [[https://example.com|url link]]"#
        );
    }

    #[test]
    fn ignore_image_links() {
        let markdown = r#"This is an ![image](image.png)"#;

        let (_md, links) = get_markdown_and_links(&VaultPath::root(), markdown);

        assert!(links.is_empty());
    }

    #[test]
    fn extract_relative_link_from_text() {
        let markdown =
            r#"This is a [link](../main.md) to a note, this is a [non](:caca) valid link"#;
        let note_path = VaultPath::new("/directory/test_note.md");

        let (_md, links) = get_markdown_and_links(&note_path, markdown);

        assert_eq!(1, links.len());
        let link = links.first().unwrap();
        assert_eq!("link", link.text);
        assert_eq!(LinkType::Note(VaultPath::new("/main.md")), link.ltype);
    }

    #[test]
    fn extract_link_from_text() {
        let markdown =
            r#"This is a [link](notes/main.md) to a note, this is a [non](:caca) valid link"#;

        let note_path = VaultPath::new("/test_note.md");
        let (_md, links) = get_markdown_and_links(&note_path, markdown);

        assert_eq!(1, links.len());
        let link = links.first().unwrap();
        assert_eq!("link", link.text);
        assert_eq!(LinkType::Note(VaultPath::new("/notes/main.md")), link.ltype);
    }

    #[test]
    fn extract_many_links_from_text() {
        // The third line used to be indented 4 spaces, which made it an
        // indented code block (links in code are not links); dedented so all
        // three links are prose, as the test means.
        let markdown = r#"This is a [link](notes/main.md) to a note, this is a [[note.md]]] valid link

Here's a [url](https://www.example.com)"#;

        let note_path = VaultPath::new("/test_note.md");
        let (_md, links) = get_markdown_and_links(&note_path, markdown);

        assert_eq!(3, links.len());
        // Now has an absolute path
        assert!(links.iter().any(|link| {
            let path = VaultPath::new("/notes/main.md");
            link.text.eq("link") && link.ltype.eq(&LinkType::Note(path))
        }));
        assert!(links.iter().any(|link| {
            let path = VaultPath::new("note.md");
            link.text.eq("note.md") && link.ltype.eq(&LinkType::Note(path))
        }));
        assert!(links.iter().any(|link| {
            debug!("{:?}", link);
            let url = "https://www.example.com".to_string();
            link.text.eq("url") && link.ltype.eq(&LinkType::Url) && link.raw_link.eq(&url)
        }));
    }

    // Indexing reads title, hash, chunks and links from
    // one walk, the same as the separate extractors.
    #[test]
    fn index_data_is_the_separate_extractors_from_one_walk() {
        let path = VaultPath::new("/dir/n.md");
        let text = "---\ntags: [x]\n---\n# Title #t\nsee [[a]] and [b](sub/b.md)\n## Next\nmore";
        let (data, chunks, links) = crate::note::NoteDetails::index_data_of(&path, text);
        assert_eq!(data, get_content_data(text));
        assert_eq!(
            (chunks, links),
            crate::note::NoteDetails::chunks_and_links_of(&path, text)
        );
    }

    #[test]
    fn check_title_yaml_frontmatter() {
        let markdown = r#"---
something: nice
other: else
---

title"#;
        let content_chunks = get_content_chunks(markdown);

        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!("title", content_chunks[0].get_text());
        assert_eq!("FrontMatter", content_chunks[1].get_breadcrumb());
        assert_eq!("something: nice\nother: else", content_chunks[1].get_text());
    }

    #[test]
    fn check_title_toml_frontmatter() {
        let markdown = r#"+++
something: nice
other: else
+++

title"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(2, content_chunks.len());
        assert_eq!("title".to_string(), data.title);
        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!("title", content_chunks[0].get_text());
        assert_eq!("FrontMatter", content_chunks[1].get_breadcrumb());
        assert_eq!("something: nice\nother: else", content_chunks[1].get_text());
    }

    #[test]
    fn check_title_in_list() {
        let markdown = r#"- First Item
- Second Item

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("First Item".to_string(), data.title);
        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "* First Item\n* Second Item\nSome text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn convert_list() {
        let markdown = r#"# Title

- First *Item*
- Second Item

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "* First Item\n* Second Item\nSome text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn convert_list_two_level() {
        let markdown = r#"# Title

- First Item
    - First subitem
    - Second subitem
- Second Item

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "* First Item\n    * First subitem\n    * Second subitem\n* Second Item\nSome text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn convert_list_empty_item() {
        let markdown = r#"# Title

- First Item
- Second Item
-

"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("* First Item\n* Second Item", content_chunks[0].get_text());
    }

    #[test]
    fn check_title_no_header() {
        let markdown = r#"[No header](https://example.com)

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("No header".to_string(), data.title);
        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!("No header\nSome text", content_chunks[0].get_text());
    }

    #[test]
    fn check_hierarchy_one() {
        let markdown = r#"# Title
Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
    }

    #[test]
    fn check_hierarchy_two() {
        let markdown = r#"# Title
Some text

## Subtitle
More text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(2, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!(
            format!("Title{0}Subtitle", crate::note::BREADCRUMB_SEP),
            content_chunks[1].get_breadcrumb()
        );
        assert_eq!("More text", content_chunks[1].get_text());
    }

    #[test]
    fn check_hierarchy_three() {
        let markdown = r#"# Title
Some text

## Subtitle
More text

### Subsubtitle
Even more text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(3, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!(
            format!("Title{0}Subtitle", crate::note::BREADCRUMB_SEP),
            content_chunks[1].get_breadcrumb()
        );
        assert_eq!("More text", content_chunks[1].get_text());
        assert_eq!(
            format!(
                "Title{0}Subtitle{0}Subsubtitle",
                crate::note::BREADCRUMB_SEP
            ),
            content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", content_chunks[2].get_text());
    }

    #[test]
    fn check_nested_hierarchy_three() {
        let markdown = r#"# Title
Some text

## Subtitle
More text

### Subsubtitle
Even more text

## Level 2 Title
There is text here"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(4, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!(
            format!("Title{0}Subtitle", crate::note::BREADCRUMB_SEP),
            content_chunks[1].get_breadcrumb()
        );
        assert_eq!("More text", content_chunks[1].get_text());
        assert_eq!(
            format!(
                "Title{0}Subtitle{0}Subsubtitle",
                crate::note::BREADCRUMB_SEP
            ),
            content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", content_chunks[2].get_text());
        assert_eq!(
            format!("Title{0}Level 2 Title", crate::note::BREADCRUMB_SEP),
            content_chunks[3].get_breadcrumb()
        );
        assert_eq!("There is text here", content_chunks[3].get_text());
    }

    #[test]
    fn check_nested_hierarchy_four() {
        let markdown = r#"# Title
Some text

## Subtitle
More text

### Subsubtitle
Even more text

## Level 2 Title
There is text here

### Fourth Subsubtitle
Before last text

# Main Title
Another main content
"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(6, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!(
            format!("Title{0}Subtitle", crate::note::BREADCRUMB_SEP),
            content_chunks[1].get_breadcrumb()
        );
        assert_eq!("More text", content_chunks[1].get_text());
        assert_eq!(
            format!(
                "Title{0}Subtitle{0}Subsubtitle",
                crate::note::BREADCRUMB_SEP
            ),
            content_chunks[2].get_breadcrumb()
        );
        assert_eq!("Even more text", content_chunks[2].get_text());
        assert_eq!(
            format!("Title{0}Level 2 Title", crate::note::BREADCRUMB_SEP),
            content_chunks[3].get_breadcrumb()
        );
        assert_eq!("There is text here", content_chunks[3].get_text());
        assert_eq!(
            format!(
                "Title{0}Level 2 Title{0}Fourth Subsubtitle",
                crate::note::BREADCRUMB_SEP
            ),
            content_chunks[4].get_breadcrumb()
        );
        assert_eq!("Before last text", content_chunks[4].get_text());
        assert_eq!("Main Title", content_chunks[5].get_breadcrumb());
        assert_eq!("Another main content", content_chunks[5].get_text());
    }

    #[test]
    fn check_nested_hierarchy_four_jump() {
        let markdown = r#"# Title
Some text

### Subtitle
More text

# Subsubtitle
Even more text

#### Level 2 Title
There is text here

## Fourth Subsubtitle
Before last text

# Main Title
Another main content
"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(6, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
        assert_eq!(
            format!("Title{0}Subtitle", crate::note::BREADCRUMB_SEP),
            content_chunks[1].get_breadcrumb()
        );
        assert_eq!("More text", content_chunks[1].get_text());
        assert_eq!("Subsubtitle", content_chunks[2].get_breadcrumb());
        assert_eq!("Even more text", content_chunks[2].get_text());
        assert_eq!(
            format!("Subsubtitle{0}Level 2 Title", crate::note::BREADCRUMB_SEP),
            content_chunks[3].get_breadcrumb()
        );
        assert_eq!("There is text here", content_chunks[3].get_text());
        assert_eq!(
            format!(
                "Subsubtitle{0}Fourth Subsubtitle",
                crate::note::BREADCRUMB_SEP
            ),
            content_chunks[4].get_breadcrumb()
        );
        assert_eq!("Before last text", content_chunks[4].get_text());
        assert_eq!("Main Title", content_chunks[5].get_breadcrumb());
        assert_eq!("Another main content", content_chunks[5].get_text());
    }

    #[test]
    fn check_title_with_link() {
        let markdown = r#"# [Title link](https://nico.red)
Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title link".to_string(), data.title);
        assert_eq!("Title link", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
    }

    #[test]
    fn check_title_with_style() {
        let markdown = r#"# Title **bold** *italic*
Some text"#;
        let content_chunks = get_content_chunks(markdown);
        debug!("===================================");
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title bold italic".to_string(), data.title);
        assert_eq!("Title bold italic", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text", content_chunks[0].get_text());
    }

    #[test]
    fn check_content_without_title() {
        let markdown = r#"Intro text

# Title

Some text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(2, content_chunks.len());
        assert_eq!("Intro text".to_string(), data.title);
        assert_eq!("", content_chunks[0].get_breadcrumb());
        assert_eq!("Intro text", content_chunks[0].get_text());
        assert_eq!("Title", content_chunks[1].get_breadcrumb());
        assert_eq!("Some text", content_chunks[1].get_text());
    }

    #[test]
    fn check_content_with_link() {
        let markdown = r#"# Title

[Some text linking](www.example.com)"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text linking", content_chunks[0].get_text());
    }

    #[test]
    fn check_content_with_wikilink() {
        let markdown = r#"# Title

[[Some text linking]]"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!("Some text linking", content_chunks[0].get_text());
    }

    #[test]
    fn check_content_with_hashtags() {
        let markdown = r#"# Title

Some text, #hashtag and more text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "Some text, hashtag and more text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn check_code() {
        let markdown = r#"# Title

Some text, `code` and more text"#;
        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "Some text, `code` and more text",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn check_code_block() {
        let markdown = r#"# Title

Some text

```bash
mkdir test
ls -la ./test
```"#;

        let content_chunks = get_content_chunks(markdown);
        let data = get_content_data(markdown);

        assert_eq!(1, content_chunks.len());
        assert_eq!("Title".to_string(), data.title);
        assert_eq!("Title", content_chunks[0].get_breadcrumb());
        assert_eq!(
            "Some text\n```bash\nmkdir test\nls -la ./test\n```",
            content_chunks[0].get_text()
        );
    }

    #[test]
    fn extract_hashtags_as_links() {
        let markdown = r#"Some text with #hashtag and another #tag123"#;

        let (md, links) = get_markdown_and_links(&VaultPath::root(), markdown);

        assert_eq!(2, links.len());
        assert!(links.iter().any(|link| {
            link.text.eq("hashtag")
                && link.ltype.eq(&LinkType::Hashtag)
                && link.raw_link.eq("#hashtag")
        }));
        assert!(links.iter().any(|link| {
            link.text.eq("tag123")
                && link.ltype.eq(&LinkType::Hashtag)
                && link.raw_link.eq("#tag123")
        }));
        assert_eq!(
            md,
            "Some text with [#hashtag](#hashtag) and another [#tag123](#tag123)"
        );
    }

    // --- get_content_chunks: new / regression tests ---

    #[test]
    fn empty_note_produces_no_chunks() {
        let chunks = get_content_chunks("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn only_frontmatter_produces_one_chunk() {
        let markdown = "---\ntitle: Hello\n---";
        let chunks = get_content_chunks(markdown);
        // Only the FrontMatter chunk; no body content.
        assert_eq!(1, chunks.len());
        assert_eq!("FrontMatter", chunks[0].get_breadcrumb());
    }

    #[test]
    fn adjacent_headers_no_empty_chunks() {
        // When two headers are back-to-back the first produces no body text,
        // so no empty chunk should be emitted.
        let markdown = "# Title\n## Subtitle\nSome text";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!(
            format!("Title{0}Subtitle", crate::note::BREADCRUMB_SEP),
            chunks[0].get_breadcrumb()
        );
        assert_eq!("Some text", chunks[0].get_text());
    }

    #[test]
    fn link_with_title_attribute_keeps_only_link_text() {
        // [text](url "title") — "title" must NOT appear in the chunk content.
        let markdown = "# Section\n[visit here](https://example.com \"My Site\")";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!("visit here", chunks[0].get_text());
    }

    #[test]
    fn image_alt_text_is_kept_not_title() {
        // ![alt](img.png "tooltip") — only alt text should appear.
        let markdown = "# Section\n![an image](photo.png \"Photo title\")";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!("an image", chunks[0].get_text());
    }

    #[test]
    fn wikilink_multi_pipe_uses_display_text() {
        // [[link|display|extra]] — display text should be kept, extra part ignored.
        let markdown = "# Section\n[[note|display text|ignored extra]]";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!("display text", chunks[0].get_text());
    }

    #[test]
    fn header_only_note_no_body_chunk() {
        // A note that is only a header line with no following text.
        let markdown = "# Just a title";
        let chunks = get_content_chunks(markdown);
        // No body text → no chunk should be emitted.
        assert!(chunks.is_empty());
    }

    #[test]
    fn deeply_nested_headers() {
        let markdown = "# H1\n## H2\n### H3\n#### H4\ntext";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        assert_eq!(
            format!("H1{0}H2{0}H3{0}H4", crate::note::BREADCRUMB_SEP),
            chunks[0].get_breadcrumb()
        );
        assert_eq!("text", chunks[0].get_text());
    }

    #[test]
    fn breadcrumb_preserves_heading_with_gt_char() {
        // Heading text containing `>` must round-trip through breadcrumb_parts
        // — earlier representations split on `>` and fabricated phantom parents.
        let markdown = "# Foo > Bar\nbody text\n";
        let chunks = get_content_chunks(markdown);
        assert_eq!(1, chunks.len());
        let parts: Vec<&str> = chunks[0].breadcrumb_parts().collect();
        assert_eq!(parts, vec!["Foo > Bar"]);
        assert_eq!(chunks[0].breadcrumb_last(), Some("Foo > Bar"));
    }

    #[test]
    fn unclosed_frontmatter_treated_as_body() {
        // An unclosed frontmatter block should not swallow the whole document.
        let markdown = "---\ntitle: Hello\nSome actual content";
        let chunks = get_content_chunks(markdown);
        // No FrontMatter chunk (delimiter never closed),
        // body should contain the remaining lines.
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.get_breadcrumb() != "FrontMatter"));
    }

    #[test]
    fn extract_mixed_links_and_hashtags() {
        let markdown =
            r#"This is a [link](note.md) and #hashtag with [[wikilink]] and #another_tag"#;

        let note_path = VaultPath::new("/test_note.md");
        let (_md, links) = get_markdown_and_links(&note_path, markdown);

        assert_eq!(4, links.len());
        // Check for note links
        assert_eq!(
            2,
            links
                .iter()
                .filter(|l| matches!(l.ltype, LinkType::Note(_)))
                .count()
        );
        // Check for hashtags
        assert_eq!(
            2,
            links
                .iter()
                .filter(|l| matches!(l.ltype, LinkType::Hashtag))
                .count()
        );
        assert!(links
            .iter()
            .any(|link| link.text.eq("hashtag") && link.ltype.eq(&LinkType::Hashtag)));
        assert!(links
            .iter()
            .any(|link| link.text.eq("another_tag") && link.ltype.eq(&LinkType::Hashtag)));
    }

    #[test]
    fn code_char_ranges_inline_code() {
        let md = "hello `#notalabel` and #real";
        let ranges = super::code_char_ranges(md);
        assert!(
            ranges.iter().any(|(s, e)| md[*s..*e].contains("notalabel")),
            "inline code span must be reported"
        );
        assert!(
            ranges.iter().all(|(s, e)| !md[*s..*e].contains("#real")),
            "non-code text must not be reported"
        );
    }

    #[test]
    fn code_char_ranges_fenced_block() {
        let md = "before\n```\n#inside\n```\nafter #outside";
        let ranges = super::code_char_ranges(md);
        assert!(
            ranges.iter().any(|(s, e)| md[*s..*e].contains("#inside")),
            "fenced block content must be reported"
        );
        assert!(
            ranges.iter().all(|(s, e)| !md[*s..*e].contains("#outside")),
            "text after fence must not be reported"
        );
    }

    #[test]
    fn code_char_ranges_none_for_plain_text() {
        let md = "no code here, just #tags";
        let ranges = super::code_char_ranges(md);
        assert!(ranges.is_empty(), "plain text yields no code ranges");
    }

    #[test]
    fn hashtag_in_inline_code_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (text, links) = super::get_markdown_and_links(&path, "use `#notalabel` and tag #real");
        assert!(
            links
                .iter()
                .all(|l| !matches!(&l.ltype, super::super::LinkType::Hashtag)
                    || l.text != "notalabel"),
            "hashtag inside inline code must not become a hashtag link"
        );
        assert!(
            links
                .iter()
                .any(|l| matches!(&l.ltype, super::super::LinkType::Hashtag) && l.text == "real"),
            "hashtag outside code is still extracted"
        );
        assert!(
            text.contains("`#notalabel`"),
            "inline code literal is preserved in rendered output: {}",
            text
        );
    }

    #[test]
    fn hashtag_in_fenced_block_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "before\n```\n#inside\n```\nafter #outside";
        let (_text, links) = super::get_markdown_and_links(&path, body);
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(hashtag_names, vec!["outside"]);
    }

    #[test]
    fn hashtag_terminates_at_non_label_char() {
        // `#tag-with-dash` yields the label `tag` and the rest
        // (`-with-dash`) is treated as following text. `HASHTAG_RX` already
        // enforces this because `[A-Za-z0-9_]+` stops at `-`.
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (_text, links) = super::get_markdown_and_links(&path, "x #tag-with-dash y");
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(hashtag_names, vec!["tag"]);
    }

    #[test]
    fn hashtag_inside_markdown_link_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "see [docs](https://example.com#section) and #real";
        let (text, links) = super::get_markdown_and_links(&path, body);

        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            hashtag_names,
            vec!["real"],
            "URL fragment must not become a label"
        );

        assert!(
            text.contains("https://example.com#section"),
            "link href must be preserved verbatim: {}",
            text
        );
        assert!(
            !text.contains("[#section](#section)"),
            "URL fragment must not be rewritten into a nested markdown link: {}",
            text
        );
    }

    #[test]
    fn hashtag_inside_html_comment_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "<!-- #internal -->\nplain #real";
        let (_text, links) = super::get_markdown_and_links(&path, body);
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(hashtag_names, vec!["real"]);
    }

    #[test]
    fn hashtag_inside_inline_html_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = r##"text <a data-foo="#bar">label</a> and #real"##;
        let (_text, links) = super::get_markdown_and_links(&path, body);
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(hashtag_names, vec!["real"]);
    }

    #[test]
    fn hashtag_needs_word_boundary_before() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");

        // Hex colour in prose: `#ffcc00` IS at a word boundary (preceded by space)
        // so it counts. That's the existing behavior; we don't change it. But
        // glued-to-text variants must NOT match.
        let (_text, links) = super::get_markdown_and_links(&path, "foo#bar baz#qux");
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            hashtag_names.is_empty(),
            "no label should be extracted when `#` is preceded by a label-character: {:?}",
            hashtag_names
        );
    }

    #[test]
    fn hashtag_at_start_of_line_still_works() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (_text, links) = super::get_markdown_and_links(&path, "#first line\nsecond #second");
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(hashtag_names, vec!["first", "second"]);
    }

    #[test]
    fn label_matches_skips_after_label_char() {
        let v: Vec<&str> = crate::note::scan::label_matches("foo#bar and #real")
            .map(|m| m.name)
            .collect();
        assert_eq!(v, vec!["real"]);
    }

    #[test]
    fn label_matches_at_start() {
        let v: Vec<&str> = crate::note::scan::label_matches("#first and #second")
            .map(|m| m.name)
            .collect();
        assert_eq!(v, vec!["first", "second"]);
    }

    #[test]
    fn label_matches_skips_double_hash() {
        let v: Vec<&str> = crate::note::scan::label_matches("##tag and ##other")
            .map(|m| m.name)
            .collect();
        assert!(v.is_empty(), "expected no labels, got {:?}", v);
    }

    #[test]
    fn label_matches_skips_triple_hash() {
        let v: Vec<&str> = crate::note::scan::label_matches("###tag")
            .map(|m| m.name)
            .collect();
        assert!(v.is_empty(), "expected no labels, got {:?}", v);
    }

    #[test]
    fn label_matches_skips_adjacent_hash() {
        // `#tag#more` — `#` immediately follows the label, so neither side
        // is a real tag boundary; reject both.
        let v: Vec<&str> = crate::note::scan::label_matches("#tag#more")
            .map(|m| m.name)
            .collect();
        assert!(v.is_empty(), "expected no labels, got {:?}", v);
    }

    // A `#` right after `&` (an HTML entity, `it&#39;s`)
    // is not a label, here as in every view built on the shared rule.
    #[test]
    fn label_matches_skips_after_ampersand() {
        let v: Vec<&str> = crate::note::scan::label_matches("it&#39;s Title&#32; a #tag &x #tag2")
            .map(|m| m.name)
            .collect();
        assert_eq!(v, vec!["tag", "tag2"]);
    }

    #[test]
    fn label_matches_after_space_then_hash() {
        let v: Vec<&str> = crate::note::scan::label_matches("# #tag")
            .map(|m| m.name)
            .collect();
        assert_eq!(v, vec!["tag"]);
    }

    // These pinned the deleted `cleanup_hashtags` rewrite; the walk now does
    // its job, so they assert the same rule through the chunker.
    #[test]
    fn chunk_hashtags_preserve_inline_code_span() {
        // `#tag` inside backticks must stay verbatim: the indexed chunk
        // should match the source text, not strip the leading `#`.
        let chunks = get_content_chunks("Use `#define X` to set X.");
        assert_eq!(chunks[0].text, "Use `#define X` to set X.");
    }

    #[test]
    fn chunk_hashtags_preserve_inside_markdown_link() {
        // `#section` inside the URL of a markdown link is not a tag.
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (chunks, links) =
            super::get_chunks_and_links(&path, "see [docs](page.md#section) for details");
        assert_eq!(chunks[0].text, "see docs for details");
        assert!(
            !links.iter().any(|l| matches!(l.ltype, LinkType::Hashtag)),
            "{links:?}"
        );
    }

    #[test]
    fn chunk_hashtags_strip_outside_excluded_zones() {
        let chunks = get_content_chunks("plain #tag and `#code` mixed");
        assert_eq!(chunks[0].text, "plain tag and `#code` mixed");
    }

    #[test]
    fn hashtag_in_frontmatter_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "---\ndescription: see #wip note\n---\nbody with #real";
        let (_text, links) = super::get_markdown_and_links(&path, body);
        let names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["real"]);
    }

    #[test]
    fn get_chunks_and_links_skips_hashtag_inside_image_alt() {
        // `![alt #tag](img.png)` — hashtag inside image alt text must not
        // emit a `NoteLink::Hashtag` and must stay verbatim in chunk text.
        // Mirrors the prior `cleanup_hashtags_with_ranges` exclusion-zone
        // rule (MD_LINK_RX matched both `[..](..)` and `![..](..)` forms).
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "preview ![alt #draft](img.png) here";
        let (chunks, links) = super::get_chunks_and_links(&path, body);
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            hashtag_names.is_empty(),
            "hashtag inside image alt must not emit, got {:?}",
            hashtag_names
        );
        let body_chunk = chunks
            .iter()
            .find(|c| c.breadcrumb != "FrontMatter")
            .expect("body chunk");
        assert!(
            body_chunk.text.contains("#draft"),
            "expected `#draft` verbatim in chunk text, got {:?}",
            body_chunk.text
        );
    }

    #[test]
    fn get_chunks_and_links_skips_hashtag_inside_invalid_wikilink() {
        // `[[#wiki_tag]]` — `#` is not a valid vault-path character, so
        // the wikilink doesn't produce a Note link. The display text
        // `#wiki_tag` ends up in `body_stripped`, but the indexer must
        // not treat it as a hashtag — it originated from a wikilink and
        // the previous `get_markdown_and_links` pipeline excluded it via
        // `md_wikilink_char_ranges`.
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "see [[#wiki_tag]] for context and a real #real_tag";
        let (_chunks, links) = super::get_chunks_and_links(&path, body);
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            hashtag_names,
            vec!["real_tag"],
            "wikilink-derived hashtag must not leak into indexed labels"
        );
    }

    #[test]
    fn get_chunks_and_links_skips_hashtag_inside_wikilink_alias() {
        // `[[foo|see #bar]]` — the alias text contains a hashtag. After
        // wikilink collapse the body reads `see #bar`, but the hashtag
        // originated from within a wikilink and must not be indexed.
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "context: [[foo|see #bar]] then a body tag #other";
        let (_chunks, links) = super::get_chunks_and_links(&path, body);
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            hashtag_names,
            vec!["other"],
            "hashtag inside wikilink alias leaked into labels: {:?}",
            hashtag_names
        );
    }

    #[test]
    fn get_chunks_and_links_preserves_hashtag_inside_link_body() {
        // `#tag` inside a markdown link's display text must stay verbatim
        // (not stripped) and NOT emit a `NoteLink::Hashtag`. Mirrors the
        // previous `cleanup_hashtags_with_ranges` exclusion-zone rule.
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "see [issue #42 board](board.md) for backlog";
        let (chunks, links) = super::get_chunks_and_links(&path, body);
        let hashtag_names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            hashtag_names.is_empty(),
            "hashtag inside link body must not emit Hashtag link, got {:?}",
            hashtag_names
        );
        // Chunk text retains the link display text with `#` intact.
        let body_chunk = chunks
            .iter()
            .find(|c| c.breadcrumb != "FrontMatter")
            .expect("body chunk");
        assert!(
            body_chunk.text.contains("#42"),
            "expected `#42` to survive in chunk text, got {:?}",
            body_chunk.text
        );
    }

    #[test]
    fn get_chunks_and_links_skips_links_inside_frontmatter() {
        // `get_chunks_and_links` removes frontmatter before extracting
        // links — so a wikilink or markdown link inside the YAML/TOML
        // header is NOT pushed as a `NoteLink`. Pins the indexing-side
        // behavior so a future refactor cannot silently re-include
        // frontmatter targets.
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "---\nrelated: see [docs](other.md) and [[refnote]]\n---\nbody with [[real]]";
        let (_chunks, links) = super::get_chunks_and_links(&path, body);
        let note_targets: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Note(p) => Some(p.to_bare_string()),
                _ => None,
            })
            .map(|s| -> &str {
                // leak intentional for test ergonomics — short-lived process
                Box::leak(s.into_boxed_str())
            })
            .collect();
        // Only the body wikilink should appear; frontmatter targets dropped.
        assert!(
            note_targets.iter().any(|t| t.contains("real")),
            "expected body wikilink target, got {:?}",
            note_targets
        );
        assert!(
            !note_targets.iter().any(|t| t.contains("refnote")),
            "frontmatter wikilink leaked into links: {:?}",
            note_targets
        );
        assert!(
            !note_targets.iter().any(|t| t.contains("other")),
            "frontmatter markdown-link leaked into links: {:?}",
            note_targets
        );
    }

    #[test]
    fn hashtag_after_unicode_letter_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (_text, links) = super::get_markdown_and_links(&path, "café#draft and plain #real");
        let names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec!["real"],
            "Unicode letter before # must suppress label extraction"
        );
    }

    #[test]
    fn hashtag_followed_by_unicode_letter_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (_text, links) = super::get_markdown_and_links(&path, "#naïve and plain #real");
        let names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec!["real"],
            "non-ASCII letter immediately after match must suppress label"
        );
    }

    #[test]
    fn hashtag_followed_by_dash_still_extracted() {
        // Confirms #tag-with-dash still produces label `tag`.
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (_text, links) = super::get_markdown_and_links(&path, "#tag-with-dash");
        let names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec!["tag"],
            "ASCII non-alphanumeric (dash) after match still allows extraction"
        );
    }

    #[test]
    fn hashtag_in_crlf_frontmatter_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        // Windows CRLF line endings.
        let body = "---\r\ndescription: see #wip note\r\n---\r\nbody with #real";
        let (_text, links) = super::get_markdown_and_links(&path, body);
        let names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec!["real"],
            "frontmatter hashtag must not be extracted regardless of line endings"
        );
    }

    #[test]
    fn hashtag_inside_wikilink_is_not_extracted() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let body = "[[#wiki_tag]] and plain #real";
        let (_text, links) = super::get_markdown_and_links(&path, body);
        let names: Vec<&str> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Hashtag => Some(l.text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec!["real"],
            "hashtag inside wikilink (invalid form) must not become a label"
        );
    }

    use super::is_inside_exclusion_zone;

    #[test]
    fn exclusion_zone_plain_text_returns_false() {
        let text = "some plain text without anything special";
        for i in 0..=text.len() {
            assert!(
                !is_inside_exclusion_zone(text, i),
                "offset {} should not be in zone",
                i
            );
        }
    }

    #[test]
    fn exclusion_zone_inside_inline_code() {
        let text = "before `code` after";
        //          0123456789012345678
        // backticks at 7 and 12; cursor between them is inside
        assert!(is_inside_exclusion_zone(text, 10));
    }

    #[test]
    fn exclusion_zone_at_inline_code_boundary_is_outside() {
        let text = "before `code` after";
        // Cursor at the opening backtick (7) is at the boundary — treated
        // as outside, so the user can type a new wikilink/hashtag adjacent
        // to existing code.
        assert!(!is_inside_exclusion_zone(text, 7));
        // Same for the position right after the closing backtick.
        assert!(!is_inside_exclusion_zone(text, 13));
    }

    #[test]
    fn exclusion_zone_inside_closed_wikilink() {
        let text = "see [[Notes]] here";
        // `[[Notes]]` spans 4..=12 inclusive; cursor inside it.
        assert!(is_inside_exclusion_zone(text, 7));
    }

    #[test]
    fn exclusion_zone_open_wikilink_does_not_exclude() {
        let text = "see [[Notes here";
        // The wikilink is unterminated; the regex does not match it. The
        // popup must be allowed to open while the user is typing inside it.
        assert!(!is_inside_exclusion_zone(text, 10));
    }

    #[test]
    fn exclusion_zone_inside_fenced_code_block() {
        let text = "para\n\n```\n#tag-inside-fence\n```\n\nafter";
        // Cursor somewhere inside the fenced region (within "#tag" itself).
        let inside = text.find("#tag").unwrap() + 1;
        assert!(is_inside_exclusion_zone(text, inside));
    }

    #[test]
    fn exclusion_zone_inside_frontmatter() {
        let text = "---\ntitle: Hi\n---\n\nbody #real";
        // The byte just before `Hi` is inside the frontmatter.
        let inside = text.find("Hi").unwrap();
        assert!(is_inside_exclusion_zone(text, inside));
        // The hashtag in the body is not.
        let outside = text.find("#real").unwrap() + 1;
        assert!(!is_inside_exclusion_zone(text, outside));
    }

    #[test]
    fn exclusion_zone_inside_markdown_link() {
        let text = "go to [click](https://example.com#section) now";
        // Cursor inside the URL portion of the link (after the `#`).
        let inside = text.find("#section").unwrap() + 1;
        assert!(is_inside_exclusion_zone(text, inside));
    }

    #[test]
    fn exclusion_zone_offset_past_end_returns_false() {
        let text = "short";
        assert!(!is_inside_exclusion_zone(text, text.len() + 5));
    }

    use super::{atx_heading_text, heading_display_text, heading_section_range};

    #[test]
    fn heading_section_range_stops_at_the_next_heading() {
        let note = "# a\nfirst\n# b\nsecond\n# c\nthird\n";
        let r = heading_section_range(note, "b").unwrap();
        assert_eq!(&note[r], "# b\nsecond\n");
    }

    #[test]
    fn heading_section_range_runs_to_note_end_when_last() {
        // Distinct from the "stops at the next heading" case above: no
        // trailing heading exists, so the range must reach the note's end.
        let note = "# a\nfirst\n# b\nsecond and third\nfourth\n";
        let r = heading_section_range(note, "b").unwrap();
        assert_eq!(&note[r], "# b\nsecond and third\nfourth\n");
    }

    #[test]
    fn heading_section_range_none_when_heading_absent() {
        assert!(heading_section_range("# a\nfirst\n", "missing").is_none());
    }

    #[test]
    fn heading_section_range_matches_diacritics_stripped_and_case_insensitive() {
        // The note's raw heading keeps its accent; a server-normalized
        // `heading` argument ("cafe", stripped + lowercased) must still hit.
        let note = "# Café\nespresso and pastries\n";
        let r = heading_section_range(note, "cafe").unwrap();
        assert_eq!(&note[r], "# Café\nespresso and pastries\n");
    }

    #[test]
    fn heading_section_range_ignores_a_bare_hashtag_line() {
        // "#projects" has no space after the `#` run — a hashtag, not a
        // heading — so it must not satisfy a `heading` lookup of "projects".
        let note = "#projects\nnot a heading\n";
        assert!(heading_section_range(note, "projects").is_none());
    }

    #[test]
    fn heading_section_range_strips_a_closing_hash_trailer() {
        let note = "## Title ##\nbody\n";
        let r = heading_section_range(note, "Title").unwrap();
        assert_eq!(&note[r], "## Title ##\nbody\n");
    }

    #[test]
    fn atx_heading_text_rejects_a_bare_hashtag() {
        assert_eq!(atx_heading_text("#projects"), None);
    }

    #[test]
    fn atx_heading_text_accepts_a_plain_heading() {
        assert_eq!(atx_heading_text("## Title"), Some("Title"));
    }

    #[test]
    fn atx_heading_text_strips_closing_run() {
        assert_eq!(atx_heading_text("## Title ##"), Some("Title"));
        assert_eq!(atx_heading_text("# ###"), Some(""));
    }

    #[test]
    fn atx_heading_text_keeps_a_trailing_hash_without_preceding_space() {
        // "C#" (the language) has no space before its trailing `#` — not a
        // closing sequence, so it must survive verbatim.
        assert_eq!(atx_heading_text("## C#"), Some("C#"));
    }

    #[test]
    fn atx_heading_text_rejects_more_than_six_hashes() {
        assert_eq!(atx_heading_text("####### Title"), None);
    }

    #[test]
    fn heading_display_text_renders_like_the_chunker() {
        let cases = [
            ("# Top", "Top"),
            ("## **Sub** One ##", "Sub One"),
            ("# See [[other note]]", "See other note"),
            ("# See [[other note|alias]]", "See alias"),
            ("# Sprint #42", "Sprint 42"),
            ("# Docs [here](https://example.com)", "Docs here"),
            ("# Tags #planning", "Tags planning"),
        ];
        for (line, expected) in cases {
            assert_eq!(
                heading_display_text(line).as_deref(),
                Some(expected),
                "{line:?}"
            );
            // The same text the OUTLINE gets from the chunker for that line.
            let chunks = get_content_chunks(format!("{line}\nbody\n"));
            assert_eq!(
                chunks[0].breadcrumb_last(),
                Some(expected),
                "chunker disagrees on {line:?}"
            );
        }
    }

    #[test]
    fn heading_display_text_is_none_off_a_heading() {
        assert_eq!(heading_display_text("plain"), None);
        assert_eq!(heading_display_text("#tag here"), None);
        assert_eq!(heading_display_text("  see #tag"), None);
        assert_eq!(heading_display_text(""), None);
    }

    #[test]
    fn heading_display_text_drops_leading_indentation() {
        // A heading nested in a list item is indented past the top-level ATX
        // limit; the chunker lists it, so the line alone must render too.
        assert_eq!(
            heading_display_text("    # Nested").as_deref(),
            Some("Nested")
        );
        let chunks = get_content_chunks("- item\n    # Nested\nbody\n");
        assert!(
            chunks.iter().any(|c| c.breadcrumb_last() == Some("Nested")),
            "the chunker lists the nested heading: {chunks:?}"
        );
    }

    #[test]
    fn the_title_of_a_wrapped_list_item_is_its_first_line() {
        assert_eq!(
            crate::note::content_extractor::extract_title("- item one\n  #tag after soft break\n"),
            "item one"
        );
        assert_eq!(
            crate::note::content_extractor::extract_title("1. one  \n   two\n"),
            "one"
        );
        // The rest of the item is still indexed.
        let chunks = get_content_chunks("- item one\n  wraps on\n");
        assert!(chunks[0].text.contains("item one wraps on"), "{chunks:?}");
    }

    fn extract_headings(text: &str) -> Vec<(u8, String)> {
        extract_outline(text)
            .into_iter()
            .map(|(level, text, _)| (level, text))
            .collect()
    }

    /// Each outline entry is what the editor's jump looks for, on the line it
    /// claims.
    fn assert_outline_lines(text: &str, expected: &[(u8, &str, usize)]) {
        let outline = extract_outline(text);
        let expected: Vec<(u8, String, usize)> = expected
            .iter()
            .map(|(l, t, r)| (*l, t.to_string(), *r))
            .collect();
        assert_eq!(outline, expected, "{text:?}");
        for (_, heading, row) in &outline {
            let line = text.lines().nth(*row).unwrap();
            assert_eq!(
                heading_display_text(line).as_deref(),
                Some(heading.as_str()),
                "row {row} of {text:?}"
            );
        }
    }

    #[test]
    fn the_outline_keeps_body_less_headings_rendered_like_their_lines() {
        assert_outline_lines(
            "---\ntitle: x\n---\n# Title\n## [[target|Shown]] #tag\nbody\n### Empty\n",
            &[(1, "Title", 3), (2, "Shown tag", 4), (3, "Empty", 6)],
        );
    }

    #[test]
    fn the_outline_lines_survive_a_trailing_blank_line_after_frontmatter() {
        assert_outline_lines("---\nt: x\n---\n# A\n## B\n\n", &[(1, "A", 3), (2, "B", 4)]);
        assert_outline_lines("---\n# A\n\n", &[(1, "A", 1)]);
        assert_outline_lines("---\n---\n# A\n\n\n", &[(1, "A", 2)]);
    }

    #[test]
    fn the_outline_lines_survive_a_wikilink_broken_across_lines() {
        assert_outline_lines(
            "# A\n[[target\n|Shown]]\n# B\nsee [[x\ny\nz|w]] and [[p\nq]]\n## C\n",
            &[(1, "A", 0), (1, "B", 3), (2, "C", 8)],
        );
    }

    #[test]
    fn every_heading_api_renders_a_heading_as_its_chunk_breadcrumb() {
        let text = "# See [[other|Other]] #tag\nbody\n## Docs [here](https://x.y) **now**\nmore\n";
        let headings: Vec<String> = extract_headings(text).into_iter().map(|(_, t)| t).collect();
        let breadcrumbs: Vec<String> = get_content_chunks(text)
            .iter()
            .filter_map(|c| c.breadcrumb_last().map(str::to_string))
            .collect();
        assert_eq!(headings, breadcrumbs);
        for (line, heading) in text.lines().filter(|l| l.starts_with('#')).zip(&headings) {
            assert_eq!(heading_display_text(line).as_ref(), Some(heading));
        }
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_first_heading() {
        assert_eq!(
            extract_outline("\u{feff}# Title\n## B\n"),
            vec![(1, "Title".to_string(), 0), (2, "B".to_string(), 1)]
        );
        assert_eq!(
            crate::note::content_extractor::extract_title("\u{feff}# Title\n"),
            "Title"
        );
    }

    #[test]
    fn the_outline_gives_each_same_named_heading_its_own_line() {
        assert_outline_lines(
            "# Notes\n## Notes\nbody\n\n## Notes\n",
            &[(1, "Notes", 0), (2, "Notes", 1), (2, "Notes", 4)],
        );
    }

    #[test]
    fn the_outline_lines_skip_code_and_count_nested_headings() {
        assert_outline_lines(
            "intro\n```\n# not a heading\n```\n- # Setup\n> # Quoted\n\n# Last\n",
            &[(1, "Setup", 4), (1, "Quoted", 5), (1, "Last", 7)],
        );
        // An unclosed frontmatter fence is body text after its first line.
        assert_outline_lines("---\n# A\n", &[(1, "A", 1)]);
        assert_outline_lines(
            "+++\na = 1\n+++\n\n\n# A\r\n## B\r\n",
            &[(1, "A", 5), (2, "B", 6)],
        );
    }

    #[test]
    fn a_line_break_tag_in_a_heading_keeps_the_words_apart() {
        assert_eq!(
            extract_headings("# Release<br>notes\n# Ctrl <kbd>K</kbd>\n"),
            vec![(1, "Release notes".to_string()), (1, "Ctrl K".to_string())]
        );
    }

    #[test]
    fn heading_display_text_renders_a_heading_inside_a_list_or_quote() {
        for (line, expected) in [
            ("- # Setup", "Setup"),
            ("  * ## Deep", "Deep"),
            ("1. # Step", "Step"),
            ("> # Quoted", "Quoted"),
            ("## <a id=\"x\"></a>Install", "Install"),
        ] {
            assert_eq!(
                heading_display_text(line).as_deref(),
                Some(expected),
                "{line:?}"
            );
        }
        assert_eq!(heading_display_text("- plain item"), None);
        assert_eq!(heading_display_text("- #tag"), None);
        // The chunker lists the same text, so the OUTLINE can jump to it.
        let chunks = get_content_chunks("- # Setup\n  make install\n");
        assert!(
            chunks.iter().any(|c| c.breadcrumb_last() == Some("Setup")),
            "{chunks:?}"
        );
    }

    #[test]
    fn heading_section_range_finds_a_heading_by_its_rendered_text() {
        let note = "# a\nfirst\n## <a id=\"x\"></a>Install\nsteps\n# c\n";
        let r = heading_section_range(note, "Install").unwrap();
        assert_eq!(&note[r], "## <a id=\"x\"></a>Install\nsteps\n");
        let note = "- # Setup\n  make install\n";
        assert!(heading_section_range(note, "Setup").is_some());
    }

    #[test]
    fn every_line_of_a_list_item_is_kept() {
        let text = "# Lists\n\n- a long bullet that wraps\n  onto zebra line.\n- parent\n  - nested child\n    continues here\n  - second child\n- hard break  \n  after the break\n\n1. ordered one\n   wraps too\n";
        let chunks = crate::note::content_extractor::get_content_chunks(text);
        let body: String = chunks.iter().map(|c| c.text.as_str()).collect();
        for kept in [
            "onto zebra line.",
            "continues here",
            "after the break",
            "wraps too",
        ] {
            assert!(body.contains(kept), "{kept:?} missing from {body:?}");
        }
        assert!(
            body.contains("    * second child"),
            "a nested item keeps its level after a wrapped sibling: {body:?}"
        );
    }

    #[test]
    fn a_heading_inside_a_list_item_is_only_its_own_text() {
        let text = "# Top\n\n- # Setup\n  make install\n";
        assert_eq!(
            extract_headings(text),
            vec![(1, "Top".to_string()), (1, "Setup".to_string())]
        );
        let chunks = crate::note::content_extractor::get_content_chunks(text);
        assert!(
            chunks
                .iter()
                .any(|c| c.breadcrumb == "Setup" && c.text.contains("make install")),
            "{chunks:?}"
        );
    }

    #[test]
    fn inline_html_never_cuts_a_heading_or_a_line() {
        let text = "# Release <kbd>v2</kbd> notes\n\nPress <kbd>Ctrl</kbd>+K now.\n\n## <a id=\"x\"></a>Install\nbody";
        assert_eq!(
            extract_headings(text),
            vec![
                (1, "Release v2 notes".to_string()),
                (2, "Install".to_string())
            ]
        );
        let chunks = crate::note::content_extractor::get_content_chunks(text);
        assert!(
            chunks
                .iter()
                .any(|c| c.text.contains("Press <kbd>Ctrl</kbd>+K now.")),
            "a paragraph with inline HTML stays one line: {chunks:?}"
        );
    }

    #[test]
    fn a_heading_directly_above_a_code_block_is_kept() {
        let text = "# Setup\n```sh\nmake\n```\n## Next\ntext";
        assert_eq!(
            extract_headings(text),
            vec![(1, "Setup".to_string()), (2, "Next".to_string())]
        );
        let chunks = crate::note::content_extractor::get_content_chunks(text);
        assert!(
            chunks
                .iter()
                .any(|c| c.breadcrumb == "Setup" && c.text.contains("make")),
            "the code is filed under its heading: {chunks:?}"
        );
    }

    #[test]
    fn frontmatter_after_a_byte_order_mark_is_still_frontmatter() {
        let text = "\u{feff}---\ntitle: x\n---\nbody";
        let chunks = get_content_chunks(text);
        assert_eq!(chunks[0].text, "body");
        assert_eq!(chunks[1].text, "title: x");
        assert_eq!(&text[super::frontmatter_end_byte(text)..], "body");
    }
}
