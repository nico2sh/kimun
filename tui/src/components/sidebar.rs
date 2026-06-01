use std::sync::Arc;

use crate::settings::themes::Theme;
use async_trait::async_trait;
use chrono::NaiveDate;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, NotesValidation, ResultType, VaultBrowseOptionsBuilder};
use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, redraw_callback};
use crate::components::file_list::{FileListEntry, SortField, SortOrder};
use crate::components::search_list::{
    Emit, Filter, KeyReaction, RowSource, SearchList, SearchMouse,
};
use crate::keys::KeyBindings;
use crate::settings::AppSettings;
use crate::settings::icons::Icons;

/// Streamed `RowSource` over one directory's listing. Pushes an `Up` row first
/// (when not at root) so it is always present, then forwards each
/// `browse_vault` result. Loads once; a local `Filter::Fuzzy` narrows the set
/// and `leading_row` provides the "Create: …" affordance.
struct DirListingSource {
    vault: Arc<NoteVault>,
    dir: VaultPath,
    sort_field: SortField,
    sort_order: SortOrder,
}

#[async_trait]
impl RowSource<FileListEntry> for DirListingSource {
    async fn load(&self, _query: &str, emit: Emit<FileListEntry>) {
        // Up row first (if not root) — pushed so it's always present.
        if !self.dir.is_root_or_empty() {
            emit.push(FileListEntry::Up {
                parent: self.dir.get_parent_path().0,
            });
        }

        let (options, rx) = VaultBrowseOptionsBuilder::new(&self.dir)
            .recursive(false)
            .validation(NotesValidation::Full)
            .build();

        let vault = self.vault.clone();
        // browse_vault fills `rx`; spawn it so we can drain concurrently.
        let browse = tokio::spawn(async move { vault.browse_vault(options).await });

        // `rx` is a std mpsc Receiver; `recv` blocks, so drain it on a blocking
        // thread, sort the gathered entries, then push them in display order.
        let vault = self.vault.clone();
        let dir = self.dir.clone();
        let field = self.sort_field;
        let order = self.sort_order;
        let drain = tokio::task::spawn_blocking(move || {
            let mut entries: Vec<FileListEntry> = Vec::new();
            while let Ok(result) = rx.recv() {
                if matches!(result.rtype, ResultType::Directory) && result.path == dir {
                    continue;
                }
                let journal_date = vault.journal_date(&result.path).map(format_journal_date);
                entries.push(FileListEntry::from_result(result, journal_date));
            }
            entries.sort_by(|a, b| {
                let ka = a.sort_key(field);
                let kb = b.sort_key(field);
                match order {
                    SortOrder::Ascending => ka.cmp(&kb),
                    SortOrder::Descending => kb.cmp(&ka),
                }
            });
            entries
        });

        if let Ok(entries) = drain.await {
            for entry in entries {
                emit.push(entry);
            }
        }
        let _ = browse.await;
        emit.done();
    }

    fn leading_row(&self, query: &str) -> Option<FileListEntry> {
        if query.is_empty() {
            None
        } else {
            let path = self
                .dir
                .append(&VaultPath::note_path_from(query))
                .flatten();
            Some(FileListEntry::CreateNote {
                filename: path.to_string(),
                path,
            })
        }
    }

    fn reload_on_query(&self) -> bool {
        // Load the directory once; the local fuzzy filter narrows it and
        // `leading_row` keeps the create affordance in sync per keystroke.
        false
    }
}

pub struct SidebarComponent {
    current_dir: VaultPath,
    list: Option<SearchList<FileListEntry>>,
    vault: Arc<NoteVault>,
    icons: Icons,
    default_sort_field: SortField,
    default_sort_order: SortOrder,
    journal_sort_field: SortField,
    journal_sort_order: SortOrder,
    rendered_rect: Rect,
    list_rect: Rect,
}

