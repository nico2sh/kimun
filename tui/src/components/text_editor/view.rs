//! The TUI's painter for the editor: a thin wrapper over the terminal-free
//! [`EditorDisplay`] that turns its [`EditorDisplay::frame`] into ratatui
//! cells and drives the terminal cursor.

use super::markdown_style::{mask_to_modifier, role_style};
use crate::logic::editor::display::{CursorShape, EditorDisplay};
use crate::logic::editor::frame::EditorFrame;
use crate::logic::editor::markdown::runs::Run;
use crate::logic::editor::overlay::OverlayKind;
use crate::logic::editor::snapshot::EditorSnapshot;
use crate::settings::themes::Theme;
use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use std::ops::{Deref, DerefMut};

/// How `kind` restyles the cells it covers.
///
/// The one place presentation for overlays lives: producers carry a kind,
/// never a `Style`, so a find bar cannot hold an opinion about colour that
/// has to be kept in sync with anything.
fn overlay_style(kind: OverlayKind, theme: &Theme, style: Style) -> Style {
    use ratatui::style::Modifier;
    match kind {
        OverlayKind::TaskBox => style.fg(theme.accent.to_ratatui()),
        OverlayKind::TaskDone => style.add_modifier(Modifier::DIM | Modifier::CROSSED_OUT),
        OverlayKind::Needle | OverlayKind::Match => style
            .fg(theme.color_search_match.to_ratatui())
            .add_modifier(Modifier::BOLD),
        OverlayKind::Selection | OverlayKind::CurrentMatch => {
            style.bg(theme.selection_bg.to_ratatui())
        }
        OverlayKind::Preview => style.bg(theme.color_replace_preview.to_ratatui()),
        // A foreground override, not a modifier: BOLD is a no-op on text
        // that is already bold, which once left the current match
        // indistinguishable from the rest.
        OverlayKind::PreviewCurrent => style
            .bg(theme.color_replace_preview.to_ratatui())
            .fg(cursor_fg(theme))
            .add_modifier(Modifier::BOLD),
    }
}

/// The `cursor` role, substituting a chromatic colour when the theme defers to
/// the terminal — `Reset` foreground on `Reset` body text marks nothing.
fn cursor_fg(theme: &Theme) -> ratatui::style::Color {
    match theme.cursor {
        crate::settings::themes::ThemeColor::Reset => theme.fg_bright.to_ratatui(),
        _ => theme.cursor.to_ratatui(),
    }
}

/// The style a [`Run`] paints with: role + emphasis, then the code-box
/// background, then each overlay in stacking order.
fn run_style(run: &Run, theme: &Theme) -> Style {
    let mut s = role_style(run.role, theme).add_modifier(mask_to_modifier(run.mods));
    if run.code_box {
        s = s.bg(theme.code_bg.to_ratatui());
    }
    for kind in run.overlays.iter() {
        s = overlay_style(kind, theme, s);
    }
    s
}

/// The editor as the terminal draws it: an [`EditorDisplay`] plus the two
/// pieces of state that only mean something on a terminal.
///
/// Every display method is reached through `Deref`, so callers keep writing
/// `view.set_overlays(..)` and friends.
#[derive(Clone)]
pub struct MarkdownEditorView {
    inner: EditorDisplay,
    /// Cursor's last on-screen position (col, row), mirrored from the painter
    /// after each [`Self::render`].
    pub last_cursor_screen: Option<(u16, u16)>,
    painter: PainterState,
}

/// What the painter remembers between frames.
#[derive(Clone, Default)]
pub(super) struct PainterState {
    /// Cursor's last on-screen position (col, row), or `None` when the
    /// cursor was scrolled off-screen or the view was unfocused at the
    /// time of the previous paint. Used as the anchor for floating
    /// overlays like the autocomplete popup, which is drawn after the
    /// editor itself.
    pub last_cursor_screen: Option<(u16, u16)>,
    /// Cursor style last written to the terminal, or `None` when the
    /// terminal is on the user's default shape. The terminal cursor style
    /// is global state, so on focus loss we must emit an explicit reset —
    /// otherwise the editor's bar/block shape leaks into every other text
    /// input (search sidebar, dialogs).
    pub applied_cursor_style: Option<CursorShape>,
}

