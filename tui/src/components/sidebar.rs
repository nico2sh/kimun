use std::sync::Arc;
use std::sync::mpsc::Receiver;

use chrono::NaiveDate;
use kimun_core::{NoteVault, ResultType};
use kimun_core::nfs::VaultPath;
use kimun_core::SearchResult;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};
use crate::settings::themes::Theme;

use crate::components::Component;
use crate::components::app_message::AppTx;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::file_list::{FileListComponent, FileListEntry};
use crate::keys::KeyBindings;

pub struct SidebarComponent {
    current_dir: VaultPath,
    pub file_list: FileListComponent,
    pending_rx: Option<Receiver<SearchResult>>,
    vault: Arc<NoteVault>,
}

impl SidebarComponent {
    pub fn new(key_bindings: KeyBindings, vault: Arc<NoteVault>) -> Self {
        Self {
            current_dir: VaultPath::root(),
            file_list: FileListComponent::new(key_bindings),
            pending_rx: None,
            vault,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.file_list.is_empty()
    }

    pub fn start_loading(&mut self, rx: Receiver<SearchResult>, current_dir: VaultPath) {
        self.current_dir = current_dir.clone();
        self.file_list.clear();
        self.file_list.loading = true;

        if !current_dir.is_root_or_empty() {
            let parent = current_dir.get_parent_path().0;
            self.file_list.add_up_entry(parent);
        }

        self.pending_rx = Some(rx);
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
                    let journal_date = self.vault.journal_date(&result.path)
                        .map(format_journal_date);
                    self.file_list.push_entry(FileListEntry::from_result(result, journal_date));
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
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        self.file_list.handle_event(event, tx)
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.poll_loading();

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(0)])
            .split(rect);

        let border_style = theme.border_style(focused);

        let header = Block::default()
            .title(self.current_dir.to_string())
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        f.render_widget(header, rows[0]);

        let search_block = Block::default()
            .title(" Search")
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        let search_inner = search_block.inner(rows[1]);
        f.render_widget(search_block, rows[1]);
        f.render_widget(
            Paragraph::new(self.file_list.search_query.as_str())
                .style(Style::default().fg(theme.fg.to_ratatui()).bg(theme.bg_panel.to_ratatui())),
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
