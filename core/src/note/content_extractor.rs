use log::debug;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use regex::{Captures, Regex};
use std::sync::LazyLock;
use url::Url;

use crate::{
    nfs::{self, VaultPath},
    note::{ContentChunk, NoteContentData},
};

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
    pub start: usize,
    pub end: usize,
    pub kind: LinkSpanKind,
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
            let target = inner.split('|').next().unwrap_or(inner).to_string();
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
        let target = inner.split('|').next().unwrap_or(inner).to_string();
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

/// Returns chunks and links for indexing.
///
/// Optimized over the naive `get_content_chunks + get_markdown_and_links`
/// pairing in three ways:
///
/// 1. Skips the rewritten-markdown String that `get_markdown_and_links`
///    builds (its `text` return value is discarded by indexing — only
///    `links` flow into the DB).
/// 2. Skips constructing `NoteLink::Url`, `NoteLink::Image`, and
///    `NoteLink::Attachment` entries — the DB's `NoteBatch::push` drops
///    those variants on the floor (see `core/src/db/mod.rs` link match).
/// 3. Shares a single `process_wikilinks` pass between chunk-building and
///    link-extraction, eliminating one regex scan + one `String`
///    allocation per indexed note.
pub fn get_chunks_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (Vec<ContentChunk>, Vec<super::NoteLink>) {
    let raw = md_text.as_ref();
    let (frontmatter, body_with_wikilinks) = remove_frontmatter(raw);

    // Pass 1: collapse wikilinks AND record the byte ranges in the output
    // string that came from wikilink display text. Hashtags that fall in
    // those ranges must not be emitted as labels — they were inside a
    // wikilink in the source and the prior `get_markdown_and_links`
    // pipeline excluded them via `md_wikilink_char_ranges`.
    let mut links: Vec<NoteLink> = Vec::new();
    let (body_stripped, wikilink_display_ranges) =
        collapse_wikilinks_with_display_ranges(&body_with_wikilinks, &mut links);

    // Pass 2: precompute hashtag positions in `body_stripped`, dropping
    // matches that fall inside wikilink display text.
    let labels: Vec<(usize, usize, String)> = label_matches_inner(&body_stripped)
        .filter(|lm| {
            !wikilink_display_ranges
                .iter()
                .any(|(s, e)| lm.byte_start >= *s && lm.byte_end <= *e)
        })
        .map(|lm| (lm.byte_start, lm.byte_end, lm.name.to_string()))
        .collect();

    // Pass 3: single pulldown_cmark walk on `body_stripped` produces
    // chunks + Note/Hashtag links in one pass — replacing the previous
    // separate `parse_text` + `code_char_ranges` + `md_link_char_ranges`
    // + `MD_LINK_RX.captures_iter` + `label_matches_inner` filter passes.
    let (text_lines, walk_links) = walk_indexing_events(&body_stripped, reference_path, &labels);
    links.extend(walk_links);

    let mut chunks = chunks_from_text_lines(text_lines);
    if !frontmatter.is_empty() {
        chunks.push(ContentChunk {
            breadcrumb: "FrontMatter".to_string(),
            text: frontmatter,
        });
    }

    (chunks, links)
}

/// Collapses wikilinks to their display text while recording the byte
/// ranges in the output string that originated from a wikilink. Pushes
/// `NoteLink::Note` for every wikilink whose target resolves to a valid
/// vault path.
///
/// The returned ranges feed `get_chunks_and_links`'s label-filter step:
/// hashtags inside wikilink display text were never indexed by the
/// previous `get_markdown_and_links` pipeline (it converted wikilinks to
/// markdown links and then suppressed hashtags inside the resulting link
/// span via `md_wikilink_char_ranges` / `md_link_char_ranges`). The
/// single-walk indexer reproduces that suppression by removing matching
/// label positions before the walker sees them.
fn collapse_wikilinks_with_display_ranges(
    body: &str,
    links: &mut Vec<NoteLink>,
) -> (String, Vec<(usize, usize)>) {
    let mut out = String::with_capacity(body.len());
    let mut display_ranges: Vec<(usize, usize)> = Vec::new();
    let mut last = 0usize;
    for caps in WIKILINK_RX.captures_iter(body) {
        let m = caps
            .get(0)
            .expect("captures_iter never yields without group 0");
        out.push_str(&body[last..m.start()]);
        let items = &caps["link_text"];
        let parts: Vec<&str> = items.split('|').collect();
        let (link, text) = match parts.len() {
            1 => (parts[0], parts[0]),
            // Extra pipes: keep the first part as link, second as display
            // text, drop the rest — matches `process_wikilinks` semantics.
            _ => (parts[0], parts[1]),
        };
        if VaultPath::is_valid(link) {
            let link_path = VaultPath::note_path_from(link);
            links.push(NoteLink::note(&link_path, text));
        }
        let display_start = out.len();
        out.push_str(text);
        let display_end = out.len();
        display_ranges.push((display_start, display_end));
        last = m.end();
    }
    out.push_str(&body[last..]);
    (out, display_ranges)
}

