//! `SourcesPanel` — the Ask workspace's drawer view (CONTEXT.md: **Ask
//! workspace**, **Source**; adr/0030): a ranked per-turn source list that
//! flips to a **Source reader** face (the full note, retrieved section
//! highlighted) without leaving the answer. Shape mirrors `SemanticPanel`
//! (`semantic_search.rs`, the closest existing drawer view): a plain struct
//! with inherent `new`/`hint_shortcuts`/`handle_input`/`render`, no
//! `Component` impl — `DrawerHost` (Task 11) calls it directly.
//!
//! The panel owns an `Arc<NoteVault>` (opening the reader spawns a note load),
//! so its `handle_input` matches the plain `Component`-style signature — no
//! vault threaded through by the caller.

use std::ops::Range;
use std::sync::Arc;

use kimun_core::NoteVault;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ask::{AskSource, locate};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, AskData, InputEvent};
use crate::components::panel::panel_block;
use crate::components::preview_highlight::wrap_line;
use crate::settings::themes::Theme;

/// The source reader's load state for the currently opened source.
enum ReaderContent {
    /// The note load is in flight.
    Loading,
    /// The note loaded successfully. `highlight` is the byte range
    /// `locate::section_range` resolved, if any.
    Loaded {
        text: String,
        highlight: Option<Range<usize>>,
    },
    /// The note load failed.
    Failed,
}

/// Which face the panel currently shows.
enum Face {
    /// The ranked source list.
    List,
    /// The full note for `sources[source_index]`, section highlighted.
    Reader {
        source_index: usize,
        content: ReaderContent,
        /// Topmost visible line of the note text.
        scroll: usize,
    },
}

/// The Ask workspace's Sources drawer view: list face + reader face over the
/// selected turn's sources.
pub struct SourcesPanel {
    turn_id: Option<u64>,
    sources: Vec<AskSource>,
    cursor: usize,
    face: Face,
    /// Reader viewport height (rows) from the last render — clamps scroll.
    reader_viewport_height: usize,
    /// Total wrapped rows the reader produced at the last render's width —
    /// clamps scroll in the same wrapped-row units the scroll offset uses
    /// (recomputed every render, so a width change re-clamps automatically).
    reader_total_rows: usize,
    /// Set when the reader face is (re)opened: the first render that has the
    /// note loaded anchors `scroll` to the first highlighted wrapped row, then
    /// clears this. Deferred to render because the wrapped-row index of the
    /// highlight depends on the render width, unknown when the load lands.
    reader_autoscroll_pending: bool,
    /// Vault handle for the reader's note load (`open_reader` spawns a
    /// `vault.get_note_text`). Owned here so `handle_input` needs no vault
    /// passed in.
    vault: Arc<NoteVault>,
}

