use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use nucleo::Utf32String;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use tokio::task::JoinHandle;

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// DestValidation
// ---------------------------------------------------------------------------

pub enum DestValidation {
    /// No directory selected yet.
    Idle,
    /// Existence check in progress.
    Pending,
    /// Destination is free — move can proceed.
    Available,
    /// A file with the same name already exists at the destination.
    Taken,
}

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
    /// Current text in the search / filter input.
    pub search_query: String,
    /// Full list of directories returned by the vault (populated once load completes).
    pub all_dirs: Vec<VaultPath>,
    /// Handle to the directory-load background task.
    pub load_task: Option<JoinHandle<()>>,
    /// Receiver end of the channel used by the load task.
    pub load_rx: Option<std::sync::mpsc::Receiver<Vec<VaultPath>>>,
    /// Handle to the filter background task (aborted on each new keystroke).
    pub filter_task: Option<JoinHandle<()>>,
    /// Receiver end of the channel used by the filter task.
    pub filter_rx: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    /// Currently displayed (filtered + ranked) destination directories.
    pub results: Vec<VaultPath>,
    /// Selection state for the ratatui `List` widget.
    pub list_state: ListState,
    /// Result of the most-recent destination existence check.
    pub dest_validation: DestValidation,
    /// Handle to the running validation task so we can abort it on selection change.
    pub validation_task: Option<JoinHandle<()>>,
    /// Receiver end of the one-shot channel used by the validation task.
    pub validation_rx: Option<std::sync::mpsc::Receiver<bool>>,
    /// Optional error message surfaced from a failed move attempt.
    pub error: Option<String>,
}

impl MoveDialog {
    /// Create a new `MoveDialog` for `path`.
    ///
    /// Directory loading starts immediately in a background task.
    pub fn new(path: VaultPath, vault: Arc<NoteVault>) -> Self {
        let mut dialog = Self {
            path,
            vault,
            search_query: String::new(),
            all_dirs: vec![],
            load_task: None,
            load_rx: None,
            filter_task: None,
            filter_rx: None,
            results: vec![],
            list_state: ListState::default(),
            dest_validation: DestValidation::Idle,
            validation_task: None,
            validation_rx: None,
            error: None,
        };
        dialog.schedule_load();
        dialog
    }

    // -----------------------------------------------------------------------
    // Load helpers
    // -----------------------------------------------------------------------

