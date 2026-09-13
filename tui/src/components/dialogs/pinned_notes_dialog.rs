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
    /// `true` while a reorder or unpin write started by this dialog, and
    /// the reload that follows it, are in flight. Core serializes the
    /// writes themselves (one lock per pin file) and anchors a move on the
    /// note's path, so overlapping writes cannot corrupt the order any
    /// more — but each write is followed by its own reload, and two reloads
    /// racing can land out of order, leaving the rows one edit behind the
    /// disk. One write-and-reload at a time (ordinary key repeat is enough
    /// to start a second) keeps the rows in step. Cleared when this
    /// dialog's own reload lands (success or failure — see
    /// `handle_loaded`); no other event may clear it.
    persist_pending: bool,
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
            persist_pending: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn rows(&self) -> &[PinnedRow] {
        &self.rows
    }

    #[cfg(test)]
    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Read the list and check each note's existence, then deliver it as
    /// [`OverlayData::PinnedNotesLoaded`] — `Err` when the read fails. Used
    /// for the first load and after every edit; the overlay host drops the
    /// event if the dialog has closed meanwhile.
    pub(crate) fn spawn_load(vault: Arc<NoteVault>, tx: &AppTx) {
        let tx = tx.clone();
        tokio::spawn(async move {
            let paths = match vault.list_pinned_notes().await {
                Ok(paths) => paths,
                Err(e) => {
                    tracing::warn!("failed to load pinned notes: {e}");
                    tx.send(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(Err(
                        format!("could not read pinned notes: {e}"),
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
            tx.send(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(Ok(
                rows,
            ))))
            .ok();
        });
    }

    /// A load or reload landed: `Ok` replaces the rows, `Err` flashes the
    /// message. Either way the dialog's own bookkeeping settles — `loaded`
    /// so a failed *first* load renders the empty-state placeholder instead
    /// of a blank body, and `persist_pending` so a failed *reload* after a
    /// write doesn't leave `J`/`K`/`d` refusing forever.
    pub fn handle_loaded(&mut self, result: &Result<Vec<PinnedRow>, String>, tx: &AppTx) {
        match result {
            Ok(rows) => self.set_rows(rows.clone()),
            Err(msg) => {
                self.loaded = true;
                self.persist_pending = false;
                tx.send(AppEvent::FlashMessage(msg.clone())).ok();
            }
        }
    }

    /// Replace the rows (a load landed). The cursor is clamped so it never
    /// points past the end after an unpin. Also clears `persist_pending`:
    /// a reload is exactly what a pending write was waiting for.
    pub fn set_rows(&mut self, rows: Vec<PinnedRow>) {
        self.rows = rows;
        self.loaded = true;
        self.persist_pending = false;
        if self.rows.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.rows.len() - 1);
        }
    }

    /// Open the row at `index` (0-based): OpenPath, or a flash when there is
    /// no such pin or its note is missing. Does not send `CloseOverlay`
    /// itself — the editor's `OpenPath` handler (`try_open_path`) already
    /// dismisses the active overlay unconditionally, so sending it here
    /// too would just be a redundant second close.
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
    }

    /// Run a persisting write, then always reload — on success so the list
    /// reflects the edit, on failure so the screen falls back to whatever
    /// is actually on disk instead of leaving an optimistic edit standing
    /// (the error itself reaches the user as a flash before the reload).
    /// Refuses to start while a previous write from this dialog is still
    /// in flight (see `persist_pending`); callers check this themselves so
    /// they can skip their own optimistic local edit too, not just the
    /// write.
    fn persist_and_reload(
        &mut self,
        tx: &AppTx,
        op: impl std::future::Future<Output = Result<(), String>> + Send + 'static,
    ) {
        self.persist_pending = true;
        let vault = self.vault.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Err(msg) = op.await {
                tx.send(AppEvent::FlashMessage(msg)).ok();
            }
            Self::spawn_load(vault, &tx);
        });
    }

    /// Move the selected row by `delta` (−1 up, +1 down), persist, reload.
    /// The cursor follows the row so a second press keeps moving it — but
    /// only once the previous move's write has landed; see
    /// `persist_pending`. The write is anchored on the row's path, not its
    /// index, so if the list changed underneath (an external edit, another
    /// process) the note the user selected still moves — or, when it is no
    /// longer pinned, nothing does and the reload shows why.
    fn move_selected(&mut self, delta: isize, tx: &AppTx) {
        if self.persist_pending || self.rows.is_empty() {
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
        let path = self.rows[to].path.clone();
        let vault = self.vault.clone();
        let flash_tx = tx.clone();
        self.persist_and_reload(tx, async move {
            match vault.move_pinned_note(&path, delta).await {
                Ok(true) => Ok(()),
                Ok(false) => {
                    flash_tx
                        .send(AppEvent::FlashMessage(format!(
                            "pinned notes changed — could not move {path}"
                        )))
                        .ok();
                    Ok(())
                }
                Err(e) => Err(format!("could not reorder pinned notes: {e}")),
            }
        });
    }

    /// Unpin the selected row, persist, reload. Refuses while a previous
    /// write is still in flight; see `persist_pending`. `unpin_note` returns
    /// `Ok(false)` when the path was no longer in the list — the list
    /// changed underneath (an external rename, another process) between the
    /// row being drawn and the keypress landing — which is flashed rather
    /// than left silent, since otherwise the keypress would just vanish.
    fn unpin_selected(&mut self, tx: &AppTx) {
        if self.persist_pending {
            return;
        }
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let path = row.path.clone();
        let vault = self.vault.clone();
        let flash_tx = tx.clone();
        self.persist_and_reload(tx, async move {
            match vault.unpin_note(&path).await {
                Ok(true) => Ok(()),
                Ok(false) => {
                    flash_tx
                        .send(AppEvent::FlashMessage(format!("not pinned: {path}")))
                        .ok();
                    Ok(())
                }
                Err(e) => Err(format!("could not unpin {path}: {e}")),
            }
        });
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        // Only unbound keys reach a modal dialog, and the combo layer drops
        // ALT — so an Alt+d chord meant for something else would arrive
        // here as a bare `d` and unpin without confirmation. Chorded keys
        // are not this dialog's; swallow them (it is modal) and do nothing.
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return EventState::Consumed;
        }
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

    /// A dialog over a vault where `names` exist and are pinned, in order,
    /// with its rows already reflecting them.
    async fn dialog_with_pins(names: &[&str]) -> (PinnedNotesDialog, Arc<NoteVault>) {
        let (mut d, vault) = dialog_with(Vec::new()).await;
        let mut rows = Vec::new();
        for n in names {
            let path = VaultPath::new(n);
            vault.create_note(&path, "hi").await.unwrap();
            vault.toggle_pinned_note(&path).await.unwrap();
            rows.push(row(n, false));
        }
        d.set_rows(rows);
        (d, vault)
    }

    /// Wait for the reload that follows a write; panics on a load failure.
    async fn wait_for_reload(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    ) -> Vec<PinnedRow> {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Some(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(Ok(rows)))) => {
                        break rows;
                    }
                    Some(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(Err(e)))) => {
                        panic!("load failed: {e}")
                    }
                    Some(_) => {}
                    None => panic!("channel closed before the reload landed"),
                }
            }
        })
        .await
        .expect("reload event")
    }

    fn paths(rows: &[PinnedRow]) -> Vec<VaultPath> {
        rows.iter().map(|r| r.path.clone()).collect()
    }

    fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>) -> Vec<AppEvent> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    #[tokio::test]
    async fn digit_opens_that_note() {
        let (mut d, _) = dialog_with(vec![row("a.md", false), row("b.md", false)]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('2')), &tx);
        let events = drain(&mut rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                AppEvent::OpenPath { path, emphasis: None } if *path == VaultPath::new("b.md")
            )),
            "expected OpenPath, got {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, AppEvent::CloseOverlay)),
            "open_row must not emit CloseOverlay; editor's OpenPath handler closes the overlay, got {events:?}"
        );
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
        assert!(!events.iter().any(|e| matches!(e, AppEvent::CloseOverlay)));
    }

    #[tokio::test]
    async fn digit_for_a_missing_note_flashes_instead_of_opening() {
        // The digit path and the Enter path both funnel through `open_row`,
        // but nothing stops a future refactor from special-casing the digit
        // arm — so the refusal is exercised on both paths, not just Enter's.
        let (mut d, _) = dialog_with(vec![row("a.md", false), row("gone.md", true)]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('2')), &tx);
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
        assert!(!events.iter().any(|e| matches!(e, AppEvent::CloseOverlay)));
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
        let (mut d, vault) = dialog_with_pins(&["a.md", "b.md"]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(shift('J'), &tx);
        // The move + reload run on a spawned task; wait for the reload event.
        let loaded = wait_for_reload(&mut rx).await;
        assert_eq!(
            paths(&loaded),
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
    async fn shift_k_moves_the_row_up_and_reloads() {
        let (mut d, vault) = dialog_with_pins(&["a.md", "b.md"]).await;
        d.selected = 1;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(shift('K'), &tx);
        let loaded = wait_for_reload(&mut rx).await;
        assert_eq!(
            paths(&loaded),
            vec![VaultPath::new("/b.md"), VaultPath::new("/a.md")]
        );
        assert_eq!(
            vault.list_pinned_notes().await.unwrap(),
            vec![VaultPath::new("/b.md"), VaultPath::new("/a.md")]
        );
        d.set_rows(loaded);
        assert_eq!(d.selected, 0, "the cursor follows the moved row");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn d_unpins_the_selected_row_and_reloads() {
        let (mut d, vault) = dialog_with_pins(&["a.md", "b.md"]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('d')), &tx);
        let loaded = wait_for_reload(&mut rx).await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].path, VaultPath::new("/b.md"));
        assert_eq!(
            vault.list_pinned_notes().await.unwrap(),
            vec![VaultPath::new("/b.md")]
        );
    }

    /// Only unbound keys reach the dialog, and the combo layer drops ALT —
    /// so a chord like Alt+d would otherwise land here as a bare `d` and
    /// unpin (irreversibly, position lost) with no confirmation.
    #[tokio::test]
    async fn chorded_keys_never_unpin_or_reorder() {
        let (mut d, vault) = dialog_with_pins(&["a.md", "b.md"]).await;
        let (tx, mut rx) = unbounded_channel();
        for m in [KeyModifiers::ALT, KeyModifiers::CONTROL] {
            d.handle_key(KeyEvent::new(KeyCode::Char('d'), m), &tx);
            d.handle_key(KeyEvent::new(KeyCode::Delete, m), &tx);
            d.handle_key(
                KeyEvent::new(KeyCode::Char('J'), m | KeyModifiers::SHIFT),
                &tx,
            );
        }
        assert!(
            drain(&mut rx).is_empty(),
            "a chorded key must start no write and no reload"
        );
        assert!(!d.persist_pending);
        assert_eq!(
            vault.list_pinned_notes().await.unwrap(),
            vec![VaultPath::new("/a.md"), VaultPath::new("/b.md")]
        );
    }

    /// The rows the user is looking at can be stale: another process (or a
    /// sync merge) unpinned the first note after the dialog loaded. `J` on
    /// that row must not move some *other* note by index — it targets the
    /// note the user selected, which is gone, so nothing moves and the
    /// reload shows the real list.
    #[tokio::test(flavor = "multi_thread")]
    async fn reorder_of_a_stale_row_moves_nothing_else() {
        let (mut d, vault) = dialog_with_pins(&["a.md", "b.md", "c.md"]).await;
        // The list changes underneath: a.md is unpinned outside the dialog.
        vault.unpin_note(&VaultPath::new("a.md")).await.unwrap();
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(shift('J'), &tx);
        let loaded = wait_for_reload(&mut rx).await;
        assert_eq!(
            paths(&loaded),
            vec![VaultPath::new("/b.md"), VaultPath::new("/c.md")],
            "b and c must keep their order"
        );
    }

    /// `unpin_note` returns `Ok(false)` when the row's path is no longer in
    /// the stored list — e.g. the list changed underneath between the row
    /// being drawn and the keypress landing. The dialog still reloads (there
    /// is nothing to persist), but the keypress must not just vanish: it
    /// flashes instead.
    #[tokio::test(flavor = "multi_thread")]
    async fn unpin_selected_flashes_when_the_row_is_already_gone() {
        // Nothing is pinned in the vault, so the dialog's row for "a.md" is
        // stale by construction.
        let (mut d, _vault) = dialog_with(vec![row("a.md", false)]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('d')), &tx);

        let mut flashed = None;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                match rx.recv().await {
                    Some(AppEvent::FlashMessage(m)) => flashed = Some(m),
                    Some(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(Ok(_)))) => break,
                    Some(AppEvent::OverlayData(OverlayData::PinnedNotesLoaded(Err(e)))) => {
                        panic!("unexpected load failure: {e}")
                    }
                    Some(_) => {}
                    None => panic!("channel closed before the reload landed"),
                }
            }
        })
        .await
        .expect("reload event");

        assert_eq!(flashed.as_deref(), Some("not pinned: a.md"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_reorder_press_is_ignored_while_a_write_is_in_flight() {
        // Without the `persist_pending` guard, the second `J` below would
        // run its own optimistic swap immediately (both key presses happen
        // synchronously, before the first press's spawned write has had a
        // chance to run) and start a second write-and-reload whose reload
        // can land before the first's, leaving the rows one edit behind
        // the disk. With the guard, the second press is a no-op: the cursor
        // stays where the first press left it, and only one write (and
        // therefore one reload) happens.
        let (mut d, _vault) = dialog_with_pins(&["a.md", "b.md", "c.md"]).await;
        let (tx, mut rx) = unbounded_channel();

        d.handle_key(shift('J'), &tx);
        assert!(d.persist_pending, "the first press started a write");

        d.handle_key(shift('J'), &tx);
        assert_eq!(
            d.selected, 1,
            "the second press must not move the cursor again while the first write is pending"
        );

        let loaded = wait_for_reload(&mut rx).await;
        assert_eq!(
            paths(&loaded),
            vec![
                VaultPath::new("/b.md"),
                VaultPath::new("/a.md"),
                VaultPath::new("/c.md")
            ],
            "only the first press's move should have been written"
        );
        // Deliver the reload, exactly as `dialogs/mod.rs` would: this is
        // what clears the guard.
        d.set_rows(loaded);
        assert!(!d.persist_pending, "the reload cleared the guard");

        // No second write means no second reload: nothing else should ever
        // arrive on this channel.
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
                .await
                .is_err(),
            "a second write must not have happened"
        );
    }

    /// A failed reload must settle the dialog (flash, clear the in-flight
    /// guard) or `J`/`K`/`d` would refuse forever.
    #[tokio::test]
    async fn a_failed_load_flashes_and_clears_the_guard() {
        let (mut d, _) = dialog_with_pins(&["a.md", "b.md"]).await;
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(shift('J'), &tx);
        assert!(d.persist_pending);
        d.handle_loaded(&Err("boom".to_string()), &tx);
        assert!(!d.persist_pending);
        assert!(d.is_loaded());
        assert!(
            drain(&mut rx)
                .iter()
                .any(|e| matches!(e, AppEvent::FlashMessage(m) if m == "boom")),
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
