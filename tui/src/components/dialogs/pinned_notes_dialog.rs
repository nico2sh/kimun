//! The pinned-notes dialog: the vault's pinned notes by number, and where
//! they are managed. A digit opens that note at once (no query input — nine
//! rows never need filtering); `j`/`k`/↑/↓ move, Enter opens the selected
//! row, `J`/`K` move it down/up, `d`/Delete unpins it, Esc closes. Every
//! change is written as it is made and the list reloads through
//! [`OverlayData::PinnedNotesLoaded`]; there is no commit or cancel step.
//! A pin whose note is missing on disk is kept, drawn dimmed with
//! "(missing)", and refuses to open with a flash.

use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, OverlayData, PinnedRow};
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::settings::themes::Theme;

const OUTER_WIDTH: u16 = 60;
/// Body rows: the cap, so the popup never resizes as pins come and go.
const BODY_ROWS: u16 = kimun_core::PINNED_NOTES_CAP as u16;

pub struct PinnedNotesDialog {
    vault: Arc<NoteVault>,
    rows: Vec<PinnedRow>,
    /// Cursor row; always `< rows.len()` unless the list is empty.
    pub(crate) selected: usize,
    /// `false` until the first load lands, so an empty vault is not drawn
    /// as "no pinned notes" for the frame before the read completes.
    loaded: bool,
}

impl PinnedNotesDialog {
    /// Build the dialog and kick off the first load.
    pub fn new(vault: Arc<NoteVault>, tx: &AppTx) -> Self {
        Self::spawn_load(vault.clone(), tx);
        Self {
            vault,
            rows: Vec::new(),
            selected: 0,
            loaded: false,
        }
    }

    /// Read the list and check each note's existence, then deliver it as
    /// [`OverlayData::PinnedNotesLoaded`]. Used for the first load and
    /// after every edit; the overlay host drops the event if the dialog
    /// has closed meanwhile.
    pub fn spawn_load(vault: Arc<NoteVault>, tx: &AppTx) {
        let tx = tx.clone();
        tokio::spawn(async move {
            let paths = match vault.list_pinned_notes().await {
                Ok(paths) => paths,
                Err(e) => {
                    tracing::warn!("failed to load pinned notes: {e}");
                    tx.send(AppEvent::OverlayData(OverlayData::Error(format!(
                        "could not read pinned notes: {e}"
                    ))))
                    .ok();
                    return;
                }
            };
            let mut rows = Vec::with_capacity(paths.len());
            for path in paths {
                let missing = !vault.exists(&path).await;
                rows.push(PinnedRow { path, missing });
            }
            tx.send(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(rows)))
                .ok();
        });
    }

    /// Replace the rows (a load landed). The cursor is clamped so it never
    /// points past the end after an unpin.
    pub fn set_rows(&mut self, rows: Vec<PinnedRow>) {
        self.rows = rows;
        self.loaded = true;
        if self.rows.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.rows.len() - 1);
        }
    }

    /// Open the row at `index` (0-based): OpenPath + close, or a flash when
    /// there is no such pin or its note is missing.
    fn open_row(&self, index: usize, tx: &AppTx) {
        let Some(row) = self.rows.get(index) else {
            tx.send(AppEvent::FlashMessage(format!(
                "no pinned note {}",
                index + 1
            )))
            .ok();
            return;
        };
        if row.missing {
            tx.send(AppEvent::FlashMessage(format!(
                "pinned note not found: {}",
                row.path
            )))
            .ok();
            return;
        }
        tx.send(AppEvent::OpenPath {
            path: row.path.clone(),
            emphasis: None,
        })
        .ok();
        tx.send(AppEvent::CloseOverlay).ok();
    }

    /// Move the selected row by `delta` (−1 up, +1 down), persist, reload.
    /// The cursor follows the row so a second press keeps moving it.
    fn move_selected(&mut self, delta: isize, tx: &AppTx) {
        if self.rows.is_empty() {
            return;
        }
        let from = self.selected;
        let to = from as isize + delta;
        if to < 0 || to as usize >= self.rows.len() {
            return;
        }
        let to = to as usize;
        self.rows.swap(from, to);
        self.selected = to;
        let vault = self.vault.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            match vault.move_pinned_note(from, to).await {
                Ok(_) => Self::spawn_load(vault, &tx2),
                Err(e) => {
                    tx2.send(AppEvent::OverlayData(OverlayData::Error(format!(
                        "could not reorder pinned notes: {e}"
                    ))))
                    .ok();
                }
            }
        });
    }

    /// Unpin the selected row, persist, reload.
    fn unpin_selected(&mut self, tx: &AppTx) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let path = row.path.clone();
        let vault = self.vault.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            match vault.unpin_note(&path).await {
                Ok(_) => Self::spawn_load(vault, &tx2),
                Err(e) => {
                    tx2.send(AppEvent::OverlayData(OverlayData::Error(format!(
                        "could not unpin {path}: {e}"
                    ))))
                    .ok();
                }
            }
        });
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char(c @ '1'..='9') if !shift => {
                self.open_row((c as u8 - b'1') as usize, tx);
            }
            KeyCode::Enter => self.open_row(self.selected, tx),
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.rows.is_empty() {
                    self.selected = (self.selected + 1).min(self.rows.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Char('J') => self.move_selected(1, tx),
            KeyCode::Char('K') => self.move_selected(-1, tx),
            KeyCode::Char('d') | KeyCode::Delete => self.unpin_selected(tx),
            KeyCode::Esc => {
                tx.send(AppEvent::CloseOverlay).ok();
            }
            _ => {}
        }
        EventState::Consumed
    }
}

