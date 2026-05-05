use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::settings::themes::Theme;
use chrono::NaiveDate;
use kimun_core::SearchResult;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, ResultType};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::file_list::{FileListComponent, FileListEntry, SortField, SortOrder};
use crate::keys::KeyBindings;
use crate::settings::AppSettings;
use crate::settings::icons::Icons;

pub struct SidebarComponent {
    current_dir: VaultPath,
    pub file_list: FileListComponent,
    pending_rx: Option<Receiver<SearchResult>>,
    vault: Arc<NoteVault>,
    default_sort_field: SortField,
    default_sort_order: SortOrder,
    journal_sort_field: SortField,
    journal_sort_order: SortOrder,
    rendered_rect: Rect,
    list_rect: Rect,
}

impl SidebarComponent {
    pub fn new(
        key_bindings: KeyBindings,
        vault: Arc<NoteVault>,
        icons: Icons,
        settings: &AppSettings,
    ) -> Self {
        Self {
            current_dir: VaultPath::root(),
            file_list: FileListComponent::new(key_bindings, icons),
            pending_rx: None,
            vault,
            default_sort_field: SortField::from(settings.default_sort_field),
            default_sort_order: SortOrder::from(settings.default_sort_order),
            journal_sort_field: SortField::from(settings.journal_sort_field),
            journal_sort_order: SortOrder::from(settings.journal_sort_order),
            rendered_rect: Rect::default(),
            list_rect: Rect::default(),
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
            let filename = path.to_string();
            self.file_list
                .set_create_entry(Some(FileListEntry::CreateNote { filename, path }));
        }
    }

