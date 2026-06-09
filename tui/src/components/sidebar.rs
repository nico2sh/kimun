use std::sync::{Arc, Mutex};

use crate::settings::themes::Theme;
use async_trait::async_trait;
use chrono::NaiveDate;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, NotesValidation, ResultType, VaultBrowseOptionsBuilder};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, AppTxExt, InputEvent, redraw_callback};
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
    /// The note currently open in the editor, if any — drives the open-note
    /// marker. `None` on the Browse screen (it never opens notes). Matched
    /// against `FileListEntry::Note` rows by `is_like`.
    open_note: Option<VaultPath>,
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
    /// Screen cell each breadcrumb segment was drawn into on the last render,
    /// with the directory it navigates to — clickable breadcrumb hit-test.
    breadcrumb_cells: Vec<(Rect, VaultPath)>,
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
            open_note: None,
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
            breadcrumb_cells: Vec::new(),
            key_bindings,
        }
    }

    /// The breadcrumb segment under the given screen cell, if any.
    fn breadcrumb_at(&self, column: u16, row: u16) -> Option<&VaultPath> {
        self.breadcrumb_cells
            .iter()
            .find(|(rect, _)| rect.contains(Position { x: column, y: row }))
            .map(|(_, dir)| dir)
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

    /// Rebuild the listing only when it is currently showing `dir`, so a
    /// create/rename/delete/move in that directory is reflected without yanking
    /// the user away from an unrelated directory they browsed to. A no-op
    /// otherwise. Shared by every screen that hosts a sidebar.
    pub fn refresh_if_showing(&mut self, dir: &VaultPath, tx: &AppTx) {
        if dir.is_like(&self.current_dir) {
            self.navigate(self.current_dir.clone(), tx);
        }
    }

    /// Set (or clear) the note the editor currently has open, then re-stamp the
    /// marker on the live rows. The editor calls this on every open and on an
    /// open-note rename.
    pub fn set_open_note(&mut self, path: Option<VaultPath>) {
        self.open_note = path;
        self.stamp_open_marker();
    }

    /// Re-apply `is_open` to the rows so exactly the open note's row is marked.
    /// Idempotent: a full reload rebuilds rows without the flag, so this runs
    /// again after each load (see `render`).
    fn stamp_open_marker(&mut self) {
        let open = self.open_note.clone();
        if let Some(list) = &mut self.list {
            list.update_rows(|row| {
                if let FileListEntry::Note { path, is_open, .. } = row {
                    let want = open.as_ref().is_some_and(|o| path.is_like(o));
                    if *is_open != want {
                        *is_open = want;
                        return true;
                    }
                }
                false
            });
        }
    }

    /// Update the title of the row whose note path matches `path`, if it is in
    /// the current listing. Called when a note is saved and its title (first
    /// body line) may have changed. Position is left unchanged (no re-sort).
    pub fn update_note_row(&mut self, path: &VaultPath, new_title: &str) {
        if let Some(list) = &mut self.list {
            list.update_rows(|row| {
                if let FileListEntry::Note {
                    path: row_path,
                    title,
                    ..
                } = row
                    && row_path.is_like(path)
                    && title != new_title
                {
                    *title = new_title.to_string();
                    return true;
                }
                false
            });
        }
    }

    /// Move the row at `from` to `to` (path + filename + journal_date) in
    /// place, for a same-directory note rename. Position is left unchanged
    /// (no re-sort). `journal_date` is recomputed so a rename into/out of a
    /// `YYYY-MM-DD` name under the journal directory flips the glyph and the
    /// secondary date line correctly.
    pub fn rename_note_row(&mut self, from: &VaultPath, to: &VaultPath) {
        let new_filename = to.get_parent_path().1;
        let new_journal_date = self.vault.journal_date(to).map(format_journal_date);
        if let Some(list) = &mut self.list {
            list.update_rows(|row| {
                if let FileListEntry::Note {
                    path,
                    filename,
                    journal_date,
                    ..
                } = row
                    && path.is_like(from)
                {
                    *path = to.clone();
                    *filename = new_filename.clone();
                    *journal_date = new_journal_date.clone();
                    return true;
                }
                false
            });
        }
    }

    /// Seed the directory the sidebar will show before its first `navigate`.
    /// Lets a screen open at a non-root path while keeping `current_dir` the
    /// single source of truth for the browsed directory.
    pub fn set_current_dir(&mut self, dir: VaultPath) {
        self.current_dir = dir;
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
                    match vault.load_or_create_note(&path, None).await {
                        Ok((_, created)) => tx2.announce_and_open(path, created),
                        Err(e) => {
                            tracing::warn!("create note failed for {path}: {e}");
                        }
                    }
                });
            }
            other => {
                tx.send(AppEvent::open(other.path().clone())).ok();
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
            // A click on a breadcrumb segment jumps up the tree.
            if matches!(
                mouse.kind,
                ratatui::crossterm::event::MouseEventKind::Down(
                    ratatui::crossterm::event::MouseButton::Left
                )
            ) && let Some(dir) = self.breadcrumb_at(mouse.column, mouse.row)
            {
                tx.send(AppEvent::open(dir.clone())).ok();
                return EventState::Consumed;
            }
            // Click-to-focus is handled centrally by `PanelSet::handle_mouse`;
            // only the sidebar's internal behavior lives here. The engine
            // hit-tests the wheel against the recorded panel rect (the whole
            // sidebar — header and search box included) and clicks against
            // the list rect.
            if let Some(list) = &mut self.list {
                match list.handle_mouse(mouse) {
                    SearchMouse::Activated(_) => self.activate_selected_entry(tx),
                    // Right-click on a file/dir row → context menu (spec §10).
                    SearchMouse::Context(_) => {
                        if let Some(entry) = list.selected_row()
                            && !matches!(
                                entry,
                                FileListEntry::Up { .. } | FileListEntry::CreateNote { .. }
                            )
                        {
                            tx.send(AppEvent::ShowFileOpsMenu(entry.path().clone()))
                                .ok();
                        }
                    }
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

        crate::components::hints::hints_for(
            &self.key_bindings,
            &[
                (ActionShortcuts::FocusEditor, "editor \u{2192}"),
                (ActionShortcuts::OpenSortDialog, "sort"),
            ],
        )
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
            .title(format!("─ Files · {} ", self.current_dir))
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(theme.panel_style());
        let header_inner = header.inner(rows[0]);
        f.render_widget(header, rows[0]);

        // Clickable breadcrumb: one span per ancestor directory, separated by
        // " / ", with the note count right-aligned. Each segment's cell is
        // recorded for the click hit-test.
        self.breadcrumb_cells.clear();
        let seg_style = Style::default()
            .fg(theme.fg_secondary.to_ratatui())
            .bg(theme.bg_panel.to_ratatui());
        let sep_style = Style::default()
            .fg(theme.gray.to_ratatui())
            .bg(theme.bg_panel.to_ratatui());
        let mut spans: Vec<Span> = Vec::new();
        let mut x = header_inner.x;
        let mut push_segment =
            |spans: &mut Vec<Span>, x: &mut u16, label: String, dir: VaultPath| {
                let w = unicode_width::UnicodeWidthStr::width(label.as_str()) as u16;
                // Only record cells that are (at least partly) visible — the
                // Paragraph clips at the header edge, so fully clipped
                // segments must not be clickable.
                if *x < header_inner.right() {
                    let visible = w.min(header_inner.right() - *x);
                    self.breadcrumb_cells
                        .push((Rect::new(*x, header_inner.y, visible, 1), dir));
                }
                spans.push(Span::styled(label, seg_style));
                *x += w;
            };
        push_segment(&mut spans, &mut x, "~".to_string(), VaultPath::root());
        let slices = self.current_dir.get_slices();
        let mut acc = String::new();
        for slice in &slices {
            spans.push(Span::styled(" / ", sep_style));
            x += 3;
            acc.push('/');
            acc.push_str(slice);
            push_segment(&mut spans, &mut x, slice.clone(), VaultPath::new(&acc));
        }
        let count = format!("{} notes", self.note_count());
        let used: u16 = x - header_inner.x;
        let pad = header_inner
            .width
            .saturating_sub(used)
            .saturating_sub(unicode_width::UnicodeWidthStr::width(count.as_str()) as u16);
        spans.push(Span::styled(" ".repeat(pad as usize), sep_style));
        spans.push(Span::styled(count, sep_style));
        f.render_widget(Paragraph::new(Line::from(spans)), header_inner);

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

        // Poll the engine so a just-completed load's rows are applied, then
        // re-stamp the open-note marker (the reload rebuilt rows without it)
        // before the list renders.
        if let Some(list) = &mut self.list {
            list.poll();
        }
        self.stamp_open_marker();
        if let Some(list) = &mut self.list {
            list.render_query(f, search_inner, theme, focused);
            list.render(f, list_inner, theme, focused);
            // Record the rendered-items rect (block inner area) for mouse
            // hit-testing: the engine maps a click to `row - rect.y`, so row 0
            // is the first item. The panel rect (whole sidebar) lets the wheel
            // scroll from anywhere within the sidebar, not just over the list.
            list.set_list_rect(list_inner);
            list.set_panel_rect(rect);
        }
    }
}

#[cfg(test)]
impl SidebarComponent {
    pub(crate) fn poll_for_test(&mut self) {
        if let Some(list) = &mut self.list {
            list.poll();
        }
        self.stamp_open_marker();
    }

    pub(crate) fn is_loading_for_test(&self) -> bool {
        self.list.as_ref().is_some_and(|l| l.is_loading())
    }

    pub(crate) fn note_row_is_open_for_test(&self, name: &str) -> bool {
        self.list.as_ref().is_some_and(|l| {
            l.rows().iter().any(|r| {
                matches!(r, FileListEntry::Note { path, is_open, .. }
                    if path.get_name() == name && *is_open)
            })
        })
    }

    pub(crate) fn note_row_title_for_test(&self, name: &str) -> Option<String> {
        self.list.as_ref().and_then(|l| {
            l.rows().iter().find_map(|r| match r {
                FileListEntry::Note { path, title, .. } if path.get_name() == name => {
                    Some(title.clone())
                }
                _ => None,
            })
        })
    }

    pub(crate) fn note_row_journal_date_for_test(&self, path: &VaultPath) -> Option<String> {
        self.list.as_ref().and_then(|l| {
            l.rows().iter().find_map(|r| match r {
                FileListEntry::Note {
                    path: row_path,
                    journal_date,
                    ..
                } if row_path.is_like(path) => journal_date.clone(),
                _ => None,
            })
        })
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
                .any(|e| matches!(e, AppEvent::OpenPath { path: p, .. } if p.to_string().contains("alpha"))),
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

    #[tokio::test(flavor = "multi_thread")]
    async fn set_open_note_stamps_matching_row() {
        let mut sb = sidebar_with_notes("sb-open", &["alpha", "beta"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sb, &tx).await;

        sb.set_open_note(Some(VaultPath::note_path_from("alpha")));
        assert!(
            sb.note_row_is_open_for_test("alpha.md"),
            "open note is marked"
        );
        assert!(
            !sb.note_row_is_open_for_test("beta.md"),
            "other note is not marked"
        );

        sb.set_open_note(Some(VaultPath::note_path_from("beta")));
        assert!(!sb.note_row_is_open_for_test("alpha.md"));
        assert!(sb.note_row_is_open_for_test("beta.md"));

        sb.set_open_note(None);
        assert!(!sb.note_row_is_open_for_test("beta.md"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_note_row_changes_title_in_place() {
        let mut sb = sidebar_with_notes("sb-title", &["alpha"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sb, &tx).await;

        sb.update_note_row(&VaultPath::note_path_from("alpha"), "Fresh Title");
        assert_eq!(
            sb.note_row_title_for_test("alpha.md").as_deref(),
            Some("Fresh Title")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rename_note_row_updates_path_and_filename() {
        let mut sb = sidebar_with_notes("sb-rename", &["alpha"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sb, &tx).await;

        let to = VaultPath::note_path_from("gamma");
        let expected_filename = to.get_parent_path().1;
        sb.rename_note_row(&VaultPath::note_path_from("alpha"), &to);
        assert!(
            sb.note_row_title_for_test("gamma.md").is_some(),
            "row now at new name"
        );
        assert!(
            sb.note_row_title_for_test("alpha.md").is_none(),
            "old name gone"
        );
        // Also verify the filename field itself was updated to the new name.
        let renamed_filename = sb
            .list
            .as_ref()
            .unwrap()
            .rows()
            .iter()
            .find_map(|r| match r {
                FileListEntry::Note { path, filename, .. } if path.is_like(&to) => {
                    Some(filename.clone())
                }
                _ => None,
            });
        assert_eq!(
            renamed_filename.as_deref(),
            Some(expected_filename.as_str()),
            "filename field must be updated to the new name"
        );
    }

    /// Renaming a journal-dir note (`YYYY-MM-DD`) to a non-date name clears
    /// `journal_date` on the row so the glyph and secondary date line update.
    #[tokio::test(flavor = "multi_thread")]
    async fn rename_note_row_clears_journal_date_when_renamed_away_from_date_name() {
        // Build a vault and create a note inside the journal directory with a
        // valid YYYY-MM-DD name so `vault.journal_date` returns Some(_).
        let vault = crate::test_support::temp_vault("sb-jdate").await;
        vault.validate_and_init().await.unwrap();
        let journal_path = vault.journal_path().clone();
        let date_name = "2026-06-09";
        let from = journal_path
            .append(&VaultPath::note_path_from(date_name))
            .absolute();
        vault.create_note(&from, "journal body").await.unwrap();

        let settings = AppSettings::default();
        let mut sb = SidebarComponent::new(
            settings.key_bindings.clone(),
            vault,
            settings.icons(),
            &settings,
        );
        let (tx, _rx) = unbounded_channel();
        // Navigate to the journal directory (not root) so the note row is listed.
        sb.navigate(journal_path.clone(), &tx);
        for _ in 0..50 {
            sb.poll_for_test();
            if !sb.is_loading_for_test() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        sb.poll_for_test();

        // Precondition: the journal row has a non-None `journal_date`.
        assert!(
            sb.note_row_journal_date_for_test(&from).is_some(),
            "journal note must have a journal_date before rename"
        );

        // Rename the note to a plain name (not a date) in the same directory.
        let to = journal_path
            .append(&VaultPath::note_path_from("meeting"))
            .absolute();
        sb.rename_note_row(&from, &to);

        // The row should now have journal_date = None.
        assert_eq!(
            sb.note_row_journal_date_for_test(&to),
            None,
            "journal_date must be cleared after renaming to a non-date name"
        );
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