impl SidebarComponent {
    pub fn new(
        _key_bindings: KeyBindings,
        vault: Arc<NoteVault>,
        icons: Icons,
        settings: &AppSettings,
    ) -> Self {
        Self {
            current_dir: VaultPath::root(),
            list: None,
            vault,
            icons,
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

    /// `true` until a directory has been loaded (no engine yet). The editor
    /// uses this to decide whether to issue the first-open navigation.
    pub fn is_empty(&self) -> bool {
        self.list.is_none()
    }

    /// Sort field/order to apply for `dir` (journal dirs get their own).
    fn sort_for(&self, dir: &VaultPath) -> (SortField, SortOrder) {
        if dir == self.vault.journal_path() {
            (self.journal_sort_field, self.journal_sort_order)
        } else {
            (self.default_sort_field, self.default_sort_order)
        }
    }

    /// (Re)build the engine for `dir`, replacing any prior listing. This is the
    /// single directory-navigation entry point: changing directory = rebuild
    /// the engine with a fresh `DirListingSource` for the new dir.
    pub fn navigate(&mut self, dir: VaultPath, tx: &AppTx) {
        self.current_dir = dir.clone();
        let (sort_field, sort_order) = self.sort_for(&dir);
        let source = DirListingSource {
            vault: self.vault.clone(),
            dir,
            sort_field,
            sort_order,
        };
        self.list = Some(
            SearchList::builder(source, redraw_callback(tx.clone()))
                .filter(Filter::Fuzzy)
                .icons(self.icons.clone())
                .build(),
        );
    }

    /// Number of note rows currently visible (excludes Up / dirs / create).
    fn note_count(&self) -> usize {
        match &self.list {
            None => 0,
            Some(list) => list
                .visible_rows()
                .iter()
                .filter(|e| matches!(e, FileListEntry::Note { .. }))
                .count(),
        }
    }

    /// Act on the selected row: Up/Note/Directory → `OpenPath` (directories and
    /// Up route back through the editor's navigate, rebuilding the engine);
    /// CreateNote → materialise the note, then open it.
    fn activate_selected_entry(&self, tx: &AppTx) {
        let Some(list) = &self.list else { return };
        let Some(entry) = list.selected_row() else {
            return;
        };
        match entry {
            FileListEntry::CreateNote { path, .. } => {
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
            }
            other => {
                tx.send(AppEvent::OpenPath(other.path().clone())).ok();
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
            // Any click in the sidebar focuses it.
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                tx.send(AppEvent::FocusSidebar).ok();
            }
            if let Some(list) = &mut self.list {
                match mouse.kind {
                    // Scroll anywhere within the sidebar bounds moves the list
                    // selection — even when the cursor is over the header or
                    // search box (the engine only hit-tests scroll inside its
                    // own list rect, so handle it here).
                    MouseEventKind::ScrollUp => list.select_prev(),
                    MouseEventKind::ScrollDown => list.select_next(),
                    _ => match list.handle_mouse(mouse) {
                        SearchMouse::Activated(_) => self.activate_selected_entry(tx),
                        SearchMouse::Selected(_)
                        | SearchMouse::Scrolled
                        | SearchMouse::None => {}
                    },
                }
            }
            return EventState::Consumed;
        }

        if let InputEvent::Key(key) = event {
            let Some(list) = &mut self.list else {
                return EventState::NotConsumed;
            };
            match list.handle_key(key) {
                KeyReaction::Submit => {
                    self.activate_selected_entry(tx);
                    EventState::Consumed
                }
                KeyReaction::Consumed | KeyReaction::Cancel => EventState::Consumed,
                KeyReaction::Intercepted(_) | KeyReaction::Unhandled => EventState::NotConsumed,
            }
        } else {
            EventState::NotConsumed
        }
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
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
            Paragraph::new(format!("{} notes", self.note_count())).style(
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

        let list_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        let list_inner = list_block.inner(rows[2]);
        f.render_widget(list_block, rows[2]);

        if let Some(list) = &mut self.list {
            list.render_query(f, search_inner, theme, focused);
            list.render(f, list_inner, theme, focused);
            // Record the OUTER list rect for mouse hit-testing (the engine
            // hit-tests as `row - rect.y - 1`, expecting row 0 to be a border).
            list.set_list_rect(rows[2]);
        }
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
        vault.validate_and_init().await.unwrap();
        let settings = AppSettings::default();
        SidebarComponent::new(
            settings.key_bindings.clone(),
            vault,
            settings.icons(),
            &settings,
        )
    }

    /// Build a sidebar over `vault` after creating each named note at root.
    async fn sidebar_with_notes(prefix: &str, names: &[&str]) -> SidebarComponent {
        let vault = temp_vault(prefix).await;
        vault.validate_and_init().await.unwrap();
        for name in names {
            vault
                .create_note(&VaultPath::note_path_from(name), "body")
                .await
                .unwrap();
        }
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

    /// Load the sidebar at the vault root, then poll the engine to idle so the
    /// streamed rows have arrived.
    async fn navigate_to_root(sidebar: &mut SidebarComponent, tx: &AppTx) {
        sidebar.navigate(VaultPath::root(), tx);
        // The streamed source spawns `browse_vault` + a blocking drain; give the
        // background work real time to land, polling the engine between waits.
        for _ in 0..50 {
            if let Some(list) = &mut sidebar.list {
                list.poll();
                if !list.is_loading() {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        if let Some(list) = &mut sidebar.list {
            list.poll();
        }
    }

    /// Two clicks on the same list row activate it: first selects, second sends
    /// `OpenPath` (or, for `CreateNote`, materialises the note then opens it).
    #[tokio::test(flavor = "multi_thread")]
    async fn mouse_double_click_on_list_row_sends_open_path() {
        let mut sidebar = sidebar_with_notes("sidebar-dbl", &["alpha"]).await;
        let (tx, mut rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;

        sidebar.rendered_rect = Rect {
            x: 0,
            y: 3,
            width: 30,
            height: 20,
        };
        // The engine records the outer list rect; clicks hit-test as row - y - 1.
        if let Some(list) = &mut sidebar.list {
            list.set_list_rect(Rect {
                x: 0,
                y: 9,
                width: 30,
                height: 14,
            });
        }
        sidebar.list_rect = Rect {
            x: 0,
            y: 9,
            width: 30,
            height: 14,
        };

        // First click: in list area, on the first row (list_rect.y + 1).
        sidebar.handle_input(&mouse_down_at(5, 10), &tx);
        // Drain the FocusSidebar event from the first click.
        let _ = rx.try_recv();

        // Second click on the same row activates the entry.
        sidebar.handle_input(&mouse_down_at(5, 10), &tx);
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
    #[tokio::test(flavor = "multi_thread")]
    async fn scroll_down_in_sidebar_bounds_scrolls_list() {
        let mut sidebar = sidebar_with_notes("sidebar-scroll", &["alpha", "beta"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;

        sidebar.rendered_rect = Rect {
            x: 0,
            y: 3,
            width: 30,
            height: 20,
        };
        if let Some(list) = &mut sidebar.list {
            list.set_list_rect(Rect {
                x: 0,
                y: 9,
                width: 30,
                height: 14,
            });
        }

        let first = sidebar
            .list
            .as_ref()
            .unwrap()
            .selected_row()
            .map(|e| e.path().to_string());

        // Scroll down with the cursor inside the sidebar header (not the list).
        let result = sidebar.handle_input(&scroll_event_at(5, 4, MouseEventKind::ScrollDown), &tx);
        assert_eq!(result, EventState::Consumed);
        let after = sidebar
            .list
            .as_ref()
            .unwrap()
            .selected_row()
            .map(|e| e.path().to_string());
        assert_ne!(first, after, "scroll-from-header should move the selection");
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

    /// Navigating loads the directory's notes via the streamed source.
    #[tokio::test(flavor = "multi_thread")]
    async fn navigate_loads_directory_notes() {
        let mut sidebar = sidebar_with_notes("sidebar-nav", &["hello"]).await;
        assert!(sidebar.is_empty());
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;
        assert!(!sidebar.is_empty());
        assert_eq!(sidebar.note_count(), 1);
    }
}