    fn activate_selected_entry(&self, tx: &AppTx) {
        if let Some(FileListEntry::CreateNote { path, .. }) = self.file_list.selected_entry() {
            let path = path.clone();
            let vault = Arc::clone(&self.vault);
            let tx2 = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = vault.load_or_create_note(&path, None).await {
                    tracing::warn!("create note failed for {path}: {e}");
                    return;
                }
                tx2.send(AppEvent::OpenPath(path)).ok();
            });
            return;
        }
        self.file_list.activate_selected(tx);
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
        if let InputEvent::Mouse(mouse) = event {
            let pos = Position {
                x: mouse.column,
                y: mouse.row,
            };
            if !self.rendered_rect.contains(pos) {
                return EventState::NotConsumed;
            }
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    tx.send(AppEvent::FocusSidebar).ok();
                    if self.list_rect.contains(pos) && mouse.row > self.list_rect.y {
                        // row 0 of the list block is the border; rows start at y+1.
                        let rel_row = mouse.row - self.list_rect.y - 1;
                        let prev = self.file_list.selected_display_idx();
                        if let Some(idx) = self.file_list.select_at_visual_row(rel_row)
                            && prev == Some(idx)
                        {
                            self.activate_selected_entry(tx);
                        }
                    }
                }
                MouseEventKind::ScrollUp => self.file_list.scroll_up(),
                MouseEventKind::ScrollDown => self.file_list.scroll_down(),
                _ => {}
            }
            return EventState::Consumed;
        }

        if let InputEvent::Key(key) = event
            && key.code == KeyCode::Enter
        {
            self.activate_selected_entry(tx);
            return EventState::Consumed;
        }

        let result = self.file_list.handle_input(event, tx);

        // After a key that modifies the search query, keep the create entry in sync.
        if let InputEvent::Key(key) = event
            && matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace)
        {
            self.sync_create_entry();
        }

        result
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        self.file_list.hint_shortcuts()
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.poll_loading();
        self.rendered_rect = rect;

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(rect);
        self.list_rect = rows[2];

        let border_style = theme.border_style(focused);

        let header = Block::default()
            .title(self.current_dir.to_string())
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        let header_inner = header.inner(rows[0]);
        f.render_widget(header, rows[0]);
        f.render_widget(
            Paragraph::new(format!("{} notes", self.file_list.note_count())).style(
                Style::default()
                    .fg(theme.fg_muted.to_ratatui())
                    .bg(theme.bg_panel.to_ratatui()),
            ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use crate::test_support::{mouse_down_at, temp_vault};
    use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    use tokio::sync::mpsc::unbounded_channel;

    async fn make_sidebar() -> SidebarComponent {
        let vault = temp_vault("sidebar").await;
        let settings = AppSettings::default();
        SidebarComponent::new(
            settings.key_bindings.clone(),
            vault,
            settings.icons(),
            &settings,
        )
    }

    /// Regression: clicking on the sidebar header (directory name + note count)
    /// or the search box must focus the sidebar, not just clicks on the file list.
    #[tokio::test]
    async fn mouse_down_on_header_focuses_sidebar() {
        let mut sidebar = make_sidebar().await;
        sidebar.rendered_rect = Rect {
            x: 0,
            y: 3,
            width: 30,
            height: 20,
        };
        let (tx, mut rx) = unbounded_channel();

        // Click inside the header (top-of-sidebar) area.
        let result = sidebar.handle_input(&mouse_down_at(5, 4), &tx);
        assert_eq!(result, EventState::Consumed);
        let evt = rx.try_recv().expect("should send a focus event");
        assert!(matches!(evt, AppEvent::FocusSidebar));
    }

    #[tokio::test]
    async fn mouse_down_on_search_box_focuses_sidebar() {
        let mut sidebar = make_sidebar().await;
        sidebar.rendered_rect = Rect {
            x: 0,
            y: 3,
            width: 30,
            height: 20,
        };
        let (tx, mut rx) = unbounded_channel();

        // Click inside the search-box area (rows 6..9 within the sidebar layout).
        let result = sidebar.handle_input(&mouse_down_at(5, 7), &tx);
        assert_eq!(result, EventState::Consumed);
        let evt = rx.try_recv().expect("should send a focus event");
        assert!(matches!(evt, AppEvent::FocusSidebar));
    }

    fn scroll_event_at(col: u16, row: u16, kind: MouseEventKind) -> InputEvent {
        InputEvent::Mouse(MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn push_note(sidebar: &mut SidebarComponent, name: &str) {
        sidebar.file_list.entries.push(FileListEntry::Note {
            path: VaultPath::note_path_from(name),
            title: name.to_string(),
            filename: format!("{name}.md"),
            journal_date: None,
        });
    }

    /// Two clicks on the same list row activate it: first selects, second sends
    /// `OpenPath` (or, for `CreateNote`, materialises the note then opens it).
    #[tokio::test]
    async fn mouse_double_click_on_list_row_sends_open_path() {
        let mut sidebar = make_sidebar().await;
        push_note(&mut sidebar, "alpha");
        sidebar.rendered_rect = Rect {
            x: 0,
            y: 3,
            width: 30,
            height: 20,
        };
        sidebar.list_rect = Rect {
            x: 0,
            y: 9,
            width: 30,
            height: 14,
        };
        let (tx, mut rx) = unbounded_channel();

        // First click: in list area, on the first row (list_rect.y + 1).
        sidebar.handle_input(&mouse_down_at(5, 10), &tx);
        // Drain the FocusSidebar event from the first click.
        let _ = rx.try_recv();

        // Second click on the same row activates the entry.
        sidebar.handle_input(&mouse_down_at(5, 10), &tx);
        // First event from the second click is FocusSidebar; the next is OpenPath.
        let mut events = Vec::new();
        while let Ok(evt) = rx.try_recv() {
            events.push(evt);
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AppEvent::OpenPath(p) if p.to_string().contains("alpha"))),
            "expected OpenPath for the activated note, got {events:?}"
        );
    }

    /// Scroll wheel anywhere in the sidebar bounds scrolls the file list — even
    /// when the cursor is over the header or search box.
    #[tokio::test]
    async fn scroll_down_in_sidebar_bounds_scrolls_list() {
        let mut sidebar = make_sidebar().await;
        push_note(&mut sidebar, "alpha");
        push_note(&mut sidebar, "beta");
        // Pin selection to index 0 so ScrollDown moves it to 1.
        sidebar.file_list.select_at_visual_row(0);
        sidebar.rendered_rect = Rect {
            x: 0,
            y: 3,
            width: 30,
            height: 20,
        };
        sidebar.list_rect = Rect {
            x: 0,
            y: 9,
            width: 30,
            height: 14,
        };
        let (tx, _rx) = unbounded_channel();
        assert_eq!(sidebar.file_list.selected_display_idx(), Some(0));

        // Scroll down with the cursor inside the sidebar header (not the list).
        let result = sidebar.handle_input(&scroll_event_at(5, 4, MouseEventKind::ScrollDown), &tx);
        assert_eq!(result, EventState::Consumed);
        assert_eq!(
            sidebar.file_list.selected_display_idx(),
            Some(1),
            "scroll-from-header should still scroll the list"
        );
    }

    #[tokio::test]
    async fn mouse_down_outside_sidebar_is_not_consumed() {
        let mut sidebar = make_sidebar().await;
        sidebar.rendered_rect = Rect {
            x: 0,
            y: 3,
            width: 30,
            height: 20,
        };
        let (tx, mut rx) = unbounded_channel();

        // Click to the right of the sidebar (in the editor area).
        let result = sidebar.handle_input(&mouse_down_at(50, 10), &tx);
        assert_eq!(result, EventState::NotConsumed);
        assert!(rx.try_recv().is_err());
    }
}
