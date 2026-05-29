use crate::settings::themes::Theme;
use pulldown_cmark::{HeadingLevel, Options, Tag};
use ratatui::style::{Modifier, Style};
#[cfg(test)]
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

mod block_opener;
mod detect;
mod parsed_buffer;
mod spanner;
pub(super) use block_opener::opener_shape;
pub use parsed_buffer::ParsedBuffer;
pub use spanner::MarkdownSpanner;

/// Shared parser options used by all pulldown-cmark call sites in this module.
pub(super) const PARSER_OPTIONS: Options = Options::ENABLE_STRIKETHROUGH;

/// Visual columns per tab stop. Must match the `tabstop` setting in the nvim backend.
const TAB_STOP: usize = 4;

/// Compute the display width of a tab character at the given visual column.
pub(super) fn tab_width_at(col: usize) -> usize {
    TAB_STOP - (col % TAB_STOP)
}

/// Sum of grapheme-cluster display widths across a string. Used to size
/// synthetic spans (e.g. image-link placeholders) injected during render.
pub(super) fn string_display_width(s: &str) -> usize {
    s.graphemes(true).map(cluster_display_width).sum()
}

/// Display-column width of a raw line with all clusters visible and tabs
/// expanded to the next tab stop. Mirrors the per-cluster column math in
/// `spanner::render_with` (tab handling + `cluster_display_width`).
pub(super) fn raw_display_width(line: &str) -> usize {
    let mut col = 0usize;
    for g in line.graphemes(true) {
        if g == "\t" {
            col += tab_width_at(col);
        } else {
            col += cluster_display_width(g);
        }
    }
    col
}

