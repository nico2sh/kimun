use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use tokio::task::JoinHandle;

use crate::components::Component;
use crate::components::dialogs::ValidationState;
use crate::components::dialogs::directory_picker::{DirectoryPicker, PickerEvent};
use crate::components::event_state::EventState;
use crate::components::hint_row::HintRow;
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::logic::file_ops;
use crate::logic::message::{AppMessage, AppTx, OverlayData};
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// MoveDialog
// ---------------------------------------------------------------------------

/// Modal dialog that lets the user move a note or directory to a different
/// directory inside the vault.
///
/// The destination list is a [`DirectoryPicker`]; a background task loads the
/// vault's directories and the dialog checks the highlighted destination for
/// a name clash.
pub struct MoveDialog {
    /// The vault path being moved.
    pub path: VaultPath,
    /// Shared reference to the vault.
    pub vault: Arc<NoteVault>,
    /// Pre-computed `"  {path}"` for zero-allocation rendering.
    pub path_display: String,
    /// The destination filter and list.
    pub picker: DirectoryPicker,
    /// Result of the most-recent destination existence check.
    pub dest_validation: ValidationState,
    /// Handle to the running validation task so we can abort it on selection change.
    pub validation_task: Option<JoinHandle<()>>,
    /// Optional error message surfaced from a failed move attempt.
    pub error: Option<String>,
    /// Why the highlighted destination is refused (the entry itself or
    /// inside it); shown in the status row, and Enter does nothing while it
    /// is set.
    pub refusal: Option<String>,
    hints: HintRow,
}

impl MoveDialog {
    /// Create a new `MoveDialog` for `path`.
    ///
    /// Directory loading starts immediately in a background task.
    pub fn new(path: VaultPath, vault: Arc<NoteVault>, tx: &AppTx) -> Self {
        let path_display = format!("  {}", path);
        let dialog = Self {
            path,
            vault,
            path_display,
            picker: DirectoryPicker::new().bordered(),
            dest_validation: ValidationState::Idle,
            validation_task: None,
            error: None,
            refusal: None,
            hints: super::confirm_hints("Move here"),
        };
        dialog.schedule_load(tx);
        dialog
    }

    /// Spawn a background task that retrieves all vault directories and sends
    /// the result as [`OverlayData::MoveDirectoriesLoaded`].
    fn schedule_load(&self, tx: &AppTx) {
        let vault = Arc::clone(&self.vault);
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || file_ops::list_directories(&vault))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            tx.send(AppMessage::OverlayData(OverlayData::MoveDirectoriesLoaded(
                result,
            )))
            .ok();
        });
    }

    /// The directories arrived (or could not be listed).
    pub fn on_directories(&mut self, directories: Result<Vec<VaultPath>, String>, tx: &AppTx) {
        self.picker.set_directories(directories, &VaultPath::root());
        self.spawn_validation(tx);
    }

    // -----------------------------------------------------------------------
    // Destination validation helpers
    // -----------------------------------------------------------------------

    /// Abort any in-flight validation task and start a new one for the
    /// currently selected directory.  The result is sent as
    /// [`OverlayData::MoveDestValidation`].  Resets to `Idle` when nothing is selected.
    pub fn spawn_validation(&mut self, tx: &AppTx) {
        if let Some(handle) = self.validation_task.take() {
            handle.abort();
        }

        self.refusal = None;
        let Some(dest_dir) = self.picker.highlighted().cloned() else {
            self.dest_validation = ValidationState::Idle;
            return;
        };

        let candidate = match file_ops::move_target(&self.path, &dest_dir) {
            Ok(candidate) => candidate,
            Err(reason) => {
                self.refusal = Some(reason);
                self.dest_validation = ValidationState::Idle;
                return;
            }
        };

        let vault = Arc::clone(&self.vault);
        let tx_clone = tx.clone();

        let handle = tokio::spawn(async move {
            let available = file_ops::availability(&vault, &candidate).await.is_free();
            tx_clone
                .send(AppMessage::OverlayData(OverlayData::MoveDestValidation {
                    available,
                }))
                .ok();
        });

        self.validation_task = Some(handle);
        self.dest_validation = ValidationState::Pending;
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    /// Clicking a destination selects it, the wheel moves the selection, and
    /// a click on a hint chip runs its key. Modal: every mouse event is
    /// consumed.
    pub fn handle_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        if let Some(key) = self.hints.hit(m) {
            return self.handle_key(key, tx);
        }
        let before = self.picker.highlighted().cloned();
        if let PickerEvent::Chosen(dir) = self.picker.handle_mouse(m) {
            self.picker.select(&dir);
        }
        self.revalidate_if_moved(before, tx);
        EventState::Consumed
    }

    /// The destination changed (or may have): check the new one.
    fn revalidate_if_moved(&mut self, before: Option<VaultPath>, tx: &AppTx) {
        if self.picker.highlighted() != before.as_ref() {
            self.spawn_validation(tx);
        }
    }

    /// Handle a raw [`KeyEvent`].  Returns [`EventState::Consumed`]: the
    /// dialog is modal.
    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        let before = self.picker.highlighted().cloned();
        match self.picker.handle_key(key) {
            PickerEvent::Chosen(dest) => {
                let target = file_ops::move_target(&self.path, &dest);
                if let (true, Ok(new_path)) =
                    (self.dest_validation != ValidationState::Taken, target)
                {
                    let from = self.path.clone();
                    let vault = Arc::clone(&self.vault);
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        // A move is a cross-directory rename; core classifies the
                        // entry and routes to the right rename (note / directory
                        // / attachment).
                        let msg = match file_ops::move_entry(&vault, &from, &new_path).await {
                            Ok(op) => AppMessage::FileOp(op),
                            Err(e) => AppMessage::OverlayData(OverlayData::Error(e)),
                        };
                        tx2.send(msg).ok();
                    });
                }
            }
            PickerEvent::Back => {
                tx.send(AppMessage::CloseOverlay).ok();
            }
            PickerEvent::None => self.revalidate_if_moved(before, tx),
        }
        EventState::Consumed
    }
}

