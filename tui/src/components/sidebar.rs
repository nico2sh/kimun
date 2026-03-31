use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::settings::themes::Theme;
use chrono::NaiveDate;
use kimun_core::SearchResult;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, ResultType};
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::file_list::{FileListComponent, FileListEntry, SortField, SortOrder};
use crate::keys::KeyBindings;
use crate::settings::icons::Icons;
use crate::settings::AppSettings;

pub struct SidebarComponent {
    current_dir: VaultPath,
    pub file_list: FileListComponent,
    pending_rx: Option<Receiver<SearchResult>>,
    vault: Arc<NoteVault>,
    default_sort_field: SortField,
    default_sort_order: SortOrder,
    journal_sort_field: SortField,
    journal_sort_order: SortOrder,
}

impl SidebarComponent {
    pub fn new(key_bindings: KeyBindings, vault: Arc<NoteVault>, icons: Icons, settings: &AppSettings) -> Self {
        Self {
            current_dir: VaultPath::root(),
            file_list: FileListComponent::new(key_bindings, icons),
            pending_rx: None,
            vault,
            default_sort_field: SortField::from(settings.default_sort_field),
            default_sort_order: SortOrder::from(settings.default_sort_order),
            journal_sort_field: SortField::from(settings.journal_sort_field),
            journal_sort_order: SortOrder::from(settings.journal_sort_order),
        }
    }

    pub fn current_dir(&self) -> &VaultPath {
        &self.current_dir
    }

    pub fn is_empty(&self) -> bool {
        self.file_list.is_empty()
    }

    pub fn start_loading(&mut self, rx: Receiver<SearchResult>, current_dir: VaultPath) {
        self.current_dir = current_dir.clone();
        self.file_list.clear();
        self.file_list.loading = true;

        // Apply the appropriate sort defaults for this directory.
        if &current_dir == self.vault.journal_path() {
            self.file_list.sort_field = self.journal_sort_field;
            self.file_list.sort_order = self.journal_sort_order;
        } else {
            self.file_list.sort_field = self.default_sort_field;
            self.file_list.sort_order = self.default_sort_order;
        }

        if !current_dir.is_root_or_empty() {
            let parent = current_dir.get_parent_path().0;
            self.file_list.add_up_entry(parent);
        }

        self.pending_rx = Some(rx);
        self.sync_create_entry();
    }

    fn sync_create_entry(&mut self) {
        if self.file_list.search_query.is_empty() {
            self.file_list.set_create_entry(None);
        } else {
            let path = self
                .current_dir
                .append(&VaultPath::note_path_from(&self.file_list.search_query))
                .flatten();
            let filename = path.get_parent_path().1;
            self.file_list
                .set_create_entry(Some(FileListEntry::CreateNote { filename, path }));
        }
    }

    fn poll_loading(&mut self) {
        let Some(rx) = &self.pending_rx else { return };
        loop {
            match rx.try_recv() {
                Ok(result) => {
                    if matches!(&result.rtype, ResultType::Directory)
                        && result.path == self.current_dir
                    {
                        continue;
                    }
                    let journal_date = self
                        .vault
                        .journal_date(&result.path)
                        .map(format_journal_date);
                    self.file_list
                        .push_entry(FileListEntry::from_result(result, journal_date));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending_rx = None;
                    self.file_list.loading = false;
                    self.file_list.finalize_sort();
                    break;
                }
            }
        }
    }
}

/// Format a `NaiveDate` as a human-readable string with day-of-week.
/// Example: "Wednesday, March 17, 2026"
fn format_journal_date(date: NaiveDate) -> String {
    date.format("%A, %B %-d, %Y").to_string()
}

impl Component for SidebarComponent {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        // Intercept Enter when the selected entry is a CreateNote.
        // The sidebar owns the vault, so it creates the note here before
        // forwarding OpenPath — mirroring the note browser modal pattern.
        if let InputEvent::Key(key) = event {
            if key.code == KeyCode::Enter {
                if let Some(FileListEntry::CreateNote { path, .. }) =
                    self.file_list.selected_entry()
                {
                    let path = path.clone();
                    let vault = Arc::clone(&self.vault);
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = vault.load_or_create_note(&path, None).await {
                            log::warn!("create note failed for {path}: {e}");
                            return;
                        }
                        tx2.send(AppEvent::OpenPath(path)).ok();
                    });
                    return EventState::Consumed;
                }
            }
        }

        let result = self.file_list.handle_input(event, tx);

        // After a key that modifies the search query, keep the create entry in sync.
        if let InputEvent::Key(key) = event {
            if matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace) {
                self.sync_create_entry();
            }
        }

        result
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        self.file_list.hint_shortcuts()
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.poll_loading();

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(rect);

        let border_style = theme.border_style(focused);

        let header = Block::default()
            .title(self.current_dir.to_string())
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        let header_inner = header.inner(rows[0]);
        f.render_widget(header, rows[0]);
        f.render_widget(
            Paragraph::new(format!("{} notes", self.file_list.note_count()))
                .style(Style::default()
                    .fg(theme.fg_muted.to_ratatui())
                    .bg(theme.bg_panel.to_ratatui())),
            header_inner,
        );

        let search_block = Block::default()
            .title(" Search")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        let search_inner = search_block.inner(rows[1]);
        f.render_widget(search_block, rows[1]);
        f.render_widget(
            Paragraph::new(self.file_list.search_query.as_str()).style(
                Style::default()
                    .fg(theme.fg.to_ratatui())
                    .bg(theme.bg_panel.to_ratatui()),
            ),
            search_inner,
        );

        // Cursor at end of search query when focused.
        if focused {
            let cursor_x = search_inner.x + self.file_list.search_query.chars().count() as u16;
            f.set_cursor_position((cursor_x, search_inner.y));
        }

        self.file_list.render(f, rows[2], theme, focused);
    }
}
