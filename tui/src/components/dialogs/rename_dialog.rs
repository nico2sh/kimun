use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use tokio::task::JoinHandle;

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// ValidationState
// ---------------------------------------------------------------------------

/// Tracks the current state of the async name-availability check.
pub enum ValidationState {
    /// No check has been triggered yet (initial state).
    Idle,
    /// A check is in progress.
    Pending,
    /// The chosen name is available (does not already exist).
    Available,
    /// The chosen name is already taken.
    Taken,
}

// ---------------------------------------------------------------------------
// RenameDialog
// ---------------------------------------------------------------------------

/// Modal dialog that lets the user rename a note or directory.
///
/// The input is pre-filled with the current filename.  As the user types,
/// an async task checks whether the new name already exists in the vault and
/// updates `validation_state` accordingly.  Pressing `Enter` while the name
/// is `Available` triggers the actual rename operation.
pub struct RenameDialog {
    /// The vault path being renamed.
    pub path: VaultPath,
    /// Shared reference to the vault for existence checks and the rename op.
    pub vault: Arc<NoteVault>,
    /// Current text in the input field.
    pub input: String,
    /// Result of the most-recent validation check.
    pub validation_state: ValidationState,
    /// Handle to the running validation task so we can abort it on new input.
    pub validation_task: Option<JoinHandle<()>>,
    /// Receiver end of the one-shot channel used by the validation task.
    pub validation_rx: Option<std::sync::mpsc::Receiver<bool>>,
    /// Channel for sending app-level events (rename result, errors, close).
    pub tx: AppTx,
    /// Optional error message surfaced from a failed rename attempt.
    pub error: Option<String>,
}

impl RenameDialog {
    /// Create a new `RenameDialog` for `path`.
    ///
    /// The input field is pre-filled with the filename component of `path`.
    pub fn new(path: VaultPath, vault: Arc<NoteVault>, tx: AppTx) -> Self {
        let (_, filename) = path.get_parent_path();
        Self {
            path,
            vault,
            input: filename,
            validation_state: ValidationState::Idle,
            validation_task: None,
            validation_rx: None,
            tx,
            error: None,
        }
    }

    // -----------------------------------------------------------------------
    // Validation helpers
    // -----------------------------------------------------------------------

    /// Poll the validation channel (non-blocking).  Call this at the start of
    /// every `render()` so the UI reflects the latest result.
    fn poll_validation(&mut self) {
        let Some(rx) = &self.validation_rx else { return };
        if let Ok(available) = rx.try_recv() {
            self.validation_state = if available {
                ValidationState::Available
            } else {
                ValidationState::Taken
            };
            self.validation_rx = None;
            self.validation_task = None;
        }
    }

