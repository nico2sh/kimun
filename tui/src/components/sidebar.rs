use std::sync::{Arc, Mutex};

use crate::settings::themes::Theme;
use async_trait::async_trait;
use chrono::NaiveDate;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, NotesValidation, ResultType, VaultBrowseOptionsBuilder};
use ratatui::Frame;
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
    /// Shared sort field/order. `load` reads it so the sidebar's interactive
    /// sort shortcuts (cycle field / reverse order) re-order the listing on
    /// reload; initialised per-directory from the default/journal settings.
    sort: Arc<Mutex<(SortField, SortOrder)>>,
    /// Shared "group directories first" flag, read by `load`.
    group_dirs: Arc<Mutex<bool>>,
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
        // Read the active sort out of the lock, then drop the guard before the
        // await on the blocking task.
        let (field, order) = *self.sort.lock().unwrap();
        let group_dirs = *self.group_dirs.lock().unwrap();
        let drain = tokio::task::spawn_blocking(move || {
            let mut entries: Vec<FileListEntry> = Vec::new();
            while let Ok(result) = rx.recv() {
                if matches!(result.rtype, ResultType::Directory) && result.path == dir {
                    continue;
                }
                let journal_date = vault.journal_date(&result.path).map(format_journal_date);
                entries.push(FileListEntry::from_result(result, journal_date));
            }
            let cmp = |a: &FileListEntry, b: &FileListEntry| {
                let ka = a.sort_key(field);
                let kb = b.sort_key(field);
                match order {
                    SortOrder::Ascending => ka.cmp(&kb),
                    SortOrder::Descending => kb.cmp(&ka),
                }
            };
            if group_dirs {
                let (mut dirs, mut rest): (Vec<_>, Vec<_>) = entries
                    .into_iter()
                    .partition(|e| matches!(e, FileListEntry::Directory { .. }));
                dirs.sort_by(&cmp);
                rest.sort_by(&cmp);
                dirs.extend(rest);
                dirs
            } else {
                entries.sort_by(&cmp);
                entries
            }
        });

        match drain.await {
            Ok(entries) => {
                for entry in entries {
                    emit.push(entry);
                }
            }
            Err(e) => tracing::warn!("sidebar directory listing drain failed: {e}"),
        }
        if let Err(e) = browse.await {
            tracing::warn!("sidebar browse_vault task failed: {e}");
        }
        emit.done();
    }

    fn leading_row(&self, query: &str) -> Option<FileListEntry> {
        if query.is_empty() {
            None
        } else {
            let path = self.dir.append(&VaultPath::note_path_from(query)).flatten();
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
    /// Shared sort field/order for the active listing. `DirListingSource::load`
    /// reads it; the sort shortcuts mutate it then reload. Re-created per
    /// `navigate` from the per-dir defaults.
    sort: Arc<Mutex<(SortField, SortOrder)>>,
    /// Shared "group directories first" flag. `DirListingSource::load` reads it;
    /// the sort dialog mutates it via `apply_sort`, then the listing reloads.
    group_dirs: Arc<Mutex<bool>>,
    rendered_rect: Rect,
    key_bindings: KeyBindings,
}

impl SidebarComponent {
    /// Build a sidebar from the application settings, pulling its key bindings
    /// and icons from `settings`. The shared constructor for the screens that
    /// host a sidebar (Editor and Browse), so the kb/icons wiring lives once.
    pub fn from_settings(vault: Arc<NoteVault>, settings: &AppSettings) -> Self {
        Self::new(
            settings.key_bindings.clone(),
            vault,
            settings.icons(),
            settings,
        )
    }

    pub fn new(
        key_bindings: KeyBindings,
        vault: Arc<NoteVault>,
        icons: Icons,
        settings: &AppSettings,
    ) -> Self {
        let default_sort_field = SortField::from(settings.default_sort_field);
        let default_sort_order = SortOrder::from(settings.default_sort_order);
        Self {
            current_dir: VaultPath::root(),
            list: None,
            vault,
            icons,
            default_sort_field,
            default_sort_order,
            journal_sort_field: SortField::from(settings.journal_sort_field),
            journal_sort_order: SortOrder::from(settings.journal_sort_order),
            sort: Arc::new(Mutex::new((default_sort_field, default_sort_order))),
            group_dirs: Arc::new(Mutex::new(settings.group_directories)),
            rendered_rect: Rect::default(),
            key_bindings,
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
        self.sort = Arc::new(Mutex::new((sort_field, sort_order)));
        let source = DirListingSource {
            vault: self.vault.clone(),
            dir,
            sort: self.sort.clone(),
            group_dirs: self.group_dirs.clone(),
        };
        self.list = Some(
            SearchList::builder(source, redraw_callback(tx.clone()))
                .filter(Filter::Fuzzy)
                .icons(self.icons.clone())
                .build(),
        );
    }

    /// Current sort field/order for the active listing.
    pub fn current_sort(&self) -> (SortField, SortOrder) {
        *self.sort.lock().unwrap()
    }

    /// Current "group directories first" flag.
    pub fn group_dirs(&self) -> bool {
        *self.group_dirs.lock().unwrap()
    }

    /// Apply a sort selection from the sort dialog and reload so the source
    /// re-orders the listing.
    pub fn apply_sort(&mut self, field: SortField, order: SortOrder, group_dirs: bool) {
        *self.sort.lock().unwrap() = (field, order);
        *self.group_dirs.lock().unwrap() = group_dirs;
        if let Some(list) = &mut self.list {
            list.reload();
        }
    }

    /// `true` when the active directory is the journal (so its sort default is
    /// the journal one). Lets the caller persist to the matching settings.
    pub fn is_current_journal(&self) -> bool {
        &self.current_dir == self.vault.journal_path()
    }

    /// Save the dialog's selection as the in-session default for the active
    /// context (journal vs. normal), then apply it live. Without this, the
    /// cached per-context defaults that `sort_for`/`navigate` read stay at their
    /// construction-time values, so a saved default would have no effect until
    /// restart. The caller is responsible for persisting to the settings file.
    pub fn save_default(&mut self, field: SortField, order: SortOrder, group_dirs: bool) {
        if self.is_current_journal() {
            self.journal_sort_field = field;
            self.journal_sort_order = order;
        } else {
            self.default_sort_field = field;
            self.default_sort_order = order;
        }
        self.apply_sort(field, order, group_dirs);
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
            // Click-to-focus is handled centrally by `PanelSet::handle_mouse`;
            // only the sidebar's internal behavior lives here. The engine
            // hit-tests the wheel against the recorded panel rect (the whole
            // sidebar — header and search box included) and clicks against
            // the list rect.
            if let Some(list) = &mut self.list {
                match list.handle_mouse(mouse) {
                    SearchMouse::Activated(_) => self.activate_selected_entry(tx),
                    // ContentScroll* are unreachable: this host records no
                    // content sub-region.
                    SearchMouse::Selected(_)
                    | SearchMouse::Scrolled
                    | SearchMouse::ContentScrollUp
                    | SearchMouse::ContentScrollDown
                    | SearchMouse::None => {}
                }
            }
            return EventState::Consumed;
        }

        if let InputEvent::Key(key) = event {
            if self.list.is_none() {
                return EventState::NotConsumed;
            }
            let reaction = self.list.as_mut().unwrap().handle_key(key);
            match reaction {
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
        use crate::keys::action_shortcuts::ActionShortcuts;

        [
            (ActionShortcuts::FocusEditor, "editor \u{2192}"),
            (ActionShortcuts::OpenSortDialog, "sort"),
        ]
        .iter()
        .filter_map(|(action, label)| {
            self.key_bindings
                .first_combo_for(action)
                .map(|k| (k, label.to_string()))
        })
        .collect()
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
                    .fg(theme.gray.to_ratatui())
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
            // Record the rendered-items rect (the block's inner area) for mouse
            // hit-testing: the engine maps a click to `row - rect.y`, row 0 being
            // the first item. The panel rect makes the wheel scroll from anywhere
            // within the sidebar.
            list.set_list_rect(list_inner);
            list.set_panel_rect(rect);
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

    /// Clicks anywhere in the sidebar bounds — header, search box, list — are
    /// consumed by the sidebar. (Click-to-focus itself is handled centrally by
    /// `PanelSet::handle_mouse`, not here.)
    #[tokio::test]
    async fn mouse_down_in_sidebar_bounds_is_consumed() {
        let mut sidebar = make_sidebar().await;
        sidebar.rendered_rect = Rect {
            x: 0,
            y: 3,
            width: 30,
            height: 20,
        };
        let (tx, _rx) = unbounded_channel();

        // Header (top-of-sidebar) area.
        assert_eq!(
            sidebar.handle_input(&mouse_down_at(5, 4), &tx),
            EventState::Consumed
        );
        // Search-box area (rows 6..9 within the sidebar layout).
        assert_eq!(
            sidebar.handle_input(&mouse_down_at(5, 7), &tx),
            EventState::Consumed
        );
        // Outside the sidebar bounds.
        assert_eq!(
            sidebar.handle_input(&mouse_down_at(40, 7), &tx),
            EventState::NotConsumed
        );
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
        // The engine records the rendered-items rect; clicks hit-test as
        // `row - rect.y`, so row 0 (y=9) is the first item.
        if let Some(list) = &mut sidebar.list {
            list.set_list_rect(Rect {
                x: 0,
                y: 9,
                width: 30,
                height: 14,
            });
        }

        // First click: in the list area, on the first row (rect.y) — selects.
        sidebar.handle_input(&mouse_down_at(5, 9), &tx);

        // Second click on the same row activates the entry.
        sidebar.handle_input(&mouse_down_at(5, 9), &tx);
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
    /// when the cursor is over the header or search box. The viewport moves and
    /// the selection is carried along (keeping its screen position), so with a
    /// 1-row viewport the selected row changes on the first scroll.
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
        // A 1-row viewport over 2 notes, so the list overflows and can scroll.
        // The panel rect covers the whole sidebar, so the wheel works from the
        // header/search box too.
        if let Some(list) = &mut sidebar.list {
            list.set_list_rect(Rect {
                x: 0,
                y: 9,
                width: 30,
                height: 1,
            });
            list.set_panel_rect(Rect {
                x: 0,
                y: 3,
                width: 30,
                height: 20,
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
        assert_ne!(
            first, after,
            "scroll-from-header should scroll the list, carrying the selection"
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

    /// Poll the (already-navigated) engine to idle so a reload's streamed rows
    /// have arrived.
    async fn poll_to_idle(sidebar: &mut SidebarComponent) {
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

    /// Names of the visible note rows, in display order.
    fn note_names(sidebar: &SidebarComponent) -> Vec<String> {
        sidebar
            .list
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .filter_map(|e| match e {
                FileListEntry::Note { filename, .. } => Some(filename.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_reverse_flips_listing_order() {
        let mut sidebar = sidebar_with_notes("sidebar-sort", &["alpha", "bravo", "charlie"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;
        let before = note_names(&sidebar);
        assert_eq!(before.len(), 3, "expected three notes, got {before:?}");
        sidebar.apply_sort(SortField::Name, SortOrder::Descending, false);
        poll_to_idle(&mut sidebar).await;
        let after = note_names(&sidebar);
        assert_eq!(
            after,
            before.iter().rev().cloned().collect::<Vec<_>>(),
            "descending order should reverse the listing"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_changes_field() {
        let mut sidebar = sidebar_with_notes("sidebar-cycle", &["alpha", "bravo"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;
        sidebar.apply_sort(SortField::Title, SortOrder::Ascending, false);
        poll_to_idle(&mut sidebar).await;
        assert_eq!(sidebar.current_sort().0, SortField::Title);
        assert_eq!(note_names(&sidebar).len(), 2, "notes survive the resort");
    }

    /// Build a sidebar over a vault with both notes and a subdirectory.
    async fn sidebar_with_notes_and_dir(prefix: &str) -> SidebarComponent {
        let vault = temp_vault(prefix).await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("alpha"), "body")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::note_path_from("z-dir/inner"), "body")
            .await
            .unwrap();
        let settings = AppSettings::default();
        SidebarComponent::new(
            settings.key_bindings.clone(),
            vault,
            settings.icons(),
            &settings,
        )
    }

    /// Kinds of the visible rows, in display order (excluding the Up row).
    fn row_kinds(sidebar: &SidebarComponent) -> Vec<&'static str> {
        sidebar
            .list
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .filter_map(|e| match e {
                FileListEntry::Note { .. } => Some("note"),
                FileListEntry::Directory { .. } => Some("dir"),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn group_dirs_puts_directories_first() {
        let mut sidebar = sidebar_with_notes_and_dir("sidebar-group").await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;
        assert_eq!(row_kinds(&sidebar), vec!["note", "dir"]);
        sidebar.apply_sort(SortField::Name, SortOrder::Ascending, true);
        poll_to_idle(&mut sidebar).await;
        assert_eq!(
            row_kinds(&sidebar),
            vec!["dir", "note"],
            "grouping must cluster directories first"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_updates_shared_state() {
        let mut sidebar = sidebar_with_notes("sidebar-apply", &["alpha", "bravo"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;
        sidebar.apply_sort(SortField::Title, SortOrder::Descending, false);
        poll_to_idle(&mut sidebar).await;
        assert_eq!(
            sidebar.current_sort(),
            (SortField::Title, SortOrder::Descending)
        );
        assert!(!sidebar.group_dirs());
    }

    /// Regression: saving a default must survive navigation. `save_default`
    /// updates the cached per-context default that `sort_for`/`navigate` read;
    /// without it, navigating re-derives the construction-time default and the
    /// saved choice is silently lost.
    #[tokio::test(flavor = "multi_thread")]
    async fn save_default_survives_navigation() {
        let mut sidebar = sidebar_with_notes("sidebar-savedef", &["alpha", "bravo"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;

        sidebar.save_default(SortField::Title, SortOrder::Descending, false);
        poll_to_idle(&mut sidebar).await;

        // Re-navigate (root is non-journal) — sort_for must now yield the saved
        // default, not the constructor-time (Name, Ascending).
        sidebar.navigate(VaultPath::root(), &tx);
        poll_to_idle(&mut sidebar).await;
        assert_eq!(
            sidebar.current_sort(),
            (SortField::Title, SortOrder::Descending),
            "saved default must persist across navigation"
        );
    }
}
