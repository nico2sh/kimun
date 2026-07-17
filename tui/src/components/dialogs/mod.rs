pub use create_note_dialog::CreateNoteDialog;
pub use delete_dialog::DeleteConfirmDialog;
pub use file_ops_menu::FileOpsMenuDialog;
pub use help_dialog::HelpDialog;
pub use move_dialog::MoveDialog;
pub use quick_note_modal::QuickNoteModal;
pub use rename_dialog::RenameDialog;
pub use save_search_dialog::SaveSearchDialog;
pub use sort_dialog::SortDialog;
pub use theme_picker::ThemePickerDialog;
pub use update_dialog::UpdateAvailableDialog;
pub use workspace_switcher::WorkspaceSwitcherModal;

use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, OverlayData, SaveSource, SortTarget};
use crate::components::file_list::{SortField, SortOrder};
use crate::components::overlay::{Overlay, OverlayKind, OverlayMsg};
use crate::settings::themes::Theme;

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
pub mod quick_note_modal;
pub mod rename_dialog;
pub mod save_search_dialog;
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
    ThemePicker(ThemePickerDialog),
    UpdateAvailable(UpdateAvailableDialog),
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
            ActiveDialog::ThemePicker(_) => {}       // no error state
            ActiveDialog::UpdateAvailable(_) => {}   // no error state
        }
    }

    // Constructors for the dialogs opened by EditorScreen via OverlayHost.
    pub fn help(key_bindings: &crate::keys::KeyBindings) -> Self {
        ActiveDialog::Help(HelpDialog::new(key_bindings))
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
    /// [`AppEvent::OverlayData(OverlayData::SavedSearchNamesLoaded)`] to drive the dialog's hint.
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

    pub fn sort(
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        group_directories: bool,
    ) -> Self {
        ActiveDialog::Sort(SortDialog::new(target, field, order, group_directories))
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
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
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
            ActiveDialog::ThemePicker(d) => d.handle_key(*key, tx),
            ActiveDialog::UpdateAvailable(d) => d.handle_key(*key, tx),
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
            ActiveDialog::ThemePicker(d) => d.render(f, rect, theme, focused),
            ActiveDialog::UpdateAvailable(d) => d.render(f, rect, theme, focused),
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

/// Renders `{enter_text}  [Esc] Cancel` split into two horizontal columns.
/// The Enter part is dimmed when `enter_active` is `false`.
pub(super) fn render_confirm_hint(
    f: &mut Frame,
    rect: Rect,
    enter_text: &str,
    enter_active: bool,
    fg: Color,
    gray: Color,
    bg: Color,
) {
    let enter_style = if enter_active {
        Style::default().fg(fg).bg(bg)
    } else {
        Style::default().fg(gray).bg(bg).add_modifier(Modifier::DIM)
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(enter_text.len() as u16 + 1),
            Constraint::Min(1),
        ])
        .split(rect);
    f.render_widget(Paragraph::new(enter_text).style(enter_style), chunks[0]);
    f.render_widget(
        Paragraph::new("  [Esc] Cancel").style(Style::default().fg(gray).bg(bg)),
        chunks[1],
    );
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

    #[test]
    fn active_dialog_help_variant_compiles() {
        let dialog = HelpDialog::new(&KeyBindings::empty());
        let _active: ActiveDialog = ActiveDialog::Help(dialog);
    }

    #[test]
    fn active_dialog_sort_variant_compiles() {
        use crate::components::events::SortTarget;
        use crate::components::file_list::{SortField, SortOrder};
        let _active: ActiveDialog = ActiveDialog::sort(
            SortTarget::Sidebar,
            SortField::Name,
            SortOrder::Ascending,
            false,
        );
    }
}
