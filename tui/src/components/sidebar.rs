use std::sync::mpsc::Receiver;

use kimun_core::ResultType;
use kimun_core::nfs::VaultPath;
use kimun_core::SearchResult;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders};

use crate::components::Component;
use crate::components::app_message::AppTx;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::file_list::{FileListComponent, FileListEntry};

pub struct SidebarComponent {
    pub focused: bool,
    current_dir: VaultPath,
    pub file_list: FileListComponent,
    pending_rx: Option<Receiver<SearchResult>>,
}

impl SidebarComponent {
    pub fn new() -> Self {
        Self {
            focused: false,
            current_dir: VaultPath::root(),
            file_list: FileListComponent::new(),
            pending_rx: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.file_list.is_empty()
    }

    pub fn start_loading(&mut self, rx: Receiver<SearchResult>, current_dir: VaultPath) {
        self.current_dir = current_dir.clone();
        self.file_list.clear();

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
                    self.file_list.push_entry(FileListEntry::from_result(result));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending_rx = None;
                    break;
                }
            }
        }
    }
}

impl Component for SidebarComponent {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        self.file_list.handle_event(event, tx)
    }

    fn render(&mut self, f: &mut Frame, rect: Rect) {
        self.poll_loading();

        // Sync focused state from sidebar into file list component.
        self.file_list.focused = self.focused;

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(rect);

        let header = Block::default()
            .title(self.current_dir.to_string())
            .borders(Borders::ALL);
        f.render_widget(header, rows[0]);

        self.file_list.render(f, rows[1]);
    }
}
