use std::sync::mpsc::Receiver;

use kimun_core::{ResultType, SearchResult};
use kimun_core::nfs::VaultPath;
use nucleo::Matcher;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::components::Component;
use crate::components::app_message::{AppMessage, AppTx};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;

/// Carries the original index through nucleo's match_list so we can map
/// matched items back to their position in real_entries.
#[derive(Clone)]
struct MatchEntry {
    idx: usize,
    text: String,
}

impl AsRef<str> for MatchEntry {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

pub struct SidebarComponent {
    pub search_query: String,
    pub focused: bool,
    current_dir: VaultPath,
    /// Parent dir entry shown at the top when not at root.
    up_entry: Option<VaultPath>,
    /// All real entries from the vault browse.
    real_entries: Vec<SearchResult>,
    /// Indices into real_entries that are currently visible (respects filter).
    /// When None every entry is visible in original order.
    display_indices: Option<Vec<usize>>,
    /// Receives the result of the latest async filter task.
    filter_rx: Option<Receiver<Vec<usize>>>,
    list_state: ListState,
    /// Receives real entries from the vault browse task.
    pending_rx: Option<Receiver<SearchResult>>,
}

impl SidebarComponent {
    pub fn new() -> Self {
        Self {
            search_query: String::new(),
            focused: false,
            current_dir: VaultPath::root(),
            up_entry: None,
            real_entries: Vec::new(),
            display_indices: None,
            filter_rx: None,
            list_state: ListState::default(),
            pending_rx: None,
        }
    }

    pub fn start_loading(&mut self, rx: Receiver<SearchResult>, current_dir: VaultPath) {
        self.current_dir = current_dir.clone();
        self.real_entries.clear();
        self.display_indices = None;
        self.filter_rx = None;
        self.search_query.clear();

        if !current_dir.is_root_or_empty() {
            let parent = current_dir.get_parent_path().0;
            self.up_entry = Some(parent);
            self.list_state.select(Some(0));
        } else {
            self.up_entry = None;
            self.list_state.select(None);
        }

        self.pending_rx = Some(rx);
    }

    /// Spawns a background task that fuzzy-filters real_entries with the
    /// current search_query. Sends Redraw through tx when the result is ready.
    fn schedule_filter(&mut self, tx: AppTx) {
        if self.search_query.is_empty() {
            self.display_indices = None;
            self.filter_rx = None;
            self.reset_selection();
            return;
        }

        let candidates: Vec<MatchEntry> = self
            .real_entries
            .iter()
            .enumerate()
            .map(|(i, e)| MatchEntry { idx: i, text: entry_search_str(e) })
            .collect();

        let query = self.search_query.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.filter_rx = Some(result_rx);

        tokio::spawn(async move {
            let indices = tokio::task::spawn_blocking(move || {
                let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
                let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
                pattern
                    .match_list(candidates, &mut matcher)
                    .into_iter()
                    .map(|(entry, _score)| entry.idx)
                    .collect::<Vec<usize>>()
            })
            .await
            .unwrap_or_default();

            result_tx.send(indices).ok();
            tx.send(AppMessage::Redraw).ok();
        });
    }

