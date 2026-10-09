pub use create_note_dialog::CreateNoteDialog;
pub use delete_dialog::DeleteConfirmDialog;
pub use file_ops_menu::FileOpsMenuDialog;
pub use help_dialog::HelpDialog;
pub use move_dialog::MoveDialog;
pub use pinned_notes_dialog::PinnedNotesDialog;
pub use properties_dialog::PropertiesDialog;
pub use quick_note_modal::QuickNoteModal;
pub use rename_dialog::RenameDialog;
pub use save_search_dialog::SaveSearchDialog;
pub use server_update_dialog::ServerUpdateDialog;
pub use sort_dialog::SortDialog;
pub use theme_picker::ThemePickerDialog;
pub use update_dialog::UpdateAvailableDialog;
pub use workspace_switcher::WorkspaceSwitcherModal;

use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, OverlayData, SaveSource, SortTarget};
use crate::components::hint_row::HintRow;
use crate::components::overlay::{Overlay, OverlayKind, OverlayMsg};
use crate::components::sortable::SortState;
use crate::settings::themes::Theme;

/// Load every property key in the background; arrives as
/// [`OverlayData::PropertyKeysLoaded`]. A failed read leaves pickers empty.
pub(crate) fn spawn_property_keys(vault: Arc<NoteVault>, tx: &AppTx) {
    let tx = tx.clone();
    tokio::spawn(async move {
        if let Ok(keys) = vault.property_keys().await {
            tx.send(AppEvent::OverlayData(OverlayData::PropertyKeysLoaded(keys)))
                .ok();
        }
    });
}

// ---------------------------------------------------------------------------
// ValidationState — shared by RenameDialog and MoveDialog
// ---------------------------------------------------------------------------

/// Tracks the current state of an async name / destination availability check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationState {
    /// No check has been triggered yet (initial state).
    Idle,
    /// A check is in progress.
    Pending,
    /// The name / destination is available (does not already exist).
    Available,
    /// The name / destination is already taken.
    Taken,
}

pub mod create_note_dialog;
pub mod delete_dialog;
pub mod file_ops_menu;
pub mod help_dialog;
pub mod move_dialog;
pub mod pinned_notes_dialog;
pub mod properties_dialog;
pub mod quick_note_modal;
pub mod rename_dialog;
pub mod save_search_dialog;
pub mod server_update_dialog;
pub mod sort_dialog;
pub mod theme_picker;
pub mod update_dialog;
pub mod workspace_switcher;

pub enum ActiveDialog {
    Menu(FileOpsMenuDialog),
    Delete(DeleteConfirmDialog),
    Rename(RenameDialog),
    Move(MoveDialog),
    CreateNote(CreateNoteDialog),
    Help(HelpDialog),
    QuickNote(QuickNoteModal),
    WorkspaceSwitcher(WorkspaceSwitcherModal),
    SaveSearch(SaveSearchDialog),
    Sort(SortDialog),
    PinnedNotes(PinnedNotesDialog),
    ThemePicker(ThemePickerDialog),
    UpdateAvailable(UpdateAvailableDialog),
    ServerUpdate(ServerUpdateDialog),
    Properties(PropertiesDialog),
}

impl ActiveDialog {
    pub fn set_error(&mut self, msg: String) {
        match self {
            ActiveDialog::Menu(_) => {} // menu has no error state
            ActiveDialog::Delete(d) => d.error = Some(msg),
            ActiveDialog::Rename(d) => d.error = Some(msg),
            ActiveDialog::Move(d) => d.error = Some(msg),
            ActiveDialog::CreateNote(d) => d.error = Some(msg),
            ActiveDialog::Help(_) => {}
            ActiveDialog::QuickNote(d) => d.error = Some(msg),
            ActiveDialog::WorkspaceSwitcher(_) => {} // no error state
            ActiveDialog::SaveSearch(_) => {}        // no error state
            ActiveDialog::Sort(_) => {}              // no error state
            ActiveDialog::PinnedNotes(_) => {} // no error state: its own failures arrive as PinnedNotesLoaded(Err) and flash
            ActiveDialog::ThemePicker(_) => {} // no error state
            ActiveDialog::UpdateAvailable(_) => {} // no error state
            ActiveDialog::ServerUpdate(_) => {} // no error state
            ActiveDialog::Properties(d) => d.error = Some(msg),
        }
    }