    /// Abort any in-flight validation task and spawn a new one for the
    /// current value of `self.input`.
    fn spawn_validation(&mut self) {
        // Abort the previous task if it is still running.
        if let Some(handle) = self.validation_task.take() {
            handle.abort();
        }

        let vault = Arc::clone(&self.vault);
        let input = self.input.clone();
        let path = self.path.clone();
        let (vtx, vrx) = std::sync::mpsc::channel();

        let handle = tokio::spawn(async move {
            let parent = path.get_parent_path().0;
            let candidate = if path.is_note() {
                parent.append(&VaultPath::note_path_from(&input))
            } else {
                parent.append(&VaultPath::new(&input))
            };
            let exists = vault.exists(&candidate).await.is_some();
            // `true` means the name is *available* (does not exist yet).
            vtx.send(!exists).ok();
        });

        self.validation_task = Some(handle);
        self.validation_rx = Some(vrx);
        self.validation_state = ValidationState::Pending;
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    /// Handle a raw [`KeyEvent`].  Returns [`EventState::Consumed`] for keys
    /// this dialog acts on; callers should forward only key events.
    pub fn handle_input(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Char(c) => {
                self.input.push(c);
                self.spawn_validation();
                EventState::Consumed
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.spawn_validation();
                EventState::Consumed
            }
            KeyCode::Enter => {
                if matches!(self.validation_state, ValidationState::Available) {
                    let from = self.path.clone();
                    let parent = from.get_parent_path().0;
                    let new_path = if from.is_note() {
                        parent.append(&VaultPath::note_path_from(&self.input))
                    } else {
                        parent.append(&VaultPath::new(&self.input))
                    };
                    let vault = Arc::clone(&self.vault);
                    let tx2 = self.tx.clone();
                    tokio::spawn(async move {
                        let result = if from.is_note() {
                            vault.rename_note(&from, &new_path).await
                        } else {
                            vault.rename_directory(&from, &new_path).await
                        };
                        match result {
                            Ok(()) => {
                                tx2.send(AppEvent::EntryRenamed {
                                    from,
                                    to: new_path,
                                })
                                .ok();
                            }
                            Err(e) => {
                                tx2.send(AppEvent::DialogError(e.to_string())).ok();
                            }
                        }
                    });
                }
                // In all cases (Available or not) consume the key so Enter
                // doesn't propagate to the underlying panel.
                EventState::Consumed
            }
            KeyCode::Esc => {
                tx.send(AppEvent::CloseDialog).ok();
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }
}

// ---------------------------------------------------------------------------
// Component trait
// ---------------------------------------------------------------------------

impl Component for RenameDialog {
    fn handle_input(
        &mut self,
        event: &crate::components::events::InputEvent,
        tx: &AppTx,
    ) -> EventState {
        let crate::components::events::InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        self.handle_input(*key, tx)
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        // Drain the validation channel before rendering so the UI is current.
        self.poll_validation();

        let popup_area = super::centered_rect(60, 50, rect);

        // Backdrop: clear whatever is rendered behind the popup.
        f.render_widget(Clear, popup_area);

        // Outer block.
        let outer_block = Block::default()
            .title(" Rename ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.fg.to_ratatui()))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        // ── Vertical layout inside the block ─────────────────────────────────
        //
        // Row 0: label "CURRENT PATH"
        // Row 1: current path value
        // Row 2: spacer
        // Row 3: label "NEW NAME"
        // Row 4: input field (height 3 for bordered box)
        // Row 5: validation status text
        // Row 6: padding
        // Row 7: hint line
        // Row 8 (optional): error line

        let mut constraints = vec![
            Constraint::Length(1), // 0: label
            Constraint::Length(1), // 1: current path
            Constraint::Length(1), // 2: spacer
            Constraint::Length(1), // 3: label
            Constraint::Length(3), // 4: input field (bordered)
            Constraint::Length(1), // 5: validation status
            Constraint::Min(0),    // 6: padding
            Constraint::Length(1), // 7: hint
        ];
        if self.error.is_some() {
            constraints.push(Constraint::Length(1)); // 8: error
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();

        // Row 0: "CURRENT PATH" label (muted).
        f.render_widget(
            Paragraph::new("  CURRENT PATH")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[0],
        );

        // Row 1: actual path string.
        f.render_widget(
            Paragraph::new(format!("  {}", self.path))
                .style(Style::default().fg(fg).bg(bg)),
            rows[1],
        );

        // Row 2: blank spacer — nothing to render.

        // Row 3: "NEW NAME" label.
        f.render_widget(
            Paragraph::new("  NEW NAME")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[3],
        );

        // Row 4: input field with cursor and validation indicator.
        //
        // We split row 4 horizontally: [input_area | indicator (3 cols)].
        let input_row = rows[4];
        let input_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(1),    // input field
                Constraint::Length(3), // validation indicator
            ])
            .split(input_row);

        // Bordered input.
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(fg_muted))
            .style(Style::default().bg(bg));
        let input_inner = input_block.inner(input_chunks[0]);
        f.render_widget(input_block, input_chunks[0]);
        f.render_widget(
            Paragraph::new(format!("{}_", self.input))
                .style(Style::default().fg(fg).bg(bg)),
            input_inner,
        );

        // Validation indicator glyph (vertically centred in the 3-row area).
        let (indicator_text, indicator_style) = match self.validation_state {
            ValidationState::Idle => ("   ", Style::default()),
            ValidationState::Pending => (
                " \u{231b} ", // ⌛
                Style::default().fg(fg_muted),
            ),
            ValidationState::Available => (
                " \u{2713} ", // ✓
                Style::default().fg(Color::Green),
            ),
            ValidationState::Taken => (
                " \u{2717} ", // ✗
                Style::default().fg(Color::Red),
            ),
        };
        // Centre the indicator in the 3-row space by putting it in the middle row.
        let indicator_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(input_chunks[1]);
        f.render_widget(
            Paragraph::new(indicator_text)
                .style(indicator_style.bg(bg)),
            indicator_chunks[1],
        );

        // Row 5: validation status text.
        let (status_text, status_style) = match self.validation_state {
            ValidationState::Idle => ("", Style::default()),
            ValidationState::Pending => (
                "  Checking...",
                Style::default().fg(fg_muted).bg(bg),
            ),
            ValidationState::Available => (
                "  Available",
                Style::default().fg(Color::Green).bg(bg),
            ),
            ValidationState::Taken => (
                "  Already exists",
                Style::default().fg(Color::Red).bg(bg),
            ),
        };
        f.render_widget(
            Paragraph::new(status_text).style(status_style),
            rows[5],
        );

        // Row 7: hint line.  Dim the Enter hint unless rename is available.
        let enter_style = if matches!(self.validation_state, ValidationState::Available) {
            Style::default().fg(fg).bg(bg)
        } else {
            Style::default()
                .fg(fg_muted)
                .bg(bg)
                .add_modifier(Modifier::DIM)
        };
        let hint_idx = rows.len() - 1 - usize::from(self.error.is_some());
        // Render the two parts of the hint separately so we can style them
        // independently.
        let hint_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(16), // "[Enter: Rename] "
                Constraint::Min(1),     // "[Esc: Cancel]"
            ])
            .split(rows[hint_idx]);
        f.render_widget(
            Paragraph::new("  [Enter: Rename]").style(enter_style),
            hint_chunks[0],
        );
        f.render_widget(
            Paragraph::new("  [Esc: Cancel]")
                .style(Style::default().fg(fg_muted).bg(bg)),
            hint_chunks[1],
        );

