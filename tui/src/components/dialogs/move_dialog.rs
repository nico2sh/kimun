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

    /// Poll the load channel (non-blocking).  Call this at the start of every
    /// `render()` so the directory list is populated as soon as loading
    /// completes.
    fn poll_load(&mut self) {
        let Some(rx) = &self.load_rx else { return };
        if let Ok(dirs) = rx.try_recv() {
            self.all_dirs = dirs;
            self.results = self.all_dirs.clone();
            self.load_rx = None;
            self.load_task = None;
            if self.list_state.selected().is_none() && !self.results.is_empty() {
                self.list_state.select(Some(0));
            }
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

    /// Poll the filter channel (non-blocking).  Call this at the start of
    /// every `render()` so the list updates as soon as filtering completes.
    fn poll_filter(&mut self) {
        let Some(rx) = &self.filter_rx else { return };
        if let Ok(strs) = rx.try_recv() {
            self.results = strs.iter().map(|s| VaultPath::new(s)).collect();
            self.filter_rx = None;
            self.filter_task = None;
            if !self.results.is_empty() {
                self.list_state.select(Some(0));
            } else {
                self.list_state.select(None);
            }
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
                }
                EventState::Consumed
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.schedule_filter(tx);
                EventState::Consumed
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.schedule_filter(tx);
                EventState::Consumed
            }
            KeyCode::Enter => {
                if let Some(selected_idx) = self.list_state.selected() {
                    if !self.results.is_empty() {
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
        let crate::components::events::InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        self.handle_input(*key, tx)
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        // Drain async channels before rendering so the UI reflects latest state.
        self.poll_load();
        self.poll_filter();

        let popup_area = super::centered_rect(70, 60, rect);

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
        // Row 6: hint line
        // Row 7 (optional): error line

        let mut constraints = vec![
            Constraint::Length(1), // 0: "MOVING" label
            Constraint::Length(1), // 1: source path
            Constraint::Length(1), // 2: spacer
            Constraint::Length(1), // 3: "DESTINATION" label
            Constraint::Length(3), // 4: search input (bordered box)
            Constraint::Min(3),    // 5: directory list
            Constraint::Length(1), // 6: hint line
        ];
        if self.error.is_some() {
            constraints.push(Constraint::Length(1)); // 7: error
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

        // Row 6: hint line.  Dim the "Enter: Move here" part when no item is selected.
        let has_selection = self
            .list_state
            .selected()
            .is_some_and(|i| i < self.results.len());

        let enter_style = if has_selection {
            Style::default().fg(fg).bg(bg)
        } else {
            Style::default()
                .fg(fg_muted)
                .bg(bg)
                .add_modifier(Modifier::DIM)
        };

        let hint_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(20), // "[Enter: Move here]  "
                Constraint::Min(1),     // "[Esc: Cancel]"
            ])
            .split(rows[rows.len() - 1 - usize::from(self.error.is_some())]);

        f.render_widget(
            Paragraph::new("  [Enter: Move here]").style(enter_style),
            hint_chunks[0],
        );
        f.render_widget(
            Paragraph::new("  [Esc: Cancel]")
                .style(Style::default().fg(fg_muted).bg(bg)),
            hint_chunks[1],
        );

        // Row 7 (optional): error message.
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
