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
        // Force-raw (inside fenced code block). Expand tabs to spaces (at the
        // editor's TAB_STOP) so the rendered width is deterministic — matching
        // `raw_display_width` (used to size the code box) and the non-force-raw
        // tab handling, instead of emitting a literal tab whose width the
        // terminal decides. The no-tab fast path borrows `content` (no alloc).
        if force_raw {
            let style = Style::default().fg(theme.fg_secondary.to_ratatui());
            if !content.contains('\t') {
                return vec![Span::styled(content, style)];
            }
            let mut expanded = String::with_capacity(content.len());
            let mut col = 0usize;
            for cluster in content.graphemes(true) {
                if cluster == "\t" {
                    let w = tab_width_at(col);
                    for _ in 0..w {
                        expanded.push(' ');
                    }
                    col += w;
                } else {
                    expanded.push_str(cluster);
                    col += cluster_display_width(cluster);
                }
            }
            return vec![Span::styled(expanded, style)];
        }

        // Blockquote gutter: when the cursor is off this line, draw a `│` bar
        // per nesting depth (in `blockquote_bar`) in place of the hidden `>`
        // markers, on EVERY visual row. When the cursor IS on the line the
        // markers are revealed raw instead (handled by the sigil path below).
        let bq_gutter: Option<Vec<Span<'a>>> = if cursor_col.is_none() {
            parsed.blockquote_depth().map(|d| {
                let style = Style::default().fg(theme.blockquote_bar.to_ratatui());
                vec![
                    Span::styled("│".repeat(d as usize), style),
                    Span::styled(" ".to_string(), style),
                ]
            })
        } else {
            None
        };

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
        let blockquote_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.blockquote_sigil_end()
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
            // Only reveal the raw `> ` markers when there is no gutter, i.e.
            // when the cursor is on this line.
            let in_blockquote_sigil =
                bq_gutter.is_none() && blockquote_sigil_end.is_some_and(|end| pos < end);
            let in_expanded_elem = expanded
                .is_some_and(|i| elements[i].start_char <= pos && pos < elements[i].end_char);
            let this_elem = parsed.elem_at(pos);
            let emit = is_content
                || in_heading_sigil
                || in_list_sigil
                || in_blockquote_sigil
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
            let this_is_sigil = (in_heading_sigil || in_list_sigil || in_blockquote_sigil)
                && !is_content
                && !in_expanded_elem;
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

        // Empty-content fallback. Skipped when a blockquote gutter will be
        // prepended, otherwise a bare `>` line would re-emit its hidden raw
        // marker on top of the gutter.
        if spans.is_empty() && bq_gutter.is_none() {
            spans.push(Span::styled(
                content,
                Style::default().fg(theme.fg.to_ratatui()),
            ));
        }
        // Prepend the blockquote bar gutter (cursor-off-line case). Placed after
        // the empty-fallback so a bare `>` line still gets its gutter without
        // panicking.
        if let Some(mut gutter) = bq_gutter {
            gutter.extend(spans);
            spans = gutter;
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
            // Tab-aware: code is rendered with tabs expanded to TAB_STOP, so the
            // rendered cursor column must sum expanded widths, not char counts.
            let mut rendered = 0usize;
            let mut char_pos = 0usize;
            for cluster in logical_line.graphemes(true) {
                if char_pos >= cursor_col {
                    break;
                }
                let pos = char_pos;
                char_pos += cluster.chars().count();
                if pos < visual_start_col {
                    continue;
                }
                rendered += if cluster == "\t" {
                    tab_width_at(rendered)
                } else {
                    cluster_display_width(cluster)
                };
            }
            return rendered;
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
        let blockquote_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.blockquote_sigil_end()
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
            let in_blockquote_sigil = blockquote_sigil_end.is_some_and(|s_end| pos < s_end);
            let in_expanded_elem = expanded
                .is_some_and(|i| elements[i].start_char <= pos && pos < elements[i].end_char);
            let in_any_element = parsed.in_any_element(pos);
            let visible = is_content
                || in_heading_sigil
                || in_list_sigil
                || in_blockquote_sigil
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
        // Reveal the blockquote marker only while the cursor is on this line;
        // otherwise it stays hidden and the view draws the `│` gutter instead.
        let blockquote_sigil_end: Option<usize> = if cursor_col.is_some() {
            parsed.blockquote_sigil_end()
        } else {
            None
        };

        (0..total)
            .map(|pos| {
                let is_content = pos < content_vis.len() && content_vis[pos];
                let in_heading_sigil = heading_sigil_end.is_some_and(|end| pos < end);
                let in_list_sigil = list_sigil_end.is_some_and(|end| pos < end);
                let in_blockquote_sigil = blockquote_sigil_end.is_some_and(|end| pos < end);
                let in_any_element = parsed.in_any_element(pos);
                let in_expanded = expanded.is_some_and(|i| {
                    parsed.elements[i].start_char <= pos && pos < parsed.elements[i].end_char
                });
                is_content
                    || in_heading_sigil
                    || in_list_sigil
                    || in_blockquote_sigil
                    || in_expanded
                    || !in_any_element
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
            // Tab-aware inverse of `rendered_cursor_col_with`'s force-raw branch:
            // walk expanded widths to find the logical char at `rendered_col`.
            let mut rendered = 0usize;
            let mut char_pos = 0usize;
            for cluster in logical_line.graphemes(true) {
                let pos = char_pos;
                if pos < visual_start_col {
                    char_pos += cluster.chars().count();
                    continue;
                }
                if rendered >= rendered_col {
                    return pos;
                }
                rendered += if cluster == "\t" {
                    tab_width_at(rendered)
                } else {
                    cluster_display_width(cluster)
                };
                char_pos += cluster.chars().count();
            }
            return char_pos;
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
        // Mirror `rendered_cursor_col_with`: on the first visual line a
        // blockquote's `> ` markers are revealed (visible) when the cursor is on
        // the row. On non-cursor rows the caller passes `visual_start_col` past
        // the markers (the gutter case), so this clause is inert there.
        let blockquote_sigil_end: Option<usize> = if is_first_visual_line {
            parsed.blockquote_sigil_end()
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
            let in_blockquote_sigil = blockquote_sigil_end.is_some_and(|end| pos < end);
            let in_any_element = parsed.in_any_element(pos);
            if is_content
                || in_heading_sigil
                || in_list_sigil
                || in_blockquote_sigil
                || !in_any_element
            {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_raw_expands_tabs_and_cursor_maps_round_trip() {
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        // "\tx" force-raw: tab at col 0 → TAB_STOP (4) spaces, then 'x' → 5 cols.
        let spans = MarkdownSpanner::render("\tx", "\tx", 0, None, true, true, 40, &theme);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "    x", "tab must expand to 4 spaces in force-raw");

        // Cursor after the tab (logical col 1) is at rendered col 4 (tab width).
        let rc = MarkdownSpanner::rendered_cursor_col("\tx", 0, 1, true, true);
        assert_eq!(rc, 4);
        // Cursor after 'x' (logical col 2) is at rendered col 5.
        let rc2 = MarkdownSpanner::rendered_cursor_col("\tx", 0, 2, true, true);
        assert_eq!(rc2, 5);

        // Inverse: rendered col 4 maps back to logical col 1 ('x'); col 0 → 0.
        assert_eq!(
            MarkdownSpanner::rendered_col_to_logical("\tx", 0, 4, true, true),
            1
        );
        assert_eq!(
            MarkdownSpanner::rendered_col_to_logical("\tx", 0, 0, true, true),
            0
        );
    }

    #[test]
    fn click_maps_over_revealed_blockquote_marker_on_cursor_row() {
        // Cursor-row blockquote (markers revealed, no gutter): rendered_col 0/1
        // map to the '>' and ' ' (logical 0/1), rendered_col 2 to 'h' (logical 2)
        // — not skipped as hidden.
        assert_eq!(
            MarkdownSpanner::rendered_col_to_logical("> hi", 0, 0, true, false),
            0
        );
        assert_eq!(
            MarkdownSpanner::rendered_col_to_logical("> hi", 0, 2, true, false),
            2
        );
    }

    #[test]
    fn blockquote_marker_visible_only_when_cursor_on_line() {
        // Cursor on the line → "> " revealed (both chars visible).
        let with_cursor = MarkdownSpanner::visible_positions("> hi", Some(2), false);
        assert_eq!(&with_cursor[0..2], &[true, true]);

        // Cursor off the line → "> " hidden (gutter draws the bar instead).
        let no_cursor = MarkdownSpanner::visible_positions("> hi", None, false);
        assert_eq!(&no_cursor[0..2], &[false, false]);
    }

    #[test]
    fn blockquote_marker_stays_visible_when_cursor_in_inner_element() {
        // Cursor (col 4) sits inside the bold span of "> **b**". elem_at resolves
        // to the Bold element (start_char=2, end_char=7), not the line-spanning
        // Blockquote, so only the new blockquote-sigil reveal keeps the "> "
        // marker (cols 0,1) visible.
        //
        // Parsed: Blockquote [0,7), Bold [2,7); blockquote_sigil_end = Some(4).
        // Without in_blockquote_sigil: cols 0,1 are in_any_element=true but
        // in_expanded=false → hidden. With it: pos < 4 → visible.
        let vis = MarkdownSpanner::visible_positions("> **b**", Some(4), false);
        assert_eq!(&vis[0..2], &[true, true]);
    }

    #[test]
    fn cursor_advances_over_blockquote_marker_on_its_line() {
        // Cursor just after "> " on a bare blockquote line. Rendered column must
        // be 2 (the "> " is revealed and visible on the cursor's own line), not 0.
        let col = MarkdownSpanner::rendered_cursor_col(
            "> ",  // logical line
            0,     // visual_start_col
            2,     // cursor_col (end of line)
            true,  // is_first_visual_line
            false, // force_raw
        );
        assert_eq!(col, 2);
    }

    #[test]
    fn blockquote_renders_bar_when_cursor_off_line() {
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        // cursor_col = None → bar gutter, raw "> " hidden.
        let spans = MarkdownSpanner::render("> hi", "> hi", 0, None, true, false, 40, &theme);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("│ "), "expected bar gutter, got {text:?}");
        assert!(
            !text.contains('>'),
            "raw marker must be hidden, got {text:?}"
        );
        assert!(text.contains("hi"));
    }

    #[test]
    fn blockquote_reveals_raw_marker_when_cursor_on_line() {
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        // cursor_col = Some(..) → raw "> hi" shown, no bar.
        let spans = MarkdownSpanner::render("> hi", "> hi", 0, Some(2), true, false, 40, &theme);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "> hi");
        assert!(!text.contains('│'));
    }

    #[test]
    fn nested_blockquote_renders_two_bars() {
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        let spans = MarkdownSpanner::render(">> x", ">> x", 0, None, true, false, 40, &theme);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("││ "), "expected two bars, got {text:?}");
    }

    #[test]
    fn bare_blockquote_renders_bar_gutter_without_panic() {
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        let spans = MarkdownSpanner::render(">", ">", 0, None, true, false, 40, &theme);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.starts_with("│ "), "expected bar gutter, got {text:?}");
        assert!(
            !text.contains('>'),
            "raw marker must be hidden, got {text:?}"
        );
    }
}