    // Constructors for the dialogs opened by EditorScreen via OverlayHost.
    pub fn help(
        key_bindings: &crate::keys::KeyBindings,
        tree: &crate::keys::leader::LeaderNode,
    ) -> Self {
        ActiveDialog::Help(HelpDialog::new(key_bindings, tree))
    }

    /// The full leader-tree cheatsheet (leader `?`).
    pub fn cheatsheet(settings: &crate::settings::AppSettings) -> Self {
        ActiveDialog::Help(HelpDialog::cheatsheet(settings))
    }

    /// The search query syntax reference (F1 over the Find drawer view).
    pub fn query_syntax() -> Self {
        ActiveDialog::Help(HelpDialog::query_syntax())
    }

    /// The live theme picker (leader `v c`).
    pub fn theme_picker(settings: &crate::settings::AppSettings) -> Self {
        ActiveDialog::ThemePicker(ThemePickerDialog::new(settings))
    }

    /// The update-available dialog.
    pub fn update(status: &crate::update::UpdateStatus) -> Self {
        ActiveDialog::UpdateAvailable(UpdateAvailableDialog::new(status))
    }

    /// The server-update hint dialog (footer `rag:` segment).
    pub fn server_update(update: &crate::server_client::sync::ServerUpdate) -> Self {
        ActiveDialog::ServerUpdate(ServerUpdateDialog::new(update))
    }

    pub fn quick_note(vault: Arc<NoteVault>) -> Self {
        ActiveDialog::QuickNote(QuickNoteModal::new(vault))
    }

    pub fn workspace_switcher(settings: &crate::settings::AppSettings) -> Self {
        ActiveDialog::WorkspaceSwitcher(WorkspaceSwitcherModal::new(settings))
    }

    pub fn create_note(
        path: kimun_core::nfs::VaultPath,
        vault: Arc<NoteVault>,
        content: Option<String>,
    ) -> Self {
        ActiveDialog::CreateNote(CreateNoteDialog::new(path, vault, content))
    }

    /// Open the save-search dialog. `provenance` is the saved-search name the
    /// query came from (the breadcrumb), pre-filled as the default name. The
    /// existing names load in the background and arrive via
    /// [`OverlayData::SavedSearchNamesLoaded`] to drive the dialog's hint.
    pub fn save_search(
        query: String,
        provenance: Option<String>,
        source: SaveSource,
        vault: Arc<NoteVault>,
        tx: &AppTx,
    ) -> Self {
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(searches) = vault.list_saved_searches().await {
                let names = searches.into_iter().map(|s| s.name).collect();
                tx.send(AppEvent::OverlayData(OverlayData::SavedSearchNamesLoaded(
                    names,
                )))
                .ok();
            }
        });
        ActiveDialog::SaveSearch(SaveSearchDialog::new(query, provenance, source))
    }

    /// The sort dialog for `target`, opened on its `state` (shown as
    /// "Unsorted" while `unsorted`). With a `vault`, the property keys load
    /// in the background for the Key picker (pass one exactly when
    /// `allows_property`).
    pub fn sort(
        target: SortTarget,
        state: SortState,
        unsorted: bool,
        allows_property: bool,
        vault: Option<Arc<NoteVault>>,
        tx: &AppTx,
    ) -> Self {
        if let Some(vault) = vault {
            spawn_property_keys(vault, tx);
        }
        ActiveDialog::Sort(SortDialog::new(target, state, allows_property).unsorted(unsorted))
    }

    /// The pinned-notes dialog (leader `f p`). Loads in the background and
    /// arrives via [`OverlayData::PinnedNotesLoaded`].
    pub fn pinned_notes(vault: Arc<NoteVault>, tx: &AppTx) -> Self {
        ActiveDialog::PinnedNotes(PinnedNotesDialog::new(vault, tx))
    }

    pub fn properties(path: kimun_core::nfs::VaultPath, vault: Arc<NoteVault>, tx: &AppTx) -> Self {
        ActiveDialog::Properties(PropertiesDialog::new(path, vault, tx))
    }

    pub fn file_ops_menu(path: kimun_core::nfs::VaultPath) -> Self {
        ActiveDialog::Menu(FileOpsMenuDialog::new(path))
    }

    pub fn delete(path: kimun_core::nfs::VaultPath, vault: Arc<NoteVault>) -> Self {
        ActiveDialog::Delete(DeleteConfirmDialog::new(path, vault))
    }

    pub fn rename(path: kimun_core::nfs::VaultPath, vault: Arc<NoteVault>) -> Self {
        ActiveDialog::Rename(RenameDialog::new(path, vault))
    }

    pub fn move_to(path: kimun_core::nfs::VaultPath, vault: Arc<NoteVault>, tx: &AppTx) -> Self {
        ActiveDialog::Move(MoveDialog::new(path, vault, tx))
    }
}