impl SourcesPanel {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self {
            turn_id: None,
            sources: Vec::new(),
            cursor: 0,
            face: Face::List,
            reader_viewport_height: 0,
            reader_total_rows: 0,
            reader_autoscroll_pending: false,
            vault,
        }
    }

    /// Repopulates the list for `turn_id` and resets to the list face. A
    /// repeated call with the same `turn_id` is a no-op — it keeps the cursor
    /// (and the reader face, if open) exactly as-is when a selection sync
    /// re-points the drawer at the already-shown turn. Regeneration replaces a
    /// turn's sources with the fresh ones on completion, but that goes through
    /// [`refresh`](Self::refresh) (which never short-circuits), not here.
    pub fn set_turn(&mut self, turn_id: u64, sources: Vec<AskSource>) {
        if self.turn_id == Some(turn_id) {
            return;
        }
        self.refresh(turn_id, sources);
    }

    /// Force the source list for `turn_id` to `sources`, even when it's the
    /// turn already shown — the answer-completion path, where a `Thinking`
    /// turn (empty sources) gains its sources once the answer lands. Unlike
    /// [`set_turn`](Self::set_turn), it never short-circuits on a matching id.
    /// Resets to the list face at the top.
    pub fn refresh(&mut self, turn_id: u64, sources: Vec<AskSource>) {
        self.turn_id = Some(turn_id);
        self.sources = sources;
        self.cursor = 0;
        self.face = Face::List;
    }

    /// Clear the panel back to its empty list face — the "new conversation"
    /// action (leader `a n`) drops the old turn's sources.
    pub fn reset(&mut self) {
        self.turn_id = None;
        self.sources = Vec::new();
        self.cursor = 0;
        self.face = Face::List;
    }

    /// Point the list cursor at the source with citation `ordinal` on the list
    /// face — a citation click in the thread asks the drawer to reveal that
    /// exact source. This is the ordinal→row boundary: the panel lists sources
    /// in vec order, so it resolves the ordinal to a position by matching, never
    /// by assuming `ordinal - 1`. An ordinal with no matching source is ignored.
    pub fn focus_source(&mut self, ordinal: usize) {
        if let Some(pos) = self.sources.iter().position(|s| s.ordinal == ordinal) {
            self.cursor = pos;
            self.face = Face::List;
        }
    }

    /// Flips to the reader face for `sources[source_index]` and spawns the
    /// note load — the same async call shape the editor screen's note-open
    /// path uses (`vault.get_note_text`). No-op for an out-of-range index.
    pub fn open_reader(&mut self, source_index: usize, tx: &AppTx) {
        let Some(source) = self.sources.get(source_index) else {
            return;
        };
        self.face = Face::Reader {
            source_index,
            content: ReaderContent::Loading,
            scroll: 0,
        };
        // Anchor to the highlighted section on the first loaded render (see the
        // field doc — the wrapped-row target needs the render width).
        self.reader_autoscroll_pending = true;
        let path = source.path.clone();
        let vault = self.vault.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let text = vault.get_note_text(&path).await.ok();
            let _ = tx.send(AppEvent::Ask(AskData::ReaderNote { path, text }));
        });
    }

    /// Accepts a `ReaderNote` only when the reader face is currently
    /// awaiting that exact path (stale-drop: a source switch, or leaving the
    /// reader, before the load lands must not clobber whatever's current).
    /// Any other `AskData` variant is addressed elsewhere and ignored.
    pub fn handle_data(&mut self, data: AskData) {
        let AskData::ReaderNote { path, text } = data else {
            return;
        };
        let Face::Reader {
            source_index,
            content,
            ..
        } = &mut self.face
        else {
            return;
        };
        let Some(source) = self.sources.get(*source_index) else {
            return;
        };
        if source.path != path {
            return;
        }
        match text {
            Some(loaded) => {
                let highlight = locate::section_range(&loaded, &source.heading, &source.text);
                // The scroll offset is set at render time (in wrapped-row units,
                // which need the render width) via `reader_autoscroll_pending`.
                *content = ReaderContent::Loaded {
                    text: loaded,
                    highlight,
                };
            }
            None => *content = ReaderContent::Failed,
        }
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        match &self.face {
            Face::List => vec![
                ("j/k".into(), "Select".into()),
                ("Enter/l".into(), "Read".into()),
                ("o".into(), "Open".into()),
                ("y".into(), "Yank path".into()),
            ],
            Face::Reader { .. } => vec![
                ("j/k".into(), "Scroll".into()),
                ("h/Esc".into(), "Back".into()),
                ("o".into(), "Open".into()),
            ],
        }
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match &self.face {
            Face::List => self.handle_list_key(key, tx),
            Face::Reader { .. } => self.handle_reader_key(key, tx),
        }
    }

    fn handle_list_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.sources.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.sources.len() - 1);
                }
                EventState::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                EventState::Consumed
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                if !self.sources.is_empty() {
                    self.open_reader(self.cursor, tx);
                }
                EventState::Consumed
            }
            KeyCode::Char('o') => {
                if let Some(source) = self.sources.get(self.cursor) {
                    tx.send(AppEvent::open(source.path.clone())).ok();
                }
                EventState::Consumed
            }
            KeyCode::Char('y') => {
                self.yank_selected_path(tx);
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn handle_reader_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.reader_scroll_by(1);
                EventState::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.reader_scroll_by(-1);
                EventState::Consumed
            }
            KeyCode::Char('h') | KeyCode::Esc => {
                self.face = Face::List;
                EventState::Consumed
            }
            KeyCode::Char('o') => {
                if let Face::Reader { source_index, .. } = &self.face
                    && let Some(source) = self.sources.get(*source_index)
                {
                    tx.send(AppEvent::open(source.path.clone())).ok();
                }
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    /// Copy the selected (list face) or open (reader face) source's path to
    /// the OS clipboard, reusing the same `arboard` seam `ThreadPanel` uses.
    fn yank_selected_path(&self, tx: &AppTx) {
        let Some(source) = self.sources.get(self.cursor) else {
            return;
        };
        let text = source.path.to_string();
        let msg = match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            Ok(()) => "path copied".to_string(),
            Err(e) => format!("clipboard: {e}"),
        };
        tx.send(AppEvent::FlashMessage(msg)).ok();
    }

    fn reader_scroll_by(&mut self, delta: i64) {
        let viewport = self.reader_viewport_height;
        // Clamp against the wrapped row count from the last render, not the raw
        // source-line count — scroll is in wrapped-row units now that long lines
        // wrap in the reader.
        let total = self.reader_total_rows;
        let Face::Reader { content, scroll, .. } = &mut self.face else {
            return;
        };
        let ReaderContent::Loaded { .. } = content else {
            return;
        };
        let max = total.saturating_sub(viewport.max(1)) as i64;
        let next = (*scroll as i64 + delta).clamp(0, max.max(0));
        *scroll = next as usize;
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        if matches!(self.face, Face::Reader { .. }) {
            self.render_reader(f, rect, theme, focused);
        } else {
            self.render_list(f, rect, theme, focused);
        }
    }

    fn render_list(&self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let block = panel_block("Sources", theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        if self.sources.is_empty() {
            let style = Style::default().fg(theme.gray.to_ratatui());
            f.render_widget(Paragraph::new("no sources — ask something").style(style), inner);
            return;
        }

        let bold = Style::default()
            .fg(theme.fg_bright.to_ratatui())
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(theme.gray.to_ratatui());
        let sel_bg = theme.selection_bg.to_ratatui();

        let mut lines: Vec<Line<'static>> = Vec::new();
        for (i, source) in self.sources.iter().enumerate() {
            let selected = focused && i == self.cursor;
            let style_row = |line: Line<'static>| {
                if selected {
                    line.style(Style::default().bg(sel_bg))
                } else {
                    line
                }
            };
            lines.push(style_row(Line::from(Span::styled(
                format!("{} {}", i + 1, source.heading),
                bold,
            ))));
            lines.push(style_row(Line::from(Span::styled(
                format!("{}  {}", source.path, score_bar(source.score)),
                dim,
            ))));
        }

        // Stateless auto-scroll: keep the cursor's row pair in view. Rows
        // are two lines each, so this is a pure function of cursor + height,
        // recomputed every render — no persisted scroll offset to drift.
        let row_height = 2u16;
        let cursor_row = self.cursor as u16 * row_height;
        let height = inner.height;
        let scroll = if height == 0 || cursor_row < height {
            0
        } else {
            cursor_row - height + row_height
        };

        f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
    }

    fn render_reader(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let source_index = match &self.face {
            Face::Reader { source_index, .. } => *source_index,
            Face::List => return,
        };
        let heading = self
            .sources
            .get(source_index)
            .map(|s| s.heading.clone())
            .unwrap_or_else(|| "Source".to_string());

        let block = panel_block(&heading, theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        self.reader_viewport_height = inner.height as usize;

        let dim = Style::default().fg(theme.gray.to_ratatui());
        let autoscroll_pending = self.reader_autoscroll_pending;
        let viewport = inner.height.max(1) as usize;
        let mut total_rows = 0usize;
        let mut loaded_rendered = false;
        let Face::Reader { content, scroll, .. } = &mut self.face else {
            return;
        };
        match content {
            ReaderContent::Loading => {
                f.render_widget(Paragraph::new("loading…").style(dim), inner);
            }
            ReaderContent::Failed => {
                let style = Style::default().fg(theme.red.to_ratatui());
                f.render_widget(Paragraph::new("failed to load note").style(style), inner);
            }
            ReaderContent::Loaded { text, highlight } => {
                let (lines, first_highlight) =
                    reader_lines(text, highlight.as_ref(), inner.width, theme);
                total_rows = lines.len();
                loaded_rendered = true;
                // First loaded render after (re)opening: anchor to the first
                // highlighted wrapped row.
                if autoscroll_pending {
                    *scroll = first_highlight.unwrap_or(0);
                }
                // Re-clamp for this width's wrapped total (a width change shrinks
                // or grows the row count under a persisted offset).
                let max = total_rows.saturating_sub(viewport);
                if *scroll > max {
                    *scroll = max;
                }
                f.render_widget(Paragraph::new(lines).scroll((*scroll as u16, 0)), inner);
            }
        }
        self.reader_total_rows = total_rows;
        if loaded_rendered {
            self.reader_autoscroll_pending = false;
        }
    }
}

