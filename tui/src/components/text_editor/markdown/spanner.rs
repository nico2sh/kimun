//! Span rendering for a single visual line.
//!
//! Given a logical line (the source markdown), a pre-parsed
//! [`ParsedLine`] (sigils, elements, image placeholders), and the
//! visual slice the editor wants to render, [`MarkdownSpanner`]
//! emits a vector of styled ratatui [`Span`]s with the right
//! fg/bg/modifier per element kind and the right
//! sigil-collapse/cursor-expand UX. Also exposes inverse mappings
//! (cursor-col, click-col → logical char index) used by the editor
//! to keep the cursor in sync after wrapping.

use super::{ElementKind, ParsedLine, cluster_display_width, span_style, tab_width_at};
use crate::settings::themes::Theme;
use ratatui::style::Style;
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
use super::{Element, PARSER_OPTIONS, detect::detect_wikilinks, tag_to_kind};
#[cfg(test)]
use pulldown_cmark::{Event, Parser, TagEnd};

pub struct MarkdownSpanner;

impl MarkdownSpanner {
    #[cfg(test)]
    pub fn parse_elements(line: &str) -> Vec<Element> {
        let parser = Parser::new_ext(line, PARSER_OPTIONS);
        let mut elements = Vec::new();
        let mut stack: Vec<(usize, ElementKind)> = Vec::new();
        for (event, range) in parser.into_offset_iter() {
            let sc = line[..range.start].chars().count();
            let ec = line[..range.end].chars().count();
            match event {
                Event::Start(ref tag) if let Some(kind) = tag_to_kind(tag) => {
                    stack.push((sc, kind));
                }
                Event::End(
                    TagEnd::Strong
                    | TagEnd::Emphasis
                    | TagEnd::Strikethrough
                    | TagEnd::Link
                    | TagEnd::Heading(_)
                    | TagEnd::BlockQuote(_),
                ) => {
                    if let Some((s, k)) = stack.pop() {
                        elements.push(Element {
                            start_char: s,
                            end_char: ec,
                            kind: k,
                        });
                    }
                }
                Event::Code(_) => elements.push(Element {
                    start_char: sc,
                    end_char: ec,
                    kind: ElementKind::InlineCode,
                }),
                _ => {}
            }
        }
        let mut dummy_vis = vec![true; line.chars().count()];
        detect_wikilinks(line, &mut dummy_vis, &mut elements);
        elements
    }