impl Overlay for ActiveDialog {
    fn kind(&self) -> OverlayKind {
        OverlayKind::Dialog
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        <Self as Component>::handle_input(self, event, tx)
    }

    fn handle_data(
        &mut self,
        data: &OverlayData,
        _vault: &Arc<NoteVault>,
        tx: &AppTx,
    ) -> OverlayMsg {
        match data {
            OverlayData::RenameValidation { available } => {
                if let ActiveDialog::Rename(d) = self {
                    d.validation_state = if *available {
                        ValidationState::Available
                    } else {
                        ValidationState::Taken
                    };
                    d.validation_task = None;
                }
                OverlayMsg::Consumed
            }
            OverlayData::PropertyKeysLoaded(keys) => {
                match self {
                    ActiveDialog::Sort(d) => d.set_keys(keys.clone()),
                    ActiveDialog::Properties(d) => d.set_keys(keys.clone()),
                    _ => {}
                }
                OverlayMsg::Consumed
            }
            OverlayData::PropertiesLoaded { path, result } => {
                if let ActiveDialog::Properties(d) = self {
                    d.on_loaded(path, result);
                }
                OverlayMsg::Consumed
            }
            OverlayData::PropertyWritten { path, result } => {
                if let ActiveDialog::Properties(d) = self {
                    d.on_written(path, result, tx);
                }
                OverlayMsg::Consumed
            }
            OverlayData::MoveDirectoriesLoaded(paths) => {
                if let ActiveDialog::Move(d) = self {
                    d.all_dirs = paths.clone();
                    d.filtered = None;
                    d.load_task = None;
                    if d.list_state.selected().is_none() && !d.results().is_empty() {
                        d.list_state.select(Some(0));
                    }
                    d.spawn_validation(tx);
                }
                OverlayMsg::Consumed
            }
            OverlayData::MoveFilterResults(paths) => {
                if let ActiveDialog::Move(d) = self {
                    d.filter_task = None;
                    d.filtered = Some(paths.clone());
                    if !d.results().is_empty() {
                        d.list_state.select(Some(0));
                    } else {
                        d.list_state.select(None);
                    }
                    d.spawn_validation(tx);
                }
                OverlayMsg::Consumed
            }
            OverlayData::MoveDestValidation { available } => {
                if let ActiveDialog::Move(d) = self {
                    d.dest_validation = if *available {
                        ValidationState::Available
                    } else {
                        ValidationState::Taken
                    };
                    d.validation_task = None;
                }
                OverlayMsg::Consumed
            }
            OverlayData::SavedSearchNamesLoaded(names) => {
                if let ActiveDialog::SaveSearch(d) = self {
                    d.set_existing_names(names.clone());
                }
                OverlayMsg::Consumed
            }
            OverlayData::PinnedNotesLoaded(result) => {
                if let ActiveDialog::PinnedNotes(d) = self {
                    d.handle_loaded(result, tx);
                }
                OverlayMsg::Consumed
            }
            OverlayData::Error(text) => {
                self.set_error(text.clone());
                OverlayMsg::Consumed
            }
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        <Self as Component>::render(self, f, area, theme, true);
    }
}

impl Component for ActiveDialog {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let key = match event {
            InputEvent::Key(key) => key,
            // Exhaustive on purpose: a new dialog must decide what the mouse
            // does in it. Each one consumes every mouse event — it is modal,
            // and a click must never reach the panels behind it.
            InputEvent::Mouse(m) => {
                return match self {
                    ActiveDialog::Sort(d) => d.handle_mouse(m, tx),
                    ActiveDialog::Properties(d) => d.handle_mouse(m, tx),
                    ActiveDialog::Delete(d) => d.handle_mouse(m, tx),
                    ActiveDialog::Menu(d) => d.handle_mouse(m, tx),
                    ActiveDialog::Rename(d) => d.handle_mouse(m, tx),
                    ActiveDialog::Move(d) => d.handle_mouse(m, tx),
                    ActiveDialog::SaveSearch(d) => d.handle_input(event, tx),
                    ActiveDialog::CreateNote(d) => d.handle_mouse(m, tx),
                    ActiveDialog::QuickNote(d) => d.handle_mouse(m, tx),
                    ActiveDialog::WorkspaceSwitcher(d) => d.handle_mouse(m, tx),
                    ActiveDialog::UpdateAvailable(d) => d.handle_mouse(m, tx),
                    ActiveDialog::ServerUpdate(d) => d.handle_mouse(m, tx),
                    ActiveDialog::ThemePicker(d) => d.handle_mouse(m, tx),
                    ActiveDialog::Help(d) => d.handle_mouse(m, tx),
                    ActiveDialog::PinnedNotes(d) => d.handle_mouse(m, tx),
                };
            }
            InputEvent::Paste(_) => return EventState::NotConsumed,
        };
        match self {
            ActiveDialog::Menu(d) => d.handle_key(*key, tx),
            ActiveDialog::Delete(d) => d.handle_key(*key, tx),
            ActiveDialog::Rename(d) => d.handle_key(*key, tx),
            ActiveDialog::Move(d) => d.handle_key(*key, tx),
            ActiveDialog::CreateNote(d) => d.handle_key(*key, tx),
            ActiveDialog::Help(d) => d.handle_key(*key, tx),
            ActiveDialog::QuickNote(d) => d.handle_key(*key, tx),
            ActiveDialog::WorkspaceSwitcher(d) => d.handle_key(*key, tx),
            ActiveDialog::SaveSearch(d) => d.handle_input(event, tx),
            ActiveDialog::Sort(d) => d.handle_input(event, tx),
            ActiveDialog::PinnedNotes(d) => d.handle_input(event, tx),
            ActiveDialog::ThemePicker(d) => d.handle_key(*key, tx),
            ActiveDialog::UpdateAvailable(d) => d.handle_key(*key, tx),
            ActiveDialog::ServerUpdate(d) => d.handle_key(*key, tx),
            ActiveDialog::Properties(d) => d.handle_key(*key, tx),
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        match self {
            ActiveDialog::Menu(d) => d.render(f, rect, theme, focused),
            ActiveDialog::Delete(d) => d.render(f, rect, theme, focused),
            ActiveDialog::Rename(d) => d.render(f, rect, theme, focused),
            ActiveDialog::Move(d) => d.render(f, rect, theme, focused),
            ActiveDialog::CreateNote(d) => d.render(f, rect, theme, focused),
            ActiveDialog::Help(d) => d.render(f, rect, theme, focused),
            ActiveDialog::QuickNote(d) => d.render(f, rect, theme, focused),
            ActiveDialog::WorkspaceSwitcher(d) => d.render(f, rect, theme, focused),
            ActiveDialog::SaveSearch(d) => d.render(f, rect, theme, focused),
            ActiveDialog::Sort(d) => d.render(f, rect, theme, focused),
            ActiveDialog::PinnedNotes(d) => d.render(f, rect, theme, focused),
            ActiveDialog::ThemePicker(d) => d.render(f, rect, theme, focused),
            ActiveDialog::UpdateAvailable(d) => d.render(f, rect, theme, focused),
            ActiveDialog::ServerUpdate(d) => d.render(f, rect, theme, focused),
            ActiveDialog::Properties(d) => d.render(f, rect, theme),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared render helpers
// ---------------------------------------------------------------------------

/// Renders a pre-computed path string (should already include leading spaces).
pub(super) fn render_path_row(f: &mut Frame, rect: Rect, path: &str, fg: Color, bg: Color) {
    f.render_widget(
        Paragraph::new(path).style(Style::default().fg(fg).bg(bg)),
        rect,
    );
}

/// Renders a single-line horizontal rule (TOP border only).
pub(super) fn render_separator(f: &mut Frame, rect: Rect, gray: Color, bg: Color) {
    Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(gray))
        .style(Style::default().bg(bg))
        .render(rect, f.buffer_mut());
}

/// Renders `  Error: {msg}` in the theme's error color on the panel background.
pub(super) fn render_error_row(f: &mut Frame, rect: Rect, msg: &str, theme: &Theme) {
    f.render_widget(
        Paragraph::new(format!("  Error: {msg}")).style(
            Style::default()
                .fg(theme.red.to_ratatui())
                .bg(theme.bg_panel.to_ratatui()),
        ),
        rect,
    );
}

/// The `[Enter] {action}   [Esc] Cancel` row shared by the form dialogs.
/// Each chip is clickable and runs its key.
pub(super) fn confirm_hints(action: &str) -> HintRow {
    HintRow::new(&[
        (KeyCode::Enter, "Enter", action),
        (KeyCode::Esc, "Esc", "Cancel"),
    ])
    .with_gap(3)
}

/// Draw a [`confirm_hints`] row. `enter_active` is only the *look*: `fg`
/// when ready, gray otherwise. `enter_clickable` must mirror exactly when the
/// Enter key acts — a dialog whose Enter still runs while it looks not-ready
/// (Move before validation lands, Save search while names load) passes
/// `true`, so the click never does less than the key.
pub(super) fn render_confirm_hints(
    f: &mut Frame,
    rect: Rect,
    hints: &mut HintRow,
    enter_active: bool,
    enter_clickable: bool,
    fg: Color,
    theme: &Theme,
) {
    let gray = theme.gray.to_ratatui();
    let bg = theme.bg_panel.to_ratatui();
    hints.set_enabled(0, enter_clickable);
    hints.set_style(
        0,
        Some(if enter_active {
            Style::default().fg(fg).bg(bg)
        } else {
            Style::default().fg(gray).bg(bg)
        }),
    );
    hints.render(f, rect, Style::default().fg(gray).bg(bg), theme);
}

// ---------------------------------------------------------------------------
// Layout helper
// ---------------------------------------------------------------------------

/// Centre a dialog of exactly `width` × `height` characters.
pub(super) use crate::components::fixed_centered_rect;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyBindings;

    /// Render `dialog` on a 100×40 screen, left-click the first cell of
    /// `text`, and return the events the click sent. Panics if `text` is not
    /// drawn — a chip that moved off-screen should fail loudly.
    pub(crate) fn click_text(dialog: &mut ActiveDialog, text: &str) -> Vec<AppEvent> {
        let (col, row) = draw_and_find(dialog, text);
        click_at(dialog, col, row)
    }

    /// Render `dialog` and return the cell where `text` starts.
    pub(crate) fn draw_and_find(dialog: &mut ActiveDialog, text: &str) -> (u16, u16) {
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(100, 40)).unwrap();
        t.draw(|f| <ActiveDialog as Component>::render(dialog, f, f.area(), &theme, true))
            .unwrap();
        crate::test_support::find_text(t.backend().buffer(), text)
            .unwrap_or_else(|| panic!("{text:?} not drawn"))
    }

