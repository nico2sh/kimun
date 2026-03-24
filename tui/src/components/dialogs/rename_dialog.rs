use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use tokio::task::JoinHandle;

use crate::components::Component;
use crate::components::dialogs::ValidationState;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

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
    /// Pre-computed `"  {path}"` for zero-allocation rendering.
    pub path_display: String,
    /// Current text in the input field.
    pub input: String,
    /// Result of the most-recent validation check.
    pub validation_state: ValidationState,
    /// Handle to the running validation task so we can abort it on new input.
    pub validation_task: Option<JoinHandle<()>>,
    /// Optional error message surfaced from a failed rename attempt.
    pub error: Option<String>,
}

impl RenameDialog {
    /// Create a new `RenameDialog` for `path`.
    ///
    /// The input field is pre-filled with the filename component of `path`.
    pub fn new(path: VaultPath, vault: Arc<NoteVault>) -> Self {
        let (_, filename) = path.get_parent_path();
        let path_display = format!("  {}", path);
        Self {
            path,
            vault,
            path_display,
            input: filename,
            validation_state: ValidationState::Idle,
            validation_task: None,
            error: None,
        }
    }

    // -----------------------------------------------------------------------
    // Validation helpers
    // -----------------------------------------------------------------------

    /// Abort any in-flight validation task and spawn a new one for the
    /// current value of `self.input`.  The result is sent as
    /// [`AppEvent::RenameValidation`] so that state updates happen in
    /// `handle_app_message` rather than in `render`.
    fn spawn_validation(&mut self, tx: &AppTx) {
        // Abort the previous task if it is still running.
        if let Some(handle) = self.validation_task.take() {
            handle.abort();
        }

        let vault = Arc::clone(&self.vault);
        let input = self.input.clone();
        let path = self.path.clone();
        let tx_clone = tx.clone();

        let handle = tokio::spawn(async move {
            let parent = path.get_parent_path().0;
            let candidate = if path.is_note() {
                parent.append(&VaultPath::note_path_from(&input))
            } else {
                parent.append(&VaultPath::new(&input))
            };
            let exists = vault.exists(&candidate).await.is_some();
            // `true` means the name is *available* (does not exist yet).
            tx_clone.send(AppEvent::RenameValidation { available: !exists }).ok();
        });

        self.validation_task = Some(handle);
        self.validation_state = ValidationState::Pending;
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    /// Handle a raw [`KeyEvent`].  Returns [`EventState::Consumed`] for keys
    /// this dialog acts on; callers should forward only key events.
    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Char(c) => {
                self.input.push(c);
                self.spawn_validation(tx);
                EventState::Consumed
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.spawn_validation(tx);
                EventState::Consumed
            }
            KeyCode::Enter => {
                if self.validation_state == ValidationState::Available {
                    let from = self.path.clone();
                    let parent = from.get_parent_path().0;
                    let new_path = if from.is_note() {
                        parent.append(&VaultPath::note_path_from(&self.input))
                    } else {
                        parent.append(&VaultPath::new(&self.input))
                    };
                    let vault = Arc::clone(&self.vault);
                    let tx2 = tx.clone();
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
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        // Fixed size: 50 wide; height depends on whether there is an error row.
        // Border(2) + spacer + path + separator + label + input(3) + validation
        //           + spacer + hint [+ error] = 11 or 12.
        let height = if self.error.is_some() { 13 } else { 12 };
        let popup_area = super::fixed_centered_rect(50, height, rect);

        f.render_widget(Clear, popup_area);

        let outer_block = Block::default()
            .title(" Rename ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.fg.to_ratatui()))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        // ── Vertical layout inside the block ─────────────────────────────────
        //
        // Row 0: spacer
        // Row 1: current path
        // Row 2: separator
        // Row 3: "NEW NAME" label
        // Row 4: input field (height 3, bordered)
        // Row 5: validation status
        // Row 6: spacer
        // Row 7: hint line
        // Row 8 (optional): error line

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: path
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: "NEW NAME" label
                Constraint::Length(3), // 4: input field (bordered)
                Constraint::Length(1), // 5: validation status
                Constraint::Length(1), // 6: spacer
                Constraint::Length(1), // 7: hint
                Constraint::Min(0),    // 8: remainder / error
            ])
            .split(inner);

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();

        // Row 1: path.
        super::render_path_row(f, rows[1], &self.path_display, fg, bg);

        // Row 2: separator.
        super::render_separator(f, rows[2], fg_muted, bg);

        // Row 3: "NEW NAME" label.
        f.render_widget(
            Paragraph::new("  NEW NAME")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[3],
        );

        // Row 4: input field with cursor and validation indicator.
        //
        // Split horizontally: [input_area | indicator (3 cols)].
        let input_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(1),    // input field
                Constraint::Length(3), // validation indicator
            ])
            .split(rows[4]);

        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(fg_muted))
            .style(Style::default().bg(bg));
        let input_inner = input_block.inner(input_chunks[0]);
        f.render_widget(input_block, input_chunks[0]);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(self.input.as_str()),
                Span::raw("_"),
            ]))
            .style(Style::default().fg(fg).bg(bg)),
            input_inner,
        );

        // Validation indicator glyph, centred vertically in the 3-row area.
        let (indicator_text, indicator_style) = match self.validation_state {
            ValidationState::Idle => ("   ", Style::default()),
            ValidationState::Pending => (" \u{231b} ", Style::default().fg(fg_muted)),
            ValidationState::Available => (" \u{2713} ", Style::default().fg(Color::Green)),
            ValidationState::Taken => (" \u{2717} ", Style::default().fg(Color::Red)),
        };
        let indicator_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(input_chunks[1]);
        f.render_widget(
            Paragraph::new(indicator_text).style(indicator_style.bg(bg)),
            indicator_rows[1],
        );

        // Row 5: validation status text.
        let (status_text, status_style) = match self.validation_state {
            ValidationState::Idle => ("", Style::default()),
            ValidationState::Pending => ("  Checking...", Style::default().fg(fg_muted).bg(bg)),
            ValidationState::Available => ("  Available", Style::default().fg(Color::Green).bg(bg)),
            ValidationState::Taken => ("  Already exists", Style::default().fg(Color::Red).bg(bg)),
        };
        f.render_widget(Paragraph::new(status_text).style(status_style), rows[5]);

        // Row 7: hint.  Dim the Enter part unless rename is available.
        super::render_confirm_hint(
            f, rows[7], "  [Enter] Rename",
            self.validation_state == ValidationState::Available,
            fg, fg_muted, bg,
        );

        // Row 8 (optional): error message.
        if let Some(msg) = &self.error {
            super::render_error_row(f, rows[8], msg, bg);
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

        let dialog = RenameDialog::new(path, vault);
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
                RenameDialog::new(VaultPath::new("notes/test.md"), vault);

            let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
            let event = rx.try_recv().expect("expected AppEvent::CloseDialog");
            assert!(matches!(event, AppEvent::CloseDialog));
        });
    }
}
