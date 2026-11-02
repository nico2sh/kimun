//! Theme-applying painter over the style-free producer.
//!
//! `MarkdownSpanner::render_with` is the theme-applying painter over
//! [`render_runs`](crate::logic::editor::markdown::runs::render_runs), used by
//! tests and the four mapping shims (cursor-col, click-col and friends). The
//! editor itself paints through `view.rs` (`run_style`), which also handles
//! code boxes and overlays.

use super::super::markdown_style::{mask_to_modifier, role_style};
use crate::logic::editor::markdown::ParsedLine;
use crate::logic::editor::markdown::runs::render_runs;
use crate::settings::themes::Theme;
use ratatui::text::Span;

#[cfg(test)]
use crate::logic::editor::markdown::{
    Element, ElementKind, PARSER_OPTIONS, detect::detect_wikilinks, tag_to_kind,
};
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
        render_runs(
            content,
            logical_line,
            parsed,
            visual_start_col,
            cursor_col,
            is_first_visual_line,
            force_raw,
            available_width,
        )
        .into_iter()
        .map(|r| {
            Span::styled(
                r.text,
                role_style(r.role, theme).add_modifier(mask_to_modifier(r.mods)),
            )
        })
        .collect()
    }

    /// Delegates to [`crate::logic::editor::markdown::reveal::rendered_cursor_col_with`].
    pub fn rendered_cursor_col_with(
        logical_line: &str,
        parsed: &ParsedLine,
        visual_start_col: usize,
        cursor_col: usize,
        is_first_visual_line: bool,
        force_raw: bool,
    ) -> usize {
        crate::logic::editor::markdown::reveal::rendered_cursor_col_with(
            logical_line,
            parsed,
            visual_start_col,
            cursor_col,
            is_first_visual_line,
            force_raw,
        )
    }

    /// Delegates to [`crate::logic::editor::markdown::reveal::rendered_col_with_reveal`].
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
        crate::logic::editor::markdown::reveal::rendered_col_with_reveal(
            logical_line,
            parsed,
            visual_start_col,
            cursor_col,
            reveal_col,
            is_first_visual_line,
            force_raw,
        )
    }

    /// Delegates to [`crate::logic::editor::markdown::reveal::visible_positions_with`].
    pub fn visible_positions_with(
        logical_line: &str,
        parsed: &ParsedLine,
        cursor_col: Option<usize>,
        force_raw: bool,
    ) -> Vec<bool> {
        crate::logic::editor::markdown::reveal::visible_positions_with(
            logical_line,
            parsed,
            cursor_col,
            force_raw,
        )
    }

    /// Delegates to [`crate::logic::editor::markdown::reveal::rendered_col_to_logical_with`].
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
        crate::logic::editor::markdown::reveal::rendered_col_to_logical_with(
            logical_line,
            parsed,
            visual_start_col,
            rendered_col,
            reveal_col,
            is_first_visual_line,
            force_raw,
        )
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