    /// Left-click (col,row) and return the events sent.
    pub(crate) fn click_at(dialog: &mut ActiveDialog, col: u16, row: u16) -> Vec<AppEvent> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        Overlay::handle_input(dialog, &crate::test_support::mouse_down_at(col, row), &tx);
        crate::test_support::drain(&mut rx)
    }

    fn closed(events: &[AppEvent]) -> bool {
        events.iter().any(|e| matches!(e, AppEvent::CloseOverlay))
    }

    fn has(events: &[AppEvent], pred: impl Fn(&AppEvent) -> bool) -> bool {
        events.iter().any(pred)
    }

    #[test]
    fn menu_actions_are_clickable() {
        use crate::components::events::FileOp;
        let path = kimun_core::nfs::VaultPath::new("a.md");
        let mut d = ActiveDialog::Menu(FileOpsMenuDialog::new(path));
        assert!(has(&click_text(&mut d, "[D]"), |e| matches!(
            e,
            AppEvent::FileOp(FileOp::ShowDelete(_))
        )));
        assert!(has(&click_text(&mut d, "Rename"), |e| matches!(
            e,
            AppEvent::FileOp(FileOp::ShowRename(_))
        )));
        assert!(has(&click_text(&mut d, "Move"), |e| matches!(
            e,
            AppEvent::FileOp(FileOp::ShowMove(_))
        )));
        assert!(closed(&click_text(&mut d, "[Esc] Cancel")));
        assert!(closed(&click_at(&mut d, 0, 0)), "outside press cancels");
        assert!(
            !closed(&click_text(&mut d, "a.md")),
            "inside press does not"
        );
    }