/// A relevance bar for the list face's meta line: filled cells proportional
/// to `score` (expected `0.0..=1.0`, the server's normalized similarity —
/// clamped defensively either way).
fn score_bar(score: f64) -> String {
    const WIDTH: usize = 10;
    let filled = (score.clamp(0.0, 1.0) * WIDTH as f64).round() as usize;
    let filled = filled.min(WIDTH);
    format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled))
}

/// Builds the reader face's content rows, wrapping each source line to the
/// drawer width so long prose doesn't overflow. Every wrapped segment of a
/// source line carries the same 2-column prefix: a `▌ ` accent bar when the
/// source line overlaps `highlight`, a plain two-space indent otherwise.
/// Returns the rows plus the index of the first highlighted wrapped row (the
/// auto-scroll anchor), if any. `width` is the drawer's inner width in columns.
fn reader_lines(
    text: &str,
    highlight: Option<&Range<usize>>,
    width: u16,
    theme: &Theme,
) -> (Vec<Line<'static>>, Option<usize>) {
    let normal = Style::default().fg(theme.fg.to_ratatui());
    let accent = Style::default().fg(theme.accent.to_ratatui());
    // Reserve the 2 prefix columns; `wrap_line` measures in characters.
    let wrap_width = (width as usize).saturating_sub(2);
    let mut lines = Vec::new();
    let mut first_highlight = None;
    let mut offset = 0usize;
    for raw_line in text.split_inclusive('\n') {
        let stripped = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line_range = offset..offset + stripped.len();
        let is_highlighted =
            highlight.is_some_and(|h| line_range.start < h.end && h.start < line_range.end);
        let (prefix, prefix_style) = if is_highlighted {
            ("▌ ", accent)
        } else {
            ("  ", normal)
        };
        for segment in wrap_line(stripped, wrap_width) {
            if is_highlighted && first_highlight.is_none() {
                first_highlight = Some(lines.len());
            }
            lines.push(Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::styled(segment, normal),
            ]));
        }
        offset += raw_line.len();
    }
    if lines.is_empty() {
        lines.push(Line::default());
    }
    (lines, first_highlight)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::VaultConfig;
    use kimun_core::nfs::VaultPath;
    use ratatui::crossterm::event::KeyModifiers;
    use tempfile::TempDir;

    fn source(path: &str, heading: &str, score: f64, text: &str) -> AskSource {
        AskSource {
            path: VaultPath::new(path),
            heading: heading.to_string(),
            score,
            text: text.to_string(),
            ordinal: 0,
        }
    }

    async fn test_vault() -> (TempDir, NoteVault) {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        (dir, vault)
    }

    /// A panel over a throwaway vault, for tests that never touch the reader's
    /// note load. The backing dir is leaked so the vault stays valid for the
    /// test's lifetime.
    async fn test_panel() -> SourcesPanel {
        let (dir, vault) = test_vault().await;
        std::mem::forget(dir);
        SourcesPanel::new(Arc::new(vault))
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[tokio::test]
    async fn new_panel_starts_empty_on_the_list_face() {
        let p = test_panel().await;
        assert!(p.sources.is_empty());
        assert!(matches!(p.face, Face::List));
    }

    #[tokio::test]
    async fn set_turn_populates_and_resets_to_list_face() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "text a")]);
        assert_eq!(p.sources.len(), 1);
        assert_eq!(p.turn_id, Some(1));
        assert!(matches!(p.face, Face::List));
    }

    #[tokio::test]
    async fn set_turn_same_id_is_a_noop_and_keeps_cursor() {
        let mut p = test_panel().await;
        p.set_turn(
            1,
            vec![
                source("a.md", "A", 0.9, "text a"),
                source("b.md", "B", 0.5, "text b"),
            ],
        );
        p.cursor = 1;
        // Same turn id, different sources passed in: must be ignored.
        p.set_turn(1, vec![source("c.md", "C", 0.1, "text c")]);
        assert_eq!(p.cursor, 1, "cursor must survive a same-id set_turn");
        assert_eq!(p.sources.len(), 2, "sources must not be replaced");
        assert_eq!(p.sources[0].heading, "A");
    }

    #[tokio::test]
    async fn set_turn_new_id_resets_cursor_and_face() {
        let mut p = test_panel().await;
        p.set_turn(
            1,
            vec![
                source("a.md", "A", 0.9, "text a"),
                source("b.md", "B", 0.5, "text b"),
            ],
        );
        p.cursor = 1;
        p.set_turn(2, vec![source("c.md", "C", 0.1, "text c")]);
        assert_eq!(p.cursor, 0);
        assert_eq!(p.sources.len(), 1);
        assert_eq!(p.sources[0].heading, "C");
    }

    #[tokio::test]
    async fn reader_note_for_the_wrong_path_is_dropped() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "alpha body")]);
        p.face = Face::Reader {
            source_index: 0,
            content: ReaderContent::Loading,
            scroll: 0,
        };
        p.handle_data(AskData::ReaderNote {
            path: VaultPath::new("other.md"),
            text: Some("nope".to_string()),
        });
        let Face::Reader { content, .. } = &p.face else {
            panic!("still in reader face");
        };
        assert!(
            matches!(content, ReaderContent::Loading),
            "wrong-path ReaderNote must be dropped, not accepted"
        );
    }

    #[tokio::test]
    async fn reader_note_for_the_right_path_loads_and_highlights() {
        let mut p = test_panel().await;
        p.set_turn(
            1,
            vec![source("a.md", "b", 0.9, "beta body")],
        );
        p.face = Face::Reader {
            source_index: 0,
            content: ReaderContent::Loading,
            scroll: 0,
        };
        p.handle_data(AskData::ReaderNote {
            path: VaultPath::new("a.md"),
            text: Some("# a\nalpha body\n# b\nbeta body\n".to_string()),
        });
        let Face::Reader { content, .. } = &p.face else {
            panic!("still in reader face");
        };
        match content {
            ReaderContent::Loaded { text, highlight } => {
                let r = highlight.clone().expect("chunk resolves");
                assert_eq!(&text[r], "beta body");
                // The scroll offset is now anchored at render time (wrapped-row
                // units); see `reader_autoscroll_anchors_to_highlighted_row`.
            }
            _ => panic!("expected Loaded"),
        }
    }

    #[tokio::test]
    async fn reader_note_load_failure_is_recorded() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "alpha body")]);
        p.face = Face::Reader {
            source_index: 0,
            content: ReaderContent::Loading,
            scroll: 0,
        };
        p.handle_data(AskData::ReaderNote {
            path: VaultPath::new("a.md"),
            text: None,
        });
        let Face::Reader { content, .. } = &p.face else {
            panic!("still in reader face");
        };
        assert!(matches!(content, ReaderContent::Failed));
    }

    #[tokio::test]
    async fn handle_data_ignores_answer_ready() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "alpha body")]);
        p.face = Face::Reader {
            source_index: 0,
            content: ReaderContent::Loading,
            scroll: 0,
        };
        p.handle_data(AskData::AnswerReady {
            turn_id: 1,
            result: Ok(("x".into(), vec![])),
        });
        let Face::Reader { content, .. } = &p.face else {
            panic!("still in reader face");
        };
        assert!(matches!(content, ReaderContent::Loading));
    }

    #[tokio::test]
    async fn jk_moves_cursor_within_bounds() {
        let mut p = test_panel().await;
        p.set_turn(
            1,
            vec![
                source("a.md", "A", 0.9, "a"),
                source("b.md", "B", 0.5, "b"),
            ],
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx);
        assert_eq!(p.cursor, 1);
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx);
        assert_eq!(p.cursor, 1, "clamped at the last row");
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('k'))), &tx);
        assert_eq!(p.cursor, 0);
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('k'))), &tx);
        assert_eq!(p.cursor, 0, "clamped at the first row");
    }

    #[tokio::test]
    async fn h_and_esc_return_to_list_face() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "a")]);
        p.face = Face::Reader {
            source_index: 0,
            content: ReaderContent::Loading,
            scroll: 0,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        p.handle_input(&InputEvent::Key(key(KeyCode::Esc)), &tx);
        assert!(matches!(p.face, Face::List));
    }

    #[tokio::test]
    async fn open_reader_round_trips_through_a_real_vault() {
        let (_dir, vault) = test_vault().await;
        let path = VaultPath::new("note.md");
        vault.create_note(&path, "# h\nbody text\n").await.unwrap();

        let mut p = SourcesPanel::new(Arc::new(vault));
        p.set_turn(1, vec![source("note.md", "h", 0.9, "body text")]);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        p.open_reader(0, &tx);
        assert!(matches!(p.face, Face::Reader { .. }));

        let event = rx.recv().await.expect("open_reader sends a ReaderNote");
        let AppEvent::Ask(data) = event else {
            panic!("expected an Ask event");
        };
        p.handle_data(data);

        let Face::Reader { content, .. } = &p.face else {
            panic!("still in reader face");
        };
        match content {
            ReaderContent::Loaded { text, .. } => assert_eq!(text, "# h\nbody text\n"),
            _ => panic!("expected Loaded"),
        }
    }

    #[test]
    fn reader_lines_wraps_a_long_highlighted_line_into_prefixed_rows() {
        let theme = Theme::default();
        // One source line (no newline), fully highlighted, longer than the
        // wrap width (10 cols inner − 2 prefix = 8 chars).
        let text = "aaaa bbbb cccc dddd eeee";
        let highlight = Some(0..text.len());
        let (lines, first) = reader_lines(text, highlight.as_ref(), 10, &theme);
        assert!(lines.len() > 1, "a long line must wrap into multiple rows");
        assert_eq!(first, Some(0), "first highlighted wrapped row is the anchor");
        for line in &lines {
            assert_eq!(
                line.spans[0].content, "▌ ",
                "every wrapped segment keeps the highlight prefix"
            );
        }
    }

    #[test]
    fn reader_lines_prefixes_normal_and_highlighted_segments_distinctly() {
        let theme = Theme::default();
        // Two source lines: the second is highlighted, and long enough to wrap.
        let text = "short\nlonglonglong wordword tail";
        let hl_start = text.find("longlonglong").unwrap();
        let highlight = Some(hl_start..text.len());
        let (lines, first) = reader_lines(text, highlight.as_ref(), 12, &theme);
        assert_eq!(lines[0].spans[0].content, "  ", "unhighlighted line indented");
        let first = first.expect("a highlighted row exists");
        assert!(lines.len() > first + 1, "highlighted line wrapped");
        for line in &lines[first..] {
            assert_eq!(line.spans[0].content, "▌ ");
        }
    }

    mod rendering {
        use super::*;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        fn draw(p: &mut SourcesPanel, theme: &Theme, width: u16, height: u16, focused: bool) {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| {
                    let area = f.area();
                    p.render(f, area, theme, focused);
                })
                .unwrap();
        }

        #[tokio::test]
        async fn render_does_not_panic_across_states_and_sizes() {
            let theme = Theme::default();
            let mut p = test_panel().await;
            draw(&mut p, &theme, 40, 10, true); // empty list

            p.set_turn(
                1,
                vec![
                    source("a.md", "Alpha section", 0.9, "alpha body"),
                    source("b.md", "Beta section", 0.4, "beta body"),
                ],
            );
            draw(&mut p, &theme, 40, 10, true); // populated list
            p.cursor = 1;
            draw(&mut p, &theme, 40, 3, true); // tiny viewport, scroll path

            p.face = Face::Reader {
                source_index: 0,
                content: ReaderContent::Loading,
                scroll: 0,
            };
            draw(&mut p, &theme, 40, 10, true); // reader loading

            p.face = Face::Reader {
                source_index: 0,
                content: ReaderContent::Failed,
                scroll: 0,
            };
            draw(&mut p, &theme, 40, 10, true); // reader failed

            p.face = Face::Reader {
                source_index: 0,
                content: ReaderContent::Loaded {
                    text: "# Alpha section\nalpha body\nmore lines\nand more\n".to_string(),
                    highlight: Some(17..28),
                },
                scroll: 0,
            };
            draw(&mut p, &theme, 40, 10, true); // reader loaded, highlighted

            draw(&mut p, &theme, 3, 3, true); // degenerate tiny rect
            draw(&mut p, &theme, 0, 0, true); // zero rect
        }

        #[tokio::test]
        async fn reader_scroll_clamps_to_wrapped_row_count() {
            let theme = Theme::default();
            let mut p = test_panel().await;
            p.set_turn(1, vec![source("a.md", "h", 0.9, "body")]);
            // A single very long source line that wraps into far more rows than
            // the viewport once rendered narrow.
            let long = "word ".repeat(40);
            p.face = Face::Reader {
                source_index: 0,
                content: ReaderContent::Loaded {
                    text: long,
                    highlight: None,
                },
                scroll: 0,
            };
            draw(&mut p, &theme, 12, 4, true);
            assert!(
                p.reader_total_rows > p.reader_viewport_height,
                "wrapping produced more rows than the viewport"
            );
            // Scroll far past the end: it must clamp to wrapped_total − viewport.
            p.reader_scroll_by(1000);
            let Face::Reader { scroll, .. } = &p.face else {
                panic!("still in reader face");
            };
            assert_eq!(
                *scroll,
                p.reader_total_rows - p.reader_viewport_height,
                "clamp is in wrapped-row units"
            );
        }

        #[tokio::test]
        async fn reader_autoscroll_anchors_to_highlighted_row() {
            let theme = Theme::default();
            let mut p = test_panel().await;
            p.set_turn(1, vec![source("a.md", "b", 0.9, "beta body")]);
            p.face = Face::Reader {
                source_index: 0,
                content: ReaderContent::Loaded {
                    text: "line0\nline1\nbeta body\ntail1\ntail2\ntail3\n".to_string(),
                    highlight: Some(12..21), // "beta body" is the 3rd source line
                },
                scroll: 0,
            };
            p.reader_autoscroll_pending = true;
            // Wide (no wrapping) but short enough that row 2 is scrollable.
            draw(&mut p, &theme, 40, 5, true);
            let Face::Reader { scroll, .. } = &p.face else {
                panic!("still in reader face");
            };
            assert_eq!(*scroll, 2, "anchored to the highlighted wrapped row");
            assert!(
                !p.reader_autoscroll_pending,
                "autoscroll intent consumed after the loaded render"
            );
        }
    }
}
