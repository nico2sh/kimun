//! The Query panel's note-preview: the expand state machine (Collapsed →
//! Context → Full), the content scroll (anchored vs user-owned), and the
//! content render. Lifted out of the panel so the scroll/anchor logic — the
//! subtle part — is testable on its own, without a vault, a `SearchList`, or a
//! `Frame`. The panel composes one of these and feeds it the selected note's
//! text + highlight needles; the panel keeps owning the list and the engine's
//! wheel-routing region (`set_content_rect`).

use std::ops::Range;

use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::components::preview_highlight;
use crate::settings::themes::Theme;

/// How much of the selected note the preview shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExpandState {
    /// List only, no preview.
    Collapsed,
    /// Half-height preview below the list; sticks across selection moves.
    Context,
    /// Preview takes the whole panel; the list is hidden.
    Full,
}

/// The Preview pane's one "what makes a line hot" seam. FIND highlights query
/// needles scattered anywhere in a line; Ask Sources highlights one contiguous
/// byte range (the retrieved section). Every other part of the render — chrome,
/// wrapping, indent, scroll anchoring — is a single body shared by both; this
/// enum is the only place that body branches on which kind of highlight is
/// active.
pub enum Highlight<'a> {
    /// FIND: a (wrapped) line is hot if it contains any of these needles,
    /// case-insensitively; each match renders bold within the line.
    Needles(&'a [String]),
    /// Ask Sources: a source line is hot if its byte range intersects this
    /// one; the whole line renders bold when it does. `None` when no section
    /// resolved (nothing is hot) — distinct from an empty needle list, which
    /// is expressed as `Needles(&[])`.
    Range(Option<&'a Range<usize>>),
}

/// Scroll state for the expanded content views (Full mode and the half-height
/// Context preview). The offset is either *anchored* — the Context render
/// recomputes it from the first needle match each frame — or user-owned after a
/// scroll. Every transition (take-over, re-anchor, clamp) lives here, so paths
/// that should re-anchor have one decision point and the offset is never out of
/// range between events.
#[derive(Clone, Copy)]
struct ContentScroll {
    /// True while the render owns the offset (anchor on the first needle
    /// match). The first tick that actually moves the view flips it;
    /// re-anchoring events set it back.
    anchored: bool,
    /// The rendered scroll offset (first visible content line).
    offset: usize,
    /// Maximum offset, recorded by render from content/viewport size.
    max: usize,
}

impl ContentScroll {
    fn new() -> Self {
        Self {
            anchored: true,
            offset: 0,
            max: 0,
        }
    }

    /// Back to the top, offset handed back to the auto-anchor.
    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Re-arm the auto-anchor without touching the offset (the next anchored
    /// render overwrites it).
    fn re_anchor(&mut self) {
        self.anchored = true;
    }

    /// One wheel/key tick up, clamped at the top. Only a tick that moves the
    /// view takes the offset over from the anchor — a saturated no-op must
    /// not silently disarm it.
    fn scroll_up(&mut self) {
        if self.offset > 0 {
            self.offset -= 1;
            self.anchored = false;
        }
    }

    /// One wheel/key tick down, clamped at `max` at mutation time so the
    /// offset is never out of range. Same no-op rule as [`scroll_up`].
    ///
    /// [`scroll_up`]: Self::scroll_up
    fn scroll_down(&mut self) {
        if self.offset < self.max {
            self.offset += 1;
            self.anchored = false;
        }
    }

    /// Render-time sync: record the current max offset and clamp — a resize
    /// can shrink the content below the held offset.
    fn set_max(&mut self, max: usize) {
        self.max = max;
        self.offset = self.offset.min(max);
    }

    /// Render-time anchor: while anchored, place the offset (clamped). A
    /// user-owned offset is left alone.
    fn anchor_to(&mut self, offset: usize) {
        if self.anchored {
            self.offset = offset.min(self.max);
        }
    }

    /// Anchor the view on the line holding the first needle match: if the
    /// content from the link to the end fits the viewport, scroll back to fill
    /// it; otherwise show two lines of context above the link. No-op unless
    /// anchored. `set_max` must run first (this clamps against `max`).
    fn anchor_to_link(&mut self, link_pos: usize, total: usize, viewport: usize) {
        let lines_after_link = total.saturating_sub(link_pos);
        let target = if lines_after_link <= viewport {
            self.max
        } else {
            link_pos.saturating_sub(2)
        };
        self.anchor_to(target);
    }
}

/// The note-preview surface beneath/over the Query panel's result list.
pub struct PreviewPane {
    expand: ExpandState,
    /// The path the expand state belongs to, so a selection change re-anchors.
    expand_path: Option<VaultPath>,
    scroll: ContentScroll,
    /// The full-expand header's screen area, recorded each render so a click on
    /// it collapses the view (mirroring Enter). Empty when full mode is off.
    full_header_rect: Rect,
}

impl Default for PreviewPane {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewPane {
    pub fn new() -> Self {
        Self {
            expand: ExpandState::Collapsed,
            expand_path: None,
            scroll: ContentScroll::new(),
            full_header_rect: Rect::default(),
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.expand == ExpandState::Collapsed
    }

    pub fn is_context(&self) -> bool {
        self.expand == ExpandState::Context
    }

    pub fn is_full(&self) -> bool {
        self.expand == ExpandState::Full
    }

    pub fn full_header_rect(&self) -> Rect {
        self.full_header_rect
    }

    /// Drop the recorded full-expand header rect (the previous frame's region).
    pub fn clear_header(&mut self) {
        self.full_header_rect = Rect::default();
    }

    /// Collapse to the list and re-arm the auto-anchor (programmatic resets:
    /// query change, sort, note change).
    pub fn reset(&mut self) {
        self.expand = ExpandState::Collapsed;
        self.expand_path = None;
        self.scroll.reset();
        self.full_header_rect = Rect::default();
    }

    /// Re-arm the auto-anchor without changing the expand state (a query edit
    /// moves the matches, so a user scroll position is stale).
    pub fn re_anchor(&mut self) {
        self.scroll.re_anchor();
    }

    /// Re-point the preview at `selected`, keeping the expand state and
    /// re-anchoring the scroll from the top — for a *directed* reveal (Ask's
    /// `open_reader`, or a same-note section change) that must land revealed on
    /// the requested source rather than collapse the way a plain selection move
    /// ([`sync`](Self::sync)) would. No-op without a selection.
    pub fn repoint(&mut self, selected: Option<VaultPath>) {
        if selected.is_none() {
            return;
        }
        self.expand_path = selected;
        self.scroll.reset();
        self.full_header_rect = Rect::default();
    }

    pub fn scroll_up(&mut self) {
        self.scroll.scroll_up();
    }

    pub fn scroll_down(&mut self) {
        self.scroll.scroll_down();
    }

    /// Re-anchor the expand state on the currently-selected row. The Context
    /// (half-height) preview sticks across selection moves: it stays open and
    /// re-anchors on the new row. Full collapses, and a vanished selection
    /// always collapses. Returns `true` when the state changed, so the caller
    /// drops the stale wheel-routing region.
    pub fn sync(&mut self, selected: Option<VaultPath>) -> bool {
        if selected == self.expand_path {
            return false;
        }
        if self.expand != ExpandState::Context || selected.is_none() {
            self.expand = ExpandState::Collapsed;
        }
        self.expand_path = selected;
        self.scroll.reset();
        self.full_header_rect = Rect::default();
        true
    }

    /// Cycle the selected row's preview: Collapsed → Context → Full →
    /// Collapsed. No-op without a selection.
    pub fn toggle(&mut self, selected: Option<VaultPath>) {
        if selected.is_none() {
            return;
        }
        self.expand_path = selected;
        match self.expand {
            ExpandState::Collapsed => {
                self.expand = ExpandState::Context;
                self.scroll.re_anchor();
            }
            ExpandState::Context => {
                self.scroll.reset();
                self.expand = ExpandState::Full;
            }
            ExpandState::Full => {
                self.scroll.reset();
                self.expand = ExpandState::Collapsed;
            }
        }
        self.full_header_rect = Rect::default();
    }

    /// Step the reveal cycle backward: Full → Context → Collapsed, stopping at
    /// Collapsed (the vim-natural `h` mirror of [`toggle`]). No-op without a
    /// selection, or already Collapsed.
    pub fn collapse_step(&mut self, selected: Option<VaultPath>) {
        if selected.is_none() {
            return;
        }
        self.expand_path = selected;
        match self.expand {
            ExpandState::Full => {
                // Back to the half-height preview, re-anchored on the row.
                self.scroll.reset();
                self.expand = ExpandState::Context;
            }
            ExpandState::Context => {
                self.scroll.reset();
                self.expand = ExpandState::Collapsed;
            }
            ExpandState::Collapsed => {}
        }
        self.full_header_rect = Rect::default();
    }

    /// Draw the full-expand chrome (fixed title header + divider), record the
    /// header rect for click-to-collapse, and return the scrollable content
    /// sub-rect. Used by [`render_full`].
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
        let find_hit = self.scroll.anchored;
        let (lines, hit) = build_lines(text, highlight, wrap_width, theme, find_hit, indent);
        let viewport = content.height as usize;
        let total = lines.len();
        self.scroll.set_max(total.saturating_sub(viewport));
        self.scroll.anchor_to_link(hit.unwrap_or(0), total, viewport);
        f.render_widget(
            Paragraph::new(lines)
                .scroll((self.scroll.offset as u16, 0))
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
        let find_hit = self.scroll.anchored;
        let (lines, hit) = build_lines(text, highlight, wrap_width, theme, find_hit, indent);
        let viewport = area.height as usize;
        let total = lines.len();
        self.scroll.set_max(total.saturating_sub(viewport));
        self.scroll.anchor_to_link(hit.unwrap_or(0), total, viewport);
        f.render_widget(
            Paragraph::new(lines)
                .scroll((self.scroll.offset as u16, 0))
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
        self.scroll.offset
    }
    pub fn is_anchored(&self) -> bool {
        self.scroll.anchored
    }
    pub fn scroll_max(&self) -> usize {
        self.scroll.max
    }
    /// Simulate a user-owned scroll without a viewport-sized content set.
    pub fn force_user_scrolled(&mut self) {
        self.scroll.anchored = false;
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
                    let ranges = preview_highlight::match_ranges(&wline, needles);
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
                let hit = range
                    .is_some_and(|h| line_range.start < h.end && h.start < line_range.end);
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

    fn path(name: &str) -> VaultPath {
        VaultPath::note_path_from(name)
    }

    fn needles(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── Expand state machine ──────────────────────────────────────────────

    #[test]
    fn toggle_cycles_collapsed_context_full() {
        let mut p = PreviewPane::new();
        let sel = || Some(path("a"));
        assert!(p.is_collapsed());
        p.toggle(sel());
        assert!(p.is_context());
        p.toggle(sel());
        assert!(p.is_full());
        p.toggle(sel());
        assert!(p.is_collapsed());
    }

    #[test]
    fn toggle_without_selection_is_noop() {
        let mut p = PreviewPane::new();
        p.toggle(None);
        assert!(p.is_collapsed());
    }

    #[test]
    fn sync_keeps_context_across_selection_change() {
        let mut p = PreviewPane::new();
        p.toggle(Some(path("a"))); // -> Context, anchored on "a"
        assert!(p.is_context());
        // Moving to another row: Context sticks, re-anchored on the new row.
        let changed = p.sync(Some(path("b")));
        assert!(changed, "selection change must clear the stale region");
        assert!(p.is_context());
    }

    #[test]
    fn sync_collapses_full_on_selection_change() {
        let mut p = PreviewPane::new();
        p.toggle(Some(path("a")));
        p.toggle(Some(path("a"))); // -> Full
        assert!(p.is_full());
        p.sync(Some(path("b")));
        assert!(p.is_collapsed(), "Full does not stick across rows");
    }

    #[test]
    fn sync_collapses_when_selection_vanishes() {
        let mut p = PreviewPane::new();
        p.toggle(Some(path("a"))); // Context
        p.sync(None);
        assert!(p.is_collapsed());
    }

    #[test]
    fn sync_same_selection_is_noop() {
        let mut p = PreviewPane::new();
        p.toggle(Some(path("a")));
        assert!(!p.sync(Some(path("a"))), "no change, no region clear");
    }

    #[test]
    fn repoint_keeps_full_and_rearms_the_anchor() {
        let mut p = PreviewPane::new();
        p.toggle(Some(path("a"))); // Context
        p.toggle(Some(path("a"))); // Full
        p.scroll_down();
        p.force_user_scrolled();
        assert!(p.is_full() && !p.is_anchored());
        // A directed re-point at another source stays Full and re-anchors.
        p.repoint(Some(path("b")));
        assert!(p.is_full(), "repoint keeps the expand state (unlike sync)");
        assert!(p.is_anchored(), "repoint re-arms the scroll anchor");
        assert_eq!(p.scroll_offset(), 0);
    }

    #[test]
    fn repoint_without_selection_is_noop() {
        let mut p = PreviewPane::new();
        p.toggle(Some(path("a")));
        p.repoint(None);
        assert!(p.is_context(), "no-selection repoint must not change state");
    }

    #[test]
    fn reset_collapses_and_rearms() {
        let mut p = PreviewPane::new();
        p.toggle(Some(path("a")));
        p.scroll_down();
        p.reset();
        assert!(p.is_collapsed());
        assert!(p.scroll.anchored && p.scroll.offset == 0);
    }

    // ── Scroll / anchor logic ─────────────────────────────────────────────

    #[test]
    fn scroll_clamps_and_takes_over_from_anchor() {
        let mut s = ContentScroll::new();
        s.set_max(3);
        assert!(s.anchored);
        s.scroll_up(); // already at top → no-op, stays anchored
        assert!(s.anchored && s.offset == 0);
        s.scroll_down();
        assert!(!s.anchored, "a real move disarms the anchor");
        assert_eq!(s.offset, 1);
        s.scroll_down();
        s.scroll_down();
        s.scroll_down(); // clamped at max
        assert_eq!(s.offset, 3);
    }

    #[test]
    fn anchor_to_link_fills_viewport_when_tail_fits() {
        let mut s = ContentScroll::new();
        // 10 lines, viewport 5 → max offset 5. Link near the end: tail fits, so
        // scroll back to max to fill the viewport.
        s.set_max(5);
        s.anchor_to_link(8, 10, 5);
        assert_eq!(s.offset, 5);
    }

    #[test]
    fn anchor_to_link_shows_two_lines_of_context_above() {
        let mut s = ContentScroll::new();
        // Link deep in long content, tail does NOT fit → show link_pos - 2.
        s.set_max(100);
        s.anchor_to_link(40, 200, 10);
        assert_eq!(s.offset, 38);
    }

    #[test]
    fn anchor_to_link_is_noop_once_user_scrolled() {
        let mut s = ContentScroll::new();
        s.set_max(100);
        s.scroll_down(); // user owns the offset now (offset 1, not anchored)
        s.anchor_to_link(40, 200, 10);
        assert_eq!(s.offset, 1, "user-owned offset is not re-anchored");
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
    fn collapse_step_steps_back_and_stops_at_collapsed() {
        let mut p = PreviewPane::new();
        let sel = || Some(path("a"));
        p.toggle(sel()); // Collapsed -> Context
        p.toggle(sel()); // Context -> Full
        assert!(p.is_full());
        p.collapse_step(sel()); // Full -> Context
        assert!(p.is_context());
        p.collapse_step(sel()); // Context -> Collapsed
        assert!(p.is_collapsed());
        p.collapse_step(sel()); // Collapsed stays Collapsed (h no-op at bottom)
        assert!(p.is_collapsed());
    }

    #[test]
    fn collapse_step_without_selection_is_noop() {
        let mut p = PreviewPane::new();
        p.toggle(Some(path("a")));
        p.collapse_step(None);
        assert!(p.is_context(), "no-selection collapse_step must not change state");
    }

    #[test]
    fn build_lines_range_highlights_and_anchors_the_section() {
        let theme = Theme::default();
        // "beta body" is the third source line; its byte range anchors row 2.
        let text = "line0\nline1\nbeta body\ntail\n";
        let start = text.find("beta body").unwrap();
        let range = start..start + "beta body".len();
        let (lines, first) = build_lines(text, Highlight::Range(Some(&range)), 80, &theme, true, 2);
        assert_eq!(first, Some(2), "anchor is the first highlighted wrapped row");
        // The highlighted content span carries the bold accent modifier; a
        // non-highlighted line does not.
        assert!(lines[2].spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert!(!lines[0].spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn build_lines_range_no_highlight_reports_no_anchor() {
        let theme = Theme::default();
        let (_lines, first) = build_lines("a\nb\nc\n", Highlight::Range(None), 80, &theme, true, 2);
        assert_eq!(first, None);
    }
}