    #[tokio::test]
    async fn rename_chips_are_clickable() {
        let vault = crate::test_support::temp_vault("dlg-rename-click").await;
        let mut d = ActiveDialog::Rename(RenameDialog::new(
            kimun_core::nfs::VaultPath::new("a.md"),
            vault,
        ));
        // Enter is inert until the name validates, like the key.
        assert!(click_text(&mut d, "[Enter] Rename").is_empty());
        assert!(closed(&click_text(&mut d, "[Esc] Cancel")));
    }

    #[tokio::test]
    async fn move_list_rows_select_and_cancel_chip_closes() {
        let vault = crate::test_support::temp_vault("dlg-move-click").await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut m = MoveDialog::new(kimun_core::nfs::VaultPath::new("a.md"), vault, &tx);
        m.all_dirs = vec![
            kimun_core::nfs::VaultPath::root(),
            kimun_core::nfs::VaultPath::new("projects"),
        ];
        m.list_state.select(Some(0));
        let mut d = ActiveDialog::Move(m);
        click_text(&mut d, "projects");
        let ActiveDialog::Move(m) = &d else {
            unreachable!()
        };
        assert_eq!(m.list_state.selected(), Some(1), "click selects the row");
        assert!(closed(&click_text(&mut d, "[Esc] Cancel")));
    }