    /// Polls the filter channel and applies results. Called at the top of render.
    fn poll_filter(&mut self) {
        let Some(rx) = &self.filter_rx else { return };
        match rx.try_recv() {
            Ok(indices) => {
                self.display_indices = Some(indices);
                self.filter_rx = None;
                self.reset_selection();
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.filter_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Polls the vault browse channel and appends new entries. Called at the
    /// top of render.
    fn poll_loading(&mut self) {
        let Some(rx) = &self.pending_rx else { return };
        let mut added = false;
        loop {
            match rx.try_recv() {
                Ok(entry) => {
                    if matches!(&entry.rtype, ResultType::Directory)
                        && entry.path == self.current_dir
                    {
                        continue;
                    }
                    self.real_entries.push(entry);
                    added = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending_rx = None;
                    break;
                }
            }
        }

        // When there's no active filter, keep selection alive as entries arrive.
        if added && self.search_query.is_empty() && self.display_indices.is_none() {
            if self.list_state.selected().is_none() && !self.real_entries.is_empty() {
                let up_offset = self.up_entry.is_some() as usize;
                self.list_state.select(Some(up_offset));
            }
        }
    }

    fn display_len(&self) -> usize {
        let up = self.up_entry.is_some() as usize;
        let real = match &self.display_indices {
            None => self.real_entries.len(),
            Some(v) => v.len(),
        };
        up + real
    }

    fn reset_selection(&mut self) {
        if self.display_len() > 0 {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    fn select_next(&mut self) {
        let len = self.display_len();
        if len == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some((current + 1) % len));
    }

    fn select_prev(&mut self) {
        let len = self.display_len();
        if len == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(if current == 0 { len - 1 } else { current - 1 }));
    }

    fn activate_selected(&self, tx: &AppTx) {
        let Some(idx) = self.list_state.selected() else { return };

        let up_offset = self.up_entry.is_some() as usize;
        let path = if idx < up_offset {
            self.up_entry.as_ref().unwrap().clone()
        } else {
            let real_idx = idx - up_offset;
            let entry_idx = match &self.display_indices {
                None => real_idx,
                Some(v) => v[real_idx],
            };
            self.real_entries[entry_idx].path.clone()
        };

        tx.send(AppMessage::OpenPath(path)).ok();
    }
}

impl Component for SidebarComponent {
    fn handle_event(&mut self, event: &AppEvent, tx: &AppTx) -> EventState {
        if let AppEvent::Key(key) = event {
            match key.code {
                KeyCode::Up => {
                    self.select_prev();
                    return EventState::Consumed;
                }
                KeyCode::Down => {
                    self.select_next();
                    return EventState::Consumed;
                }
                KeyCode::Enter => {
                    self.activate_selected(tx);
                    return EventState::Consumed;
                }
                KeyCode::Char(c) => {
                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        self.search_query.push(c.to_ascii_uppercase());
                    } else {
                        self.search_query.push(c);
                    }
                    self.schedule_filter(tx.clone());
                    return EventState::Consumed;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.schedule_filter(tx.clone());
                    return EventState::Consumed;
                }
                _ => {}
            }
        }
        EventState::NotConsumed
    }

    fn render(&mut self, f: &mut Frame, rect: Rect) {
        self.poll_loading();
        self.poll_filter();

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(rect);

        let header = Block::default()
            .title(self.current_dir.to_string())
            .borders(Borders::ALL);
        f.render_widget(header, rows[0]);

        let search_block = Block::default()
            .title("Search")
            .borders(Borders::ALL)
            .border_style(if self.focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            });
        f.render_widget(
            Paragraph::new(self.search_query.as_str()).block(search_block),
            rows[1],
        );

        if self.focused {
            let cursor_x = rows[1].x + 1 + self.search_query.len() as u16;
            let cursor_y = rows[1].y + 1;
            f.set_cursor_position((cursor_x, cursor_y));
        }

        // Build the visible list.
        let mut items: Vec<ListItem> = Vec::new();

        if self.up_entry.is_some() {
            items.push(up_list_item());
        }

        let real_iter: Box<dyn Iterator<Item = &SearchResult>> = match &self.display_indices {
            None => Box::new(self.real_entries.iter()),
            Some(indices) => Box::new(indices.iter().map(|&i| &self.real_entries[i])),
        };
        for entry in real_iter {
            items.push(real_entry_to_list_item(entry));
        }

        let notes_list = List::new(items)
            .block(Block::default().title("Files").borders(Borders::ALL))
            .highlight_style(Style::default().fg(Color::Black).bg(Color::Yellow));
        f.render_stateful_widget(notes_list, rows[2], &mut self.list_state);
    }
}

fn entry_search_str(entry: &SearchResult) -> String {
    let filename = entry.path.get_parent_path().1;
    match &entry.rtype {
        ResultType::Note(data) => format!("{} {}", data.title, filename),
        ResultType::Directory | ResultType::Attachment => filename,
    }
}

fn up_list_item() -> ListItem<'static> {
    ListItem::new(Text::from(vec![
        Line::from(Span::styled("↑  ..", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled(
            "─".repeat(28),
            Style::default().fg(Color::DarkGray),
        )),
    ]))
}

fn real_entry_to_list_item(entry: &SearchResult) -> ListItem<'static> {
    let filename = entry.path.get_parent_path().1;
    let divider = Line::from(Span::styled(
        "─".repeat(28),
        Style::default().fg(Color::DarkGray),
    ));

    let lines = match &entry.rtype {
        ResultType::Note(data) => {
            let title = if data.title.trim().is_empty() {
                "<no title>".to_string()
            } else {
                data.title.clone()
            };
            vec![
                Line::from(title),
                Line::from(Span::styled(
                    filename,
                    Style::default().add_modifier(Modifier::ITALIC),
                )),
                divider,
            ]
        }
        ResultType::Directory => vec![Line::from(format!("📁 {}", filename)), divider],
        ResultType::Attachment => vec![
            Line::from(Span::styled(
                filename,
                Style::default().add_modifier(Modifier::ITALIC),
            )),
            divider,
        ],
    };

    ListItem::new(Text::from(lines))
}
