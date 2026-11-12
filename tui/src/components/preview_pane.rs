//! The Query panel's note-preview: the toolkit-free state (expand cycle and
//! content scroll) is [`PreviewModel`] in [`crate::logic::preview`]; this
//! wrapper adds what only a terminal render needs — the full-view header's
//! screen area (click-to-collapse) and the content render itself. The panel
//! composes one of these and feeds it the selected note's text + highlight
//! needles; the panel keeps owning the list and the engine's wheel-routing
//! region (`set_content_rect`).

use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::components::preview_highlight;
use crate::logic::preview::PreviewModel;
pub use crate::logic::preview::{ExpandState, Highlight};
use crate::settings::themes::Theme;

/// The note-preview surface beneath/over the Query panel's result list:
/// a [`PreviewModel`] plus the header region the terminal render records.
pub struct PreviewPane {
    model: PreviewModel,
    /// The full-expand header's screen area, recorded each render so a click on
    /// it collapses the view (mirroring Enter). Empty when full mode is off:
    /// cleared by every state change that re-points or re-expands the preview.
    full_header_rect: Rect,
    /// The shrink chevron in the full-expand header (its last column), recorded
    /// each render. A click on it steps the view back one state; empty with
    /// the header. Any host of a `PreviewPane` reads it the same way.
    full_chevron_rect: Rect,
}

impl Default for PreviewPane {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewPane {
    pub fn new() -> Self {
        Self {
            model: PreviewModel::new(),
            full_header_rect: Rect::default(),
            full_chevron_rect: Rect::default(),
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.model.is_collapsed()
    }

    pub fn is_context(&self) -> bool {
        self.model.is_context()
    }

    pub fn is_full(&self) -> bool {
        self.model.is_full()
    }

    pub fn full_header_rect(&self) -> Rect {
        self.full_header_rect
    }

    /// The header's shrink chevron cell, or an empty rect outside full mode.
    pub fn full_chevron_rect(&self) -> Rect {
        self.full_chevron_rect
    }

    /// Drop the recorded full-expand header rects (the previous frame's regions).
    pub fn clear_header(&mut self) {
        self.full_header_rect = Rect::default();
        self.full_chevron_rect = Rect::default();
    }

    /// See [`PreviewModel::reset`]; also drops the header region.
    pub fn reset(&mut self) {
        self.model.reset();
        self.clear_header();
    }

    /// See [`PreviewModel::re_anchor`].
    pub fn re_anchor(&mut self) {
        self.model.re_anchor();
    }

    /// See [`PreviewModel::repoint`]; drops the header region unless it was a
    /// no-op (no selection).
    pub fn repoint(&mut self, selected: Option<VaultPath>) {
        if selected.is_some() {
            self.clear_header();
        }
        self.model.repoint(selected);
    }

    pub fn scroll_up(&mut self) {
        self.model.scroll_up();
    }

    pub fn scroll_down(&mut self) {
        self.model.scroll_down();
    }

    /// See [`PreviewModel::sync`]; drops the header region when the state
    /// changed. Returns `true` then, so the caller drops the stale
    /// wheel-routing region.
    pub fn sync(&mut self, selected: Option<VaultPath>) -> bool {
        let changed = self.model.sync(selected);
        if changed {
            self.clear_header();
        }
        changed
    }

    /// See [`PreviewModel::toggle`]; drops the header region unless it was a
    /// no-op (no selection).
    pub fn toggle(&mut self, selected: Option<VaultPath>) {
        if selected.is_some() {
            self.clear_header();
        }
        self.model.toggle(selected);
    }

    /// Grow one step (Collapsed -> Context -> Full). Full stays Full, unlike
    /// [`Self::toggle`], which wraps. Returns `false` when nothing changed
    /// (no selection, or already Full).
    pub fn grow_step(&mut self, selected: Option<VaultPath>) -> bool {
        if selected.is_none() || self.is_full() {
            return false;
        }
        self.toggle(selected);
        true
    }

    /// See [`PreviewModel::collapse_step`]; drops the header region unless it
    /// was a no-op (no selection).
    pub fn collapse_step(&mut self, selected: Option<VaultPath>) {
        if selected.is_some() {
            self.clear_header();
        }
        self.model.collapse_step(selected);
    }

    /// Draw the full-expand chrome (fixed title header + divider), record the
    /// header rect for click-to-collapse, and return the scrollable content
    /// sub-rect. Used by [`Self::render_full`].
    fn render_full_chrome(
        &mut self,
        f: &mut Frame,
        inner: Rect,
        title: &str,
        filename: &str,
        theme: &Theme,
    ) -> Rect {
        let gray = theme.gray.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();
        let title_display = if title.is_empty() { filename } else { title };

        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // title
                Constraint::Length(1), // divider
                Constraint::Min(0),    // content
            ])
            .split(inner);

