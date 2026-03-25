use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use crate::settings::themes::Theme;

/// Shared parser options used by all pulldown-cmark call sites in this module.
const PARSER_OPTIONS: Options = Options::ENABLE_STRIKETHROUGH;

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub start_char: usize,
    pub end_char: usize,
    pub kind: ElementKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ElementKind {
    Bold, Italic, InlineCode, Link,
    HeadingH1, HeadingH2, HeadingH3, Blockquote,
}

/// Pre-parsed result for a single logical line.
/// Build once per frame via `ParsedLine::parse`, then reuse across render, cursor,
/// wrap-width, and click-mapping calls to avoid redundant pulldown-cmark invocations.
#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub elements: Vec<Element>,
    /// Per-char visibility: `true` = this char is rendered content (not a markdown sigil).
    pub content_vis: Vec<bool>,
}

impl ParsedLine {
    pub fn parse(line: &str) -> Self {
        Self {
            elements: MarkdownSpanner::parse_elements(line),
            content_vis: MarkdownSpanner::content_positions(line),
        }
    }

    /// Innermost element index at `pos`, or `None`.
    pub fn elem_at(&self, pos: usize) -> Option<usize> {
        self.elements.iter().enumerate().rev()
            .find(|(_, e)| e.start_char <= pos && pos < e.end_char)
            .map(|(i, _)| i)
    }

    /// Whether `pos` falls inside any tracked element.
    pub fn in_any_element(&self, pos: usize) -> bool {
        self.elements.iter().any(|e| e.start_char <= pos && pos < e.end_char)
    }

    /// Returns the char offset of the first *content* char inside a heading element
    /// (i.e. the end of the "# " / "## " / "### " sigil region), or `None` if this
    /// line has no heading element.
    ///
    /// Defaults to `e.end_char` so that a heading with no content text (e.g. `"#"`) is
    /// fully treated as sigil — fixes the F-02 bug where `e.start_char` was used.
    pub fn heading_sigil_end(&self) -> Option<usize> {
        self.elements.iter()
            .find(|e| matches!(e.kind,
                ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3))
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
}

pub struct MarkdownSpanner;

impl MarkdownSpanner {
    pub fn parse_elements(line: &str) -> Vec<Element> {
        let parser = Parser::new_ext(line, PARSER_OPTIONS);
        let mut elements = Vec::new();
        let mut stack: Vec<(usize, ElementKind)> = Vec::new();
        for (event, range) in parser.into_offset_iter() {
            let sc = line[..range.start].chars().count();
            let ec = line[..range.end].chars().count();
            match event {
                Event::Start(Tag::Strong) => stack.push((sc, ElementKind::Bold)),
                Event::End(TagEnd::Strong) => if let Some((s,k)) = stack.pop() {
                    elements.push(Element { start_char: s, end_char: ec, kind: k });
                },
                Event::Start(Tag::Emphasis) => stack.push((sc, ElementKind::Italic)),
                Event::End(TagEnd::Emphasis) => if let Some((s,k)) = stack.pop() {
                    elements.push(Element { start_char: s, end_char: ec, kind: k });
                },
                Event::Start(Tag::Link { .. }) => stack.push((sc, ElementKind::Link)),
                Event::End(TagEnd::Link) => if let Some((s,k)) = stack.pop() {
                    elements.push(Element { start_char: s, end_char: ec, kind: k });
                },
                Event::Code(_) => elements.push(Element { start_char: sc, end_char: ec, kind: ElementKind::InlineCode }),
                Event::Start(Tag::Heading { level, .. }) => {
                    let kind = match level {
                        HeadingLevel::H1 => ElementKind::HeadingH1,
                        HeadingLevel::H2 => ElementKind::HeadingH2,
                        _ => ElementKind::HeadingH3,
                    };
                    stack.push((sc, kind));
                }
                Event::End(TagEnd::Heading(_)) => if let Some((s,k)) = stack.pop() {
                    elements.push(Element { start_char: s, end_char: ec, kind: k });
                },
                Event::Start(Tag::BlockQuote(_)) => stack.push((sc, ElementKind::Blockquote)),
                Event::End(TagEnd::BlockQuote(_)) => if let Some((s,k)) = stack.pop() {
                    elements.push(Element { start_char: s, end_char: ec, kind: k });
                },
                _ => {}
            }
        }
        elements
    }

