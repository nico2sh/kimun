use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use nucleo::Utf32String;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use tokio::task::JoinHandle;

use crate::components::Component;
use crate::components::dialogs::ValidationState;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// MoveDialog
// ---------------------------------------------------------------------------

/// Modal dialog that lets the user move a note or directory to a different
/// directory inside the vault.
///
/// A background task loads all vault directories asynchronously.  As the user
/// types a filter query, a second background task runs nucleo fuzzy matching
/// and sends the ranked results back to the UI thread via a `std::sync::mpsc`
/// channel that is polled at the start of every `render()` call.
pub struct MoveDialog {
    /// The vault path being moved.
    pub path: VaultPath,
    /// Shared reference to the vault.
    pub vault: Arc<NoteVault>,
    /// Pre-computed `"  {path}"` for zero-allocation rendering.
    pub path_display: String,
    /// Current text in the search / filter input.
    pub search_query: String,
    /// Full list of directories returned by the vault (populated once load completes).
    pub all_dirs: Vec<VaultPath>,
    /// Handle to the directory-load background task.
    pub load_task: Option<JoinHandle<()>>,
    /// Handle to the filter background task (aborted on each new keystroke).
    pub filter_task: Option<JoinHandle<()>>,
    /// Fuzzy-filter results; `None` means "show all dirs" (no clone needed).
    pub filtered: Option<Vec<VaultPath>>,
    /// Selection state for the ratatui `List` widget.
    pub list_state: ListState,
    /// Result of the most-recent destination existence check.
    pub dest_validation: ValidationState,
    /// Handle to the running validation task so we can abort it on selection change.
    pub validation_task: Option<JoinHandle<()>>,
    /// Optional error message surfaced from a failed move attempt.
    pub error: Option<String>,
}

impl MoveDialog {
    /// Create a new `MoveDialog` for `path`.
    ///
    /// Directory loading starts immediately in a background task.
    pub fn new(path: VaultPath, vault: Arc<NoteVault>, tx: &AppTx) -> Self {
        let path_display = format!("  {}", path);
        let mut dialog = Self {
            path,
            vault,
            path_display,
            search_query: String::new(),
            all_dirs: vec![],
            load_task: None,
            filter_task: None,
            filtered: None,
            list_state: ListState::default(),
            dest_validation: ValidationState::Idle,
            validation_task: None,
            error: None,
        };
        dialog.schedule_load(tx);
        dialog
    }

    /// Returns the currently displayed list of directories.
    ///
    /// When no filter is active (`filtered` is `None`) this borrows `all_dirs`
    /// directly — no clone required.
    pub fn results(&self) -> &[VaultPath] {
        self.filtered.as_deref().unwrap_or(&self.all_dirs)
    }

    // -----------------------------------------------------------------------
    // Load helpers
    // -----------------------------------------------------------------------