    // ── Public API (parse-on-the-fly wrappers, used in tests only) ───────────

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        content: &str,
        logical_line: &str,
        visual_start_col: usize,
        cursor_col: Option<usize>,
        is_first_visual_line: bool,
        force_raw: bool,
        available_width: u16,
        theme: &Theme,
    ) -> Vec<Span<'static>> {
        let parsed = ParsedLine::parse(logical_line);
        Self::render_with(
            content,
            logical_line,
            &parsed,
            visual_start_col,
            cursor_col,
            is_first_visual_line,
            force_raw,
            available_width,
            theme,
        )
        .into_iter()
        .map(|s| Span::styled(s.content.into_owned(), s.style))
        .collect()
    }

    #[cfg(test)]
    pub fn rendered_cursor_col(
        logical_line: &str,
        visual_start_col: usize,
        cursor_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        let parsed = ParsedLine::parse(logical_line);
        Self::rendered_cursor_col_with(
            logical_line,
            &parsed,
            visual_start_col,
            cursor_col,
            is_first_visual_line,
            force_raw,
        )
    }

    #[cfg(test)]
    pub fn visible_positions(
        logical_line: &str,
        cursor_col: Option<usize>,
        force_raw: bool,
    ) -> Vec<bool> {
        let parsed = ParsedLine::parse(logical_line);
        Self::visible_positions_with(logical_line, &parsed, cursor_col, force_raw)
    }

    #[cfg(test)]
    pub fn rendered_col_to_logical(
        logical_line: &str,
        visual_start_col: usize,
        rendered_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        let parsed = ParsedLine::parse(logical_line);
        Self::rendered_col_to_logical_with(
            logical_line,
            &parsed,
            visual_start_col,
            rendered_col,
            is_first_visual_line,
            force_raw,
        )
    }

    // ── `_with` variants: accept pre-parsed `&ParsedLine` ────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn render_with<'a>(
        content: &'a str,
        logical_line: &'a str,
        parsed: &'a ParsedLine,
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
                return vec![Span::styled(
                    content,
                    Style::default().fg(theme.fg_muted.to_ratatui()),
                )];
            }
            return vec![Span::styled(
                "─".repeat(available_width as usize),
                Style::default().fg(theme.fg_muted.to_ratatui()),
            )];
        }
        // Force-raw (inside fenced code block)
        if force_raw {
            return vec![Span::styled(
                content,
                Style::default().fg(theme.fg_secondary.to_ratatui()),
            )];
        }

        let elements = &parsed.elements;
        let content_vis = &parsed.content_vis;
        let content_char_count = content.chars().count();

        let expanded: Option<usize> = cursor_col.and_then(|c| parsed.elem_at(c));

        let heading_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.heading_sigil_end()
        } else {
            None
        };
        let list_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.list_sigil_end()
        } else {
            None
        };

        let mut spans: Vec<Span<'a>> = Vec::new();
        let mut seg_str: String = String::new();
        let mut seg_elem: Option<usize> = None;
        let mut seg_is_sigil = false;
        let mut seg_is_expanded = false;
        // Tracks the current rendered visual column for tab-stop calculation.
        let mut visual_col = 0usize;

        let flush = |seg_str: &mut String,
                     seg_elem: Option<usize>,
                     seg_is_sigil: bool,
                     seg_is_expanded: bool,
                     spans: &mut Vec<Span<'a>>| {
            if seg_str.is_empty() {
                return;
            }
            let seg = std::mem::take(seg_str);
            let style = if seg_is_expanded {
                Style::default().fg(theme.fg_muted.to_ratatui())
            } else {
                span_style(seg_elem.map(|i| elements[i].kind), seg_is_sigil, theme)
            };
            spans.push(Span::styled(seg, style));
        };

        // Iterate the visual-line slice rather than walking the whole logical
        // line and skipping clusters before `visual_start_col`. For a paragraph
        // wrapped across N visual rows this used to scan the full logical line
        // N times per frame; now each row's iteration is bounded to its own
        // slice. `char_pos` is seeded with `visual_start_col` so positions
        // continue to index into `content_vis`, `elements`, and the image
        // placeholders, which are all addressed in logical-line coordinates.
        let mut char_pos = visual_start_col;
        let visual_end_col = visual_start_col + content_char_count;
        for cluster in content.graphemes(true) {
            let pos = char_pos;
            char_pos += cluster.chars().count();
            if pos >= visual_end_col {
                break;
            }

            // Image placeholder: at the start of an `![..](..)` range, emit a
            // single styled placeholder span and let the existing emit logic
            // skip the underlying chars (they have content_vis=false). When the
            // cursor sits inside the image element we fall through and render
            // the raw markdown instead, matching the "expanded element" UX.
            if let Some(img) = parsed
                .image_placeholders
                .iter()
                .find(|p| p.start_char == pos)
            {
                let cursor_in_image = expanded.is_some_and(|i| {
                    elements[i].start_char == img.start_char && elements[i].end_char == img.end_char
                });
                if !cursor_in_image {
                    flush(
                        &mut seg_str,
                        seg_elem,
                        seg_is_sigil,
                        seg_is_expanded,
                        &mut spans,
                    );
                    let style = span_style(Some(ElementKind::Image), false, theme);
                    visual_col += img.placeholder_width;
                    spans.push(Span::styled(img.placeholder.as_str(), style));
                    seg_elem = None;
                    seg_is_sigil = false;
                    seg_is_expanded = false;
                }
            }

            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
            let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
            let in_expanded_elem = expanded
                .is_some_and(|i| elements[i].start_char <= pos && pos < elements[i].end_char);
            let this_elem = parsed.elem_at(pos);
            let emit = is_content
                || in_heading_sigil
                || in_list_sigil
                || in_expanded_elem
                || this_elem.is_none();
            if !emit {
                flush(
                    &mut seg_str,
                    seg_elem,
                    seg_is_sigil,
                    seg_is_expanded,
                    &mut spans,
                );
                seg_elem = None;
                seg_is_sigil = false;
                seg_is_expanded = false;
                continue;
            }
            let this_is_expanded = in_expanded_elem;
            let this_is_sigil =
                (in_heading_sigil || in_list_sigil) && !is_content && !in_expanded_elem;
            if this_elem != seg_elem
                || this_is_sigil != seg_is_sigil
                || this_is_expanded != seg_is_expanded
            {
                flush(
                    &mut seg_str,
                    seg_elem,
                    seg_is_sigil,
                    seg_is_expanded,
                    &mut spans,
                );
                seg_elem = this_elem;
                seg_is_sigil = this_is_sigil;
                seg_is_expanded = this_is_expanded;
            }
            if cluster == "\t" {
                let tw = tab_width_at(visual_col);
                for _ in 0..tw {
                    seg_str.push(' ');
                }
                visual_col += tw;
            } else {
                seg_str.push_str(cluster);
                visual_col += cluster_display_width(cluster);
            }
        }
        flush(
            &mut seg_str,
            seg_elem,
            seg_is_sigil,
            seg_is_expanded,
            &mut spans,
        );

        if spans.is_empty() {
            spans.push(Span::styled(
                content,
                Style::default().fg(theme.fg.to_ratatui()),
            ));
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
            parsed.list_sigil_end()
        } else {
            None
        };

        let end = cursor_col.min(logical_char_count);
        let mut rendered_col = 0usize;
        let mut char_pos = 0usize;
        for cluster in logical_line.graphemes(true) {
            if char_pos >= end {
                break;
            }
            let pos = char_pos;
            char_pos += cluster.chars().count();
            if pos < visual_start_col {
                continue;
            }

            // Account for placeholder width when crossing the start of an image
            // span — kept consistent with `render_with`'s placeholder injection.
            if let Some(img) = parsed
                .image_placeholders
                .iter()
                .find(|p| p.start_char == pos)
            {
                let cursor_in_image = expanded.is_some_and(|i| {
                    elements[i].start_char == img.start_char && elements[i].end_char == img.end_char
                });
                if !cursor_in_image {
                    rendered_col += img.placeholder_width;
                }
            }

            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.is_some_and(|s_end| pos < s_end);
            let in_list_sigil = list_sigil_end.is_some_and(|s_end| pos < s_end);
            let in_expanded_elem = expanded
                .is_some_and(|i| elements[i].start_char <= pos && pos < elements[i].end_char);
            let in_any_element = parsed.in_any_element(pos);
            let visible = is_content
                || in_heading_sigil
                || in_list_sigil
                || in_expanded_elem
                || !in_any_element;
            if visible {
                rendered_col += if cluster == "\t" {
                    tab_width_at(rendered_col)
                } else {
                    cluster_display_width(cluster)
                };
            }
        }
        rendered_col
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
        let expanded: Option<usize> = cursor_col.and_then(|c| parsed.elem_at(c));
        let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
        let list_sigil_end = parsed.list_sigil_end();

        (0..total)
            .map(|pos| {
                let is_content = pos < content_vis.len() && content_vis[pos];
                let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
                let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
                let in_any_element = parsed.in_any_element(pos);
                let in_expanded = expanded.is_some_and(|i| {
                    parsed.elements[i].start_char <= pos && pos < parsed.elements[i].end_char
                });
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
            parsed.list_sigil_end()
        } else {
            None
        };

        let mut rendered_count = 0;
        let mut char_pos = 0usize;
        for cluster in logical_line.graphemes(true) {
            let pos = char_pos;
            char_pos += cluster.chars().count();
            if pos < visual_start_col {
                continue;
            }

            if rendered_count >= rendered_col {
                return pos;
            }
            // A click landing inside the placeholder region maps back to the
            // start of the image span (the only logical position that visually
            // corresponds to the placeholder).
            if let Some(img) = parsed
                .image_placeholders
                .iter()
                .find(|p| p.start_char == pos)
            {
                if rendered_count + img.placeholder_width > rendered_col {
                    return pos;
                }
                rendered_count += img.placeholder_width;
            }
            let is_content = pos < content_vis.len() && content_vis[pos];
            let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
            let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
            let in_any_element = parsed.in_any_element(pos);
            if is_content || in_heading_sigil || in_list_sigil || !in_any_element {
                rendered_count += if cluster == "\t" {
                    tab_width_at(rendered_count)
                } else {
                    cluster_display_width(cluster)
                };
            }
        }
        logical_char_count
    }
}