        // Fixed title header — clicking it collapses the view (mirroring Enter).
        self.full_header_rect = parts[0];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("\u{25BC} {} ", title_display),
                    Style::default()
                        .fg(theme.selection_fg.to_ratatui())
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {filename}"), Style::default().fg(gray).bg(bg)),
            ]))
            .style(Style::default().bg(bg)),
            parts[0],
        );
        // The shrink chevron closes the header; a click on it steps back one
        // state instead of collapsing like a click on the title does.
        self.full_chevron_rect = crate::components::expand_chevron::paint(
            f,
            parts[0],
            crate::components::expand_chevron::Reach::Shrink,
            theme,
        );

        // Fixed divider.
        f.render_widget(
            Paragraph::new("\u{2500}".repeat(parts[1].width as usize))
                .style(Style::default().fg(gray).bg(bg)),
            parts[1],
        );
        parts[2]
    }

    /// Render the full-screen preview (fixed title + divider, scrollable
    /// content) into `inner`. Records the header rect for click-to-collapse.
    /// Anchors the initial scroll to the first hot line (deliberate
    /// unification: FIND's needle preview now anchors like the Context
    /// preview and the Ask Sources range preview already did — previously
    /// this variant alone ignored the anchor and always opened at the top).
    #[allow(clippy::too_many_arguments)]
    pub fn render_full(
        &mut self,
        f: &mut Frame,
        inner: Rect,
        title: &str,
        filename: &str,
        text: &str,
        highlight: Highlight,
        theme: &Theme,
    ) {
        let bg = theme.bg_panel.to_ratatui();
        let content = self.render_full_chrome(f, inner, title, filename, theme);
        let indent = 2usize;
        let wrap_width = content.width.saturating_sub(indent as u16 + 1) as usize;
        let find_hit = self.model.wants_hit();
        let (lines, hit) = build_lines(text, highlight, wrap_width, theme, find_hit, indent);
        let viewport = content.height as usize;
        let offset = self.model.place(lines.len(), viewport, hit);
        f.render_widget(
            Paragraph::new(lines)
                .scroll((offset as u16, 0))
                .style(Style::default().bg(bg)),
            content,
        );
    }

    /// Render the half-height Context preview into `area`, scrolled so the
    /// first hot line shows with context above (while anchored).
    pub fn render_context(
        &mut self,
        f: &mut Frame,
        area: Rect,
        text: &str,
        highlight: Highlight,
        theme: &Theme,
    ) {
        let bg = theme.bg_panel.to_ratatui();
        let indent = 2usize;
        let wrap_width = area.width.saturating_sub(indent as u16 + 1) as usize;
        // The hit-line scan only matters while anchored (a user-owned scroll
        // never reads it), so skip the per-line work otherwise.
        let find_hit = self.model.wants_hit();
        let (lines, hit) = build_lines(text, highlight, wrap_width, theme, find_hit, indent);
        let viewport = area.height as usize;
        let offset = self.model.place(lines.len(), viewport, hit);
        f.render_widget(
            Paragraph::new(lines)
                .scroll((offset as u16, 0))
                .style(Style::default().bg(bg)),
            area,
        );
    }
}

#[cfg(test)]
impl PreviewPane {
    /// Test observers for the composing panel's integration tests, which assert
    /// the scroll/anchor state after a real render.
    pub fn scroll_offset(&self) -> usize {
        self.model.scroll().offset()
    }
    pub fn is_anchored(&self) -> bool {
        self.model.scroll().is_anchored()
    }
    pub fn scroll_max(&self) -> usize {
        self.model.scroll().max()
    }
    /// Simulate a user-owned scroll without a viewport-sized content set.
    pub fn force_user_scrolled(&mut self) {
        self.model.force_user_scrolled();
    }
}