// ---------------------------------------------------------------------------
// Component trait
// ---------------------------------------------------------------------------

impl Component for MoveDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let popup_area = crate::components::centered_rect(50, 60, rect);

        let inner = modal_chrome(
            f,
            popup_area,
            theme,
            ModalSpec {
                title: Some(" Move "),
                border: Some(Style::default().fg(theme.fg.to_ratatui())),
                ..Default::default()
            },
        );

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();

        // ── Vertical layout inside the block ─────────────────────────────────
        //
        // Row 0: "MOVING" label (muted)
        // Row 1: source path value
        // Row 2: spacer
        // Row 3: "DESTINATION" label (muted)
        // Row 4: destination filter and directory list (fills available space)
        // Row 5: validation status
        // Row 6: hint line
        // Row 7 (optional): error line

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: "MOVING" label
                Constraint::Length(1), // 1: source path
                Constraint::Length(1), // 2: spacer
                Constraint::Length(1), // 3: "DESTINATION" label
                Constraint::Min(6),    // 4: filter + list
                Constraint::Length(1), // 5: validation status
                Constraint::Length(1), // 6: hint line
                Constraint::Length(if self.error.is_some() { 1 } else { 0 }), // 7: error
            ])
            .split(inner);

        // Row 0: "MOVING" label.
        f.render_widget(
            Paragraph::new("  MOVING").style(Style::default().fg(gray).bg(bg)),
            rows[0],
        );

        // Row 1: source path.
        super::render_path_row(f, rows[1], &self.path_display, fg, bg);

        // Row 2: blank spacer — nothing to render.

        // Row 3: "DESTINATION" label.
        f.render_widget(
            Paragraph::new("  DESTINATION").style(Style::default().fg(gray).bg(bg)),
            rows[3],
        );

        // Row 4: destination filter and list.
        self.picker.render(f, rows[4], theme);

        // Row 5: validation status.
        match &self.refusal {
            // Why the destination is refused, where the status would be.
            Some(reason) => f.render_widget(
                Paragraph::new(format!("  {reason}"))
                    .style(Style::default().fg(theme.red.to_ratatui()).bg(bg)),
                rows[5],
            ),
            None => super::render_validation_status(f, rows[5], self.dest_validation, theme),
        }

        // Row 6: hint line.  Dim Enter when there's no valid selection.
        // Enter moves unless the destination is Taken or refused — also while
        // the check is still Idle/Pending — so the chip must too.
        let enter_acts = self.dest_validation != ValidationState::Taken
            && self.refusal.is_none()
            && self.picker.highlighted().is_some();
        super::render_confirm_hints(
            f,
            rows[6],
            &mut self.hints,
            self.dest_validation == ValidationState::Available,
            enter_acts,
            fg,
            theme,
        );

        // Row 7 (optional): error message.
        if let Some(msg) = &self.error {
            super::render_error_row(f, rows[7], msg, theme);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

impl_dialog!(MoveDialog, error);

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::VaultConfig;
    use tokio::sync::mpsc;

    /// Compile-time smoke test: verify that the struct fields and key types
    /// are accessible without needing a real vault.
    #[test]
    fn struct_fields_accessible() {
        // Verify the `error` field exists and is `Option<String>`.
        fn _check_error_field(d: &MoveDialog) -> Option<&String> {
            d.error.as_ref()
        }
        // Verify the picker exposes the highlighted destination.
        fn _check_highlighted(d: &MoveDialog) -> Option<&VaultPath> {
            d.picker.highlighted()
        }
    }

    /// Pressing `Esc` must send `AppMessage::CloseOverlay` and return
    /// `EventState::Consumed`, without requiring a real vault.
    #[test]
    fn esc_sends_close_dialog() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir().join("kimun_move_esc_test");
            std::fs::create_dir_all(&tmp).unwrap();

            let vault_result =
                NoteVault::new(VaultConfig::new(crate::test_support::sys(tmp))).await;
            let Ok(vault) = vault_result else {
                // No vault available in CI — skip gracefully.
                return;
            };

            let vault = Arc::new(vault);
            let (tx, mut rx) = mpsc::unbounded_channel::<AppMessage>();
            let mut dialog = MoveDialog::new(VaultPath::new("notes/test.md"), vault, &tx);

            let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
            // Drain the channel — background tasks (e.g. MoveDirectoriesLoaded)
            // may have sent events before or after the Esc key was processed.
            let mut found = false;
            while let Ok(event) = rx.try_recv() {
                if matches!(event, AppMessage::CloseOverlay) {
                    found = true;
                    break;
                }
            }
            assert!(found, "expected AppMessage::CloseOverlay in channel");
        });
    }

    /// A new `MoveDialog` must start with no error and nothing highlighted.
    ///
    /// NOTE: gated `#[ignore]` because constructing `NoteVault` requires a
    /// real SQLite database on disk.  Run explicitly with:
    ///
    /// ```text
    /// cargo test -- --ignored move_dialog::tests::new_initial_state
    /// ```
    #[tokio::test]
    #[ignore = "requires a real vault directory with kimun.sqlite"]
    async fn new_initial_state() {
        use std::path::PathBuf;

        let tmp = std::env::temp_dir().join("kimun_move_test_vault");
        std::fs::create_dir_all(&tmp).unwrap();

        let vault = Arc::new(
            NoteVault::new(VaultConfig::new(crate::test_support::sys(PathBuf::from(
                &tmp,
            ))))
            .await
            .expect("vault creation failed"),
        );

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<AppMessage>();
        let path = VaultPath::new("notes/projects/kimun.md");
        let dialog = MoveDialog::new(path, vault, &tx);

        assert!(dialog.error.is_none());
        // Directory load is async: nothing is highlighted until it lands.
        assert!(dialog.picker.highlighted().is_none());
    }

    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    async fn dialog_with_dirs() -> (MoveDialog, AppTx, mpsc::UnboundedReceiver<AppMessage>) {
        let vault = crate::test_support::temp_vault("move-dialog").await;
        for d in ["/a", "/a/sub", "/b"] {
            vault.create_directory(&VaultPath::new(d)).await.unwrap();
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut d = MoveDialog::new(VaultPath::new("/a"), vault, &tx);
        let dirs = loop {
            if let AppMessage::OverlayData(OverlayData::MoveDirectoriesLoaded(r)) =
                next(&mut rx).await
            {
                break r;
            }
        };
        d.on_directories(dirs, &tx);
        (d, tx, rx)
    }

    async fn next(rx: &mut mpsc::UnboundedReceiver<AppMessage>) -> AppMessage {
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for a message")
            .expect("channel closed")
    }

    async fn pick(d: &mut MoveDialog, dir: &str, tx: &AppTx) {
        d.picker.select(&VaultPath::new(dir));
        assert_eq!(d.picker.highlighted(), Some(&VaultPath::new(dir)));
        d.spawn_validation(tx);
    }

    async fn settle(d: &mut MoveDialog, rx: &mut mpsc::UnboundedReceiver<AppMessage>) {
        loop {
            if let AppMessage::OverlayData(OverlayData::MoveDestValidation { available }) =
                next(rx).await
            {
                d.dest_validation = if available {
                    ValidationState::Available
                } else {
                    ValidationState::Taken
                };
                d.validation_task = None;
                return;
            }
        }
    }

    #[tokio::test]
    async fn moving_a_directory_into_itself_or_below_shows_why_and_blocks_enter() {
        for dest in ["/a", "/a/sub"] {
            let (mut d, tx, mut rx) = dialog_with_dirs().await;
            pick(&mut d, dest, &tx).await;
            assert!(
                matches!(&d.refusal, Some(r) if r == "Cannot move a directory into itself"),
                "{dest}"
            );
            assert_eq!(d.dest_validation, ValidationState::Idle);
            d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            while let Ok(m) = rx.try_recv() {
                assert!(
                    !matches!(
                        m,
                        AppMessage::FileOp(_) | AppMessage::OverlayData(OverlayData::Error(_))
                    ),
                    "{dest}: a move ran"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_valid_destination_is_checked_and_enter_moves() {
        let (mut d, tx, mut rx) = dialog_with_dirs().await;
        pick(&mut d, "/b", &tx).await;
        assert!(d.refusal.is_none());
        assert_eq!(d.dest_validation, ValidationState::Pending);
        settle(&mut d, &mut rx).await;
        assert_eq!(d.dest_validation, ValidationState::Available);
        d.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);
        loop {
            match next(&mut rx).await {
                AppMessage::FileOp(_) => break,
                AppMessage::OverlayData(OverlayData::Error(e)) => panic!("move failed: {e}"),
                _ => {}
            }
        }
    }
}