    /// Compute which char positions in `line` are "content" (not markdown sigils).
    pub fn content_positions(line: &str) -> Vec<bool> {
        let total = line.chars().count();
        let mut visible = vec![false; total];
        let parser = Parser::new_ext(line, PARSER_OPTIONS);
        for (event, range) in parser.into_offset_iter() {
            match &event {
                Event::Text(_) | Event::SoftBreak | Event::HardBreak => {
                    let sc = line[..range.start].chars().count();
                    let ec = line[..range.end].chars().count();
                    for i in sc..ec {
                        if i < total { visible[i] = true; }
                    }
                }
                Event::Code(code_text) => {
                    let range_sc = line[..range.start].chars().count();
                    let code_len = code_text.chars().count();
                    let range_char_len = line[range.start..range.end].chars().count();
                    let sigil_each = (range_char_len - code_len) / 2;
                    let content_start = range_sc + sigil_each;
                    let content_end = content_start + code_len;
                    for i in content_start..content_end {
                        if i < total { visible[i] = true; }
                    }
                }
                _ => {}
            }
        }
        visible
    }

    // ── Public API ────────────────────────────────────────────────────────────

    pub fn render<'a>(
        content: &'a str,
        logical_line: &'a str,
        visual_start_col: usize,
        cursor_col: Option<usize>,
        is_first_visual_line: bool,
        force_raw: bool,
        available_width: u16,
        theme: &Theme,
    ) -> Vec<Span<'a>> {
        let parsed = ParsedLine::parse(logical_line);
        Self::render_with(content, logical_line, &parsed, visual_start_col, cursor_col,
            is_first_visual_line, force_raw, available_width, theme)
    }

    pub fn rendered_cursor_col(
        logical_line: &str,
        visual_start_col: usize,
        cursor_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        let parsed = ParsedLine::parse(logical_line);
        Self::rendered_cursor_col_with(logical_line, &parsed, visual_start_col, cursor_col,
            is_first_visual_line, force_raw)
    }

    pub fn visible_positions(
        logical_line: &str,
        cursor_col: Option<usize>,
        force_raw: bool,
    ) -> Vec<bool> {
        let parsed = ParsedLine::parse(logical_line);
        Self::visible_positions_with(logical_line, &parsed, cursor_col, force_raw)
    }

    pub fn rendered_col_to_logical(
        logical_line: &str,
        visual_start_col: usize,
        rendered_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        let parsed = ParsedLine::parse(logical_line);
        Self::rendered_col_to_logical_with(logical_line, &parsed, visual_start_col, rendered_col,
            is_first_visual_line, force_raw)
    }

    // ── `_with` variants: accept pre-parsed `&ParsedLine` ────────────────────

    pub fn render_with<'a>(
        content: &'a str,
        logical_line: &'a str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        cursor_col: Option<usize>,
        is_first_visual_line: bool,
        force_raw: bool,
        available_width: u16,
        theme: &Theme,
    ) -> Vec<Span<'a>> {
        // HR
        let trimmed = logical_line.trim();
        if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
            if cursor_col.is_some() {
                return vec![Span::styled(content, Style::default().fg(theme.fg_muted.to_ratatui()))];
            }
            return vec![Span::styled(
                "─".repeat(available_width as usize),
                Style::default().fg(theme.fg_muted.to_ratatui()),
            )];
        }
        // Force-raw (inside fenced code block)
        if force_raw {
            return vec![Span::styled(content, Style::default().fg(theme.fg_secondary.to_ratatui()))];
        }

        let elements = &parsed.elements;
        let content_vis = &parsed.content_vis;
        let content_char_count = content.chars().count();

        let elem_at = |pos: usize| -> Option<usize> {
            elements.iter().enumerate().rev()
                .find(|(_, e)| e.start_char <= pos && pos < e.end_char)
                .map(|(i, _)| i)
        };
        let expanded: Option<usize> = cursor_col.and_then(|c| elem_at(c));

        let heading_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.heading_sigil_end()
        } else {
            None
        };
        let list_sigil_end: Option<usize> = if is_first_visual_line {
            detect_list_marker(logical_line)
        } else {
            None
        };

        let mut spans: Vec<Span<'a>> = Vec::new();
        let mut seg_str: String = String::new();
        let mut seg_elem: Option<usize> = None;
        let mut seg_is_sigil = false;
        let mut seg_is_expanded = false;

        let flush = |seg_str: &mut String, seg_elem: Option<usize>,
                     seg_is_sigil: bool, seg_is_expanded: bool, spans: &mut Vec<Span<'a>>| {
            if seg_str.is_empty() { return; }
            let seg = std::mem::take(seg_str);
            let style = if seg_is_expanded {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                span_style(seg_elem.map(|i| elements[i].kind), seg_is_sigil, theme)
            };
            spans.push(Span::styled(seg, style));
        };

        for (pos, ch) in logical_line.chars().enumerate()
            .skip(visual_start_col)
            .take(content_char_count)
        {
            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.map_or(false, |end| pos < end);
            let in_list_sigil = list_sigil_end.map_or(false, |end| pos < end);
            let in_expanded_elem = expanded.map_or(false, |i| {
                elements[i].start_char <= pos && pos < elements[i].end_char
            });
            let this_elem = elem_at(pos);
            let emit = is_content || in_heading_sigil || in_list_sigil || in_expanded_elem
                || this_elem.is_none();
            if !emit {
                flush(&mut seg_str, seg_elem, seg_is_sigil, seg_is_expanded, &mut spans);
                seg_elem = None;
                seg_is_sigil = false;
                seg_is_expanded = false;
                continue;
            }
            let this_is_expanded = in_expanded_elem;
            let this_is_sigil = (in_heading_sigil || in_list_sigil) && !is_content && !in_expanded_elem;
            if this_elem != seg_elem || this_is_sigil != seg_is_sigil || this_is_expanded != seg_is_expanded {
                flush(&mut seg_str, seg_elem, seg_is_sigil, seg_is_expanded, &mut spans);
                seg_elem = this_elem;
                seg_is_sigil = this_is_sigil;
                seg_is_expanded = this_is_expanded;
            }
            seg_str.push(ch);
        }
        flush(&mut seg_str, seg_elem, seg_is_sigil, seg_is_expanded, &mut spans);

        if spans.is_empty() {
            spans.push(Span::styled(content, Style::default().fg(theme.fg.to_ratatui())));
        }
        spans
    }

    pub fn rendered_cursor_col_with(
        logical_line: &str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        cursor_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        if force_raw {
            return cursor_col.saturating_sub(visual_start_col);
        }
        let trimmed = logical_line.trim();
        if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
            return cursor_col.saturating_sub(visual_start_col);
        }

        let elements = &parsed.elements;
        let content_vis = &parsed.content_vis;
        let logical_char_count = logical_line.chars().count();

        let expanded: Option<usize> = parsed.elem_at(cursor_col);
        let heading_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.heading_sigil_end()
        } else {
            None
        };
        let list_sigil_end: Option<usize> = if is_first_visual_line {
            detect_list_marker(logical_line)
        } else {
            None
        };

        let end = cursor_col.min(logical_char_count);
        (visual_start_col..end).filter(|&pos| {
            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.map_or(false, |s_end| pos < s_end);
            let in_list_sigil = list_sigil_end.map_or(false, |s_end| pos < s_end);
            let in_expanded_elem = expanded.map_or(false, |i| {
                elements[i].start_char <= pos && pos < elements[i].end_char
            });
            let in_any_element = parsed.in_any_element(pos);
            is_content || in_heading_sigil || in_list_sigil || in_expanded_elem || !in_any_element
        }).count()
    }

    pub fn visible_positions_with(
        logical_line: &str,
        parsed: &ParsedLine,
        cursor_col: Option<usize>,
        force_raw: bool,
    ) -> Vec<bool> {
        let total = logical_line.chars().count();
        if total == 0 {
            return vec![];
        }
        if force_raw {
            return vec![true; total];
        }
        let trimmed = logical_line.trim();
        if matches!(trimmed, "---" | "***" | "___") {
            return vec![true; total];
        }

        let content_vis = &parsed.content_vis;
        let expanded: Option<&Element> = cursor_col.and_then(|c| {
            parsed.elements.iter().rev().find(|e| e.start_char <= c && c < e.end_char)
        });
        let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
        let list_sigil_end = detect_list_marker(logical_line);

        (0..total)
            .map(|pos| {
                let is_content = pos < content_vis.len() && content_vis[pos];
                let in_heading_sigil = heading_sigil_end.map_or(false, |end| pos < end);
                let in_list_sigil = list_sigil_end.map_or(false, |end| pos < end);
                let in_any_element = parsed.in_any_element(pos);
                let in_expanded = expanded
                    .map_or(false, |e| e.start_char <= pos && pos < e.end_char);
                is_content || in_heading_sigil || in_list_sigil || in_expanded || !in_any_element
            })
            .collect()
    }

    pub fn rendered_col_to_logical_with(
        logical_line: &str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        rendered_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        if force_raw {
            return visual_start_col + rendered_col;
        }
        let trimmed = logical_line.trim();
        if is_first_visual_line && matches!(trimmed, "---" | "***" | "___") {
            return visual_start_col + rendered_col;
        }

        let content_vis = &parsed.content_vis;
        let logical_char_count = logical_line.chars().count();
        let heading_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.heading_sigil_end()
        } else {
            None
        };
        let list_sigil_end: Option<usize> = if is_first_visual_line {
            detect_list_marker(logical_line)
        } else {
            None
        };

        let mut rendered_count = 0;
        for pos in visual_start_col..logical_char_count {
            if rendered_count == rendered_col {
                return pos;
            }
            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.map_or(false, |end| pos < end);
            let in_list_sigil = list_sigil_end.map_or(false, |end| pos < end);
            let in_any_element = parsed.in_any_element(pos);
            if is_content || in_heading_sigil || in_list_sigil || !in_any_element {
                rendered_count += 1;
            }
        }
        logical_char_count
    }
}