    /// The Enter key moves while validation is still Idle/Pending, so the
    /// chip must too — a click may never do less than the key.
    #[tokio::test]
    async fn move_enter_chip_acts_before_validation_lands() {
        let vault = crate::test_support::temp_vault("dlg-move-enter").await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut m = MoveDialog::new(kimun_core::nfs::VaultPath::new("a.md"), vault, &tx);
        m.all_dirs = vec![kimun_core::nfs::VaultPath::new("projects")];
        m.list_state.select(Some(0));
        assert_eq!(m.dest_validation, ValidationState::Idle);
        let mut d = ActiveDialog::Move(m);
        let (col, row) = draw_and_find(&mut d, "[Enter] Move here");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        Overlay::handle_input(&mut d, &crate::test_support::mouse_down_at(col, row), &tx);
        // The move itself runs on a task; `a.md` does not exist, so it
        // reports an error — proof the click reached the move.
        let ev = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("the click started the move");
        assert!(matches!(
            ev,
            Some(AppEvent::OverlayData(OverlayData::Error(_))) | Some(AppEvent::FileOp(_))
        ));
    }

    #[test]
    fn save_search_chips_are_clickable() {
        use crate::components::events::SavedSearchFlow;
        let mut d = ActiveDialog::SaveSearch(SaveSearchDialog::new(
            "#tag".into(),
            None,
            SaveSource::QueryPanel,
        ));
        if let ActiveDialog::SaveSearch(s) = &mut d {
            s.set_existing_names(vec![]);
        }
        let ev = click_text(&mut d, "[Enter] Save new");
        assert!(has(&ev, |e| matches!(
            e,
            AppEvent::SavedSearch(SavedSearchFlow::Confirmed { .. })
        )));
        assert!(closed(&click_text(&mut d, "[Esc] Cancel")));
    }

    #[tokio::test]
    async fn create_note_chips_are_clickable() {
        let vault = crate::test_support::temp_vault("dlg-create-click").await;
        let mut d = ActiveDialog::CreateNote(CreateNoteDialog::new(
            kimun_core::nfs::VaultPath::new("new.md"),
            vault,
            None,
        ));
        assert!(closed(&click_text(&mut d, "[Esc] Cancel")));
    }

    #[tokio::test]
    async fn quick_note_chips_are_clickable() {
        let vault = crate::test_support::temp_vault("dlg-quick-click").await;
        let mut d = ActiveDialog::QuickNote(QuickNoteModal::new(vault));
        // Empty input: Enter closes, same as the key.
        assert!(closed(&click_text(&mut d, "[Enter] Save")));
        assert!(closed(&click_text(&mut d, "[Shift+Enter]")));
        assert!(closed(&click_text(&mut d, "[Esc] Cancel")));
    }

    #[test]
    fn workspace_switcher_rows_and_outside_click() {
        let mut d = ActiveDialog::WorkspaceSwitcher(WorkspaceSwitcherModal::with_workspaces(vec![
            ("alpha".into(), true),
            ("beta".into(), false),
        ]));
        assert!(click_text(&mut d, "beta").is_empty(), "first click selects");
        let ev = click_text(&mut d, "beta");
        assert!(
            has(
                &ev,
                |e| matches!(e, AppEvent::WorkspaceSwitched(n) if n == "beta")
            ),
            "clicking the selected row switches"
        );
        assert!(closed(&click_text(&mut d, "[Esc] Cancel")));
        assert!(closed(&click_at(&mut d, 0, 0)), "outside press cancels");
    }

