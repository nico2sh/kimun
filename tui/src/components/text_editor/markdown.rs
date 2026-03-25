use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use crate::settings::themes::Theme;

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

pub struct MarkdownSpanner;

impl MarkdownSpanner {
    pub fn parse_elements(line: &str) -> Vec<Element> {
        let parser = Parser::new_ext(line, Options::ENABLE_STRIKETHROUGH);
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
    /// Returns a Vec<bool> of length line.chars().count() where true = visible content.
    fn content_positions(line: &str) -> Vec<bool> {
        let total = line.chars().count();
        let mut visible = vec![false; total];
        let parser = Parser::new_ext(line, Options::ENABLE_STRIKETHROUGH);
        for (event, range) in parser.into_offset_iter() {
            match &event {
                Event::Text(_) | Event::SoftBreak | Event::HardBreak => {
                    let sc = line[..range.start].chars().count();
                    let ec = line[..range.end].chars().count();
                    for i in sc..ec {
                        if i < total {
                            visible[i] = true;
                        }
                    }
                }
                Event::Code(code_text) => {
                    // range includes backtick delimiters; the content is just the inner text.
                    // Find the content by scanning inward from range boundaries.
                    let range_sc = line[..range.start].chars().count();
                    let code_len = code_text.chars().count();
                    // The content occupies `code_len` chars; the rest are backtick delimiters.
                    // Opening backtick(s) are at the start of the range; content follows.
                    let range_char_len = line[range.start..range.end].chars().count();
                    let sigil_each = (range_char_len - code_len) / 2;
                    let content_start = range_sc + sigil_each;
                    let content_end = content_start + code_len;
                    for i in content_start..content_end {
                        if i < total {
                            visible[i] = true;
                        }
                    }
                }
                // For headings, the "# " prefix is not a Text event but we want to keep it
                // as sigil (handled separately in render via is_sigil logic).
                // The heading content text events will mark their chars as visible.
                _ => {}
            }
        }
        visible
    }

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

        let elements = Self::parse_elements(logical_line);
        let logical_chars: Vec<char> = logical_line.chars().collect();
        let visual_end_col = (visual_start_col + content.chars().count()).min(logical_chars.len());
        let content_vis = Self::content_positions(logical_line);

        // Innermost element index at a logical char position
        let elem_at = |pos: usize| -> Option<usize> {
            elements.iter().enumerate().rev()
                .find(|(_, e)| e.start_char <= pos && pos < e.end_char)
                .map(|(i, _)| i)
        };
        // Which element the cursor sits inside (for expand)
        let expanded: Option<usize> = cursor_col.and_then(|c| elem_at(c));

        // For headings, we keep the sigil ("# ", "## ", etc.) characters visible
        // but style them as muted. Determine if this line starts with a heading element.
        let heading_sigil_end: Option<usize> = if is_first_visual_line {
            elements.iter().find(|e| matches!(e.kind,
                ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3))
                .map(|e| {
                    // Find the first content char within the heading element
                    let mut first_content = e.start_char;
                    for i in e.start_char..e.end_char {
                        if i < content_vis.len() && content_vis[i] {
                            first_content = i;
                            break;
                        }
                    }
                    first_content
                })
        } else {
            None
        };

        // Walk visual region, emitting spans
        // For each char position in [visual_start_col, visual_end_col):
        // - if not expanded and not content and not heading sigil: skip (don't emit)
        // - else emit with appropriate style
        let mut spans: Vec<Span<'a>> = Vec::new();
        let mut seg_chars: Vec<char> = Vec::new();
        let mut seg_elem: Option<usize> = None;
        let mut seg_is_sigil = false;
        let mut seg_is_expanded = false;

        let flush = |seg_chars: &mut Vec<char>, seg_elem: Option<usize>,
                     seg_is_sigil: bool, seg_is_expanded: bool, spans: &mut Vec<Span<'a>>,
                     elements: &[Element], theme: &Theme| {
            if seg_chars.is_empty() { return; }
            let seg: String = seg_chars.drain(..).collect();
            let style = if seg_is_expanded {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                span_style(seg_elem.map(|i| elements[i].kind), seg_is_sigil, theme)
            };
            spans.push(Span::styled(seg, style));
        };

        for pos in visual_start_col..visual_end_col {
            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.map_or(false, |end| pos < end);
            let in_expanded_elem = expanded.map_or(false, |i| {
                elements[i].start_char <= pos && pos < elements[i].end_char
            });

            // Determine if this char should be emitted
            let emit = is_content || in_heading_sigil || in_expanded_elem;
            if !emit {
                // Flush current segment before skipping
                flush(&mut seg_chars, seg_elem, seg_is_sigil, seg_is_expanded, &mut spans, &elements, theme);
                seg_elem = None;
                seg_is_sigil = false;
                seg_is_expanded = false;
                continue;
            }

            let this_elem = elem_at(pos);
            let this_is_expanded = in_expanded_elem;
            let this_is_sigil = in_heading_sigil && !is_content && !in_expanded_elem;

            // If element or sigil status changes, flush
            if this_elem != seg_elem || this_is_sigil != seg_is_sigil || this_is_expanded != seg_is_expanded {
                flush(&mut seg_chars, seg_elem, seg_is_sigil, seg_is_expanded, &mut spans, &elements, theme);
                seg_elem = this_elem;
                seg_is_sigil = this_is_sigil;
                seg_is_expanded = this_is_expanded;
            }
            seg_chars.push(logical_chars[pos]);
        }
        // Flush remaining
        flush(&mut seg_chars, seg_elem, seg_is_sigil, seg_is_expanded, &mut spans, &elements, theme);

        if spans.is_empty() {
            spans.push(Span::styled(content, Style::default().fg(theme.fg.to_ratatui())));
        }
        spans
    }
}

fn span_style(kind: Option<ElementKind>, is_sigil_region: bool, theme: &Theme) -> Style {
    match kind {
        None => Style::default().fg(theme.fg.to_ratatui()),
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
}
