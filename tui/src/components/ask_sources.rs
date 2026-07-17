//! `SourcesPanel` — the Ask workspace's drawer view (CONTEXT.md: **Ask
//! workspace**, **Source**; adr/0030): a ranked per-turn source list that
//! flips to a **Source reader** face (the full note, retrieved section
//! highlighted) without leaving the answer. Shape mirrors `SemanticPanel`
//! (`semantic_search.rs`, the closest existing drawer view): a plain struct
//! with inherent `new`/`hint_shortcuts`/`handle_input`/`render`, no
//! `Component` impl — `DrawerHost` (Task 11) calls it directly.
//!
//! `handle_input` carries a `vault: &NoteVault` the trait signature has no
//! room for (same reason `ThreadPanel::handle_input` carries a client — see
//! its module doc): opening the reader needs to spawn a note load.

use std::ops::Range;

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
}

impl SourcesPanel {
    pub fn new() -> Self {
        Self {
            turn_id: None,
            sources: Vec::new(),
            cursor: 0,
            face: Face::List,
            reader_viewport_height: 0,
        }
    }

    /// Repopulates the list for `turn_id` and resets to the list face. A
    /// repeated call with the same `turn_id` is a no-op — it keeps the
    /// cursor (and the reader face, if open) exactly as-is, since a turn's
    /// sources never change once set (`ThreadPanel::regenerate` reuses
    /// them).
    pub fn set_turn(&mut self, turn_id: u64, sources: Vec<AskSource>) {
        if self.turn_id == Some(turn_id) {
            return;
        }
        self.turn_id = Some(turn_id);
        self.sources = sources;
        self.cursor = 0;
        self.face = Face::List;
    }

    /// Flips to the reader face for `sources[source_index]` and spawns the
    /// note load — the same async call shape the editor screen's note-open
    /// path uses (`vault.get_note_text`). No-op for an out-of-range index.
    pub fn open_reader(&mut self, source_index: usize, tx: &AppTx, vault: &NoteVault) {
        let Some(source) = self.sources.get(source_index) else {
            return;
        };
        self.face = Face::Reader {
            source_index,
            content: ReaderContent::Loading,
            scroll: 0,
        };
        let path = source.path.clone();
        let vault = vault.clone();
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
            scroll,
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
                *scroll = highlight
                    .as_ref()
                    .map(|r| loaded[..r.start].matches('\n').count())
                    .unwrap_or(0);
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

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx, vault: &NoteVault) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match &self.face {
            Face::List => self.handle_list_key(key, tx, vault),
            Face::Reader { .. } => self.handle_reader_key(key, tx),
        }
    }

    fn handle_list_key(&mut self, key: &KeyEvent, tx: &AppTx, vault: &NoteVault) -> EventState {
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
                    self.open_reader(self.cursor, tx, vault);
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
        let Face::Reader { content, scroll, .. } = &mut self.face else {
            return;
        };
        let ReaderContent::Loaded { text, .. } = content else {
            return;
        };
        let total = text.lines().count();
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
        let Face::Reader { content, scroll, .. } = &self.face else {
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
                let lines = reader_lines(text, highlight.as_ref(), theme);
                f.render_widget(Paragraph::new(lines).scroll((*scroll as u16, 0)), inner);
            }
        }
    }
}