/// Build the wrapped, highlighted, indented content lines. The per-line hit
/// test is the only place this branches on [`Highlight`]: needles test each
/// wrapped line's text for scattered matches (styling each match span bold);
/// a range tests the *source* line's byte span against the highlighted range
/// (styling the whole wrapped line bold when it overlaps). Byte offsets are
/// tracked over `split_inclusive('\n')` (line terminators kept) so a `Range`
/// highlight maps correctly onto the original text regardless of wrapping.
/// When `find_hit` is set, also reports the first wrapped-line index carrying
/// a hit (for the Context/Full anchor); skip the scan once user-scrolled.
fn build_lines(
    text: &str,
    highlight: Highlight,
    wrap_width: usize,
    theme: &Theme,
    find_hit: bool,
    indent: usize,
) -> (Vec<Line<'static>>, Option<usize>) {
    let bg = theme.bg_panel.to_ratatui();
    let normal = Style::default().fg(theme.gray.to_ratatui()).bg(bg);
    let bold = Style::default()
        .fg(theme.accent.to_ratatui())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let mut lines = Vec::new();
    let mut first_hit = None;
    let mut offset = 0usize;
    for raw in text.split_inclusive('\n') {
        let stripped = raw.strip_suffix('\n').unwrap_or(raw);
        // CRLF notes: drop the carriage return too, so it never reaches
        // wrapping or highlight matching as a phantom trailing char.
        let stripped = stripped.strip_suffix('\r').unwrap_or(stripped);
        let line_range = offset..offset + stripped.len();
        offset += raw.len();

        match highlight {
            Highlight::Needles(needles) => {
                for wline in preview_highlight::wrap_line(stripped, wrap_width) {
                    // One scan per wrapped line: the hit probe and the span
                    // styling share it.
                    let ranges = crate::logic::text_match::match_ranges(&wline, needles);
                    if find_hit && first_hit.is_none() && !ranges.is_empty() {
                        first_hit = Some(lines.len());
                    }
                    let mut indented =
                        vec![Span::styled(" ".repeat(indent), Style::default().bg(bg))];
                    indented.extend(preview_highlight::style_ranges(
                        &wline,
                        &ranges,
                        |s, hit| Span::styled(s.to_string(), if hit { bold } else { normal }),
                    ));
                    lines.push(Line::from(indented));
                }
            }
            Highlight::Range(range) => {
                let hit =
                    range.is_some_and(|h| line_range.start < h.end && h.start < line_range.end);
                let style = if hit { bold } else { normal };
                for wline in preview_highlight::wrap_line(stripped, wrap_width) {
                    if find_hit && hit && first_hit.is_none() {
                        first_hit = Some(lines.len());
                    }
                    lines.push(Line::from(vec![
                        Span::styled(" ".repeat(indent), Style::default().bg(bg)),
                        Span::styled(wline, style),
                    ]));
                }
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::default());
    }
    (lines, first_hit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn needles(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // The next two tests drive the SAME `build_lines` render path with each
    // `Highlight` variant, proving the single body serves both: one needle
    // scan (scattered matches within a line) and one range scan (a
    // contiguous byte span across lines).

    #[test]
    fn build_lines_needles_reports_first_match_line() {
        let theme = Theme::default();
        let text = "alpha\nbeta widget\ngamma";
        let ns = needles(&["widget"]);
        let (lines, hit) = build_lines(text, Highlight::Needles(&ns), 80, &theme, true, 2);
        assert_eq!(lines.len(), 3);
        assert_eq!(hit, Some(1), "the match is on the second line");
    }

    #[test]
    fn build_lines_range_highlights_and_anchors_the_section() {
        let theme = Theme::default();
        // "beta body" is the third source line; its byte range anchors row 2.
        let text = "line0\nline1\nbeta body\ntail\n";
        let start = text.find("beta body").unwrap();
        let range = start..start + "beta body".len();
        let (lines, first) = build_lines(text, Highlight::Range(Some(&range)), 80, &theme, true, 2);
        assert_eq!(
            first,
            Some(2),
            "anchor is the first highlighted wrapped row"
        );
        // The highlighted content span carries the bold accent modifier; a
        // non-highlighted line does not.
        assert!(
            lines[2].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            !lines[0].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn build_lines_range_no_highlight_reports_no_anchor() {
        let theme = Theme::default();
        let (_lines, first) = build_lines("a\nb\nc\n", Highlight::Range(None), 80, &theme, true, 2);
        assert_eq!(first, None);
    }
}