/// Display width of a grapheme cluster.
///
/// For multi-codepoint clusters (ZWJ sequences like 👨‍👩‍👧‍👦, variation selectors,
/// skin-tone modifiers) the width is determined by the first codepoint. The
/// combining codepoints that follow contribute 0 additional columns, which
/// matches the rendering behaviour of modern terminal emulators.
pub(super) fn cluster_display_width(cluster: &str) -> usize {
    cluster
        .chars()
        .next()
        .and_then(unicode_width::UnicodeWidthChar::width)
        .unwrap_or(1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub start_char: usize,
    pub end_char: usize,
    pub kind: ElementKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElementKind {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link,
    HeadingH1,
    HeadingH2,
    HeadingH3,
    Blockquote,
    WikiLink,
    Image,
    Label,
}

/// A single image-link span on a parsed line, replaced visually with a
/// placeholder when rendering. `start_char`..`end_char` covers the full
/// `![alt](url)` source range. `placeholder_width` is precomputed so the
/// per-render hot path does not re-walk the placeholder graphemes.
#[derive(Debug, Clone)]
pub struct ImagePlaceholder {
    pub start_char: usize,
    pub end_char: usize,
    pub placeholder: String,
    pub placeholder_width: usize,
}

/// Pre-parsed result for a single logical line.
/// Build once per frame via `ParsedLine::parse`, then reuse across render, cursor,
/// wrap-width, and click-mapping calls to avoid redundant pulldown-cmark invocations.
#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub elements: Vec<Element>,
    /// Per-char visibility: `true` = this char is rendered content (not a markdown sigil).
    pub content_vis: Vec<bool>,
    /// Per-char: `true` = this char falls within any element's char range.
    /// Enables O(1) `in_any_element` without iterating `elements`.
    elem_vis: Vec<bool>,
    /// Per-char element index, 1-based (0 = no element). Enables O(1) `elem_at`.
    /// Stored as `u16`; supports up to 65535 elements per line.
    elem_index: Vec<u16>,
    /// Char offset where the list-item sigil (indent + marker + space) ends on
    /// this line, or `None` if this line is not the first line of a list item.
    list_sigil_end: Option<usize>,
    /// Image-link spans on this line, sorted by `start_char`. Their underlying
    /// chars are hidden (`content_vis = false`) and replaced visually by
    /// `placeholder` when rendering.
    pub image_placeholders: Vec<ImagePlaceholder>,
    /// Blockquote nesting depth (number of leading `>`), or `None` if this
    /// line is not a blockquote. Set by `ParsedBuffer::parse`'s post-pass.
    blockquote_depth: Option<u8>,
}

impl ParsedLine {
    /// Parse a single line in isolation. Internally delegates to
    /// `ParsedBuffer::parse`; kept for test convenience.
    ///
    /// When the line looks like an indented list item (e.g. `    - foo` or
    /// `\t- foo`), pulldown-cmark treats it as an indented code block rather
    /// than a list item on its own. To preserve the real-editor behaviour
    /// (where context from surrounding lines resolves it as a nested list
    /// item), prepend a synthetic parent list marker before handing the input
    /// to `ParsedBuffer::parse` and return the result for the original line.
    pub fn parse(line: &str) -> Self {
        let owned = line.to_string();
        if needs_synthetic_list_parent(line) {
            // "- " opens a list at column 0; the indented `line` that follows
            // becomes a nested list item with full context.
            ParsedBuffer::parse(&["- ".to_string(), owned])
                .lines
                .pop()
                .expect("ParsedBuffer::parse returns one row per input line")
        } else {
            ParsedBuffer::parse(std::slice::from_ref(&owned))
                .lines
                .pop()
                .expect("ParsedBuffer::parse always returns at least one ParsedLine")
        }
    }

    /// Element index at `pos`, or `None`. O(1) via precomputed `elem_index`.
    pub fn elem_at(&self, pos: usize) -> Option<usize> {
        self.elem_index.get(pos).and_then(|&tag| {
            if tag == 0 {
                None
            } else {
                Some((tag as usize) - 1)
            }
        })
    }

    /// Whether `pos` falls inside any tracked element. O(1) via precomputed `elem_vis`.
    pub fn in_any_element(&self, pos: usize) -> bool {
        self.elem_vis.get(pos).copied().unwrap_or(false)
    }

    /// Returns the char offset of the first *content* char inside a heading element
    /// (i.e. the end of the "# " / "## " / "### " sigil region), or `None` if this
    /// line has no heading element.
    ///
    /// Defaults to `e.end_char` so that a heading with no content text (e.g. `"#"`) is
    /// fully treated as sigil — fixes the F-02 bug where `e.start_char` was used.
    pub fn heading_sigil_end(&self) -> Option<usize> {
        self.elements
            .iter()
            .find(|e| {
                matches!(
                    e.kind,
                    ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3
                )
            })
            .map(|e| {
                let mut first_content = e.end_char; // default: all chars are sigil
                for i in e.start_char..e.end_char {
                    if i < self.content_vis.len() && self.content_vis[i] {
                        first_content = i;
                        break;
                    }
                }
                first_content
            })
    }

    /// Char offset where the list-item sigil ends on this line, or `None` if this
    /// line is not the first line of a list item.
    pub fn list_sigil_end(&self) -> Option<usize> {
        self.list_sigil_end
    }

    /// Blockquote nesting depth for this line, or `None` if not a blockquote.
    pub fn blockquote_depth(&self) -> Option<u8> {
        self.blockquote_depth
    }

    /// Char offset where the blockquote marker region (`>`/spaces) ends, i.e.
    /// the first content char. `None` if this line is not a blockquote.
    /// Mirrors `heading_sigil_end`: defaults to the element end when the quote
    /// has no content (e.g. a bare `>`).
    pub fn blockquote_sigil_end(&self) -> Option<usize> {
        self.blockquote_depth?;
        self.elements
            .iter()
            .find(|e| e.kind == ElementKind::Blockquote)
            .map(|e| {
                let mut first_content = e.end_char;
                for i in e.start_char..e.end_char {
                    if i < self.content_vis.len() && self.content_vis[i] {
                        first_content = i;
                        break;
                    }
                }
                first_content
            })
            .or(Some(0))
    }

    /// Diagnostic helper: compare every field for byte-identity. Used by
    /// the view's debug-only correctness assertion. Returns Ok(()) when
    /// all fields match, Err with a human-readable message describing the
    /// first divergence.
    #[cfg(debug_assertions)]
    pub(super) fn debug_assert_eq_to(&self, other: &Self, row: usize) {
        assert_eq!(
            self.content_vis, other.content_vis,
            "row {row} content_vis diverge"
        );
        assert_eq!(self.elem_vis, other.elem_vis, "row {row} elem_vis diverge");
        assert_eq!(
            self.elem_index, other.elem_index,
            "row {row} elem_index diverge"
        );
        assert_eq!(
            self.list_sigil_end, other.list_sigil_end,
            "row {row} list_sigil_end diverge"
        );
        assert_eq!(
            self.blockquote_depth, other.blockquote_depth,
            "row {row} blockquote_depth diverge"
        );
        assert_eq!(
            self.elements.len(),
            other.elements.len(),
            "row {row} elements.len() diverge"
        );
    }
}

/// Detects whether a line is an indented list item (leading spaces or tab,
/// followed by `-`/`*`/`+`/digit-dot + space). Used by `ParsedLine::parse`
/// to decide whether to feed pulldown-cmark a synthetic parent-list context
/// for single-line degenerate inputs.
fn needs_synthetic_list_parent(line: &str) -> bool {
    let trimmed = line.trim_start_matches([' ', '\t']);
    if trimmed.len() == line.len() {
        return false; // no leading whitespace → nothing to compensate for
    }
    list_marker_len(trimmed).is_some()
}

/// If the string begins with an unordered list marker (`- `, `* `, `+ `) or an
/// ordered list marker (digits followed by `. `), returns the marker's length
/// in bytes (including the trailing space). Otherwise `None`.
///
/// Digits are ASCII only, so byte length == char length here.
/// Byte length of the leading run of ASCII space/tab characters in `line`.
/// Equal to the char count for that run (whitespace is ASCII).
pub(super) fn leading_ws_byte_len(line: &str) -> usize {
    line.bytes()
        .take_while(|b| *b == b' ' || *b == b'\t')
        .count()
}

/// Maps a pulldown-cmark start `Tag` to its corresponding `ElementKind`, for
/// the tags whose end events emit a stacked element via the standard
/// push-on-start / pop-on-end pattern. Tags handled specially (e.g. `Item`,
/// `Code`) return `None`.
pub(super) fn tag_to_kind(tag: &Tag) -> Option<ElementKind> {
    Some(match tag {
        Tag::Strong => ElementKind::Bold,
        Tag::Emphasis => ElementKind::Italic,
        Tag::Strikethrough => ElementKind::Strikethrough,
        Tag::Link { .. } => ElementKind::Link,
        Tag::BlockQuote(_) => ElementKind::Blockquote,
        Tag::Heading { level, .. } => match level {
            HeadingLevel::H1 => ElementKind::HeadingH1,
            HeadingLevel::H2 => ElementKind::HeadingH2,
            _ => ElementKind::HeadingH3,
        },
        _ => return None,
    })
}

pub(super) fn list_marker_len(s: &str) -> Option<usize> {
    if s.starts_with("- ") || s.starts_with("* ") || s.starts_with("+ ") {
        return Some(2);
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
        Some(i + 2)
    } else {
        None
    }
}

pub(super) fn span_style(kind: Option<ElementKind>, is_sigil_region: bool, theme: &Theme) -> Style {
    match kind {
        None => {
            if is_sigil_region {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                Style::default().fg(theme.fg.to_ratatui())
            }
        }
        Some(ElementKind::Bold) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::BOLD),
        Some(ElementKind::Italic) => Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(Modifier::ITALIC),
        Some(ElementKind::Strikethrough) => Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .add_modifier(Modifier::CROSSED_OUT),
        Some(ElementKind::InlineCode) => Style::default()
            .fg(theme.fg.to_ratatui())
            .bg(theme.bg_selected.to_ratatui()),
        Some(ElementKind::Link) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::UNDERLINED),
        Some(ElementKind::Image) => Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::ITALIC),
        Some(ElementKind::HeadingH1) => {
            if is_sigil_region {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                Style::default()
                    .fg(theme.accent.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            }
        }
        Some(ElementKind::HeadingH2) => {
            if is_sigil_region {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                Style::default()
                    .fg(theme.fg.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            }
        }
        Some(ElementKind::HeadingH3) => {
            if is_sigil_region {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                Style::default().fg(theme.fg_secondary.to_ratatui())
            }
        }
        Some(ElementKind::Blockquote) => Style::default().fg(theme.fg_secondary.to_ratatui()),
        Some(ElementKind::WikiLink) => Style::default()
            .fg(theme.color_directory.to_ratatui())
            .add_modifier(Modifier::UNDERLINED),
        Some(ElementKind::Label) => Style::default()
            .fg(theme.color_tag.to_ratatui())
            .add_modifier(Modifier::BOLD),
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse_incremental::LineConstructKind;
    use super::*;
    use ratatui::style::Modifier;
    fn t() -> Theme {
        Theme::default()
    }
    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn blockquote_depth_and_sigil_end() {
        let p = ParsedLine::parse("> hello");
        assert_eq!(p.blockquote_depth(), Some(1));
        // sigil region is "> " (2 chars); content starts at index 2.
        assert_eq!(p.blockquote_sigil_end(), Some(2));

        let p2 = ParsedLine::parse(">> deep");
        assert_eq!(p2.blockquote_depth(), Some(2));

        let plain = ParsedLine::parse("not a quote");
        assert_eq!(plain.blockquote_depth(), None);
        assert_eq!(plain.blockquote_sigil_end(), None);
    }
    #[test]
    fn parse_bold_range() {
        let e = MarkdownSpanner::parse_elements("**bold**");
        let b = e.iter().find(|x| x.kind == ElementKind::Bold).unwrap();
        assert_eq!((b.start_char, b.end_char), (0, 8));
    }
    #[test]
    fn parse_italic() {
        assert!(
            MarkdownSpanner::parse_elements("*hi*")
                .iter()
                .any(|e| e.kind == ElementKind::Italic)
        );
    }
    #[test]
    fn parse_strikethrough() {
        let e = MarkdownSpanner::parse_elements("~~gone~~");
        let s = e
            .iter()
            .find(|x| x.kind == ElementKind::Strikethrough)
            .unwrap();
        assert_eq!((s.start_char, s.end_char), (0, 8));
    }
    #[test]
    fn strikethrough_renders_with_crossed_out_modifier() {
        let s = MarkdownSpanner::render("~~gone~~", "~~gone~~", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "gone");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::CROSSED_OUT))
        );
    }
    #[test]
    fn parse_inline_code() {
        assert!(
            MarkdownSpanner::parse_elements("`x`")
                .iter()
                .any(|e| e.kind == ElementKind::InlineCode)
        );
    }
    #[test]
    fn parse_link() {
        assert!(
            MarkdownSpanner::parse_elements("[t](u)")
                .iter()
                .any(|e| e.kind == ElementKind::Link)
        );
    }

    #[test]
    fn parse_image_emits_image_element_and_placeholder() {
        let line = "see ![alt](../assets/img.png) here";
        let parsed = ParsedLine::parse(line);
        let img = parsed
            .elements
            .iter()
            .find(|e| e.kind == ElementKind::Image)
            .expect("image element");
        assert_eq!(line.chars().nth(img.start_char), Some('!'));
        assert_eq!(line.chars().nth(img.end_char - 1), Some(')'));
        let ph = parsed
            .image_placeholders
            .iter()
            .find(|p| p.start_char == img.start_char)
            .expect("placeholder for image");
        assert_eq!(ph.placeholder, "[img.png]");
        for pos in img.start_char..img.end_char {
            assert!(
                !parsed.content_vis[pos],
                "char {pos} should be hidden inside image span"
            );
        }
    }

    #[test]
    fn render_image_substitutes_placeholder_text() {
        let line = "before ![alt](pic.gif) after";
        let parsed = ParsedLine::parse(line);
        let spans =
            MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 80, &t());
        let rendered: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("[pic.gif]"),
            "rendered text {rendered:?} should include placeholder"
        );
        assert!(
            !rendered.contains("![alt]"),
            "raw image syntax should not appear in rendered output: {rendered:?}"
        );
    }

    #[test]
    fn render_image_with_empty_alt_uses_filename() {
        let line = "![](image.png)";
        let parsed = ParsedLine::parse(line);
        let spans =
            MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 40, &t());
        let rendered: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(rendered, "[image.png]");
    }

    #[test]
    fn rendered_cursor_col_accounts_for_placeholder_width() {
        // "![](x.png)" → placeholder "[x.png]" (7 chars) replaces 10 source chars.
        let line = "a ![](x.png) b";
        let parsed = ParsedLine::parse(line);
        let after_placeholder = MarkdownSpanner::rendered_cursor_col_with(
            line,
            &parsed,
            0,
            "a ![](x.png) b".chars().count(), // cursor at end
            true,
            false,
        );
        // "a " (2) + "[x.png]" (7) + " b" (2) = 11.
        assert_eq!(after_placeholder, 11);
    }
    #[test]
    fn parse_h1() {
        assert!(
            MarkdownSpanner::parse_elements("# T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH1)
        );
    }
    #[test]
    fn parse_h2() {
        assert!(
            MarkdownSpanner::parse_elements("## T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH2)
        );
    }
    #[test]
    fn parse_h3() {
        assert!(
            MarkdownSpanner::parse_elements("### T")
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH3)
        );
    }
    #[test]
    fn force_raw_no_styling() {
        let s = MarkdownSpanner::render("**x**", "**x**", 0, None, true, true, 40, &t());
        assert_eq!(text(&s), "**x**");
        assert!(
            !s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD))
        );
    }
    #[test]
    fn plain_text_passthrough() {
        let s = MarkdownSpanner::render("hi", "hi", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "hi");
    }
    #[test]
    fn bold_without_cursor_hides_markers() {
        let s = MarkdownSpanner::render("**bold**", "**bold**", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "bold");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD))
        );
    }
    #[test]
    fn bold_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("**bold**", "**bold**", 0, Some(3), true, false, 40, &t());
        assert_eq!(text(&s), "**bold**");
    }
    #[test]
    fn bold_cursor_outside_stays_rendered() {
        let line = "hello **bold** world";
        let s = MarkdownSpanner::render(line, line, 0, Some(1), true, false, 40, &t());
        assert!(!text(&s).contains("**"));
    }
    #[test]
    fn italic_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("*hi*", "*hi*", 0, Some(1), true, false, 40, &t());
        assert_eq!(text(&s), "*hi*");
    }
    #[test]
    fn inline_code_hides_backticks() {
        let s = MarkdownSpanner::render("`x`", "`x`", 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "x");
    }
    #[test]
    fn h1_first_line_contains_hash() {
        let s = MarkdownSpanner::render("# T", "# T", 0, None, true, false, 40, &t());
        assert!(text(&s).contains('#'));
        assert!(text(&s).contains('T'));
    }
    #[test]
    fn continuation_line_no_hash() {
        let s = MarkdownSpanner::render("cont", "# T cont", 2, None, false, false, 40, &t());
        assert!(!text(&s).contains('#'));
    }
    #[test]
    fn unordered_list_shows_marker() {
        let s = MarkdownSpanner::render("- item", "- item", 0, None, true, false, 40, &t());
        assert!(
            text(&s).starts_with("- "),
            "expected '- item', got '{}'",
            text(&s)
        );
        assert!(text(&s).contains("item"));
    }
    #[test]
    fn ordered_list_shows_marker() {
        let s = MarkdownSpanner::render("1. item", "1. item", 0, None, true, false, 40, &t());
        assert!(
            text(&s).starts_with("1. "),
            "expected '1. item', got '{}'",
            text(&s)
        );
    }
    #[test]
    fn nested_list_4space_link_rendered() {
        // 4-space indent + list marker + markdown link.
        let line = "    - [my link](url)";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        // Link styling must appear (UNDERLINED modifier) and the raw "](url)" sigils
        // must be hidden.
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED)),
            "link text should be underlined on a 4-space-indented nested list item"
        );
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            rendered.contains("my link"),
            "link display text should be visible; got {:?}",
            rendered
        );
        assert!(
            !rendered.contains("](url)"),
            "link URL sigil should be hidden; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_tab_bold_rendered() {
        let line = "\t- **bold nested**";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::BOLD)),
            "bold text should be styled on a tab-indented nested list item"
        );
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            !rendered.contains("**"),
            "bold markers should be hidden; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_4space_wikilink_rendered() {
        let line = "    - [[Target Note]]";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        let rendered: String = s.iter().map(|sp| sp.content.as_ref()).collect();
        assert!(
            !rendered.contains("[["),
            "wikilink brackets should be hidden; got {:?}",
            rendered
        );
        assert!(
            rendered.contains("Target Note"),
            "wikilink target text should render; got {:?}",
            rendered
        );
    }

    #[test]
    fn nested_list_2space_still_renders_link() {
        // Existing 2-space case — must not regress.
        let line = "  - [link](url)";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 80, &t());
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn empty_heading_shows_hash_sigil() {
        let line = "# ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert!(
            text(&s).contains('#'),
            "hash sigil should render in empty heading"
        );
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 1, true, false);
        assert_eq!(col, 1, "cursor after '#' should be at rendered col 1");
    }
    #[test]
    fn empty_heading_hash_only_shows() {
        let line = "#";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert!(text(&s).contains('#'));
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 1, true, false);
        assert_eq!(col, 1);
    }
    #[test]
    fn heading_trailing_spaces_are_rendered() {
        let line = "# Hello   ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(
            text(&s),
            "# Hello   ",
            "trailing spaces in heading should render"
        );
    }
    #[test]
    fn heading_trailing_spaces_cursor_col_correct() {
        let line = "# Hello   ";
        // cursor at logical pos 9 (last trailing space): positions 0..9 all emit → rendered col 9
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 9, true, false);
        assert_eq!(
            col, 9,
            "cursor in trailing space of heading should map to rendered col 9"
        );
    }
    #[test]
    fn trailing_spaces_are_rendered() {
        let line = "hello   ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "hello   ");
    }
    #[test]
    fn trailing_spaces_cursor_col_correct() {
        let line = "hello   ";
        let col = MarkdownSpanner::rendered_cursor_col(line, 0, 7, true, false);
        assert_eq!(col, 7);
    }
    #[test]
    fn list_marker_on_continuation_line_hidden() {
        let s = MarkdownSpanner::render("cont", "- cont", 2, None, false, false, 40, &t());
        assert!(!text(&s).starts_with("- "));
    }
    #[test]
    fn parsed_line_heading_sigil_end_empty_heading() {
        // "#" alone: no content chars, sigil_end should equal e.end_char (1)
        let p = ParsedLine::parse("#");
        assert_eq!(p.heading_sigil_end(), Some(1));
    }
    #[test]
    fn parsed_line_heading_sigil_end_with_content() {
        // "# T": sigil is "# " (2 chars), first content at pos 2
        let p = ParsedLine::parse("# T");
        assert_eq!(p.heading_sigil_end(), Some(2));
    }
    #[test]
    fn parsed_line_reuse_matches_individual() {
        let line = "**hello** world";
        let parsed = ParsedLine::parse(line);
        let s1 = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        let s2 = MarkdownSpanner::render_with(line, line, &parsed, 0, None, true, false, 40, &t());
        assert_eq!(
            s1.iter().map(|s| s.content.as_ref()).collect::<String>(),
            s2.iter().map(|s| s.content.as_ref()).collect::<String>(),
        );
    }

    // ── WikiLink tests ────────────────────────────────────────────────────────

    #[test]
    fn parse_wikilink() {
        let e = MarkdownSpanner::parse_elements("[[My Note]]");
        let wl = e.iter().find(|x| x.kind == ElementKind::WikiLink).unwrap();
        assert_eq!((wl.start_char, wl.end_char), (0, 11));
    }

    #[test]
    fn wikilink_without_cursor_hides_brackets() {
        let line = "[[My Note]]";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "My Note");
        assert!(
            s.iter()
                .any(|sp| sp.style.add_modifier.contains(Modifier::UNDERLINED))
        );
    }

    #[test]
    fn wikilink_cursor_inside_shows_brackets() {
        let line = "[[My Note]]";
        // cursor at pos 4 (inside "My Note")
        let s = MarkdownSpanner::render(line, line, 0, Some(4), true, false, 40, &t());
        assert_eq!(text(&s), "[[My Note]]");
    }

    #[test]
    fn wikilink_cursor_outside_hides_brackets() {
        let line = "hello [[My Note]] world";
        let s = MarkdownSpanner::render(line, line, 0, Some(1), true, false, 40, &t());
        assert!(!text(&s).contains("[["));
        assert!(!text(&s).contains("]]"));
    }

    #[test]
    fn wikilink_mid_sentence() {
        let line = "See [[Topic]] for details";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert_eq!(text(&s), "See Topic for details");
    }

    #[test]
    fn wikilink_cursor_col_accounts_for_brackets() {
        // "[[Hi]]" — cursor at pos 2 ('H') is inside the element, so it expands.
        // Rendered col counts pos 0 ('['), pos 1 ('[') as visible (expanded sigils) → col = 2.
        let col = MarkdownSpanner::rendered_cursor_col("[[Hi]]", 0, 2, true, false);
        assert_eq!(col, 2);

        // Cursor outside the wikilink (pos 0 on a plain-text line before it):
        // "See [[Hi]] x" with cursor at pos 0 — wikilink not expanded, brackets hidden.
        // pos 0 ('S') is plain text, rendered col = 0.
        let col2 = MarkdownSpanner::rendered_cursor_col("See [[Hi]] x", 0, 0, true, false);
        assert_eq!(col2, 0);
    }

    #[test]
    fn buffer_parse_nested_list_under_parent() {
        // Canonical nested-list pattern: parent at col 0, child indented 4.
        let lines = vec![
            "- parent".to_string(),
            "    - [child link](url)".to_string(),
        ];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert_eq!(parsed.len(), 2);

        // Parent line: list sigil at col 2.
        assert_eq!(parsed[0].list_sigil_end(), Some(2));

        // Child line: pulldown-cmark reports the item marker at col 4.
        assert_eq!(
            parsed[1].list_sigil_end(),
            Some(6),
            "child's sigil_end should be after '    - ' (6 chars)"
        );

        // Child line has a Link element.
        assert!(
            parsed[1]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Link),
            "nested list item should contain a Link element"
        );
    }

    #[test]
    fn buffer_parse_standalone_2space_list_still_works() {
        // Regression: 2-space indent works on its own too.
        let lines = vec!["  - [link](url)".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert!(
            parsed[0]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Link)
        );
        assert_eq!(parsed[0].list_sigil_end(), Some(4));
    }

    #[test]
    fn buffer_parse_top_level_unchanged() {
        // Ensure nothing about top-level rendering changed.
        let lines = vec!["- [link](url)".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert!(
            parsed[0]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Link)
        );
        assert_eq!(parsed[0].list_sigil_end(), Some(2));
    }

    #[test]
    fn buffer_parse_empty_lines_preserved() {
        let lines = vec![
            "# Title".to_string(),
            String::new(),
            "paragraph".to_string(),
        ];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[1].elements.len(), 0);
        assert_eq!(parsed[1].content_vis.len(), 0);
    }

    #[test]
    fn buffer_parse_ordered_nested_list() {
        let lines = vec!["1. first".to_string(), "    1. nested".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert_eq!(parsed[0].list_sigil_end(), Some(3));
        assert_eq!(parsed[1].list_sigil_end(), Some(7));
    }

    #[test]
    fn buffer_parse_setext_h1_spans_two_rows() {
        // Setext H1: the `=====` line is part of the heading span.
        // Under the old per-line parser, row 1 rendered as plain text; under the
        // whole-buffer parser, pulldown emits one HeadingH1 covering both rows and
        // row 1 has no Text events, so the underline renders in the sigil color.
        // Pin this behavior — a regression would silently un-style setext headings.
        let lines = vec!["My Heading".to_string(), "==========".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert!(
            parsed[0]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH1),
            "setext underline must tag row 0 as HeadingH1"
        );
        assert!(
            parsed[1]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::HeadingH1),
            "setext underline must tag row 1 as HeadingH1"
        );
        // Row 1 has no Text events — content_vis is all false.
        assert!(
            parsed[1].content_vis.iter().all(|v| !v),
            "setext underline row has no content"
        );
    }

    #[test]
    fn buffer_parse_multiline_blockquote() {
        // Two blockquote lines in a row — pulldown folds them into one blockquote.
        // Both rows must carry a Blockquote element so rendering is consistent.
        let lines = vec!["> first line".to_string(), "> second line".to_string()];
        let parsed = ParsedBuffer::parse(&lines).lines;
        assert!(
            parsed[0]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Blockquote),
            "row 0 must tag as Blockquote"
        );
        assert!(
            parsed[1]
                .elements
                .iter()
                .any(|e| e.kind == ElementKind::Blockquote),
            "row 1 must tag as Blockquote"
        );
    }

    #[test]
    fn parse_line_emits_label_for_hashtag() {
        let line = "see #rust later";
        let parsed = ParsedLine::parse(line);
        let label = parsed
            .elements
            .iter()
            .find(|e| matches!(e.kind, ElementKind::Label));
        assert!(
            label.is_some(),
            "expected Label element: {:?}",
            parsed.elements
        );
        let l = label.unwrap();
        let span: String = line
            .chars()
            .skip(l.start_char)
            .take(l.end_char - l.start_char)
            .collect();
        assert_eq!(span, "#rust");
    }

    #[test]
    fn parse_line_skips_label_inside_inline_code() {
        let parsed = ParsedLine::parse("use `#foo` here");
        let has_label = parsed
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Label));
        assert!(!has_label, "should not emit Label inside inline code");
    }

    // ── New label-parity tests (F2, F3, F4) ──────────────────────────────────

    #[test]
    fn parse_line_skips_label_inside_markdown_link() {
        let parsed = ParsedLine::parse("[see docs](#section) and #real");
        let labels: Vec<_> = parsed
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Label))
            .collect();
        assert_eq!(
            labels.len(),
            1,
            "only #real should be a label, not #section in the link"
        );
        let l = labels[0];
        let span: String = "[see docs](#section) and #real"
            .chars()
            .skip(l.start_char)
            .take(l.end_char - l.start_char)
            .collect();
        assert_eq!(span, "#real");
    }

    #[test]
    fn parse_line_skips_label_inside_link_display_text() {
        let parsed = ParsedLine::parse("[#todo](notes/project.md)");
        let has_label = parsed
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Label));
        assert!(
            !has_label,
            "hashtag inside link display text should not become Label"
        );
    }

    #[test]
    fn parse_line_skips_label_after_label_char() {
        let parsed = ParsedLine::parse("foo#bar baz");
        let has_label = parsed
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Label));
        assert!(
            !has_label,
            "word#tag should not emit Label without word boundary"
        );
    }

    #[test]
    fn parse_line_skips_label_for_double_hash() {
        // `##draft` is Markdown header territory, not a label — pin the
        // highlighter to the same rule the indexer enforces so a future
        // core relaxation cannot silently re-color this span.
        let parsed = ParsedLine::parse("##draft");
        let has_label = parsed
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Label));
        assert!(!has_label, "##draft should not emit Label");
    }

    #[test]
    fn parse_line_skips_label_for_adjacent_hash_run() {
        // `#tag#more` — adjacent `#` invalidates both halves at the index
        // level; the highlighter must agree to avoid suggesting tags that
        // will never appear in the labels table.
        let parsed = ParsedLine::parse("#tag#more");
        let labels: Vec<_> = parsed
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Label))
            .collect();
        assert!(
            labels.is_empty(),
            "#tag#more should not emit Label, got {:?}",
            labels
        );
    }

    #[test]
    fn parse_buffer_skips_label_inside_fenced_block() {
        let buffer = vec![
            "before".to_string(),
            "```".to_string(),
            "#inside".to_string(),
            "```".to_string(),
            "after #outside".to_string(),
        ];
        let lines = ParsedBuffer::parse(&buffer).lines;
        let inside_labels: Vec<_> = lines[2]
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Label))
            .collect();
        assert!(
            inside_labels.is_empty(),
            "no Label emitted for hashtags in fenced blocks"
        );

        let outside_labels: Vec<_> = lines[4]
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Label))
            .collect();
        assert_eq!(outside_labels.len(), 1, "#outside still extracted");
    }

    #[test]
    fn parse_range_full_equals_parse() {
        let lines: Vec<String> = vec!["hello".into(), "world".into(), "".into(), "**bold**".into()];
        let full = ParsedBuffer::parse(&lines);
        let range_full = ParsedBuffer::parse_range(&lines, 0..lines.len());
        assert_eq!(full.lines.len(), range_full.lines.len());
        assert_eq!(full.kinds, range_full.kinds);
        for (a, b) in full.lines.iter().zip(range_full.lines.iter()) {
            assert_eq!(a.content_vis, b.content_vis);
            assert_eq!(a.elements.len(), b.elements.len());
        }
    }

    #[test]
    fn parse_range_paragraph_only_slice() {
        let lines: Vec<String> = vec![
            "intro paragraph".into(),
            "".into(),
            "middle line".into(),
            "".into(),
            "outro".into(),
        ];
        let slice = ParsedBuffer::parse_range(&lines, 2..3);
        assert_eq!(slice.lines.len(), 1);
        assert_eq!(slice.kinds, vec![LineConstructKind::Plain]);
    }

    #[test]
    fn splice_replaces_range() {
        let mut pb = ParsedBuffer::parse(&["alpha".into(), "beta".into(), "gamma".into()]);
        let replacement = ParsedBuffer::parse(&["BETA-NEW".into()]);
        let replacement_kind = replacement.kinds[0];
        pb.splice(1..2, replacement);
        assert_eq!(pb.lines.len(), 3);
        assert_eq!(pb.kinds.len(), 3);
        assert_eq!(
            pb.kinds[1], replacement_kind,
            "replacement landed at the wrong index"
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "splice")]
    fn splice_panics_on_length_mismatch_in_debug() {
        let mut pb = ParsedBuffer::parse(&["a".into(), "b".into()]);
        let too_short = ParsedBuffer::parse(&["X".into()]);
        pb.splice(0..2, too_short);
    }

    // ── V2 lazy_depth tracking ───────────────────────────────────────────────

    /// CORRECTED FROM SPEC: tasks.md 2.1 asserted `[1, 1, 1, 0]`,
    /// claiming blockquote lazy-extends across blanks. This is
    /// incorrect per CommonMark §5.1 — a blank line ENDS a
    /// blockquote (see Example 209). Pulldown closes the blockquote
    /// at the first blank, so lazy_depth drops there. The §5.1 lazy
    /// "paragraph continuation" cited in the spec is about non-`>`
    /// lines continuing an OPEN paragraph (still on the same line
    /// run), not extending the blockquote across blanks.
    #[test]
    fn lazy_depth_blockquote_closes_at_first_blank() {
        let lines: Vec<String> = vec!["> a".into(), "".into(), "".into(), "x".into()];
        let pb = ParsedBuffer::parse(&lines);
        assert_eq!(
            pb.lazy_depth,
            vec![1, 0, 0, 0],
            "blockquote closes at first blank per CommonMark §5.1; got {:?}",
            pb.lazy_depth,
        );
    }

    /// IndentedCode lazy-extends across a blank row joining two
    /// indented chunks (CommonMark §4.4). All three rows must
    /// report lazy_depth ≥ 1 — including the last content row, so
    /// the v2 structural guard catches edits anywhere inside the
    /// block.
    #[test]
    fn lazy_depth_indented_code_across_blanks() {
        let lines: Vec<String> = vec!["    code".into(), "".into(), "    more".into()];
        let pb = ParsedBuffer::parse(&lines);
        assert_eq!(
            pb.lazy_depth,
            vec![1, 1, 1],
            "indented code multi-chunk should keep lazy_depth > 0 across the blank \
             AND through the last content row; got {:?}",
            pb.lazy_depth,
        );
    }

    /// Fenced code blocks are NOT lazy-continuable — their closing
    /// fence is a hard terminator. lazy_depth must remain 0 on
    /// every row.
    #[test]
    fn lazy_depth_fenced_code_does_not_count() {
        let lines: Vec<String> = vec!["```".into(), "x".into(), "```".into(), "".into()];
        let pb = ParsedBuffer::parse(&lines);
        assert_eq!(
            pb.lazy_depth,
            vec![0, 0, 0, 0],
            "fenced code is not lazy-continuable; got {:?}",
            pb.lazy_depth,
        );
    }

    /// Regression: BlockQuote followed by a trailing blank row must
    /// drop `lazy_depth` AT the blank row, not past it. The buggy
    /// past-EOF heuristic in `byte_to_row_col_unclamped` mis-fired
    /// for End events landing on the START of a trailing empty row
    /// (binary_search returned `Ok(r)` with `r < lines.len()` and a
    /// 0-length row), shunting the decrement into the past-array
    /// sentinel slot and leaving `lazy_depth[r]` elevated. That in
    /// turn suppressed the legitimate reset boundary at row r and
    /// forced full rebuilds on every edit adjacent to a trailing
    /// blank.
    #[test]
    fn lazy_depth_blockquote_with_trailing_blank_drops_at_blank() {
        let lines: Vec<String> = vec!["> a".into(), "".into()];
        let pb = ParsedBuffer::parse(&lines);
        assert_eq!(
            pb.lazy_depth,
            vec![1, 0],
            "blockquote must close at the trailing blank; got {:?}",
            pb.lazy_depth,
        );
        assert!(
            pb.reset_boundaries.contains(&1),
            "the trailing blank at row 1 must be a reset boundary; got {:?}",
            pb.reset_boundaries,
        );
    }

    /// Boundary detection must skip rows inside a lazy-continuable
    /// block. Using the IndentedCode multi-chunk fixture (the
    /// canonical §4.4 case) every row has lazy_depth > 0, so no
    /// interior boundary can land. Only the sentinels remain.
    ///
    /// CORRECTED FROM SPEC: tasks.md 2.4 used the blockquote
    /// fixture from 2.1, which does NOT produce interior
    /// lazy_depth > 0 rows (blanks end the blockquote). The
    /// IndentedCode multi-chunk fixture is the correct one for
    /// this invariant.
    #[test]
    fn boundaries_skip_rows_inside_lazy_block() {
        let lines: Vec<String> = vec!["    code".into(), "".into(), "    more".into()];
        let pb = ParsedBuffer::parse(&lines);
        assert_eq!(
            pb.reset_boundaries,
            vec![0, lines.len()],
            "no boundary should land on a blank row inside the open indented-code block; \
             got {:?}",
            pb.reset_boundaries,
        );
    }
}