    #[test]
    fn update_actions_are_clickable() {
        use crate::components::events::UpdateFlow;
        let status = crate::update::UpdateStatus {
            current: "0.1.0".into(),
            latest: "9.9.9".into(),
            channel: crate::update::InstallChannel::Script,
            update_available: true,
            dismissed: false,
        };
        let mut d = ActiveDialog::update(&status);
        assert!(has(&click_text(&mut d, "Update now"), |e| matches!(
            e,
            AppEvent::Update(UpdateFlow::Apply)
        )));
        assert!(has(&click_text(&mut d, "[S]"), |e| matches!(
            e,
            AppEvent::Update(UpdateFlow::Dismiss(_))
        )));
        assert!(closed(&click_text(&mut d, "[Esc] Close")));
        assert!(closed(&click_at(&mut d, 0, 0)), "outside press closes");
        assert!(
            !closed(&click_text(&mut d, "Releases:")),
            "inside press does not"
        );
    }

    #[test]
    fn theme_picker_click_previews_then_keeps() {
        let settings = crate::settings::AppSettings::default();
        let mut d = ActiveDialog::theme_picker(&settings);
        // Any theme other than the current one.
        let name = {
            let current = settings.effective_theme_name();
            settings
                .theme_list()
                .into_iter()
                .map(|t| t.name)
                .find(|n| *n != current)
                .expect("more than one theme")
        };
        let ev = click_text(&mut d, &name);
        assert!(has(&ev, |e| matches!(
            e,
            AppEvent::ApplyTheme { persist: false, .. }
        )));
        assert!(!closed(&ev), "first click only previews");
        let ev = click_text(&mut d, &name);
        assert!(has(&ev, |e| matches!(
            e,
            AppEvent::ApplyTheme { persist: true, .. }
        )));
        assert!(closed(&ev), "clicking the previewed theme keeps it");
    }

    #[test]
    fn theme_picker_outside_click_reverts() {
        let settings = crate::settings::AppSettings::default();
        let mut d = ActiveDialog::theme_picker(&settings);
        let name = {
            let current = settings.effective_theme_name();
            settings
                .theme_list()
                .into_iter()
                .map(|t| t.name)
                .find(|n| *n != current)
                .expect("more than one theme")
        };
        click_text(&mut d, &name);
        let ev = click_at(&mut d, 0, 0);
        assert!(
            has(&ev, |e| matches!(
                e,
                AppEvent::ApplyTheme { persist: false, .. }
            )),
            "reverts the preview"
        );
        assert!(closed(&ev));
    }

    #[test]
    fn help_close_chip_and_outside_click_close() {
        let mut d = ActiveDialog::cheatsheet(&crate::settings::AppSettings::default());
        assert!(closed(&click_text(&mut d, "[Esc] Close")));
        assert!(
            !closed(&click_text(&mut d, "Scroll")),
            "the scroll hint is inert"
        );
        assert!(closed(&click_at(&mut d, 0, 0)));
    }

