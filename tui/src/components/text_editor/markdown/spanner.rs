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

use super::{
    ElementKind, ParsedLine, blockquote_gutter, cluster_display_width, cluster_width_at,
    mask_to_modifier, span_style, tab_width_at,
};
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
            None,
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
                    Style::default().fg(theme.gray.to_ratatui()),
                )];
            }
            return vec![Span::styled(
                "─".repeat(available_width as usize),
                Style::default().fg(theme.gray.to_ratatui()),
            )];
        }
        // Force-raw (inside fenced code block). Expand tabs to spaces (at the
        // editor's TAB_STOP) so the rendered width is deterministic — matching
        // `raw_display_width` (used to size the code box) and the non-force-raw
        // tab handling, instead of emitting a literal tab whose width the
        // terminal decides. The no-tab fast path borrows `content` (no alloc).
        if force_raw {
            // Use `fg` (the primary text color) so fenced code text matches
            // indented code, which renders through the plain-text path (`fg`).
            let style = Style::default().fg(theme.fg.to_ratatui());
            if !content.contains('\t') {
                return vec![Span::styled(content, style)];
            }
            let mut expanded = String::with_capacity(content.len());
            let mut col = 0usize;
            for cluster in content.graphemes(true) {
                let w = cluster_width_at(cluster, col);
                if cluster == "\t" {
                    for _ in 0..w {
                        expanded.push(' ');
                    }
                } else {
                    expanded.push_str(cluster);
                }
                col += w;
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
                vec![Span::styled(blockquote_gutter(d), style)]
            })
        } else {
            None
        };

        let elements = &parsed.elements;
        let content_vis = &parsed.content_vis;
        let content_char_count = content.chars().count();

        let expanded: Option<usize> = cursor_col.and_then(|c| parsed.elem_at(c));
        // The caret's row is never left invisible: see `row_reveals_whole`.
        let reveals_whole_row =
            Self::row_reveals_whole(logical_line, parsed, cursor_col, force_raw);

        // Ungated on the visual row, matching `visible_positions_with` — which
        // is the wrap mask, and so decides how many cells each row reserves. A
        // sigil region can outrun the pane (a setext underline, a heading whose
        // `#` run is the whole row), and the mask reserves those cells on the
        // continuation row. Gating here left that row with no spans at all.
        // Inert for an ordinary `# Title` / `- item`, whose `visual_start_col`
        // is already past the sigil on every row but the first.
        let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
        let list_sigil_end: Option<usize> = parsed.list_sigil_end();
        // Ungated too, and for the same reason — the reveal window is not the
        // `>` run: `blockquote_sigil_end` is the element's first content char,
        // which is the *whole row* when the quote holds no `Event::Text` (an
        // HTML block inside a quote), so it outruns the pane readily. The
        // cursor-vs-gutter distinction that does the real work here is
        // `bq_gutter.is_none()` at the emit site below, which makes this
        // predicate identical to the mask's `cursor_col.is_some()` gate.
        let blockquote_sigil_end: Option<usize> = parsed.blockquote_sigil_end();

        let mut spans: Vec<Span<'a>> = Vec::new();
        let mut seg_str: String = String::new();
        let mut seg_elem: Option<usize> = None;
        let mut seg_is_sigil = false;
        let mut seg_is_expanded = false;
        let mut seg_mods: u8 = 0;
        // Tracks the current rendered visual column for tab-stop calculation.
        let mut visual_col = 0usize;

        let flush = |seg_str: &mut String,
                     seg_elem: Option<usize>,
                     seg_is_sigil: bool,
                     seg_is_expanded: bool,
                     seg_mods: u8,
                     spans: &mut Vec<Span<'a>>| {
            if seg_str.is_empty() {
                return;
            }
            let seg = std::mem::take(seg_str);
            let style = if seg_is_expanded {
                Style::default().fg(theme.gray.to_ratatui())
            } else {
                // OR in emphasis modifiers from outer elements so a Link /
                // WikiLink nested in `**…**` / `*…*` keeps the bold/italic the
                // innermost-element style alone would drop.
                span_style(seg_elem.map(|i| elements[i].kind), seg_is_sigil, theme)
                    .add_modifier(mask_to_modifier(seg_mods))
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
                        seg_mods,
                        &mut spans,
                    );
                    let style = span_style(Some(ElementKind::Image), false, theme);
                    visual_col += img.placeholder_width;
                    spans.push(Span::styled(img.placeholder.as_str(), style));
                    seg_elem = None;
                    seg_is_sigil = false;
                    seg_is_expanded = false;
                    seg_mods = 0;
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
                || reveals_whole_row
                || this_elem.is_none();
            if !emit {
                flush(
                    &mut seg_str,
                    seg_elem,
                    seg_is_sigil,
                    seg_is_expanded,
                    seg_mods,
                    &mut spans,
                );
                seg_elem = None;
                seg_is_sigil = false;
                seg_is_expanded = false;
                seg_mods = 0;
                continue;
            }
            // A row revealing in full is styled as an expanded element would
            // be — muted, so it reads as raw source under the caret rather than
            // as a live link the reader could follow.
            let this_is_expanded = in_expanded_elem || reveals_whole_row;
            let this_is_sigil = (in_heading_sigil || in_list_sigil || in_blockquote_sigil)
                && !is_content
                && !this_is_expanded;
            let this_mods = parsed.modifiers_at(pos);
            if this_elem != seg_elem
                || this_is_sigil != seg_is_sigil
                || this_is_expanded != seg_is_expanded
                || this_mods != seg_mods
            {
                flush(
                    &mut seg_str,
                    seg_elem,
                    seg_is_sigil,
                    seg_is_expanded,
                    seg_mods,
                    &mut spans,
                );
                seg_elem = this_elem;
                seg_is_sigil = this_is_sigil;
                seg_is_expanded = this_is_expanded;
                seg_mods = this_mods;
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
            seg_mods,
            &mut spans,
        );

        // Prepend the blockquote bar gutter (cursor-off-line case).
        if let Some(mut gutter) = bq_gutter {
            gutter.extend(spans);
            spans = gutter;
        }
        spans
    }

    /// Rendered screen column for `cursor_col`, treating that same column as
    /// the caret — so the markdown element it lands in counts as revealed.
    ///
    /// Correct for mapping the real caret. To map an arbitrary column (a
    /// selection edge, a **replace preview** span boundary) use
    /// [`Self::rendered_col_with_reveal`] and pass the caret separately:
    /// otherwise the mapper reveals whatever element the *boundary* touches,
    /// counts that element's hidden sigils as drawn, and the highlight lands
    /// right of the text it belongs to.
    pub fn rendered_cursor_col_with(
        logical_line: &str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        cursor_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        Self::rendered_col_with_reveal(
            logical_line,
            parsed,
            visual_start_col,
            cursor_col,
            Some(cursor_col),
            is_first_visual_line,
            force_raw,
        )
    }

    /// Rendered screen column for `target_col`, with element reveal driven by
    /// `reveal_col` — the row's real caret column, or `None` when the caret is
    /// on another row.
    ///
    /// The split matters because `render_with` reveals only the element under
    /// the *caret*. Any mapping that assumed the measured column was the caret
    /// would disagree with what was actually drawn.
    #[allow(clippy::too_many_arguments)]
    pub fn rendered_col_with_reveal(
        logical_line: &str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        cursor_col: usize,
        reveal_col: Option<usize>,
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
                rendered += cluster_width_at(cluster, rendered);
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

        // Reveal follows the caret, never the column being measured.
        let expanded: Option<usize> = reveal_col.and_then(|c| parsed.elem_at(c));
        // Must match `render_with`, or the caret draws in the wrong cell on a
        // row that reveals in full.
        let reveals_whole_row =
            Self::row_reveals_whole(logical_line, parsed, reveal_col, force_raw);
        // Ungated on the visual row, as in `render_with`: a sigil region that
        // outruns the pane draws on its continuation row, so a column there
        // must measure it rather than collapsing to zero.
        let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
        let list_sigil_end: Option<usize> = parsed.list_sigil_end();
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
                || reveals_whole_row
                || !in_any_element;
            if visible {
                rendered_col += cluster_width_at(cluster, rendered_col);
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
        let mut visible = Self::visible_positions_raw(logical_line, parsed, cursor_col, force_raw);
        if cursor_col.is_some() && Self::draws_nothing(&visible) {
            visible.iter_mut().for_each(|v| *v = true);
        }
        visible
    }

    /// Whether a row's visibility mask would leave it entirely unpainted.
    fn draws_nothing(visible: &[bool]) -> bool {
        !visible.is_empty() && visible.iter().all(|v| !v)
    }

    /// Whether the caret's own row reveals in full rather than element-wise.
    ///
    /// **Reveal** is scoped to the element under the caret, and at end of line
    /// there is no element under the caret — `elem_at` is half-open — so a row
    /// that is *entirely* concealed markdown (`[](url)`, `**<br/>**`) reveals
    /// nothing and draws nothing, while the caret sits on it. A row the caret
    /// is on must never be invisible, so the whole row reveals instead.
    ///
    /// A fact about the logical row, not about a visual slice of it, so both
    /// the wrap mask and the renderer derive it from the same predicate and
    /// cannot disagree. The empty-content fallback this replaces was keyed on
    /// the *slice* — which is decided by the wrap that the mask feeds, and so
    /// could never be stated as a hint.
    fn row_reveals_whole(
        logical_line: &str,
        parsed: &ParsedLine,
        cursor_col: Option<usize>,
        force_raw: bool,
    ) -> bool {
        cursor_col.is_some()
            && Self::draws_nothing(&Self::visible_positions_raw(
                logical_line,
                parsed,
                cursor_col,
                force_raw,
            ))
    }

    fn visible_positions_raw(
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

    /// Logical column for the cell at `rendered_col`, inverse of
    /// [`Self::rendered_col_with_reveal`].
    ///
    /// `reveal_col` is the row's real caret column, or `None` when the caret is
    /// on another row — the same value the render loop passes as `cursor_col`.
    /// It has to be threaded here because `render_with` reveals the element
    /// under the caret, and a revealed element's sigils occupy cells: measuring
    /// the row as if nothing were revealed put every column past the first
    /// revealed sigil out by the width of what was wrongly skipped.
    #[allow(clippy::too_many_arguments)]
    pub fn rendered_col_to_logical_with(
        logical_line: &str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        rendered_col: usize,
        reveal_col: Option<usize>,
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
                rendered += cluster_width_at(cluster, rendered);
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
        // Ungated on the visual row, as in `render_with`: the inverse mapping
        // has to land inside a sigil region that outran the pane, not past it.
        let heading_sigil_end: Option<usize> = parsed.heading_sigil_end();
        let list_sigil_end: Option<usize> = parsed.list_sigil_end();
        // Reveal, exactly as `render_with` applies it.
        let expanded: Option<usize> = reveal_col.and_then(|c| parsed.elem_at(c));
        let reveals_whole_row =
            Self::row_reveals_whole(logical_line, parsed, reveal_col, force_raw);
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
            let in_expanded_elem = expanded.is_some_and(|i| {
                parsed.elements[i].start_char <= pos && pos < parsed.elements[i].end_char
            });
            let in_any_element = parsed.in_any_element(pos);
            let drawn = is_content
                || in_heading_sigil
                || in_list_sigil
                || in_blockquote_sigil
                || in_expanded_elem
                || reveals_whole_row
                || !in_any_element;
            // An undrawn column belongs to the drawn column that follows it, so
            // the cell resolves past a concealed run rather than to its head.
            // `ropetext::Layout::position_at_cell` steps the same way, and this
            // is what keeps `position_at_cell ∘ cell_of` monotone — the reason
            // to prefer it over "the first position at this cell boundary",
            // which lands the caret inside the markup that was concealed.
            if drawn {
                if rendered_count >= rendered_col {
                    return pos;
                }
                rendered_count += cluster_width_at(cluster, rendered_count);
            }
        }
        logical_char_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mapping an arbitrary column must not reveal the element that column
    /// happens to land in — only the caret reveals.
    ///
    /// A wikilink renders with its `[[` `]]` hidden, so `[[note]] x` draws as
    /// `note x`. Asking for the rendered column of a boundary *inside* the
    /// link used to answer as though the link were expanded, counting the four
    /// hidden sigil chars as drawn, and every highlight anchored there landed
    /// four cells right of its text (the replace preview, and selections whose
    /// edge fell inside a link).
    #[test]
    fn mapping_a_column_inside_a_link_does_not_reveal_it() {
        let line = "[[note]] x";
        let parsed = ParsedLine::parse(line);

        // Caret elsewhere (or absent): the link stays collapsed, so logical
        // col 2 — the "n" of "note" — is rendered col 0.
        let collapsed =
            MarkdownSpanner::rendered_col_with_reveal(line, &parsed, 0, 2, None, true, false);
        assert_eq!(
            collapsed, 0,
            "with the caret away, the hidden `[[` occupies no screen columns"
        );

        // Caret inside the link: it is revealed raw, so the same logical
        // column now really is two cells in.
        let revealed =
            MarkdownSpanner::rendered_col_with_reveal(line, &parsed, 0, 2, Some(2), true, false);
        assert_eq!(revealed, 2, "the caret's own element renders raw");

        // The legacy entry point maps the caret, so it must keep agreeing with
        // the revealed case — this is the behaviour every existing caller has.
        assert_eq!(
            MarkdownSpanner::rendered_cursor_col_with(line, &parsed, 0, 2, true, false),
            revealed
        );
    }

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