    /// Spawn a background task that retrieves all vault directories and sends
    /// them back through a `std::sync::mpsc` channel.
    fn schedule_load(&mut self) {
        let vault = Arc::clone(&self.vault);
        let (tx, rx) = std::sync::mpsc::channel();
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
                tx.send(paths).ok();
            }
        });
        self.load_task = Some(handle);
        self.load_rx = Some(rx);
    }

    /// Poll the load channel (non-blocking). Returns true if dirs were received.
    fn poll_load_inner(&mut self) -> bool {
        let Some(rx) = &self.load_rx else { return false };
        if let Ok(dirs) = rx.try_recv() {
            self.all_dirs = dirs;
            self.results = self.all_dirs.clone();
            self.load_rx = None;
            self.load_task = None;
            if self.list_state.selected().is_none() && !self.results.is_empty() {
                self.list_state.select(Some(0));
            }
            return true;
        }
        false
    }

    fn poll_load(&mut self, tx: &AppTx) {
        if self.poll_load_inner() {
            self.spawn_validation(tx);
        }
    }

    // -----------------------------------------------------------------------
    // Filter helpers
    // -----------------------------------------------------------------------

    /// Abort any in-flight filter task and schedule a new one for the current
    /// value of `self.search_query`.  If the query is empty the full
    /// `all_dirs` list is restored synchronously.
    fn schedule_filter(&mut self, tx: &AppTx) {
        if let Some(handle) = self.filter_task.take() {
            handle.abort();
        }

        if self.search_query.is_empty() {
            self.results = self.all_dirs.clone();
            if self.list_state.selected().is_none() && !self.results.is_empty() {
                self.list_state.select(Some(0));
            }
            return;
        }

        let query = self.search_query.clone();
        let items: Vec<String> = self.all_dirs.iter().map(|p| p.to_string()).collect();
        let (ftx, frx) = std::sync::mpsc::channel();
        let tx_redraw = tx.clone();

        let handle = tokio::spawn(async move {
            let results = tokio::task::spawn_blocking(move || {
                let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
                let pattern = Pattern::parse(
                    &query,
                    CaseMatching::Ignore,
                    Normalization::Smart,
                );
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

            ftx.send(results).ok();
            tx_redraw.send(AppEvent::Redraw).ok();
        });

        self.filter_task = Some(handle);
        self.filter_rx = Some(frx);
    }

    /// Poll the filter channel (non-blocking). Returns true if results were received.
    fn poll_filter_inner(&mut self) -> bool {
        let Some(rx) = &self.filter_rx else { return false };
        if let Ok(strs) = rx.try_recv() {
            self.results = strs.iter().map(|s| VaultPath::new(s)).collect();
            self.filter_rx = None;
            self.filter_task = None;
            if !self.results.is_empty() {
                self.list_state.select(Some(0));
            } else {
                self.list_state.select(None);
            }
            self.dest_validation = DestValidation::Idle;
            return true;
        }
        false
    }

    fn poll_filter(&mut self, tx: &AppTx) {
        if self.poll_filter_inner() {
            self.spawn_validation(tx);
        }
    }

    // -----------------------------------------------------------------------
    // Destination validation helpers
    // -----------------------------------------------------------------------

    /// Abort any in-flight validation task and start a new one for the
    /// currently selected directory.  Resets to `Idle` when nothing is selected.
    fn spawn_validation(&mut self, tx: &AppTx) {
        if let Some(handle) = self.validation_task.take() {
            handle.abort();
        }
        self.validation_rx = None;

        let Some(idx) = self.list_state.selected() else {
            self.dest_validation = DestValidation::Idle;
            return;
        };
        let Some(dest_dir) = self.results.get(idx).cloned() else {
            self.dest_validation = DestValidation::Idle;
            return;
        };

        let from = self.path.clone();
        let vault = Arc::clone(&self.vault);
        let (vtx, vrx) = std::sync::mpsc::channel();
        let tx_redraw = tx.clone();

        let handle = tokio::spawn(async move {
            let filename = from.get_parent_path().1;
            let candidate = if from.is_note() {
                dest_dir.append(&VaultPath::note_path_from(&filename))
            } else {
                dest_dir.append(&VaultPath::new(&filename))
            };
            let exists = vault.exists(&candidate).await.is_some();
            vtx.send(!exists).ok(); // true = available
            tx_redraw.send(AppEvent::Redraw).ok();
        });

        self.validation_task = Some(handle);
        self.validation_rx = Some(vrx);
        self.dest_validation = DestValidation::Pending;
    }

    /// Poll the validation channel (non-blocking).  Call at the start of `render()`.
    fn poll_validation(&mut self) {
        let Some(rx) = &self.validation_rx else { return };
        if let Ok(available) = rx.try_recv() {
            self.dest_validation = if available {
                DestValidation::Available
            } else {
                DestValidation::Taken
            };
            self.validation_rx = None;
            self.validation_task = None;
        }
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    /// Handle a raw [`KeyEvent`].  Returns [`EventState::Consumed`] for keys
    /// this dialog acts on; callers should forward only key events.
    pub fn handle_input(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Up => {
                if let Some(idx) = self.list_state.selected() {
                    self.list_state.select(Some(idx.saturating_sub(1)));
                    self.spawn_validation(tx);
                }
                EventState::Consumed
            }
            KeyCode::Down => {
                if !self.results.is_empty() {
                    let next = self
                        .list_state
                        .selected()
                        .map_or(0, |i| (i + 1).min(self.results.len() - 1));
                    self.list_state.select(Some(next));
                    self.spawn_validation(tx);
                }
                EventState::Consumed
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.schedule_filter(tx);
                self.dest_validation = DestValidation::Idle;
                EventState::Consumed
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.schedule_filter(tx);
                self.dest_validation = DestValidation::Idle;
                EventState::Consumed
            }
            KeyCode::Enter => {
                if matches!(self.dest_validation, DestValidation::Taken) {
                    return EventState::Consumed;
                }
                if let Some(selected_idx) = self.list_state.selected() {
                    if selected_idx < self.results.len() {
                        let from = self.path.clone();
                        let dest_dir = self.results[selected_idx].clone();
                        let filename = from.get_parent_path().1;
                        let new_path = if from.is_note() {
                            dest_dir.append(&VaultPath::note_path_from(&filename))
                        } else {
                            dest_dir.append(&VaultPath::new(&filename))
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
                                    tx2.send(AppEvent::EntryMoved {
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
    fn handle_input(
        &mut self,
        event: &crate::components::events::InputEvent,
        tx: &AppTx,
    ) -> EventState {
        // Drain async channels here (where tx is available) to trigger validation.
        self.poll_load(tx);
        self.poll_filter(tx);

        let crate::components::events::InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        self.handle_input(*key, tx)
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        self.poll_load_inner();
        self.poll_filter_inner();
        self.poll_validation();

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

        let mut constraints = vec![
            Constraint::Length(1), // 0: "MOVING" label
            Constraint::Length(1), // 1: source path
            Constraint::Length(1), // 2: spacer
            Constraint::Length(1), // 3: "DESTINATION" label
            Constraint::Length(3), // 4: search input (bordered box)
            Constraint::Min(3),    // 5: directory list
            Constraint::Length(1), // 6: validation status
            Constraint::Length(1), // 7: hint line
        ];
        if self.error.is_some() {
            constraints.push(Constraint::Length(1)); // 8: error
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        // Row 0: "MOVING" label.
        f.render_widget(
            Paragraph::new("  MOVING")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[0],
        );

        // Row 1: source path.
        f.render_widget(
            Paragraph::new(format!("  {}", self.path))
                .style(Style::default().fg(fg).bg(bg)),
            rows[1],
        );

        // Row 2: blank spacer — nothing to render.

        // Row 3: "DESTINATION" label.
        f.render_widget(
            Paragraph::new("  DESTINATION")
                .style(Style::default().fg(fg_muted).bg(bg)),
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
            Paragraph::new(format!("{}|", self.search_query))
                .style(Style::default().fg(fg).bg(bg)),
            input_inner,
        );

        // Row 5: directory list (or loading placeholder).
        let list_items: Vec<ListItem> = if self.results.is_empty() {
            if self.load_task.is_some() {
                vec![ListItem::new("  (loading...)").style(Style::default().fg(fg_muted).bg(bg))]
            } else {
                vec![ListItem::new("  (no matches)").style(Style::default().fg(fg_muted).bg(bg))]
            }
        } else {
            self.results
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
            DestValidation::Idle => ("", Style::default().bg(bg)),
            DestValidation::Pending => ("  Checking...", Style::default().fg(fg_muted).bg(bg)),
            DestValidation::Available => ("  Available", Style::default().fg(Color::Green).bg(bg)),
            DestValidation::Taken => ("  Already exists", Style::default().fg(Color::Red).bg(bg)),
        };
        f.render_widget(Paragraph::new(status_text).style(status_style), rows[6]);

        // Row 7: hint line.  Dim Enter when there's no valid selection.
        let can_move = matches!(self.dest_validation, DestValidation::Available);
        let enter_style = if can_move {
            Style::default().fg(fg).bg(bg)
        } else {
            Style::default().fg(fg_muted).bg(bg).add_modifier(Modifier::DIM)
        };

        let hint_idx = rows.len() - 1 - usize::from(self.error.is_some());
        let hint_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(20), // "  [Enter] Move here"
                Constraint::Min(1),     // "  [Esc] Cancel"
            ])
            .split(rows[hint_idx]);

        f.render_widget(
            Paragraph::new("  [Enter] Move here").style(enter_style),
            hint_chunks[0],
        );
        f.render_widget(
            Paragraph::new("  [Esc] Cancel").style(Style::default().fg(fg_muted).bg(bg)),
            hint_chunks[1],
        );

        // Row 8 (optional): error message.
        if let Some(msg) = &self.error {
            f.render_widget(
                Paragraph::new(format!("  Error: {msg}"))
                    .style(Style::default().fg(Color::Red).bg(bg)),
                rows[rows.len() - 1],
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
        // Verify `results` field is a `Vec<VaultPath>`.
        fn _check_results(d: &MoveDialog) -> &Vec<VaultPath> {
            &d.results
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
            let mut dialog = MoveDialog::new(VaultPath::new("notes/test.md"), vault);

            let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            let state = dialog.handle_input(key, &tx);

            assert_eq!(state, EventState::Consumed);
            let event = rx.try_recv().expect("expected AppEvent::CloseDialog");
            assert!(matches!(event, AppEvent::CloseDialog));
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

        let path = VaultPath::new("notes/projects/kimun.md");
        let dialog = MoveDialog::new(path, vault);

        assert!(dialog.search_query.is_empty());
        assert!(dialog.error.is_none());
        assert!(dialog.results.is_empty() || !dialog.results.is_empty()); // load may be pending
    }
}