    #[test]
    fn help_wheel_scrolls() {
        use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
        let mut d = ActiveDialog::cheatsheet(&crate::settings::AppSettings::default());
        let (col, row) = draw_and_find(&mut d, "[Esc] Close");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        Overlay::handle_input(
            &mut d,
            &InputEvent::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: col,
                row,
                modifiers: KeyModifiers::NONE,
            }),
            &tx,
        );
        let ActiveDialog::Help(h) = &d else {
            unreachable!()
        };
        assert!(h.scroll() > 0);
    }

    #[tokio::test]
    async fn pinned_rows_and_chips_are_clickable() {
        use crate::components::events::PinnedRow;
        use kimun_core::nfs::VaultPath;
        let vault = crate::test_support::temp_vault("dlg-pinned-click").await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut p = PinnedNotesDialog::new(vault, &tx);
        p.set_rows(vec![
            PinnedRow {
                path: VaultPath::new("alpha.md"),
                missing: false,
            },
            PinnedRow {
                path: VaultPath::new("beta.md"),
                missing: false,
            },
        ]);
        let mut d = ActiveDialog::PinnedNotes(p);
        assert!(
            click_text(&mut d, "beta.md").is_empty(),
            "first click selects"
        );
        let ActiveDialog::PinnedNotes(p) = &d else {
            unreachable!()
        };
        assert_eq!(p.selected, 1);
        assert!(
            !click_text(&mut d, "beta.md").is_empty(),
            "second click opens"
        );
        assert!(closed(&click_text(&mut d, "[Esc] Close")));
        assert!(closed(&click_at(&mut d, 0, 0)));
    }

    fn sidebar_sort() -> ActiveDialog {
        use crate::components::file_list::{SortField, SortOrder};
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let state = SortState {
            field: SortField::Name,
            order: SortOrder::Ascending,
            group_dirs: Some(false),
        };
        ActiveDialog::sort(SortTarget::Sidebar, state, false, false, None, &tx)
    }

    #[test]
    fn sort_footer_chips_save_and_close() {
        let mut d = sidebar_sort();
        assert!(has(&click_text(&mut d, "[s] Save default"), |e| matches!(
            e,
            AppEvent::SortChanged { persist: true, .. }
        )));
        assert!(closed(&click_text(&mut d, "[Esc] Close")));
        assert!(
            click_text(&mut d, "[Space] Toggle").is_empty(),
            "informational chips are inert"
        );
    }

    #[tokio::test]
    async fn delete_cancel_chip_closes() {
        let vault = crate::test_support::temp_vault("dlg-delete-click").await;
        let mut d = ActiveDialog::Delete(DeleteConfirmDialog::new(
            kimun_core::nfs::VaultPath::new("a.md"),
            vault,
        ));
        assert!(closed(&click_text(&mut d, "[Esc] Cancel")));
        assert!(
            !closed(&click_text(&mut d, "This cannot")),
            "body text is inert"
        );
    }

    #[test]
    fn active_dialog_help_variant_compiles() {
        let dialog = HelpDialog::new(&KeyBindings::empty(), &crate::keys::leader::leader_tree());
        let _active: ActiveDialog = ActiveDialog::Help(dialog);
    }

    #[test]
    fn active_dialog_sort_variant_compiles() {
        use crate::components::events::SortTarget;
        use crate::components::file_list::{SortField, SortOrder};
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let state = crate::components::sortable::SortState {
            field: SortField::Name,
            order: SortOrder::Ascending,
            group_dirs: Some(false),
        };
        let _active: ActiveDialog =
            ActiveDialog::sort(SortTarget::Sidebar, state, false, false, None, &tx);
    }

    /// Every dialog test for `PinnedNotesDialog` calls `set_rows` directly,
    /// bypassing the routing this module owns. A regression here (e.g. the
    /// `PinnedNotesLoaded` arm losing its `if let` guard, or matching the
    /// wrong variant) would leave the dialog permanently empty with none of
    /// those tests failing — so this drives the real `handle_data` path.
    #[tokio::test]
    async fn active_dialog_routes_pinned_notes_loaded_rows() {
        use crate::components::events::PinnedRow;
        use kimun_core::nfs::VaultPath;
        use tokio::sync::mpsc::unbounded_channel;

        let vault = crate::test_support::temp_vault("active-dialog-pinned").await;
        let (tx, _rx) = unbounded_channel();
        let mut active = ActiveDialog::pinned_notes(vault.clone(), &tx);

        let rows = vec![PinnedRow {
            path: VaultPath::new("a.md"),
            missing: false,
        }];
        let msg = active.handle_data(
            &OverlayData::PinnedNotesLoaded(Ok(rows.clone())),
            &vault,
            &tx,
        );
        assert!(matches!(msg, OverlayMsg::Consumed));

        match &active {
            ActiveDialog::PinnedNotes(d) => assert_eq!(d.rows(), rows.as_slice()),
            _ => panic!("expected ActiveDialog::PinnedNotes"),
        }
    }

    /// A foreign `OverlayData::Error` — a rename or paste task failing after
    /// its own dialog closed — must not be routed into the pinned-notes
    /// dialog as if it were the reload it is waiting on: that would clear
    /// its in-flight guard and reopen the overlapping-write race.
    #[tokio::test]
    async fn active_dialog_leaves_pinned_notes_alone_on_a_foreign_error() {
        use tokio::sync::mpsc::unbounded_channel;

        let vault = crate::test_support::temp_vault("active-dialog-pinned-err").await;
        let (tx, mut rx) = unbounded_channel();
        let mut active = ActiveDialog::pinned_notes(vault.clone(), &tx);

        let msg = active.handle_data(
            &OverlayData::Error("rename failed".to_string()),
            &vault,
            &tx,
        );
        assert!(matches!(msg, OverlayMsg::Consumed));
        assert!(
            rx.try_recv().is_err(),
            "a foreign error must not be flashed as a pinned-notes failure"
        );
        match &active {
            ActiveDialog::PinnedNotes(d) => assert!(
                !d.is_loaded(),
                "a foreign error must not settle the dialog's own load"
            ),
            _ => panic!("expected ActiveDialog::PinnedNotes"),
        }
    }
}