impl Default for SourcesPanel {
    fn default() -> Self {
        Self::new()
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

/// Builds the reader face's content lines: each line gets a `▌` accent
/// prefix when it overlaps `highlight`, a plain two-space indent otherwise.
fn reader_lines(text: &str, highlight: Option<&Range<usize>>, theme: &Theme) -> Vec<Line<'static>> {
    let normal = Style::default().fg(theme.fg.to_ratatui());
    let accent = Style::default().fg(theme.accent.to_ratatui());
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for raw_line in text.split_inclusive('\n') {
        let stripped = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line_range = offset..offset + stripped.len();
        let is_highlighted =
            highlight.is_some_and(|h| line_range.start < h.end && h.start < line_range.end);
        let prefix = if is_highlighted {
            Span::styled("▌ ", accent)
        } else {
            Span::styled("  ", normal)
        };
        lines.push(Line::from(vec![
            prefix,
            Span::styled(stripped.to_string(), normal),
        ]));
        offset += raw_line.len();
    }
    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
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
        }
    }

    async fn test_vault() -> (TempDir, NoteVault) {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        (dir, vault)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn new_panel_starts_empty_on_the_list_face() {
        let p = SourcesPanel::new();
        assert!(p.sources.is_empty());
        assert!(matches!(p.face, Face::List));
    }

    #[test]
    fn set_turn_populates_and_resets_to_list_face() {
        let mut p = SourcesPanel::new();
        p.set_turn(1, vec![source("a.md", "A", 0.9, "text a")]);
        assert_eq!(p.sources.len(), 1);
        assert_eq!(p.turn_id, Some(1));
        assert!(matches!(p.face, Face::List));
    }

    #[test]
    fn set_turn_same_id_is_a_noop_and_keeps_cursor() {
        let mut p = SourcesPanel::new();
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

    #[test]
    fn set_turn_new_id_resets_cursor_and_face() {
        let mut p = SourcesPanel::new();
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

    #[test]
    fn reader_note_for_the_wrong_path_is_dropped() {
        let mut p = SourcesPanel::new();
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

    #[test]
    fn reader_note_for_the_right_path_loads_and_highlights() {
        let mut p = SourcesPanel::new();
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
        let Face::Reader { content, scroll, .. } = &p.face else {
            panic!("still in reader face");
        };
        match content {
            ReaderContent::Loaded { text, highlight } => {
                let r = highlight.clone().expect("chunk resolves");
                assert_eq!(&text[r], "beta body");
                assert_eq!(*scroll, 3, "auto-scrolled to the highlighted line");
            }
            _ => panic!("expected Loaded"),
        }
    }

    #[test]
    fn reader_note_load_failure_is_recorded() {
        let mut p = SourcesPanel::new();
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

    #[test]
    fn handle_data_ignores_answer_ready() {
        let mut p = SourcesPanel::new();
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

    #[test]
    fn jk_moves_cursor_within_bounds() {
        let mut p = SourcesPanel::new();
        p.set_turn(
            1,
            vec![
                source("a.md", "A", 0.9, "a"),
                source("b.md", "B", 0.5, "b"),
            ],
        );
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let vault = rt.block_on(async { test_vault().await.1 });

        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx, &vault);
        assert_eq!(p.cursor, 1);
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx, &vault);
        assert_eq!(p.cursor, 1, "clamped at the last row");
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('k'))), &tx, &vault);
        assert_eq!(p.cursor, 0);
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('k'))), &tx, &vault);
        assert_eq!(p.cursor, 0, "clamped at the first row");
    }

    #[test]
    fn h_and_esc_return_to_list_face() {
        let mut p = SourcesPanel::new();
        p.set_turn(1, vec![source("a.md", "A", 0.9, "a")]);
        p.face = Face::Reader {
            source_index: 0,
            content: ReaderContent::Loading,
            scroll: 0,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let vault = rt.block_on(async { test_vault().await.1 });
        p.handle_input(&InputEvent::Key(key(KeyCode::Esc)), &tx, &vault);
        assert!(matches!(p.face, Face::List));
    }

    #[tokio::test]
    async fn open_reader_round_trips_through_a_real_vault() {
        let (_dir, vault) = test_vault().await;
        let path = VaultPath::new("note.md");
        vault.create_note(&path, "# h\nbody text\n").await.unwrap();

        let mut p = SourcesPanel::new();
        p.set_turn(1, vec![source("note.md", "h", 0.9, "body text")]);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        p.open_reader(0, &tx, &vault);
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

        #[test]
        fn render_does_not_panic_across_states_and_sizes() {
            let theme = Theme::default();
            let mut p = SourcesPanel::new();
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
    }
}