pub fn get_content_data<S: AsRef<str>>(md_text: S) -> NoteContentData {
    let hash = nfs::hash_text(md_text.as_ref());
    let title = extract_title(md_text);

    NoteContentData { title, hash }
}

pub fn get_content_chunks<S: AsRef<str>>(md_text: S) -> Vec<ContentChunk> {
    let (frontmatter, text) = remove_frontmatter(md_text.as_ref());

    // Clean up wikilinks and hashtags for indexing
    let text = process_wikilinks(&text, |_link, _text| None);
    let text = cleanup_hashtags(&text);

    let mut content_chunks = parse_text(&text);

    if !frontmatter.is_empty() {
        content_chunks.push(ContentChunk {
            breadcrumb: "FrontMatter".to_string(),
            text: frontmatter,
        })
    }

    content_chunks
}

/// Process wikilinks with a custom handler function
/// Handler returns None to remove the wikilink (keep only text), or Some(String) to replace it
fn process_wikilinks<F>(md_text: &str, handler: F) -> String
where
    F: Fn(&str, &str) -> Option<String>,
{
    WIKILINK_RX
        .replace_all(md_text, |caps: &Captures| {
            let items = &caps["link_text"];
            let parts: Vec<&str> = items.split('|').collect();

            let (link, text) = match parts.len() {
                1 => (parts[0], parts[0]),
                2 => (parts[0], parts[1]),
                // Extra pipes: use first part as link, second as display text, ignore rest
                _ => (parts[0], parts[1]),
            };

            handler(link, text).unwrap_or_else(|| text.to_string())
        })
        .into_owned()
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
    pub fn from_text(text: &str) -> Self {
        Self {
            frontmatter_end: frontmatter_end_byte(text),
            code: code_char_ranges(text),
            md_links: md_link_char_ranges(text),
            wikilinks: md_wikilink_char_ranges(text),
        }
    }

    /// True when `byte_offset` falls inside any zone (frontmatter, code,
    /// markdown link body, or closed wikilink). Matches the boolean of
    /// `is_inside_exclusion_zone(text, byte_offset)` for the same `text`.
    pub fn contains(&self, byte_offset: usize) -> bool {
        if byte_offset < self.frontmatter_end {
            return true;
        }
        Self::in_any(byte_offset, &self.code)
            || Self::in_any(byte_offset, &self.md_links)
            || Self::in_any(byte_offset, &self.wikilinks)
    }

    /// True when `byte_offset` falls inside frontmatter, code, or a
    /// markdown link body — but NOT inside a closed wikilink span. Matches
    /// `is_inside_code_link_or_frontmatter(text, byte_offset)`.
    pub fn contains_code_link_or_frontmatter(&self, byte_offset: usize) -> bool {
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

fn cleanup_hashtags(md_text: &str) -> String {
    let code_ranges = code_char_ranges(md_text);
    let link_ranges = md_link_char_ranges(md_text);
    cleanup_hashtags_with_ranges(md_text, &code_ranges, &link_ranges)
}

/// Inner implementation of [`cleanup_hashtags`] taking precomputed
/// exclusion-zone ranges. Used by [`get_chunks_and_links`] to share a
/// single range computation between the chunk-build pass and the
/// hashtag-link extraction pass.
///
/// Only strips the leading `#` from matches that pass the
/// word-boundary guard in [`label_matches_inner`]. A `#tag` inside
/// `` `#tag` `` or inside a markdown link body stays verbatim — the
/// indexed chunk text must match the source faithfully (FTS preview
/// snippets surface this text via MCP `get_chunks`).
///
/// Frontmatter and wikilinks are already stripped/collapsed by callers
/// before reaching this helper.
fn cleanup_hashtags_with_ranges(
    md_text: &str,
    code_ranges: &[(usize, usize)],
    link_ranges: &[(usize, usize)],
) -> String {
    let in_excluded_zone = |start: usize, end: usize| -> bool {
        code_ranges.iter().any(|(s, e)| start >= *s && end <= *e)
            || link_ranges.iter().any(|(s, e)| start >= *s && end <= *e)
    };
    let mut out = String::with_capacity(md_text.len());
    let mut last_end = 0usize;
    for lm in label_matches_inner(md_text) {
        out.push_str(&md_text[last_end..lm.byte_start]);
        if in_excluded_zone(lm.byte_start, lm.byte_end) {
            out.push_str(&md_text[lm.byte_start..lm.byte_end]);
        } else {
            out.push_str(lm.name);
        }
        last_end = lm.byte_end;
    }
    out.push_str(&md_text[last_end..]);
    out
}

/// Internal label iterator that powers [`crate::note::label_matches`].
///
/// Encapsulates the regex match + the word-boundary guard. A `#tag` is
/// rejected when the preceding or following character is alphanumeric, `_`,
/// or another `#` — so mid-word (`hello#tag`), stacked-hash (`##tag`,
/// Markdown header territory), and adjacent-hash (`#tag#more`) cases are
/// all skipped. Code-span / HTML / link overlap suppression is left to the
/// caller because those checks are context-specific.
pub(crate) fn label_matches_inner(
    text: &str,
) -> impl Iterator<Item = crate::note::LabelMatch<'_>> + '_ {
    HASHTAG_RX.captures_iter(text).filter_map(move |caps| {
        let m = caps.get(0)?;
        let preceding_blocks_label = m.start() != 0
            && text[..m.start()]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric() || c == '_' || c == '#')
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
        Some(crate::note::LabelMatch {
            byte_start: m.start(),
            byte_end: m.end(),
            name,
        })
    })
}

/// Returns the converted text into Markdown (replacing note wikilinks to markdown links)
/// Normalizes the links urls when needed (lowercasing the path for vault paths)
/// And a list of the links existing in the note, relative links are transformed to absolute links.
/// Hashtags are converted to markdown links and added to the links list.
pub(crate) fn get_markdown_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (String, Vec<NoteLink>) {
    let mut links = vec![];

    // Convert wikilinks to markdown links
    let md_text = process_wikilinks(md_text.as_ref(), |link, text| {
        if VaultPath::is_valid(link) {
            let link_path = VaultPath::note_path_from(link);
            Some(format!("[{}]({})", text, link_path))
        } else {
            // Keep invalid wikilinks as-is
            Some(format!(
                "[[{}]]",
                if link == text {
                    link.to_string()
                } else {
                    format!("{}|{}", link, text)
                }
            ))
        }
    });

    // Process markdown links and extract them
    let md_text = MD_LINK_RX.replace_all(&md_text, |caps: &Captures| {
        let bang = &caps["bang"];
        let text = &caps["text"];
        let link = caps["link"].trim();

        // Ignore image links
        if !bang.is_empty() {
            return format!("![{}]({})", text, link);
        }

        debug!("checking link {}", link);

        let clean_link = if is_remote_url(link) {
            // URL link
            links.push(NoteLink::url(link, text));
            link.to_string()
        } else if VaultPath::is_valid(link) {
            // Vault path link
            let path = VaultPath::new(link);

            if path.is_note_file() {
                // Absolute note path
                links.push(NoteLink::note(&path, text));
                path.to_string()
            } else {
                // Relative path - resolve it
                let ref_path = if reference_path.is_note() {
                    reference_path.get_parent_path().0
                } else {
                    reference_path.to_owned()
                };

                let abs_path = ref_path.append(&path).flatten();

                if abs_path.is_note() {
                    links.push(NoteLink::note(&abs_path, text));
                } else {
                    links.push(NoteLink::vault_path(&abs_path, text));
                }

                abs_path.to_string()
            }
        } else {
            debug!("link not counting {}", link);
            link.to_string()
        };

        format!("[{}]({})", text, clean_link)
    });

    // Process hashtags and convert them to links. The label_matches_inner
    // iterator already enforces the word-boundary rule (a `#tag` preceded by
    // an alphanumeric/underscore char is skipped). Here we additionally skip
    // any label that overlaps a code span, an HTML region, a markdown-link
    // span, or the YAML/TOML frontmatter — those are call-site concerns.
    let fm_end = frontmatter_end_byte(&md_text);
    let code_ranges = code_char_ranges(&md_text);
    let link_ranges = md_link_char_ranges(&md_text);
    let wikilink_ranges = md_wikilink_char_ranges(&md_text);
    let mut out = String::with_capacity(md_text.len());
    let mut last_end = 0usize;
    for lm in label_matches_inner(&md_text) {
        let in_frontmatter = lm.byte_start < fm_end;
        let in_code = code_ranges
            .iter()
            .any(|(s, e)| lm.byte_start >= *s && lm.byte_end <= *e);
        let in_link = link_ranges
            .iter()
            .any(|(s, e)| lm.byte_start >= *s && lm.byte_end <= *e)
            || wikilink_ranges
                .iter()
                .any(|(s, e)| lm.byte_start >= *s && lm.byte_end <= *e);
        out.push_str(&md_text[last_end..lm.byte_start]);
        if in_frontmatter || in_code || in_link {
            out.push_str(&md_text[lm.byte_start..lm.byte_end]);
        } else {
            links.push(NoteLink::hashtag(lm.name));
            out.push_str(&format!("[#{}](#{})", lm.name, lm.name));
        }
        last_end = lm.byte_end;
    }
    out.push_str(&md_text[last_end..]);
    let clean_md_text: std::borrow::Cow<'_, str> = std::borrow::Cow::Owned(out);

    (clean_md_text.to_string(), links)
}

/// Rewrites all links in `md_text` that target `old_path` so they target `new_path` instead.
///
/// Handles three link forms:
/// - WikiLinks: `[[old-name]]` → `[[new-name]]`, `[[old-name|display]]` → `[[new-name|display]]`
/// - Markdown links by full vault path: `[text](/old/path.md)` → `[text](/new/path.md)`
/// - Markdown links by filename: `[text](old-name.md)` → `[text](new-name.md)`
///
/// Returns `(updated_text, changed)` where `changed` is true when at least one replacement was made.
pub(crate) fn replace_note_links(
    md_text: &str,
    old_path: &VaultPath,
    new_path: &VaultPath,
) -> (String, bool) {
    let old_name = old_path.get_name(); // e.g. "old-title.md"
    let old_full = old_path.to_string(); // e.g. "/notes/old-title.md"
    let new_clean = new_path.get_clean_name(); // e.g. "new-title" (no extension)
    let new_name = new_path.get_name(); // e.g. "new-title.md"
    let new_full = new_path.to_string(); // e.g. "/notes/new-title.md"

    // Step 1: rewrite wikilinks whose resolved name matches old_path
    let after_wikilinks = WIKILINK_RX.replace_all(md_text, |caps: &Captures| {
        let items = &caps["link_text"];
        let parts: Vec<&str> = items.split('|').collect();
        let (link, display) = match parts.len() {
            1 => (parts[0], parts[0]),
            _ => (parts[0], parts[1]),
        };
        if VaultPath::note_path_from(link).get_name() == old_name {
            if link == display {
                format!("[[{}]]", new_clean)
            } else {
                format!("[[{}|{}]]", new_clean, display)
            }
        } else {
            // Keep unchanged — reconstruct the original form
            format!("[[{}]]", items)
        }
    });

    // Step 2: rewrite markdown links by full vault path or bare filename
    let after_links = MD_LINK_RX.replace_all(&after_wikilinks, |caps: &Captures| {
        let bang = &caps["bang"];
        let text = &caps["text"];
        let link = caps["link"].trim();
        if !bang.is_empty() {
            return format!("![{}]({})", text, link); // image — skip
        }
        if link == old_full {
            format!("[{}]({})", text, new_full)
        } else if link == old_name {
            format!("[{}]({})", text, new_name)
        } else {
            format!("[{}]({})", text, link)
        }
    });

    let result = after_links.to_string();
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

pub fn extract_title<S: AsRef<str>>(md_text: S) -> String {
    let (_frontmatter, md_text) = remove_frontmatter(md_text);
    let mut parser = Parser::new(md_text.as_ref());
    let result = loop_events(&mut parser);

    result
        .iter()
        .find_map(|tt| match tt {
            TextLine::Empty => None,
            TextLine::Header(_level, text) => Some(text.to_owned()),
            TextLine::Text(text) => Some(text.to_owned()),
            TextLine::ListItem(_level, text) => Some(text.to_owned()),
        })
        .unwrap_or_default()
}

fn parse_text(md_text: &str) -> Vec<ContentChunk> {
    let mut parser = Parser::new(md_text);
    let lines = loop_events(&mut parser);
    chunks_from_text_lines(lines)
}

/// Converts a sequence of [`TextLine`] events into [`ContentChunk`]s.
/// Shared between [`parse_text`] (standalone) and [`get_chunks_and_links`]'s
/// single-walk indexing path.
fn chunks_from_text_lines(lines: Vec<TextLine>) -> Vec<ContentChunk> {
    let mut content_chunks = vec![];
    let mut current_breadcrumb: Vec<(u8, String)> = vec![];
    let mut current_content = vec![];

    for text_line in lines {
        match text_line {
            TextLine::Header(level, text) => {
                if !current_breadcrumb.is_empty() || !current_content.is_empty() {
                    let content = crate::utilities::remove_diacritics(&current_content.join("\n"));
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
        let content = crate::utilities::remove_diacritics(&current_content.join("\n"));
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

/// Single-walk markdown event processor for the indexing path.
///
/// Mirrors [`loop_events`] but operates on `Parser::into_offset_iter` so it
/// can use the same pulldown_cmark pass to:
/// - Build [`TextLine`]s for chunk construction (same shape as `loop_events`).
/// - Track inline code-span and link state so hashtag exclusion-zone rules
///   can be applied without a second `code_char_ranges` / `md_link_char_ranges`
///   pass.
/// - Emit `NoteLink::Note` from markdown links via `Event::Start(Link)` /
///   `Event::End(Link)`, eliminating the separate `MD_LINK_RX.captures_iter`
///   scan.
/// - Strip the leading `#` from valid hashtag positions inline (using
///   precomputed `labels` from `label_matches_inner` on `body`), eliminating
///   the separate `cleanup_hashtags` text-mutation pass.
///
/// Inputs:
/// - `body`: the text to walk (frontmatter removed and wikilinks already
///   collapsed to display text by the caller).
/// - `ref_path`: reference path for resolving relative markdown links.
/// - `labels`: precomputed hashtag positions on `body` (byte_start, byte_end,
///   name without `#`).
///
/// Returns `(text_lines, links)` — feed `text_lines` into
/// [`chunks_from_text_lines`]; `links` carry both Note (vault-path markdown
/// links) and Hashtag entries.
fn walk_indexing_events(
    body: &str,
    ref_path: &VaultPath,
    labels: &[(usize, usize, String)],
) -> (Vec<TextLine>, Vec<NoteLink>) {
    let mut text_lines: Vec<TextLine> = vec![];
    let mut tag_stack: Vec<Tag> = vec![];
    let mut links: Vec<NoteLink> = vec![];

    let mut code_depth: u32 = 0;
    let mut in_link: bool = false;
    let mut pending_link_dest: Option<String> = None;

    let parser = Parser::new(body).into_offset_iter();

    for (event, range) in parser {
        // Update inline state BEFORE dispatching the main TextLine-building
        // logic, so Event::Text knows whether we are inside code or a link
        // body when it decides whether to strip hashtags.
        match &event {
            Event::Start(Tag::CodeBlock(_)) => {
                code_depth += 1;
            }
            Event::End(TagEnd::CodeBlock) => {
                code_depth = code_depth.saturating_sub(1);
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                pending_link_dest = Some(dest_url.to_string());
                in_link = true;
            }
            Event::End(TagEnd::Link) => {
                if let Some(dest) = pending_link_dest.take() {
                    emit_md_note_link(&dest, ref_path, &mut links);
                }
                in_link = false;
            }
            // Image alt text is inside an exclusion zone for the indexing
            // path: the previous `cleanup_hashtags_with_ranges` saw images
            // via `MD_LINK_RX` (which matches both `[text](url)` and
            // `![alt](url)`), so a hashtag inside `![alt #foo](img.png)`
            // was kept verbatim and no `NoteLink::Hashtag` was emitted.
            // We mirror that by setting `in_link` for images too.
            Event::Start(Tag::Image { .. }) => {
                in_link = true;
            }
            Event::End(TagEnd::Image) => {
                in_link = false;
            }
            _ => {}
        }

        match event {
            Event::Start(tag) => {
                let current_line = text_lines.pop().unwrap_or_default();
                let new_lines = parse_tag(&tag, current_line);
                text_lines.extend(new_lines);
                tag_stack.push(tag);
            }
            Event::End(tag_end) => {
                let Some(start_tag) = tag_stack.pop() else {
                    debug!("Non matching tag end (empty stack): {:?}", tag_end);
                    continue;
                };
                if tag_end != start_tag.to_end() {
                    debug!(
                        "Non matching tags: expected {:?}, got {:?}",
                        start_tag.to_end(),
                        tag_end
                    );
                    tag_stack.push(start_tag);
                    continue;
                }
                let current_line = text_lines.pop().unwrap_or_default();
                let new_lines = parse_tag_end(&tag_end, current_line);
                text_lines.extend(new_lines);
            }
            Event::Text(cow_str) => {
                let last_text = text_lines.pop().unwrap_or_default();
                let appended = if code_depth > 0 {
                    cow_str.to_string()
                } else {
                    strip_hashtags_in_text_event(
                        &cow_str, &range, body, labels, in_link, &mut links,
                    )
                };
                text_lines.push(last_text.append_text(appended));
            }
            Event::Code(cow_str) => {
                let current_line = text_lines.pop().unwrap_or_default();
                text_lines.push(current_line.append_text(format!("`{}`", cow_str)));
            }
            Event::InlineMath(cow_str)
            | Event::DisplayMath(cow_str)
            | Event::Html(cow_str)
            | Event::InlineHtml(cow_str)
            | Event::FootnoteReference(cow_str) => {
                text_lines.push(TextLine::Text(cow_str.to_string()));
            }
            Event::SoftBreak => {
                text_lines.push(TextLine::Empty);
            }
            Event::HardBreak => {
                text_lines.push(TextLine::Empty);
                text_lines.push(TextLine::Empty);
            }
            Event::Rule => {
                text_lines.push(TextLine::Empty);
            }
            Event::TaskListMarker(result) => {
                text_lines.push(TextLine::Text(result.to_string()));
            }
        }
    }

    (text_lines, links)
}

/// Resolves a markdown link destination to a `NoteLink::Note` for indexing,
/// matching the policy of the original `MD_LINK_RX.captures_iter` pass:
/// remote URLs and non-note vault paths (attachments) are skipped because
/// the DB drops them. The link text is not stored on the DB row, so we
/// pass an empty string to avoid an extra allocation.
fn emit_md_note_link(dest: &str, ref_path: &VaultPath, links: &mut Vec<NoteLink>) {
    if is_remote_url(dest) {
        return;
    }
    if !VaultPath::is_valid(dest) {
        return;
    }
    let path = VaultPath::new(dest);
    if path.is_note_file() {
        links.push(NoteLink::note(&path, ""));
    } else {
        let ref_p = if ref_path.is_note() {
            ref_path.get_parent_path().0
        } else {
            ref_path.to_owned()
        };
        let abs = ref_p.append(&path).flatten();
        if abs.is_note() {
            links.push(NoteLink::note(&abs, ""));
        }
    }
}

/// Returns the text to append for an `Event::Text` payload, with the
/// leading `#` of each valid hashtag stripped and a `NoteLink::Hashtag`
/// pushed to `links`. Skips hashtag emission when inside a link body (its
/// hashtag would be treated as part of the link text, not a tag).
///
/// Uses source slices (not the decoded `cow_str`) to keep hashtag positions
/// aligned with the precomputed `labels`. Falls back to the decoded text
/// when source length differs (HTML-entity decoding is active) and still
/// emits hashtag links — chunk text may then contain `#` literals for that
/// segment, which FTS tokenization handles transparently.
fn strip_hashtags_in_text_event(
    decoded: &str,
    range: &std::ops::Range<usize>,
    body: &str,
    labels: &[(usize, usize, String)],
    in_link: bool,
    links: &mut Vec<NoteLink>,
) -> String {
    // Fast path: no labels overlap this Text event.
    let mut overlap_start = None;
    for (i, (s, _, _)) in labels.iter().enumerate() {
        if *s >= range.start {
            overlap_start = Some(i);
            break;
        }
    }
    let start_idx = match overlap_start {
        Some(i) => i,
        None => return decoded.to_string(),
    };
    let mut overlapping: Vec<&(usize, usize, String)> = Vec::new();
    for entry in labels[start_idx..].iter() {
        if entry.1 > range.end {
            break;
        }
        overlapping.push(entry);
    }
    if overlapping.is_empty() {
        return decoded.to_string();
    }

    let source = &body[range.clone()];
    if source.len() != decoded.len() {
        // HTML-entity decoding active — positions don't align. Emit links
        // but leave chunk text as decoded (with `#` intact).
        for (_, _, name) in &overlapping {
            if !in_link {
                links.push(NoteLink::hashtag(name));
            }
        }
        return decoded.to_string();
    }

    let mut out = String::with_capacity(source.len());
    let mut last = range.start;
    for (lstart, lend, name) in &overlapping {
        out.push_str(&body[last..*lstart]);
        if in_link {
            // Inside a link body, keep `#tag` verbatim and do NOT emit a
            // hashtag NoteLink — matches the previous
            // `cleanup_hashtags_with_ranges` exclusion-zone behaviour.
            out.push_str(&body[*lstart..*lend]);
        } else {
            // Skip the `#` byte at *lstart, copy the label name
            // (`body[*lstart+1..*lend]`) and emit the hashtag link.
            out.push_str(&body[*lstart + 1..*lend]);
            links.push(NoteLink::hashtag(name));
        }
        last = *lend;
    }
    out.push_str(&body[last..range.end]);
    out
}

/// Returns the byte offset immediately after the closing `\n` of the opening
/// delimiter line, or `None` if the text does not start with a valid frontmatter
/// delimiter (`---` or `+++`).  Strips a trailing `\r` so CRLF files work the
/// same as LF files.
fn frontmatter_delimiter(text: &str) -> Option<(&str, usize)> {
    let newline_pos = text.find('\n')?;
    let raw_first = &text[..newline_pos];
    let first_line = raw_first.trim_end_matches('\r');
    if first_line != "---" && first_line != "+++" {
        return None;
    }
    // Return the canonical delimiter (without \r) and the byte offset just
    // after the opening '\n'.
    Some((first_line, newline_pos + 1))
}

/// Returns the byte offset of the first character after the closing delimiter
/// of a YAML/TOML frontmatter block (`---` or `+++`), or `0` if no valid
/// frontmatter is present. Tolerates both LF and CRLF line endings.
fn frontmatter_end_byte(text: &str) -> usize {
    let (delimiter, mut offset) = match frontmatter_delimiter(text) {
        Some(d) => d,
        None => return 0,
    };

    for line in text[offset..].split('\n') {
        let trimmed = line.trim_end_matches('\r');
        if trimmed == delimiter {
            // Advance past this closing delimiter line (and its '\n' if present).
            offset += line.len();
            if text.as_bytes().get(offset) == Some(&b'\n') {
                offset += 1;
            }
            return offset;
        }
        offset += line.len() + 1; // +1 for '\n'
    }

    // No closing delimiter found — treat as no frontmatter
    0
}

fn remove_frontmatter<S: AsRef<str>>(text: S) -> (String, String) {
    let mut lines = text.as_ref().lines();

    let Some(first_line) = lines.next() else {
        return (String::new(), String::new());
    };

    if first_line != "---" && first_line != "+++" {
        return (String::new(), text.as_ref().to_string());
    }

    let delimiter = first_line;
    let mut frontmatter = vec![];
    let mut content = vec![];
    let mut closed_fm = false;

    for line in lines {
        if line == delimiter && !closed_fm {
            closed_fm = true;
        } else if closed_fm {
            content.push(line);
        } else {
            frontmatter.push(line);
        }
    }

    if closed_fm {
        (frontmatter.join("\n"), content.join("\n"))
    } else {
        (String::new(), frontmatter.join("\n"))
    }
}

#[derive(Debug, Default, Clone)]
enum TextLine {
    #[default]
    Empty,
    Header(u8, String),
    Text(String),
    ListItem(u8, String),
}

impl TextLine {
    fn append_text(&self, text: String) -> TextLine {
        match self {
            TextLine::Empty => TextLine::Text(text),
            TextLine::Header(level, header_text) => {
                TextLine::Header(*level, format!("{}{}", header_text, text))
            }
            TextLine::Text(line_text) => TextLine::Text(format!("{}{}", line_text, text)),
            TextLine::ListItem(level, item_text) => {
                TextLine::ListItem(*level, format!("{}{}", item_text, text))
            }
        }
    }

    fn to_text(&self) -> String {
        match self {
            TextLine::Empty => String::new(),
            TextLine::Header(level, text) => {
                format!("{} {}", "#".repeat(*level as usize), text)
            }
            TextLine::Text(text) => text.to_owned(),
            TextLine::ListItem(level, text) => {
                format!("{}* {}", " ".repeat((*level as usize) * 4), text)
            }
        }
    }

    fn trim(&self) -> Self {
        match self {
            TextLine::Empty => TextLine::Empty,
            TextLine::Header(level, text) => TextLine::Header(*level, text.trim().to_string()),
            TextLine::Text(text) => TextLine::Text(text.trim().to_string()),
            TextLine::ListItem(level, text) => TextLine::ListItem(*level, text.trim().to_string()),
        }
    }
}

fn loop_events(parser: &mut Parser) -> Vec<TextLine> {
    let mut text_lines: Vec<TextLine> = vec![];
    let mut tag_stack = vec![];

    for event in parser.by_ref() {
        match event {
            Event::Start(tag) => {
                let current_line = text_lines.pop().unwrap_or_default();
                let new_lines = parse_tag(&tag, current_line);
                text_lines.extend(new_lines);
                tag_stack.push(tag);
            }
            Event::End(tag_end) => {
                let Some(start_tag) = tag_stack.pop() else {
                    debug!("Non matching tag end (empty stack): {:?}", tag_end);
                    continue;
                };

                if tag_end != start_tag.to_end() {
                    debug!(
                        "Non matching tags: expected {:?}, got {:?}",
                        start_tag.to_end(),
                        tag_end
                    );
                    tag_stack.push(start_tag);
                    continue;
                }

                let current_line = text_lines.pop().unwrap_or_default();
                let new_lines = parse_tag_end(&tag_end, current_line);
                text_lines.extend(new_lines);
            }
            Event::Text(cow_str) => {
                let last_text = text_lines.pop().unwrap_or_default();
                text_lines.push(last_text.append_text(cow_str.to_string()));
            }
            Event::Code(cow_str) => {
                let current_line = text_lines.pop().unwrap_or_default();
                text_lines.push(current_line.append_text(format!("`{}`", cow_str)));
            }
            Event::InlineMath(cow_str)
            | Event::DisplayMath(cow_str)
            | Event::Html(cow_str)
            | Event::InlineHtml(cow_str)
            | Event::FootnoteReference(cow_str) => {
                text_lines.push(TextLine::Text(cow_str.to_string()));
            }
            Event::SoftBreak => {
                text_lines.push(TextLine::Empty);
            }
            Event::HardBreak => {
                text_lines.push(TextLine::Empty);
                text_lines.push(TextLine::Empty);
            }
            Event::Rule => {
                text_lines.push(TextLine::Empty);
            }
            Event::TaskListMarker(result) => {
                text_lines.push(TextLine::Text(result.to_string()));
            }
        }
    }

    text_lines
}

fn parse_tag(tag: &Tag, current_line: TextLine) -> Vec<TextLine> {
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
            vec![current_line, TextLine::Header(level, String::new())]
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
            vec![TextLine::Text(open), TextLine::Empty]
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
            _ => vec![TextLine::ListItem(0, String::new())],
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
        _ => {
            vec![current_line]
        }
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

    #[test]
    fn replace_wikilink_no_display() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("See [[old-note]].", &old, &new);
        assert!(changed);
        assert_eq!(result, "See [[new-note]].");
    }

    #[test]
    fn replace_wikilink_with_display_text() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("See [[old-note|my note]].", &old, &new);
        assert!(changed);
        assert_eq!(result, "See [[new-note|my note]].");
    }

    #[test]
    fn replace_markdown_link_full_path() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("[click](/notes/old-note.md)", &old, &new);
        assert!(changed);
        assert_eq!(result, "[click](/notes/new-note.md)");
    }

    #[test]
    fn replace_markdown_link_filename_only() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let (result, changed) = replace_note_links("[click](old-note.md)", &old, &new);
        assert!(changed);
        assert_eq!(result, "[click](new-note.md)");
    }

    #[test]
    fn replace_does_not_touch_unrelated_links() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        let text = "[[other-note]] [x](/notes/unrelated.md) [y](unrelated.md)";
        let (result, changed) = replace_note_links(text, &old, &new);
        assert!(!changed);
        assert_eq!(result, text);
    }

    #[test]
    fn replace_does_not_touch_images() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/notes/new-note.md");
        // Images that happen to match the name must not be touched
        let text = "![old-note.md](old-note.md)";
        let (result, changed) = replace_note_links(text, &old, &new);
        assert!(!changed);
        assert_eq!(result, text);
    }

    #[test]
    fn replace_mixed_content() {
        let old = VaultPath::new("/notes/old-note.md");
        let new = VaultPath::new("/archive/new-note.md");
        let text = "[[old-note]] and [[old-note|read this]] plus [link](/notes/old-note.md) end.";
        let (result, changed) = replace_note_links(text, &old, &new);
        assert!(changed);
        assert_eq!(
            result,
            "[[new-note]] and [[new-note|read this]] plus [link](/archive/new-note.md) end."
        );
    }

    #[test]
    fn replace_returns_unchanged_false_when_no_match() {
        let old = VaultPath::new("/notes/missing.md");
        let new = VaultPath::new("/notes/also-missing.md");
        let text = "No references here at all.";
        let (result, changed) = replace_note_links(text, &old, &new);
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
        // Per spec: `#tag-with-dash` yields the label `tag` and the rest
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
        let v: Vec<&str> = crate::note::label_matches("foo#bar and #real")
            .map(|m| m.name)
            .collect();
        assert_eq!(v, vec!["real"]);
    }

    #[test]
    fn label_matches_at_start() {
        let v: Vec<&str> = crate::note::label_matches("#first and #second")
            .map(|m| m.name)
            .collect();
        assert_eq!(v, vec!["first", "second"]);
    }

    #[test]
    fn label_matches_skips_double_hash() {
        let v: Vec<&str> = crate::note::label_matches("##tag and ##other")
            .map(|m| m.name)
            .collect();
        assert!(v.is_empty(), "expected no labels, got {:?}", v);
    }

    #[test]
    fn label_matches_skips_triple_hash() {
        let v: Vec<&str> = crate::note::label_matches("###tag")
            .map(|m| m.name)
            .collect();
        assert!(v.is_empty(), "expected no labels, got {:?}", v);
    }

    #[test]
    fn label_matches_skips_adjacent_hash() {
        // `#tag#more` — `#` immediately follows the label, so neither side
        // is a real tag boundary; reject both.
        let v: Vec<&str> = crate::note::label_matches("#tag#more")
            .map(|m| m.name)
            .collect();
        assert!(v.is_empty(), "expected no labels, got {:?}", v);
    }

    #[test]
    fn label_matches_after_space_then_hash() {
        let v: Vec<&str> = crate::note::label_matches("# #tag")
            .map(|m| m.name)
            .collect();
        assert_eq!(v, vec!["tag"]);
    }

    #[test]
    fn cleanup_hashtags_preserves_inline_code_span() {
        // `#tag` inside backticks must stay verbatim: the indexed chunk
        // should match the source text, not strip the leading `#`.
        let cleaned = super::cleanup_hashtags("Use `#define X` to set X.");
        assert_eq!(cleaned, "Use `#define X` to set X.");
    }

    #[test]
    fn cleanup_hashtags_preserves_inside_markdown_link() {
        // `#section` inside the URL of a markdown link must stay verbatim.
        let cleaned = super::cleanup_hashtags("see [docs](page.md#section) for details");
        assert_eq!(cleaned, "see [docs](page.md#section) for details");
    }

    #[test]
    fn cleanup_hashtags_strips_outside_excluded_zones() {
        let cleaned = super::cleanup_hashtags("plain #tag and `#code` mixed");
        assert_eq!(cleaned, "plain tag and `#code` mixed");
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
}