impl crate::components::Component for PinnedNotesDialog {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        if let InputEvent::Key(key) = event {
            self.handle_key(*key, tx)
        } else {
            EventState::NotConsumed
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        // body rows + borders(2) + footer(1)
        let outer_height = BODY_ROWS + 3;
        let popup = super::fixed_centered_rect(OUTER_WIDTH, outer_height, rect);
        let inner = modal_chrome(
            f,
            popup,
            theme,
            ModalSpec {
                title: Some(" Pinned notes "),
                border: Some(Style::default().fg(theme.fg.to_ratatui())),
                ..Default::default()
            },
        );
        if inner.height < 2 {
            return;
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let body = chunks[0];
        let footer_area = chunks[1];

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();
        let fg_sel = theme.selection_fg.to_ratatui();
        let bg_sel = theme.selection_bg.to_ratatui();

        if self.loaded && self.rows.is_empty() {
            f.render_widget(
                Paragraph::new("  No pinned notes yet — leader m i pins the open note")
                    .style(Style::default().fg(gray).bg(bg)),
                Rect {
                    x: body.x,
                    y: body.y,
                    width: body.width,
                    height: 1,
                },
            );
        }

        for (i, row) in self.rows.iter().enumerate() {
            let y = body.y + i as u16;
            if y >= body.y + body.height {
                break;
            }
            let selected = i == self.selected;
            let style = if selected {
                Style::default()
                    .fg(fg_sel)
                    .bg(bg_sel)
                    .add_modifier(Modifier::BOLD)
            } else if row.missing {
                Style::default().fg(gray).bg(bg)
            } else {
                Style::default().fg(fg).bg(bg)
            };
            let marker = if selected { ">" } else { " " };
            let name = row.path.get_clean_name();
            let suffix = if row.missing { "  (missing)" } else { "" };
            let text = format!(" {marker} {}  {name}{suffix}   {}", i + 1, row.path);
            f.render_widget(
                Paragraph::new(text).style(style),
                Rect {
                    x: body.x,
                    y,
                    width: body.width,
                    height: 1,
                },
            );
        }

        f.render_widget(
            Paragraph::new("  [1-9/Enter] Open  [j/k] Move  [J/K] Reorder  [d] Unpin  [Esc] Close")
                .style(Style::default().fg(gray).bg(bg)),
            footer_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::AppEvent;
    use kimun_core::nfs::VaultPath;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn shift(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }

    fn row(path: &str, missing: bool) -> PinnedRow {
        PinnedRow {
            path: VaultPath::new(path),
            missing,
        }
    }

    async fn dialog_with(rows: Vec<PinnedRow>) -> (PinnedNotesDialog, Arc<NoteVault>) {
        let vault = crate::test_support::temp_vault("pinned-dialog").await;
        vault.validate_and_init().await.unwrap();
        let (tx, _rx) = unbounded_channel();
        let mut d = PinnedNotesDialog::new(vault.clone(), &tx);
        d.set_rows(rows);
        (d, vault)
    }

    fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>) -> Vec<AppEvent> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    #[tokio::test]
    async fn digit_opens_that_note_and_closes() {
        let (mut d, _) = dialog_with(vec![row("a.md", false), row("b.md", false)]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('2')), &tx);
        let events = drain(&mut rx);
        assert!(matches!(
            &events[0],
            AppEvent::OpenPath { path, emphasis: None } if *path == VaultPath::new("b.md")
        ));
        assert!(matches!(events[1], AppEvent::CloseOverlay));
    }

    #[tokio::test]
    async fn digit_past_the_end_flashes() {
        let (mut d, _) = dialog_with(vec![row("a.md", false)]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('5')), &tx);
        let events = drain(&mut rx);
        assert!(matches!(&events[0], AppEvent::FlashMessage(m) if m == "no pinned note 5"));
        assert!(!events.iter().any(|e| matches!(e, AppEvent::CloseOverlay)));
    }