        // Row 8 (optional): error message.
        if let Some(msg) = &self.error {
            let error_idx = rows.len() - 1;
            f.render_widget(
                Paragraph::new(format!("  Error: {msg}"))
                    .style(Style::default().fg(Color::Red).bg(bg)),
                rows[error_idx],
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    /// Compile-time smoke test: verify all `ValidationState` variants are
    /// accessible and exhaustively matched without a real vault.
    #[test]
    fn validation_state_variants_compile() {
        let states = [
            ValidationState::Idle,
            ValidationState::Pending,
            ValidationState::Available,
            ValidationState::Taken,
        ];
        for state in states {
            let _label = match state {
                ValidationState::Idle => "idle",
                ValidationState::Pending => "pending",
                ValidationState::Available => "available",
                ValidationState::Taken => "taken",
            };
        }
    }

    /// Verifies that the `input` field is pre-filled with the filename
    /// component of the supplied path.
    ///
    /// This test does not exercise the vault at all — `new()` never calls
    /// any async vault method — so it runs without any file-system setup.
    ///
    /// NOTE: It is gated `#[ignore]` because constructing `NoteVault` requires
    /// a real SQLite database on disk.  Run it explicitly with:
    ///
    /// ```text
    /// cargo test -- --ignored rename_dialog::tests::new_prefills_input
    /// ```
    #[tokio::test]
    #[ignore = "requires a real vault directory with kimun.sqlite"]
    async fn new_prefills_input() {
        use std::path::PathBuf;

        let tmp = std::env::temp_dir().join("kimun_rename_test_vault");
        std::fs::create_dir_all(&tmp).unwrap();

        let vault = Arc::new(
            NoteVault::new(PathBuf::from(&tmp))
                .await
                .expect("vault creation failed"),
        );

        let (tx, _rx) = mpsc::unbounded_channel::<AppEvent>();
        let path = VaultPath::new("notes/projects/kimun.md");
        let (_, expected_filename) = path.get_parent_path();

        let dialog = RenameDialog::new(path, vault, tx);
        assert_eq!(dialog.input, expected_filename);
    }

    /// Verifies that pressing `Esc` sends `AppEvent::CloseDialog` and returns
    /// `EventState::Consumed`, without touching the vault.
    #[test]
    fn esc_sends_close_dialog() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir().join("kimun_rename_esc_test");
            std::fs::create_dir_all(&tmp).unwrap();

            let vault_result = NoteVault::new(tmp).await;
            let Ok(vault) = vault_result else {
                // No vault available in CI — skip gracefully.
                return;
            };

            let vault = Arc::new(vault);
            let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
            let mut dialog =
                RenameDialog::new(VaultPath::new("notes/test.md"), vault, tx.clone());

            let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            let state = dialog.handle_input(key, &tx);

            assert_eq!(state, EventState::Consumed);
            let event = rx.try_recv().expect("expected AppEvent::CloseDialog");
            assert!(matches!(event, AppEvent::CloseDialog));
        });
    }
}