/// Returns the length (in chars) of the list marker prefix, or None if not a list line.
fn detect_list_marker(line: &str) -> Option<usize> {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return Some(2);
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
        return Some(i + 2);
    }
    None
}

fn span_style(kind: Option<ElementKind>, is_sigil_region: bool, theme: &Theme) -> Style {
    match kind {
        None => if is_sigil_region {
            Style::default().fg(theme.fg_muted.to_ratatui())
        } else {
            Style::default().fg(theme.fg.to_ratatui())
        },
        Some(ElementKind::Bold) =>
            Style::default().fg(theme.accent.to_ratatui()).add_modifier(Modifier::BOLD),
        Some(ElementKind::Italic) =>
            Style::default().fg(theme.fg_secondary.to_ratatui()).add_modifier(Modifier::ITALIC),
        Some(ElementKind::InlineCode) =>
            Style::default().fg(theme.fg.to_ratatui()).bg(theme.bg_selected.to_ratatui()),
        Some(ElementKind::Link) =>
            Style::default().fg(theme.accent.to_ratatui()).add_modifier(Modifier::UNDERLINED),
        Some(ElementKind::HeadingH1) => if is_sigil_region {
            Style::default().fg(theme.fg_muted.to_ratatui())
        } else {
            Style::default().fg(theme.accent.to_ratatui()).add_modifier(Modifier::BOLD)
        },
        Some(ElementKind::HeadingH2) => if is_sigil_region {
            Style::default().fg(theme.fg_muted.to_ratatui())
        } else {
            Style::default().fg(theme.fg.to_ratatui()).add_modifier(Modifier::BOLD)
        },
        Some(ElementKind::HeadingH3) => if is_sigil_region {
            Style::default().fg(theme.fg_muted.to_ratatui())
        } else {
            Style::default().fg(theme.fg_secondary.to_ratatui())
        },
        Some(ElementKind::Blockquote) =>
            Style::default().fg(theme.fg_secondary.to_ratatui()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;
    fn t() -> Theme { Theme::default() }
    fn text(spans: &[Span]) -> String { spans.iter().map(|s| s.content.as_ref()).collect() }

    #[test]
    fn parse_bold_range() {
        let e = MarkdownSpanner::parse_elements("**bold**");
        let b = e.iter().find(|x| x.kind == ElementKind::Bold).unwrap();
        assert_eq!((b.start_char, b.end_char), (0, 8));
    }
    #[test]
    fn parse_italic() {
        assert!(MarkdownSpanner::parse_elements("*hi*").iter().any(|e| e.kind == ElementKind::Italic));
    }
    #[test]
    fn parse_inline_code() {
        assert!(MarkdownSpanner::parse_elements("`x`").iter().any(|e| e.kind == ElementKind::InlineCode));
    }
    #[test]
    fn parse_link() {
        assert!(MarkdownSpanner::parse_elements("[t](u)").iter().any(|e| e.kind == ElementKind::Link));
    }
    #[test]
    fn parse_h1() {
        assert!(MarkdownSpanner::parse_elements("# T").iter().any(|e| e.kind == ElementKind::HeadingH1));
    }
    #[test]
    fn parse_h2() {
        assert!(MarkdownSpanner::parse_elements("## T").iter().any(|e| e.kind == ElementKind::HeadingH2));
    }
    #[test]
    fn parse_h3() {
        assert!(MarkdownSpanner::parse_elements("### T").iter().any(|e| e.kind == ElementKind::HeadingH3));
    }
    #[test]
    fn force_raw_no_styling() {
        let s = MarkdownSpanner::render("**x**","**x**",0,None,true,true,40,&t());
        assert_eq!(text(&s), "**x**");
        assert!(!s.iter().any(|sp| sp.style.add_modifier.contains(Modifier::BOLD)));
    }
    #[test]
    fn plain_text_passthrough() {
        let s = MarkdownSpanner::render("hi","hi",0,None,true,false,40,&t());
        assert_eq!(text(&s), "hi");
    }
    #[test]
    fn bold_without_cursor_hides_markers() {
        let s = MarkdownSpanner::render("**bold**","**bold**",0,None,true,false,40,&t());
        assert_eq!(text(&s), "bold");
        assert!(s.iter().any(|sp| sp.style.add_modifier.contains(Modifier::BOLD)));
    }
    #[test]
    fn bold_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("**bold**","**bold**",0,Some(3),true,false,40,&t());
        assert_eq!(text(&s), "**bold**");
    }
    #[test]
    fn bold_cursor_outside_stays_rendered() {
        let line = "hello **bold** world";
        let s = MarkdownSpanner::render(line,line,0,Some(1),true,false,40,&t());
        assert!(!text(&s).contains("**"));
    }
    #[test]
    fn italic_cursor_inside_shows_raw() {
        let s = MarkdownSpanner::render("*hi*","*hi*",0,Some(1),true,false,40,&t());
        assert_eq!(text(&s), "*hi*");
    }
    #[test]
    fn inline_code_hides_backticks() {
        let s = MarkdownSpanner::render("`x`","`x`",0,None,true,false,40,&t());
        assert_eq!(text(&s), "x");
    }
    #[test]
    fn h1_first_line_contains_hash() {
        let s = MarkdownSpanner::render("# T","# T",0,None,true,false,40,&t());
        assert!(text(&s).contains('#'));
        assert!(text(&s).contains('T'));
    }
    #[test]
    fn continuation_line_no_hash() {
        let s = MarkdownSpanner::render("cont","# T cont",2,None,false,false,40,&t());
        assert!(!text(&s).contains('#'));
    }
    #[test]
    fn unordered_list_shows_marker() {
        let s = MarkdownSpanner::render("- item","- item",0,None,true,false,40,&t());
        assert!(text(&s).starts_with("- "), "expected '- item', got '{}'", text(&s));
        assert!(text(&s).contains("item"));
    }
    #[test]
    fn ordered_list_shows_marker() {
        let s = MarkdownSpanner::render("1. item","1. item",0,None,true,false,40,&t());
        assert!(text(&s).starts_with("1. "), "expected '1. item', got '{}'", text(&s));
    }
    #[test]
    fn empty_heading_shows_hash_sigil() {
        let line = "# ";
        let s = MarkdownSpanner::render(line, line, 0, None, true, false, 40, &t());
        assert!(text(&s).contains('#'), "hash sigil should render in empty heading");
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
        let s = MarkdownSpanner::render("cont","- cont",2,None,false,false,40,&t());
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
}