/// Paint an [`EditorFrame`] into `rect`: its rows as styled lines, then the
/// terminal cursor (position and shape), recording both in `painter`.
pub(super) fn paint_frame(
    f: &mut Frame,
    rect: Rect,
    theme: &Theme,
    frame: EditorFrame,
    cursor_shape: Option<CursorShape>,
    painter: &mut PainterState,
) {
    if rect.height == 0 {
        return;
    }
    let cursor = frame.cursor;
    let lines: Vec<Line> = frame
        .rows
        .into_iter()
        .map(|row| {
            Line::from(
                row.runs
                    .into_iter()
                    .map(|r| {
                        let style = run_style(&r, theme);
                        Span::styled(r.text, style)
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    f.render_widget(
        Paragraph::new(Text::from(lines)).style(theme.base_style()),
        rect,
    );

    painter.last_cursor_screen = None;
    let mut desired_style: Option<CursorShape> = None;
    // The frame is sized by the viewport the last `update` set; a cell
    // below `rect` (a render rect shorter than the update rect) is not
    // ours to place the cursor in.
    if let Some((c, r)) = cursor.filter(|&(_, r)| r < rect.height) {
        let (cx, cy) = (rect.x + c, rect.y + r);
        f.set_cursor_position(Position { x: cx, y: cy });
        painter.last_cursor_screen = Some((cx, cy));
        desired_style = cursor_shape;
    }
    if desired_style != painter.applied_cursor_style {
        use ratatui::crossterm::cursor::SetCursorStyle;
        let style = match desired_style {
            Some(CursorShape::Block) => SetCursorStyle::SteadyBlock,
            Some(CursorShape::Bar) => SetCursorStyle::SteadyBar,
            None => SetCursorStyle::DefaultUserShape,
        };
        let _ = ratatui::crossterm::execute!(std::io::stdout(), style);
        painter.applied_cursor_style = desired_style;
    }
}

impl Deref for MarkdownEditorView {
    type Target = EditorDisplay;
    fn deref(&self) -> &EditorDisplay {
        &self.inner
    }
}

impl DerefMut for MarkdownEditorView {
    fn deref_mut(&mut self) -> &mut EditorDisplay {
        &mut self.inner
    }
}

impl MarkdownEditorView {
    pub fn new() -> Self {
        Self {
            inner: EditorDisplay::new(),
            last_cursor_screen: None,
            painter: PainterState::default(),
        }
    }

    pub fn update(&mut self, snap: &EditorSnapshot, rect: Rect) {
        self.inner.update_sized(snap, rect.width, rect.height);
    }

    pub fn render(
        &mut self,
        f: &mut Frame,
        rect: Rect,
        theme: &Theme,
        focused: bool,
        cursor_shape: Option<CursorShape>,
    ) {
        if rect.height == 0 {
            return;
        }
        let frame = self.frame(rect.width, focused);
        paint_frame(f, rect, theme, frame, cursor_shape, &mut self.painter);
        self.last_cursor_screen = self.painter.last_cursor_screen;
    }
}

impl Default for MarkdownEditorView {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::editor::display::tests::{frame_row_text, rect, update_view};

    /// [`crate::logic::editor::display::tests::make_view_for_lines`], wrapped
    /// so the result can paint.
    fn make_view_for_lines(
        lines: &[String],
        cursor: (usize, usize),
        width: u16,
    ) -> MarkdownEditorView {
        let mut v = MarkdownEditorView::new();
        update_view(&mut v, lines, cursor, (width, 24), 1, None);
        v
    }

    #[test]
    fn frame_cursor_for_empty_buffers_matches_render() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut v = make_view_for_lines(&[], (0, 0), 40);
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(40, 5)).unwrap();
        terminal
            .draw(|f| v.render(f, f.area(), &theme, true, None))
            .unwrap();
        let old = v.last_cursor_screen;
        assert_eq!(v.frame(40, true).cursor, old);
        // An empty note is still one (empty) row, so the cursor is at the origin.
        assert_eq!(old, Some((0, 0)));
        // A view that never saw an update has no rows at all: no cursor.
        let fresh = MarkdownEditorView::new().frame(40, true);
        assert!(fresh.rows.is_empty());
        assert_eq!(fresh.cursor, None);
    }

    #[test]
    fn frame_rows_match_what_render_paints() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let lines: Vec<String> = [
            "# Title",
            "some **bold** text",
            "```",
            "let x = 1;",
            "```",
            "tail",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let theme = Theme::default();
        let mut v = make_view_for_lines(&lines, (5, 1), 30);
        let mut terminal = Terminal::new(TestBackend::new(30, 8)).unwrap();
        terminal
            .draw(|f| v.render(f, f.area(), &theme, true, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let frame = v.frame(30, true);
        assert_eq!(frame.rows.len(), 6);
        for (y, row) in frame.rows.iter().enumerate() {
            let painted: String = (0..30u16)
                .map(|x| buf[(x, y as u16)].symbol().to_string())
                .collect();
            assert_eq!(
                painted.trim_end(),
                frame_row_text(row).trim_end(),
                "row {y}"
            );
        }
    }

    #[test]
    fn run_style_overlay_bg_wins_over_code_box_bg() {
        // Fails if the code_box bg is applied after the overlay loop.
        let theme = Theme::default();
        let mut run = Run::new(
            "x",
            crate::logic::editor::markdown::runs::Role::CodeBoxPad,
            0,
        );
        run.code_box = true;
        assert_eq!(run_style(&run, &theme).bg, Some(theme.code_bg.to_ratatui()));
        run.overlays.insert(OverlayKind::Selection);
        assert_ne!(theme.selection_bg.to_ratatui(), theme.code_bg.to_ratatui());
        assert_eq!(
            run_style(&run, &theme).bg,
            Some(theme.selection_bg.to_ratatui())
        );
    }

    #[test]
    fn run_style_later_overlay_kind_wins_on_the_same_channel() {
        // Selection and PreviewCurrent both set bg; PreviewCurrent is later.
        // Fails if overlays are iterated in reverse.
        let theme = Theme::default();
        let mut run = Run::new(
            "x",
            crate::logic::editor::markdown::runs::Role::CodeBoxPad,
            0,
        );
        run.overlays.insert(OverlayKind::Selection);
        run.overlays.insert(OverlayKind::PreviewCurrent);
        assert_ne!(
            theme.selection_bg.to_ratatui(),
            theme.color_replace_preview.to_ratatui()
        );
        assert_eq!(
            run_style(&run, &theme).bg,
            Some(theme.color_replace_preview.to_ratatui())
        );
    }

    #[test]
    fn run_style_paints_code_box_pad_with_only_the_code_background() {
        let theme = Theme::default();
        let mut pad = Run::new(
            "   ",
            crate::logic::editor::markdown::runs::Role::CodeBoxPad,
            0,
        );
        pad.code_box = true;
        let s = run_style(&pad, &theme);
        assert_eq!(s.bg, Some(theme.code_bg.to_ratatui()));
        assert_eq!(s.fg, None);
        assert!(s.add_modifier.is_empty());
    }

    #[test]
    fn code_box_background_reaches_rendered_cells() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let lines = vec![
            "```".to_string(),
            "let x = 1;".to_string(),
            "```".to_string(),
            "plain".to_string(),
        ];
        let theme = crate::settings::themes::Theme::gruvbox_dark();
        let mut view = make_view_for_lines(&lines, (3, 0), 40);
        let mut terminal = Terminal::new(TestBackend::new(40, 5)).unwrap();
        terminal
            .draw(|f| view.render(f, f.area(), &theme, true, Some(CursorShape::Bar)))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let code_bg = theme.code_bg.to_ratatui();
        let cell = |x: u16, y: u16| &buf.content[(y as usize) * 40 + (x as usize)];

        // A cell on the fenced code content row carries the code-box bg...
        assert_eq!(
            cell(0, 1).bg,
            code_bg,
            "code content cell must have code_bg"
        );
        // ...including the padding past the text (box is a solid rectangle).
        assert_eq!(cell(8, 1).bg, code_bg, "code-box padding must have code_bg");
        // A prose row outside the block does NOT get the code bg.
        assert_ne!(cell(0, 3).bg, code_bg, "prose row must not have code_bg");
    }

    #[test]
    fn render_does_not_panic_on_stale_cursor_past_line_count() {
        // Regression: render() previously did self.parsed_cache[cursor.0]
        // and self.layout.visual_lines()[cursor_vrow] directly. A stale
        // Nvim snapshot whose cursor row landed past the new line count
        // would panic the render thread. Now the test exercises the
        // producer-side clamp (via `update_view`'s mirror of
        // `snapshot_from_backend`): the snapshot constructor clamps
        // the cursor, render trusts the invariant, and direct
        // indexing is safe.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::gruvbox_dark();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut v = MarkdownEditorView::new();
        // Populate with 2 lines and a valid cursor first so parsed_cache /
        // layout are non-empty.
        update_view(
            &mut v,
            &["alpha".to_string(), "beta".to_string()],
            (0, 0),
            rect(8),
            1,
            None,
        );
        // Now feed a cursor row that exceeds the line count for this update
        // (simulates a stale snapshot arriving after a shrink). update() at
        // line 277 already uses `lines.get(cursor.0)` so it won't panic; the
        // real risk was the [] indexes inside render(). cursor_snapshot ends
        // up at (5, 0) which exceeds the parsed_cache len of 2 below.
        update_view(
            &mut v,
            &["alpha".to_string(), "beta".to_string()],
            (5, 0),
            rect(8),
            1,
            None,
        );
        // Render with focus so the cursor branch runs.
        terminal
            .draw(|f| v.render(f, f.area(), &theme, true, Some(CursorShape::Bar)))
            .expect("render must not panic on stale cursor");
    }

    /// The painter must paint exactly what the frame says: every cell a run
    /// covers carries that run's `run_style` (over the pane's base style).
    /// Pins the seam between `EditorDisplay::frame` and `render` so neither
    /// can grow an opinion the other does not share.
    #[test]
    fn painted_cells_carry_the_run_style_of_the_frame() {
        use crate::logic::editor::overlay::Overlay;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::style::Color;
        use unicode_width::UnicodeWidthStr;

        // One indented-code row: a code box, with a selection over part of it.
        let lines = vec!["    let x = 1;".to_string()];
        let theme = Theme::gruvbox_dark();
        let mut v = make_view_for_lines(&lines, (0, 0), 30);
        v.set_overlays(vec![Overlay::new(0, 4, 7, OverlayKind::Selection)]);
        let frame = v.frame(30, true);
        let runs = &frame.rows[0].runs;
        assert!(
            runs.iter().any(|r| r.code_box),
            "fixture must have a code box"
        );
        assert!(
            runs.iter().any(|r| r.overlays.iter().next().is_some()),
            "fixture must carry an overlay"
        );

        let mut terminal = Terminal::new(TestBackend::new(30, 3)).unwrap();
        terminal
            .draw(|f| v.render(f, f.area(), &theme, true, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();

        let mut x = 0u16;
        for run in runs {
            let want = theme.base_style().patch(run_style(run, &theme));
            for _ in 0..run.text.width() {
                let cell = &buf[(x, 0)];
                assert_eq!(cell.fg, want.fg.unwrap_or(Color::Reset), "fg at {x}");
                assert_eq!(cell.bg, want.bg.unwrap_or(Color::Reset), "bg at {x}");
                assert_eq!(
                    cell.modifier,
                    want.add_modifier - want.sub_modifier,
                    "modifier at {x}"
                );
                x += 1;
            }
        }
        assert!(x > 0, "the frame painted nothing");
    }
}