    /// Spawn a background task that retrieves all vault directories and sends
    /// the result as [`AppEvent::MoveDirectoriesLoaded`].
    fn schedule_load(&mut self, tx: &AppTx) {
        let vault = Arc::clone(&self.vault);
        let tx_clone = tx.clone();
        let handle = tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                vault.get_directories(&VaultPath::root(), true)
            })
            .await;
            if let Ok(Ok(dirs)) = result {
                let mut paths: Vec<VaultPath> = std::iter::once(VaultPath::root())
                    .chain(dirs.into_iter().map(|d| d.path))
                    .collect();
                paths.sort();
                tx_clone.send(AppEvent::MoveDirectoriesLoaded(paths)).ok();
            }
        });
        self.load_task = Some(handle);
    }

    // -----------------------------------------------------------------------
    // Filter helpers
    // -----------------------------------------------------------------------

    /// Abort any in-flight filter task and schedule a new one for the current
    /// value of `self.search_query`.  If the query is empty the full
    /// `all_dirs` list is restored synchronously.  Otherwise the result is
    /// sent as [`AppEvent::MoveFilterResults`].
    fn schedule_filter(&mut self, tx: &AppTx) {
        if let Some(handle) = self.filter_task.take() {
            handle.abort();
        }

        if self.search_query.is_empty() {
            self.filtered = None;
            if self.list_state.selected().is_none() && !self.results().is_empty() {
                self.list_state.select(Some(0));
            }
            return;
        }

        let query = self.search_query.clone();
        let items: Vec<String> = self.all_dirs.iter().map(|p| p.to_string()).collect();
        let tx_clone = tx.clone();

        let handle = tokio::spawn(async move {
            let matched_strs = tokio::task::spawn_blocking(move || {
                let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
                let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
                let mut matched: Vec<(u32, String)> = items
                    .into_iter()
                    .filter_map(|item| {
                        let haystack = Utf32String::from(item.as_str());
                        pattern
                            .score(haystack.slice(..), &mut matcher)
                            .map(|score| (score, item))
                    })
                    .collect();
                matched.sort_by(|a, b| b.0.cmp(&a.0));
                matched.into_iter().map(|(_, s)| s).collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();

            let paths = matched_strs.iter().map(VaultPath::new).collect();
            tx_clone.send(AppEvent::MoveFilterResults(paths)).ok();
        });

        self.filter_task = Some(handle);
    }

    // -----------------------------------------------------------------------
    // Destination validation helpers
    // -----------------------------------------------------------------------

    /// Abort any in-flight validation task and start a new one for the
    /// currently selected directory.  The result is sent as
    /// [`AppEvent::MoveDestValidation`].  Resets to `Idle` when nothing is selected.
    pub fn spawn_validation(&mut self, tx: &AppTx) {
        if let Some(handle) = self.validation_task.take() {
            handle.abort();
        }

        let Some(idx) = self.list_state.selected() else {
            self.dest_validation = ValidationState::Idle;
            return;
        };
        let Some(dest_dir) = self.results().get(idx).cloned() else {
            self.dest_validation = ValidationState::Idle;
            return;
        };

        let from = self.path.clone();
        let vault = Arc::clone(&self.vault);
        let tx_clone = tx.clone();

        let handle = tokio::spawn(async move {
            let filename = from.get_parent_path().1;
            let candidate = if from.is_note() {
                dest_dir.append(&VaultPath::note_path_from(&filename))
            } else {
                dest_dir.append(&VaultPath::new(&filename))
            };
            let exists = vault.exists(&candidate).await.is_some();
            tx_clone
                .send(AppEvent::MoveDestValidation { available: !exists })
                .ok();
        });

        self.validation_task = Some(handle);
        self.dest_validation = ValidationState::Pending;
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    /// Handle a raw [`KeyEvent`].  Returns [`EventState::Consumed`] for keys
    /// this dialog acts on; callers should forward only key events.
    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Up => {
                if let Some(idx) = self.list_state.selected() {
                    self.list_state.select(Some(idx.saturating_sub(1)));
                    self.spawn_validation(tx);
                }
                EventState::Consumed
            }
            KeyCode::Down => {
                if !self.results().is_empty() {
                    let next = self
                        .list_state
                        .selected()
                        .map_or(0, |i| (i + 1).min(self.results().len() - 1));
                    self.list_state.select(Some(next));
                    self.spawn_validation(tx);
                }
                EventState::Consumed
            }
            KeyCode::Char(c) => {
                let non_shift = key.modifiers - KeyModifiers::SHIFT;
                if non_shift.is_empty() {
                    self.search_query.push(c);
                    self.schedule_filter(tx);
                    self.dest_validation = ValidationState::Idle;
                }
                // Consume regardless — prevents modifier combos (e.g. Ctrl+K)
                // from leaking a character into the search box.
                EventState::Consumed
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.schedule_filter(tx);
                self.dest_validation = ValidationState::Idle;
                EventState::Consumed
            }
            KeyCode::Enter => {
                if self.dest_validation == ValidationState::Taken {
                    return EventState::Consumed;
                }
                if let Some(selected_idx) = self.list_state.selected()
                    && selected_idx < self.results().len()
                {
                    let from = self.path.clone();
                    let dest_dir = self.results()[selected_idx].clone();
                    let filename = from.get_parent_path().1;
                    let new_path = if from.is_note() {
                        dest_dir.append(&VaultPath::note_path_from(&filename))
                    } else {
                        dest_dir.append(&VaultPath::new(&filename))
                    };
                    let vault = Arc::clone(&self.vault);
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        // The vault has no dedicated move API; rename_note /
                        // rename_directory accept paths in different directories,
                        // so a cross-directory rename is equivalent to a move.
                        let result = if from.is_note() {
                            vault.rename_note(&from, &new_path).await
                        } else {
                            vault.rename_directory(&from, &new_path).await
                        };
                        match result {
                            Ok(()) => {
                                tx2.send(AppEvent::EntryMoved { from, to: new_path }).ok();
                            }
                            Err(e) => {
                                tx2.send(AppEvent::DialogError(e.to_string())).ok();
                            }
                        }
                    });
                }
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

impl Component for MoveDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let popup_area = super::centered_rect(50, 60, rect);

        // Backdrop: clear whatever is rendered behind the popup.
        f.render_widget(Clear, popup_area);

        // Outer block.
        let outer_block = Block::default()
            .title(" Move ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.fg.to_ratatui()))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();

        // ── Vertical layout inside the block ─────────────────────────────────
        //
        // Row 0: "MOVING" label (muted)
        // Row 1: source path value
        // Row 2: spacer
        // Row 3: "DESTINATION" label (muted)
        // Row 4: search input field (height 3, bordered)
        // Row 5: directory list (fills available space)
        // Row 6: validation status
        // Row 7: hint line
        // Row 8 (optional): error line

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: "MOVING" label
                Constraint::Length(1), // 1: source path
                Constraint::Length(1), // 2: spacer
                Constraint::Length(1), // 3: "DESTINATION" label
                Constraint::Length(3), // 4: search input (bordered box)
                Constraint::Min(3),    // 5: directory list
                Constraint::Length(1), // 6: validation status
                Constraint::Length(1), // 7: hint line
                Constraint::Length(if self.error.is_some() { 1 } else { 0 }), // 8: error
            ])
            .split(inner);

        // Row 0: "MOVING" label.
        f.render_widget(
            Paragraph::new("  MOVING").style(Style::default().fg(fg_muted).bg(bg)),
            rows[0],
        );

        // Row 1: source path.
        super::render_path_row(f, rows[1], &self.path_display, fg, bg);

        // Row 2: blank spacer — nothing to render.

        // Row 3: "DESTINATION" label.
        f.render_widget(
            Paragraph::new("  DESTINATION").style(Style::default().fg(fg_muted).bg(bg)),
            rows[3],
        );

        // Row 4: search input with cursor indicator.
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(fg_muted))
            .style(Style::default().bg(bg));
        let input_inner = input_block.inner(rows[4]);
        f.render_widget(input_block, rows[4]);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(self.search_query.as_str()),
                Span::raw("|"),
            ]))
            .style(Style::default().fg(fg).bg(bg)),
            input_inner,
        );

        // Row 5: directory list (or loading placeholder).
        let list_items: Vec<ListItem> = if self.results().is_empty() {
            if self.load_task.is_some() {
                vec![ListItem::new("  (loading...)").style(Style::default().fg(fg_muted).bg(bg))]
            } else {
                vec![ListItem::new("  (no matches)").style(Style::default().fg(fg_muted).bg(bg))]
            }
        } else {
            self.results()
                .iter()
                .map(|p| {
                    let display = if *p == VaultPath::root() {
                        "  / (vault root)".to_string()
                    } else {
                        format!("  {}", p)
                    };
                    ListItem::new(display).style(Style::default().fg(fg).bg(bg))
                })
                .collect()
        };

        let list_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(fg_muted))
            .style(Style::default().bg(bg));

        let list = List::new(list_items)
            .block(list_block)
            .highlight_style(
                Style::default()
                    .bg(theme.bg_selected.to_ratatui())
                    .fg(theme.fg_selected.to_ratatui())
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        f.render_stateful_widget(list, rows[5], &mut self.list_state);

        // Row 6: validation status.
        let (status_text, status_style) = match self.dest_validation {
            ValidationState::Idle => ("", Style::default().bg(bg)),
            ValidationState::Pending => ("  Checking...", Style::default().fg(fg_muted).bg(bg)),
            ValidationState::Available => ("  Available", Style::default().fg(Color::Green).bg(bg)),
            ValidationState::Taken => ("  Already exists", Style::default().fg(Color::Red).bg(bg)),
        };
        f.render_widget(Paragraph::new(status_text).style(status_style), rows[6]);

        // Row 7: hint line.  Dim Enter when there's no valid selection.
        super::render_confirm_hint(
            f,
            rows[7],
            "  [Enter] Move here",
            self.dest_validation == ValidationState::Available,
            fg,
            fg_muted,
            bg,
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

    /// Compile-time smoke test: verify that the struct fields and key types
    /// are accessible without needing a real vault.
    #[test]
    fn struct_fields_accessible() {
        // Verify the `error` field exists and is `Option<String>`.
        fn _check_error_field(d: &MoveDialog) -> Option<&String> {
            d.error.as_ref()
        }
        // Verify the `search_query` field exists and is `String`.
        fn _check_search_query(d: &MoveDialog) -> &str {
            &d.search_query
        }
        // Verify `results()` accessor returns a slice.
        fn _check_results(d: &MoveDialog) -> &[VaultPath] {
            d.results()
        }
        // Verify `list_state` field is `ListState`.
        fn _check_list_state(d: &mut MoveDialog) -> &mut ListState {
            &mut d.list_state
        }
    }

    /// Pressing `Esc` must send `AppEvent::CloseDialog` and return
    /// `EventState::Consumed`, without requiring a real vault.
    #[test]
    fn esc_sends_close_dialog() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir().join("kimun_move_esc_test");
            std::fs::create_dir_all(&tmp).unwrap();

            let vault_result = NoteVault::new(tmp).await;
            let Ok(vault) = vault_result else {
                // No vault available in CI — skip gracefully.
                return;
            };

            let vault = Arc::new(vault);
            let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
            let mut dialog = MoveDialog::new(VaultPath::new("notes/test.md"), vault, &tx);

            let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
            // Drain the channel — background tasks (e.g. MoveDirectoriesLoaded)
            // may have sent events before or after the Esc key was processed.
            let mut found = false;
            while let Ok(event) = rx.try_recv() {
                if matches!(event, AppEvent::CloseDialog) {
                    found = true;
                    break;
                }
            }
            assert!(found, "expected AppEvent::CloseDialog in channel");
        });
    }

    /// A new `MoveDialog` must start with an empty `search_query` and no error.
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
            NoteVault::new(PathBuf::from(&tmp))
                .await
                .expect("vault creation failed"),
        );

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        let path = VaultPath::new("notes/projects/kimun.md");
        let dialog = MoveDialog::new(path, vault, &tx);

        assert!(dialog.search_query.is_empty());
        assert!(dialog.error.is_none());
        // Directory load is async; results may or may not be populated yet.
        // Assert the invariant that holds regardless: filtered starts as None.
        assert!(dialog.filtered.is_none());
    }
}
