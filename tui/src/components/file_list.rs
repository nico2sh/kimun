use std::sync::mpsc::Receiver;

use kimun_core::nfs::VaultPath;
use kimun_core::{ResultType, SearchResult};
use nucleo::Matcher;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::events::{AppTx, InputEvent};
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;
use crate::settings::{SortFieldSetting, SortOrderSetting};

// ---------------------------------------------------------------------------
// Sort options
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum SortField {
    Name,
    Title,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl From<SortFieldSetting> for SortField {
    fn from(s: SortFieldSetting) -> Self {
        match s {
            SortFieldSetting::Name => Self::Name,
            SortFieldSetting::Title => Self::Title,
        }
    }
}

impl From<SortOrderSetting> for SortOrder {
    fn from(s: SortOrderSetting) -> Self {
        match s {
            SortOrderSetting::Ascending => Self::Ascending,
            SortOrderSetting::Descending => Self::Descending,
        }
    }
}

impl SortField {
    fn label(self) -> char {
        match self {
            Self::Name => 'N',
            Self::Title => 'T',
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Name => Self::Title,
            Self::Title => Self::Name,
        }
    }
}

impl SortOrder {
    fn label(self) -> char {
        match self {
            Self::Ascending => '↑',
            Self::Descending => '↓',
        }
    }

    fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

// ---------------------------------------------------------------------------
// FileListEntry
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub enum FileListEntry {
    Up {
        parent: VaultPath,
    },
    Note {
        path: VaultPath,
        title: String,
        filename: String,
        journal_date: Option<String>,
    },
    Directory {
        path: VaultPath,
        name: String,
    },
    Attachment {
        path: VaultPath,
        filename: String,
    },
    CreateNote {
        filename: String,
        path: VaultPath,
    },
}

impl FileListEntry {
    pub fn from_result(result: SearchResult, journal_date: Option<String>) -> Self {
        let filename = result.path.get_parent_path().1;
        match result.rtype {
            ResultType::Note(data) => {
                let title = if data.title.trim().is_empty() {
                    "<no title>".to_string()
                } else {
                    data.title
                };
                Self::Note {
                    path: result.path,
                    title,
                    filename,
                    journal_date,
                }
            }
            ResultType::Directory => Self::Directory {
                path: result.path,
                name: filename,
            },
            ResultType::Attachment => Self::Attachment {
                path: result.path,
                filename,
            },
        }
    }

    pub fn path(&self) -> &VaultPath {
        match self {
            Self::Up { parent } => parent,
            Self::Note { path, .. } => path,
            Self::Directory { path, .. } => path,
            Self::Attachment { path, .. } => path,
            Self::CreateNote { path, .. } => path,
        }
    }

    pub fn search_str(&self) -> Option<String> {
        match self {
            Self::Up { .. } => None,
            Self::Note {
                title, filename, ..
            } => Some(format!("{} {}", title, filename)),
            Self::Directory { name, .. } => Some(name.clone()),
            Self::Attachment { filename, .. } => Some(filename.clone()),
            Self::CreateNote { filename, .. } => Some(filename.clone()),
        }
    }

    /// Sort key for the given field.
    fn sort_key(&self, field: SortField) -> String {
        match self {
            Self::Up { .. } => String::new(),
            Self::Note {
                title, filename, ..
            } => match field {
                SortField::Title => title.to_lowercase(),
                SortField::Name => filename.to_lowercase(),
            },
            Self::Directory { name, .. } => name.to_lowercase(),
            Self::Attachment { filename, .. } => filename.to_lowercase(),
            Self::CreateNote { filename, .. } => filename.to_lowercase(),
        }
    }

    /// Terminal rows this entry occupies when rendered.
    pub fn visual_height(&self) -> u16 {
        match self {
            Self::Note { journal_date, .. } => {
                if journal_date.is_some() {
                    3
                } else {
                    2
                }
            }
            _ => 1,
        }
    }

    fn to_list_item(&self, theme: &Theme, icons: &Icons) -> ListItem<'static> {
        let lines: Vec<Line> = match self {
            Self::Up { .. } => vec![Line::from(Span::styled(
                format!("{} [UP] ..", icons.directory_up),
                Style::default().fg(theme.fg_muted.to_ratatui()),
            ))],
            Self::Note {
                title,
                filename,
                journal_date,
                ..
            } => {
                let mut lines = vec![];
                if let Some(date) = journal_date {
                    lines.push(Line::from(format!("{} {}", icons.journal, title)));
                    lines.push(Line::from(Span::styled(
                        format!(" {}", date),
                        Style::default().fg(theme.color_journal_date.to_ratatui()),
                    )));
                } else {
                    lines.push(Line::from(format!("{} {}", icons.note, title)));
                }
                lines.push(Line::from(Span::styled(
                    format!(" {}", filename),
                    Style::default()
                        .add_modifier(Modifier::ITALIC)
                        .fg(theme.fg_secondary.to_ratatui()),
                )));
                lines
            }
            Self::Directory { name, .. } => vec![Line::from(Span::styled(
                format!("{} {}", icons.directory, name),
                Style::default().fg(theme.color_directory.to_ratatui()),
            ))],
            Self::Attachment { filename, .. } => vec![Line::from(Span::styled(
                format!("{} {}", icons.attachment, filename),
                Style::default()
                    .add_modifier(Modifier::ITALIC)
                    .fg(theme.fg_secondary.to_ratatui()),
            ))],
            Self::CreateNote { filename, .. } => vec![Line::from(Span::styled(
                format!("+ Create: {}", filename),
                Style::default().fg(theme.accent.to_ratatui()),
            ))],
        };
        ListItem::new(Text::from(lines))
    }
}

// ---------------------------------------------------------------------------
// Nucleo helper
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// FileListComponent
// ---------------------------------------------------------------------------

pub struct FileListComponent {
    pub entries: Vec<FileListEntry>,
    pub loading: bool,
    display_indices: Option<Vec<usize>>,
    list_state: ListState,
    rendered_rect: Rect,
    // Search
    pub search_query: String,
    filter_rx: Option<Receiver<Vec<usize>>>,
    filter_task: Option<tokio::task::JoinHandle<()>>,
    // Sort
    pub sort_field: SortField,
    pub sort_order: SortOrder,
    // Always-visible pinned entry shown above all others (used for "Create…").
    // Not stored in `entries`; not touched by the filter.
    create_entry: Option<FileListEntry>,
    // Keybindings
    key_bindings: KeyBindings,
    // Icons resolved once at construction
    icons: Icons,
}

impl FileListComponent {
    pub fn new(key_bindings: KeyBindings, icons: Icons) -> Self {
        Self {
            entries: Vec::new(),
            loading: false,
            display_indices: None,
            list_state: ListState::default(),
            rendered_rect: Rect::default(),
            search_query: String::new(),
            filter_rx: None,
            filter_task: None,
            sort_field: SortField::Name,
            sort_order: SortOrder::Ascending,
            create_entry: None,
            key_bindings,
            icons,
        }
    }

    pub fn set_create_entry(&mut self, entry: Option<FileListEntry>) {
        self.create_entry = entry;
        self.reset_selection();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn push_entry(&mut self, entry: FileListEntry) {
        if matches!(entry, FileListEntry::Attachment { .. } | FileListEntry::CreateNote { .. }) {
            return;
        }
        self.entries.push(entry);
        if self.display_indices.is_none() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
    }

    /// Sort entries once after all items have been loaded.
    pub fn finalize_sort(&mut self) {
        self.apply_sort();
    }

    pub fn add_up_entry(&mut self, parent: VaultPath) {
        self.entries.insert(0, FileListEntry::Up { parent });
        self.list_state.select(Some(0));
    }

    pub fn prepend_create_entry(&mut self, entry: FileListEntry) {
        // Reset any active filter — inserting at 0 would shift all stored indices.
        self.display_indices = None;
        self.entries.insert(0, entry);
        self.list_state.select(Some(0));
    }

    pub fn clear(&mut self) {
        if let Some(handle) = self.filter_task.take() {
            handle.abort();
        }
        self.entries.clear();
        self.display_indices = None;
        self.filter_rx = None;
        self.search_query.clear();
        self.create_entry = None;
        self.list_state.select(None);
        self.loading = false;
    }

    /// Sort entries in-place, keeping any leading Up entry at position 0.
    fn apply_sort(&mut self) {
        let up_count = self
            .entries
            .iter()
            .take_while(|e| matches!(e, FileListEntry::Up { .. }))
            .count();
        let field = self.sort_field;
        let order = self.sort_order;
        self.entries[up_count..].sort_by(|a, b| {
            let ka = a.sort_key(field);
            let kb = b.sort_key(field);
            match order {
                SortOrder::Ascending => ka.cmp(&kb),
                SortOrder::Descending => kb.cmp(&ka),
            }
        });
    }

    fn set_sort(&mut self, field: SortField, order: SortOrder, tx: AppTx) {
        self.sort_field = field;
        self.sort_order = order;
        self.apply_sort();
        // Re-run filter so indices stay valid after in-place sort.
        if !self.search_query.is_empty() {
            self.schedule_filter(tx);
        } else {
            self.display_indices = None;
            self.reset_selection();
        }
    }

    fn schedule_filter(&mut self, tx: AppTx) {
        if self.search_query.is_empty() {
            self.display_indices = None;
            self.filter_rx = None;
            self.reset_selection();
            return;
        }

        let candidates: Vec<MatchEntry> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.search_str().map(|text| MatchEntry { idx: i, text }))
            .collect();

        let query = self.search_query.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.filter_rx = Some(result_rx);

        if let Some(handle) = self.filter_task.take() {
            handle.abort();
        }

        let handle = tokio::spawn(async move {
            let indices = tokio::task::spawn_blocking(move || {
                let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
                let pattern = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
                pattern
                    .match_list(candidates, &mut matcher)
                    .into_iter()
                    .map(|(e, _)| e.idx)
                    .collect::<Vec<usize>>()
            })
            .await
            .unwrap_or_default();

            result_tx.send(indices).ok();
            tx.send(AppEvent::Redraw).ok();
        });
        self.filter_task = Some(handle);
    }

    pub fn poll_filter(&mut self) {
        let Some(rx) = &self.filter_rx else { return };
        match rx.try_recv() {
            Ok(indices) => {
                let up_indices: Vec<usize> = self
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| matches!(e, FileListEntry::Up { .. }))
                    .map(|(i, _)| i)
                    .collect();
                let mut combined = up_indices;
                combined.extend(indices);
                self.display_indices = Some(combined);
                self.filter_rx = None;
                self.reset_selection();
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.filter_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    pub fn display_len(&self) -> usize {
        let base = match &self.display_indices {
            None => self.entries.len(),
            Some(v) => v.len(),
        };
        base + usize::from(self.create_entry.is_some())
    }

    /// Number of entries currently visible in the list (respects active filter).
    pub fn len(&self) -> usize {
        self.display_len()
    }

    /// Number of note entries currently visible (excludes directories, Up, attachments).
    pub fn note_count(&self) -> usize {
        match &self.display_indices {
            None => self.entries.iter().filter(|e| matches!(e, FileListEntry::Note { .. })).count(),
            Some(indices) => indices.iter()
                .filter(|&&i| matches!(self.entries.get(i), Some(FileListEntry::Note { .. })))
                .count(),
        }
    }

    fn reset_selection(&mut self) {
        self.list_state.select(if self.display_len() > 0 {
            Some(0)
        } else {
            None
        });
    }

    pub fn scroll_up(&mut self) {
        let offset = self.list_state.offset();
        if offset > 0 {
            *self.list_state.offset_mut() = offset - 1;
            if let Some(sel) = self.list_state.selected() {
                if sel > 0 {
                    self.list_state.select(Some(sel - 1));
                }
            }
        }
    }

    pub fn scroll_down(&mut self) {
        let len = self.display_len();
        let offset = self.list_state.offset();
        if len > 0 && offset + 1 < len {
            *self.list_state.offset_mut() = offset + 1;
            if let Some(sel) = self.list_state.selected() {
                if sel + 1 < len {
                    self.list_state.select(Some(sel + 1));
                }
            }
        }
    }

    pub fn select_next(&mut self) {
        let len = self.display_len();
        if len == 0 {
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some((cur + 1) % len));
    }

    pub fn select_prev(&mut self) {
        let len = self.display_len();
        if len == 0 {
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0);
        self.list_state
            .select(Some(if cur == 0 { len - 1 } else { cur - 1 }));
    }

    pub fn rendered_rect(&self) -> Rect {
        self.rendered_rect
    }

    /// Returns the currently selected display index (not entry index).
    pub fn selected_display_idx(&self) -> Option<usize> {
        self.list_state.selected()
    }

    /// Select the entry at `rel_row` (rows from top of the inner list area,
    /// i.e. after the block border). Returns the selected display index if a
    /// valid item was found, `None` otherwise.
    pub fn select_at_visual_row(&mut self, rel_row: u16) -> Option<usize> {
        let idx = self.display_idx_at_row(rel_row)?;
        self.list_state.select(Some(idx));
        Some(idx)
    }

    pub fn selected_entry(&self) -> Option<&FileListEntry> {
        let display_idx = self.list_state.selected()?;
        if self.create_entry.is_some() {
            if display_idx == 0 {
                return self.create_entry.as_ref();
            }
            let adjusted = display_idx - 1;
            let entry_idx = match &self.display_indices {
                None => adjusted,
                Some(v) => *v.get(adjusted)?,
            };
            return self.entries.get(entry_idx);
        }
        let entry_idx = match &self.display_indices {
            None => display_idx,
            Some(v) => *v.get(display_idx)?,
        };
        self.entries.get(entry_idx)
    }

    fn activate_selected(&self, tx: &AppTx) {
        let Some(display_idx) = self.list_state.selected() else {
            return;
        };
        if self.create_entry.is_some() && display_idx == 0 {
            if let Some(entry) = &self.create_entry {
                tx.send(AppEvent::OpenPath(entry.path().clone())).ok();
            }
            return;
        }
        let adjusted = if self.create_entry.is_some() {
            display_idx - 1
        } else {
            display_idx
        };
        let entry_idx = match &self.display_indices {
            None => adjusted,
            Some(v) => v[adjusted],
        };
        tx.send(AppEvent::OpenPath(self.entries[entry_idx].path().clone()))
            .ok();
    }

    fn display_idx_at_row(&self, row: u16) -> Option<usize> {
        let offset = self.list_state.offset();
        let len = self.display_len();
        let mut y = 0u16;
        for display_idx in offset..len {
            let h = if self.create_entry.is_some() && display_idx == 0 {
                self.create_entry.as_ref().map(|e| e.visual_height()).unwrap_or(1)
            } else {
                let adjusted = if self.create_entry.is_some() {
                    display_idx - 1
                } else {
                    display_idx
                };
                let entry_idx = match &self.display_indices {
                    None => adjusted,
                    Some(v) => v[adjusted],
                };
                self.entries.get(entry_idx).map(|e| e.visual_height()).unwrap_or(1)
            };
            if row < y + h {
                return Some(display_idx);
            }
            y += h;
        }
        None
    }

    fn header_title(&self) -> String {
        format!(" [{}{}]", self.sort_field.label(), self.sort_order.label())
    }
}

impl Component for FileListComponent {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Key(key) => {
                // Check keybindings first for action shortcuts.
                if let Some(combo) = key_event_to_combo(key) {
                    match self.key_bindings.get_action(&combo) {
                        Some(ActionShortcuts::FocusEditor) => {
                            tx.send(AppEvent::FocusEditor).ok();
                            return EventState::Consumed;
                        }
                        Some(ActionShortcuts::CycleSortField) => {
                            let field = self.sort_field.cycle();
                            self.set_sort(field, self.sort_order, tx.clone());
                            return EventState::Consumed;
                        }
                        Some(ActionShortcuts::SortReverseOrder) => {
                            let order = self.sort_order.toggle();
                            self.set_sort(self.sort_field, order, tx.clone());
                            return EventState::Consumed;
                        }
                        Some(ActionShortcuts::FileOperations) => {
                            if let Some(entry) = self.selected_entry() {
                                if !matches!(entry, FileListEntry::Up { .. }) {
                                    tx.send(AppEvent::ShowFileOpsMenu(entry.path().clone())).ok();
                                    return EventState::Consumed;
                                }
                            }
                            return EventState::NotConsumed;
                        }
                        _ => {}
                    }
                }
                // Navigation and search input.
                match key.code {
                    KeyCode::Up => {
                        self.select_prev();
                        EventState::Consumed
                    }
                    KeyCode::Down => {
                        self.select_next();
                        EventState::Consumed
                    }
                    KeyCode::Enter => {
                        self.activate_selected(tx);
                        EventState::Consumed
                    }
                    KeyCode::Char(c) => {
                        let non_shift = key.modifiers - KeyModifiers::SHIFT;
                        if non_shift.is_empty() {
                            if key.modifiers.contains(KeyModifiers::SHIFT) {
                                self.search_query.push(c.to_ascii_uppercase());
                            } else {
                                self.search_query.push(c);
                            }
                            self.schedule_filter(tx.clone());
                        }
                        // Consume regardless — prevents modifier combos (e.g. Ctrl+K)
                        // from leaking a character into the search box.
                        EventState::Consumed
                    }
                    KeyCode::Backspace => {
                        self.search_query.pop();
                        self.schedule_filter(tx.clone());
                        EventState::Consumed
                    }
                    _ => EventState::NotConsumed,
                }
            }
            InputEvent::Mouse(mouse) => {
                let r = &self.rendered_rect;
                let in_bounds = mouse.column >= r.x
                    && mouse.column < r.x + r.width
                    && mouse.row >= r.y
                    && mouse.row < r.y + r.height;
                if !in_bounds {
                    return EventState::NotConsumed;
                }
                match mouse.kind {
                    MouseEventKind::Down(_) => {
                        tx.send(AppEvent::FocusSidebar).ok();
                        // row 0 is the border/header; list starts at row 1
                        if mouse.row > r.y {
                            let rel_row = mouse.row - r.y - 1;
                            if let Some(idx) = self.display_idx_at_row(rel_row) {
                                if self.list_state.selected() == Some(idx) {
                                    self.activate_selected(tx);
                                } else {
                                    self.list_state.select(Some(idx));
                                }
                            }
                        }
                        EventState::Consumed
                    }
                    MouseEventKind::ScrollUp => {
                        self.scroll_up();
                        EventState::Consumed
                    }
                    MouseEventKind::ScrollDown => {
                        self.scroll_down();
                        EventState::Consumed
                    }
                    _ => EventState::NotConsumed,
                }
            }
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.poll_filter();
        self.rendered_rect = rect;
        let title = self.header_title();

        let bg_even = theme.bg.to_ratatui();
        let bg_odd = theme.bg_panel.to_ratatui();

        let entry_iter: Box<dyn Iterator<Item = &FileListEntry>> = match &self.display_indices {
            None => Box::new(self.entries.iter()),
            Some(indices) => Box::new(indices.iter().map(|&i| &self.entries[i])),
        };
        let create_iter: Box<dyn Iterator<Item = &FileListEntry>> = match &self.create_entry {
            Some(e) => Box::new(std::iter::once(e)),
            None => Box::new(std::iter::empty()),
        };
        let items: Vec<ListItem> = create_iter
            .chain(entry_iter)
            .enumerate()
            .map(|(i, e)| {
                let bg = if i % 2 == 0 { bg_even } else { bg_odd };
                e.to_list_item(theme, &self.icons)
                    .style(Style::default().bg(bg))
            })
            .collect();

        let border_style = theme.border_style(focused);

        let make_block = || {
            Block::default()
                .title(title.as_str())
                .borders(Borders::ALL)
                .border_style(border_style)
                .style(theme.panel_style())
        };

        let has_content = self
            .entries
            .iter()
            .any(|e| !matches!(e, FileListEntry::Up { .. }));
        if self.loading && !has_content {
            let loading = Paragraph::new("Loading…")
                .style(
                    Style::default()
                        .fg(theme.fg_muted.to_ratatui())
                        .bg(theme.bg_panel.to_ratatui()),
                )
                .block(make_block());
            f.render_widget(loading, rect);
        } else {
            let list = List::new(items).block(make_block()).highlight_style(
                Style::default()
                    .fg(theme.fg_selected.to_ratatui())
                    .bg(theme.bg_selected.to_ratatui()),
            );
            f.render_stateful_widget(list, rect, &mut self.list_state);
        }
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        [
            (ActionShortcuts::FocusEditor, "focus editor"),
            (ActionShortcuts::CycleSortField, "cycle sort"),
            (ActionShortcuts::SortReverseOrder, "reverse"),
            (ActionShortcuts::FileOperations, "file ops"),
        ]
        .iter()
        .filter_map(|(action, label)| {
            self.key_bindings
                .first_combo_for(action)
                .map(|k| (k, label.to_string()))
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use kimun_core::nfs::VaultPath;

    use super::*;

    fn make_tx() -> AppTx {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        tx
    }

    fn make_list() -> FileListComponent {
        FileListComponent::new(
            crate::keys::KeyBindings::empty(),
            crate::settings::icons::Icons::new(true),
        )
    }

    #[tokio::test]
    async fn schedule_filter_stores_handle_and_cancels_previous() {
        let tx = make_tx();
        let mut list = FileListComponent::new(crate::keys::KeyBindings::empty(), crate::settings::icons::Icons::new(true));
        for i in 0..20 {
            list.push_entry(make_note(&format!("{i}.md"), &format!("Note {i}")));
        }

        list.search_query = "note".to_string();
        list.schedule_filter(tx.clone());

        // After scheduling, a task handle must be stored.
        assert!(
            list.filter_task.is_some(),
            "filter_task should be Some after first schedule"
        );

        // Schedule again — the implementation must abort the old task and store a new handle.
        list.search_query = "note 1".to_string();
        list.schedule_filter(tx.clone());

        assert!(
            list.filter_task.is_some(),
            "filter_task should still be Some after re-schedule"
        );
    }

    #[tokio::test]
    async fn clear_aborts_filter_task() {
        let tx = make_tx();
        let mut list = FileListComponent::new(crate::keys::KeyBindings::empty(), crate::settings::icons::Icons::new(true));
        for i in 0..20 {
            list.push_entry(make_note(&format!("{i}.md"), &format!("Note {i}")));
        }
        list.search_query = "note".to_string();
        list.schedule_filter(tx);

        assert!(list.filter_task.is_some());
        list.clear();
        // After clear, the handle should be gone.
        assert!(
            list.filter_task.is_none(),
            "filter_task should be None after clear"
        );
    }

    fn make_note(filename: &str, title: &str) -> FileListEntry {
        FileListEntry::Note {
            path: VaultPath::new(filename),
            title: title.to_string(),
            filename: filename.to_string(),
            journal_date: None,
        }
    }

    fn entry_filenames(list: &FileListComponent) -> Vec<&str> {
        list.entries
            .iter()
            .filter_map(|e| match e {
                FileListEntry::Note { filename, .. } => Some(filename.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn render_accepts_focused_parameter() {
        // Verifies the new API: render(f, rect, theme, focused: bool) via Component trait.
        use crate::components::Component;
        use ratatui::{Terminal, backend::TestBackend};
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut list = FileListComponent::new(crate::keys::KeyBindings::empty(), crate::settings::icons::Icons::new(true));
        terminal
            .draw(|f| {
                list.render(
                    f,
                    f.area(),
                    &crate::settings::themes::Theme::default(),
                    false,
                );
            })
            .unwrap();
    }

    #[test]
    fn file_list_implements_component_trait() {
        // RED: fails to compile until FileListComponent implements Component.
        // GREEN: compiles once `impl Component for FileListComponent` is added.
        use crate::components::Component;
        let mut list = FileListComponent::new(crate::keys::KeyBindings::empty(), crate::settings::icons::Icons::new(true));
        let _: &mut dyn Component = &mut list;
    }

    #[test]
    fn selected_entry_returns_highlighted_item() {
        let mut list = FileListComponent::new(
            crate::keys::KeyBindings::empty(),
            crate::settings::icons::Icons::new(true),
        );
        list.push_entry(make_note("a.md", "A"));
        list.push_entry(make_note("b.md", "B"));
        // Default selection is index 0
        let entry = list.selected_entry();
        assert!(entry.is_some());
        if let Some(FileListEntry::Note { filename, .. }) = entry {
            assert_eq!(filename, "a.md");
        } else {
            panic!("expected Note entry");
        }
    }

    #[test]
    fn selected_entry_returns_none_when_empty() {
        let list = FileListComponent::new(
            crate::keys::KeyBindings::empty(),
            crate::settings::icons::Icons::new(true),
        );
        assert!(list.selected_entry().is_none());
    }

    #[test]
    fn prepend_create_entry_inserts_at_position_zero() {
        let mut list = FileListComponent::new(
            crate::keys::KeyBindings::empty(),
            crate::settings::icons::Icons::new(true),
        );
        list.push_entry(make_note("a.md", "A"));
        list.prepend_create_entry(FileListEntry::CreateNote {
            filename: "new-note.md".to_string(),
            path: VaultPath::new("new-note.md"),
        });
        assert!(matches!(
            &list.entries[0],
            FileListEntry::CreateNote { filename, .. } if filename == "new-note.md"
        ));
    }

    #[test]
    fn push_entry_does_not_sort() {
        let mut list = FileListComponent::new(crate::keys::KeyBindings::empty(), crate::settings::icons::Icons::new(true));
        list.push_entry(make_note("z.md", "Z Note"));
        list.push_entry(make_note("a.md", "A Note"));
        list.push_entry(make_note("m.md", "M Note"));
        // Without sorting, entries stay in insertion order
        assert_eq!(entry_filenames(&list), vec!["z.md", "a.md", "m.md"]);
    }

    #[test]
    fn finalize_sort_sorts_by_name() {
        let mut list = FileListComponent::new(crate::keys::KeyBindings::empty(), crate::settings::icons::Icons::new(true));
        list.push_entry(make_note("z.md", "Z Note"));
        list.push_entry(make_note("a.md", "A Note"));
        list.push_entry(make_note("m.md", "M Note"));
        list.finalize_sort();
        assert_eq!(entry_filenames(&list), vec!["a.md", "m.md", "z.md"]);
    }

    fn make_keybindings_with_file_ops() -> crate::keys::KeyBindings {
        use crate::keys::key_strike::KeyStrike;
        let mut kb = crate::keys::KeyBindings::empty();
        kb.batch_add()
            .add(KeyStrike::F2, crate::keys::action_shortcuts::ActionShortcuts::FileOperations);
        kb
    }

    #[tokio::test]
    async fn f2_sends_show_file_ops_menu() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use crate::components::events::InputEvent;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        let kb = make_keybindings_with_file_ops();
        let mut list = FileListComponent::new(kb, crate::settings::icons::Icons::new(true));
        list.push_entry(make_note("test.md", "Test Note"));

        let key_event = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
        let input = InputEvent::Key(key_event);
        let result = list.handle_input(&input, &tx);

        assert!(
            matches!(result, EventState::Consumed),
            "expected Consumed but got {:?}",
            result
        );

        let event = rx.try_recv().expect("expected ShowFileOpsMenu to be sent");
        assert!(
            matches!(event, AppEvent::ShowFileOpsMenu(_)),
            "expected ShowFileOpsMenu but got {:?}",
            event
        );
    }

    #[tokio::test]
    async fn file_ops_not_consumed_for_up_entry() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use crate::components::events::InputEvent;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        let kb = make_keybindings_with_file_ops();
        let mut list = FileListComponent::new(kb, crate::settings::icons::Icons::new(true));
        list.add_up_entry(VaultPath::root());
        // Up entry is selected (index 0)

        let key_event = KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE);
        let input = InputEvent::Key(key_event);
        let result = list.handle_input(&input, &tx);

        assert!(
            matches!(result, EventState::NotConsumed),
            "expected NotConsumed for Up entry but got {:?}",
            result
        );
        assert!(rx.try_recv().is_err(), "no event should be sent for Up entry");
    }

    #[test]
    fn set_create_entry_shows_at_virtual_index_zero() {
        let mut list = make_list();
        list.push_entry(make_note("a.md", "A"));
        list.push_entry(make_note("b.md", "B"));
        list.set_create_entry(Some(FileListEntry::CreateNote {
            filename: "new.md".to_string(),
            path: VaultPath::new("new.md"),
        }));
        // 2 notes + 1 virtual create entry
        assert_eq!(list.display_len(), 3);
        // Selection resets to 0, which is the CreateNote
        assert!(matches!(
            list.selected_entry(),
            Some(FileListEntry::CreateNote { filename, .. }) if filename == "new.md"
        ));
    }

    #[test]
    fn set_create_entry_none_hides_it() {
        let mut list = make_list();
        list.push_entry(make_note("a.md", "A"));
        list.set_create_entry(Some(FileListEntry::CreateNote {
            filename: "new.md".to_string(),
            path: VaultPath::new("new.md"),
        }));
        list.set_create_entry(None);
        assert_eq!(list.display_len(), 1);
        assert!(matches!(list.selected_entry(), Some(FileListEntry::Note { .. })));
    }

    #[test]
    fn clear_removes_create_entry() {
        let mut list = make_list();
        list.set_create_entry(Some(FileListEntry::CreateNote {
            filename: "new.md".to_string(),
            path: VaultPath::new("new.md"),
        }));
        list.clear();
        assert!(list.create_entry.is_none());
        assert_eq!(list.display_len(), 0);
    }
}