    #[tokio::test]
    async fn missing_note_flashes_instead_of_opening() {
        let (mut d, _) = dialog_with(vec![row("gone.md", true)]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Enter), &tx);
        let events = drain(&mut rx);
        assert!(matches!(
            &events[0],
            AppEvent::FlashMessage(m) if m == "pinned note not found: gone.md"
        ));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AppEvent::OpenPath { .. }))
        );
    }

    #[tokio::test]
    async fn j_k_move_the_cursor_within_bounds() {
        let (mut d, _) = dialog_with(vec![row("a.md", false), row("b.md", false)]).await;
        let (tx, _rx) = unbounded_channel();
        assert_eq!(d.selected, 0);
        d.handle_key(key(KeyCode::Char('j')), &tx);
        assert_eq!(d.selected, 1);
        d.handle_key(key(KeyCode::Down), &tx);
        assert_eq!(d.selected, 1, "clamped at the last row");
        d.handle_key(key(KeyCode::Char('k')), &tx);
        d.handle_key(key(KeyCode::Up), &tx);
        assert_eq!(d.selected, 0, "clamped at the first row");
    }

    #[tokio::test]
    async fn esc_closes_without_opening() {
        let (mut d, _) = dialog_with(vec![row("a.md", false)]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Esc), &tx);
        let events = drain(&mut rx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], AppEvent::CloseOverlay));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shift_j_moves_the_row_down_and_reloads() {
        let (mut d, vault) = dialog_with(Vec::new()).await;
        for n in ["a.md", "b.md"] {
            vault.toggle_pinned_note(&VaultPath::new(n)).await.unwrap();
        }
        d.set_rows(vec![row("a.md", true), row("b.md", true)]);
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(shift('J'), &tx);
        // The move + reload run on a spawned task; wait for the reload event.
        let loaded = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(rows))) =
                    rx.recv().await
                {
                    break rows;
                }
            }
        })
        .await
        .expect("reload event");
        assert_eq!(
            loaded.iter().map(|r| r.path.clone()).collect::<Vec<_>>(),
            vec![VaultPath::new("/b.md"), VaultPath::new("/a.md")]
        );
        assert_eq!(
            vault.list_pinned_notes().await.unwrap(),
            vec![VaultPath::new("/b.md"), VaultPath::new("/a.md")]
        );
        d.set_rows(loaded);
        assert_eq!(d.selected, 1, "the cursor follows the moved row");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn d_unpins_the_selected_row_and_reloads() {
        let (mut d, vault) = dialog_with(Vec::new()).await;
        for n in ["a.md", "b.md"] {
            vault.toggle_pinned_note(&VaultPath::new(n)).await.unwrap();
        }
        d.set_rows(vec![row("a.md", true), row("b.md", true)]);
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('d')), &tx);
        let loaded = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(rows))) =
                    rx.recv().await
                {
                    break rows;
                }
            }
        })
        .await
        .expect("reload event");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, VaultPath::new("/b.md"));
        assert_eq!(
            vault.list_pinned_notes().await.unwrap(),
            vec![VaultPath::new("/b.md")]
        );
    }

    #[tokio::test]
    async fn set_rows_clamps_the_cursor() {
        let (mut d, _) = dialog_with(vec![row("a.md", false), row("b.md", false)]).await;
        d.selected = 1;
        d.set_rows(vec![row("a.md", false)]);
        assert_eq!(d.selected, 0);
        d.set_rows(Vec::new());
        assert_eq!(d.selected, 0);
    }
}
