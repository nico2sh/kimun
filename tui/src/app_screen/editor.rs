use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::error::{FSError, VaultError};
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::dialogs::{
    ActiveDialog, CreateNoteDialog, DeleteConfirmDialog, FileOpsMenuDialog, MoveDialog,
    RenameDialog, ValidationState,
};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent};
use crate::components::note_browser::NoteBrowserModal;
use crate::components::note_browser::file_finder_provider::FileFinderProvider;
use crate::components::note_browser::search_provider::SearchNotesProvider;
use crate::components::sidebar::SidebarComponent;
use crate::components::text_editor::TextEditorComponent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::keys::key_strike::KeyStrike;
use crate::settings::AppSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

#[derive(Clone, Copy)]
enum Focus {
    Sidebar,
    Editor,
    NoteBrowser,
    Dialog,
}

pub struct EditorScreen {
    vault: Arc<NoteVault>,
    settings: AppSettings,
    icons: Icons,
    theme: Theme,
    editor: TextEditorComponent,
    sidebar: SidebarComponent,
    path: VaultPath,
    focus: Focus,
    sidebar_visible: bool,
    settings_key: String,
    quit_key: String,
    toggle_key: String,
    autosave_handle: Option<tokio::task::JoinHandle<()>>,
    key_flash: Option<(String, std::time::Instant)>,
    note_browser: Option<NoteBrowserModal>,
    active_dialog: Option<ActiveDialog>,
    pre_dialog_focus: Option<Focus>,
}

impl EditorScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: AppSettings) -> Self {
        let kb = settings.key_bindings.clone();
        let theme = settings.get_theme();
        let kb_map = kb.to_hashmap();
        let first_key = |action: &ActionShortcuts| {
            kb_map
                .get(action)
                .and_then(|v| v.first().cloned())
                .map(|c| c.to_string())
                .unwrap_or_default()
        };
        let quit_key = first_key(&ActionShortcuts::Quit);
        let settings_key = first_key(&ActionShortcuts::OpenSettings);
        let toggle_key = first_key(&ActionShortcuts::ToggleSidebar);
        let icons = settings.icons();
        let sidebar = SidebarComponent::new(kb.clone(), vault.clone(), icons.clone(), &settings);
        let editor = TextEditorComponent::new(kb, &settings);
        Self {
            settings,
            icons,
            theme,
            editor,
            sidebar,
            vault,
            path,
            focus: Focus::Editor,
            sidebar_visible: true,
            settings_key,
            quit_key,
            toggle_key,
            autosave_handle: None,
            key_flash: None,
            note_browser: None,
            active_dialog: None,
            pre_dialog_focus: None,
        }
    }
}

impl Drop for EditorScreen {
    fn drop(&mut self) {
        if let Some(handle) = self.autosave_handle.take() {
            handle.abort();
        }
    }
}

impl EditorScreen {
    async fn follow_link(&mut self, target: String, tx: &AppTx) {
        // External URL — hand off to the OS browser/handler.
        if target.starts_with("http://") || target.starts_with("https://") {
            if let Err(e) = open::that_detached(&target) {
                self.key_flash = Some((format!("Cannot open URL: {e}"), std::time::Instant::now()));
            }
            return;
        }

        // Note reference — look it up in the vault.
        // Strip any `#fragment` suffix before resolving (e.g. `notes/design.md#goals`
        // should resolve to `notes/design.md`, not `notes/design.md#goals.md`).
        let target_clean = target.split('#').next().unwrap_or(&target).trim_end();
        let path = kimun_core::nfs::VaultPath::note_path_from(target_clean);
        match self.vault.open_or_search(&path).await {
            Ok(results) if results.is_empty() => {
                if !matches!(self.focus, Focus::Dialog) {
                    self.pre_dialog_focus = Some(self.focus);
                }
                self.active_dialog = Some(ActiveDialog::CreateNote(CreateNoteDialog::new(
                    path,
                    self.vault.clone(),
                )));
                self.focus = Focus::Dialog;
            }
            Ok(mut results) if results.len() == 1 => {
                let (entry, _) = results.remove(0);
                self.open_path(entry.path, tx).await;
            }
            Ok(results) => {
                // Multiple matches — show picker.
                use crate::components::note_browser::link_results_provider::LinkResultsProvider;
                let provider = LinkResultsProvider::from_results(results);
                self.note_browser = Some(NoteBrowserModal::new(
                    format!("Follow: {target}"),
                    provider,
                    self.vault.clone(),
                    self.settings.key_bindings.clone(),
                    self.settings.icons(),
                    tx.clone(),
                ));
                self.focus = Focus::NoteBrowser;
            }
            Err(e) => {
                self.key_flash = Some((format!("Link error: {e}"), std::time::Instant::now()));
            }
        }
    }

    pub async fn open_path(&mut self, path: VaultPath, tx: &AppTx) {
        if !path.is_note() {
            tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(
                self.vault.clone(),
                path,
            )))
            .ok();
            return;
        }

        // Save current note before switching
        self.try_save().await;

        self.settings.add_path_history(&path);
        let settings_snapshot = self.settings.clone();
        tokio::spawn(async move {
            settings_snapshot.save_to_disk().ok();
        });

        self.path = path.clone();
        match self.vault.get_note_text(&self.path).await {
            Ok(content) => {
                self.editor.set_text(content);
                tx.send(AppEvent::Redraw).ok();
            }
            Err(e) => {
                if matches!(e, VaultError::FSError(FSError::VaultPathNotFound { .. })) {
                    if !matches!(self.focus, Focus::Dialog) {
                        self.pre_dialog_focus = Some(self.focus);
                    }
                    self.active_dialog = Some(ActiveDialog::CreateNote(CreateNoteDialog::new(
                        self.path.clone(),
                        self.vault.clone(),
                    )));
                    self.focus = Focus::Dialog;
                } else {
                    log::error!("Failed to read note {}: {e}", self.path);
                    let parent = self.path.get_parent_path().0;
                    tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(
                        self.vault.clone(),
                        parent,
                    )))
                    .ok();
                }
                return;
            }
        }

        // Load the sidebar on first open only; refreshes happen via explicit
        // create/rename/delete/move events, not on every note open.
        let note_parent = path.get_parent_path().0;
        if self.sidebar.is_empty() {
            self.navigate_sidebar(note_parent, tx).await;
        }

        // Abort any existing timer and spawn a fresh one for the new note.
        if let Some(h) = self.autosave_handle.take() {
            h.abort();
        }
        let interval_secs = self.settings.autosave_interval_secs;
        let tx2 = tx.clone();
        self.autosave_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                if tx2.send(AppEvent::Autosave).is_err() {
                    break;
                }
            }
        }));
    }

    pub async fn navigate_sidebar(&mut self, dir: VaultPath, tx: &AppTx) {
        let (options, rx) = VaultBrowseOptionsBuilder::new(&dir)
            .non_recursive()
            .full_validation()
            .build();

        let vault = self.vault.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            if let Err(e) = vault.browse_vault(options).await {
                log::error!("browse_vault failed: {e}");
            }
            tx2.send(AppEvent::Redraw).ok();
        });

        self.sidebar.start_loading(rx, dir);
    }

    async fn try_save(&mut self) {
        if self.editor.is_dirty() {
            let text = self.editor.get_text();
            if self.vault.save_note(&self.path, &text).await.is_ok() {
                self.editor.mark_saved(text);
            }
        }
    }

    fn restore_focus(&mut self) {
        self.active_dialog = None;
        self.focus = self.pre_dialog_focus.take().unwrap_or(Focus::Editor);
    }

    async fn on_entry_op(&mut self, from: VaultPath, tx: &AppTx) {
        self.restore_focus();
        if from == self.path {
            if let Some(h) = self.autosave_handle.take() {
                h.abort();
            }
            self.try_save().await;
            let parent = self.path.get_parent_path().0;
            tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(
                self.vault.clone(),
                parent,
            )))
            .ok();
        } else if from.get_parent_path().0.is_like(self.sidebar.current_dir()) {
            let dir = self.sidebar.current_dir().clone();
            self.navigate_sidebar(dir, tx).await;
        }
    }
}

impl EditorScreen {
    pub fn focus_editor(&mut self) {
        self.focus = Focus::Editor;
    }

    pub fn focus_sidebar(&mut self) {
        self.sidebar_visible = true;
        self.focus = Focus::Sidebar;
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        if !self.sidebar_visible {
            self.focus_editor();
        }
    }
}

#[async_trait]
impl AppScreen for EditorScreen {
    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Editor
    }

    async fn on_enter(&mut self, tx: &AppTx) {
        self.open_path(self.path.clone(), tx).await;
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        if let InputEvent::Key(key) = event
            && let Some(combo) = key_event_to_combo(key) {
                let is_fkey = matches!(
                    combo.key,
                    KeyStrike::F1
                        | KeyStrike::F2
                        | KeyStrike::F3
                        | KeyStrike::F4
                        | KeyStrike::F5
                        | KeyStrike::F6
                        | KeyStrike::F7
                        | KeyStrike::F8
                        | KeyStrike::F9
                        | KeyStrike::F10
                        | KeyStrike::F11
                        | KeyStrike::F12
                );
                if is_fkey
                    || ((combo.modifiers.is_ctrl() || combo.modifiers.is_alt())
                        && combo.key >= KeyStrike::KeyA
                        && combo.key <= KeyStrike::KeyZ)
                {
                    self.key_flash = Some((combo.to_string(), std::time::Instant::now()));
                    let tx2 = tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        tx2.send(AppEvent::Redraw).ok();
                    });
                }
                match self.settings.key_bindings.get_action(&combo) {
                    Some(ActionShortcuts::ToggleSidebar) => {
                        self.toggle_sidebar();
                        return EventState::Consumed;
                    }
                    Some(ActionShortcuts::NewJournal) => {
                        tx.send(AppEvent::OpenJournal).ok();
                        return EventState::Consumed;
                    }
                    Some(ActionShortcuts::ToggleNoteBrowser) => {
                        if self.note_browser.is_some() {
                            self.note_browser = None;
                            if matches!(self.focus, Focus::NoteBrowser) {
                                self.focus = Focus::Editor;
                            }
                        } else {
                            let provider = SearchNotesProvider::new(
                                self.vault.clone(),
                                self.settings.last_paths.clone(),
                            );
                            self.note_browser = Some(NoteBrowserModal::new(
                                "Note Browser",
                                provider,
                                self.vault.clone(),
                                self.settings.key_bindings.clone(),
                                self.settings.icons(),
                                tx.clone(),
                            ));
                            self.focus = Focus::NoteBrowser;
                        }
                        return EventState::Consumed;
                    }
                    Some(ActionShortcuts::OpenNote) => {
                        if self.note_browser.is_some() {
                            self.note_browser = None;
                            if matches!(self.focus, Focus::NoteBrowser) {
                                self.focus = Focus::Editor;
                            }
                        } else {
                            let current_dir = self.path.get_parent_path().0;
                            let provider = FileFinderProvider::new(self.vault.clone(), current_dir);
                            self.note_browser = Some(NoteBrowserModal::new(
                                "Find Note",
                                provider,
                                self.vault.clone(),
                                self.settings.key_bindings.clone(),
                                self.settings.icons(),
                                tx.clone(),
                            ));
                            self.focus = Focus::NoteBrowser;
                        }
                        return EventState::Consumed;
                    }
                    Some(ActionShortcuts::FileOperations)
                        if matches!(self.focus, Focus::Editor) =>
                    {
                        tx.send(AppEvent::ShowFileOpsMenu(self.path.clone())).ok();
                        return EventState::Consumed;
                    }
                    Some(ActionShortcuts::FollowLink) if matches!(self.focus, Focus::Editor) => {
                        if let Some(target) = self.editor.link_at_cursor() {
                            tx.send(AppEvent::FollowLink(target)).ok();
                        }
                        return EventState::Consumed;
                    }
                    _ => {}
                }
            }

        // Mouse events are routed to all components regardless of focus so that
        // clicking anywhere can transfer focus correctly.
        if matches!(event, InputEvent::Mouse(_)) {
            // Dialog swallows all mouse events while open.
            if matches!(self.focus, Focus::Dialog) {
                return EventState::Consumed;
            }
            // Note browser modal intercepts all mouse events when open.
            if matches!(self.focus, Focus::NoteBrowser)
                && let Some(modal) = &mut self.note_browser {
                    return modal.handle_input(event, tx);
                }
            if self.sidebar_visible && self.sidebar.handle_input(event, tx).is_consumed() {
                return EventState::Consumed;
            }
            return self.editor.handle_input(event, tx);
        }

        match self.focus {
            Focus::Sidebar => self.sidebar.handle_input(event, tx),
            Focus::Editor => self.editor.handle_input(event, tx),
            Focus::NoteBrowser => {
                if let Some(modal) = &mut self.note_browser {
                    modal.handle_input(event, tx)
                } else {
                    EventState::NotConsumed
                }
            }
            Focus::Dialog => {
                if let Some(dialog) = &mut self.active_dialog {
                    dialog.handle_input(event, tx)
                } else {
                    EventState::NotConsumed
                }
            }
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let theme = &self.theme;
        f.render_widget(
            ratatui::widgets::Block::default().style(theme.base_style()),
            f.area(),
        );

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.area());

        let header = Block::default()
            .title("Kimün")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .style(theme.base_style())
            .title_style(Style::default().fg(theme.accent.to_ratatui()));
        let header_inner = header.inner(rows[0]);
        f.render_widget(header, rows[0]);
        f.render_widget(
            Paragraph::new(self.path.to_string())
                .style(Style::default().fg(theme.fg_secondary.to_ratatui())),
            header_inner,
        );

        let columns = if self.sidebar_visible {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(30), Constraint::Min(0)])
                .split(rows[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0)])
                .split(rows[1])
        };

        let editor_focused = matches!(self.focus, Focus::Editor);
        let sidebar_focused = matches!(self.focus, Focus::Sidebar);

        let editor_area = if self.sidebar_visible {
            self.sidebar.render(f, columns[0], theme, sidebar_focused);
            columns[1]
        } else {
            columns[0]
        };

        let editor_border_style = theme.border_style(editor_focused);
        let editor_title = if self.editor.is_dirty() {
            "Editor [+]"
        } else {
            "Editor"
        };
        let editor_block = Block::default()
            .title(editor_title)
            .borders(Borders::ALL)
            .border_style(editor_border_style)
            .style(theme.base_style());
        let editor_inner = editor_block.inner(editor_area);
        f.render_widget(editor_block, editor_area);
        self.editor.render(f, editor_inner, theme, editor_focused);

        // Expire stale key flash
        if let Some((_, instant)) = &self.key_flash
            && instant.elapsed() >= std::time::Duration::from_secs(2) {
                self.key_flash = None;
            }

        let focus_label = match self.focus {
            Focus::Editor => "EDITOR",
            Focus::Sidebar => "SIDEBAR",
            Focus::NoteBrowser => "NOTE BROWSER",
            Focus::Dialog => "DIALOG",
        };
        let mut footer = Block::default()
            .title(format!(
                "[{focus_label}]  {}: Preferences |  {}: Toggle sidebar | {}: Quit",
                self.settings_key, self.toggle_key, self.quit_key,
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .style(theme.base_style())
            .title_style(Style::default().fg(theme.fg_secondary.to_ratatui()));
        if let Some((flash, _)) = &self.key_flash {
            footer = footer.title_top(Line::from(format!(" {} ", flash)).right_aligned());
        }
        let footer_inner = footer.inner(rows[2]);
        f.render_widget(footer, rows[2]);

        // Hints inside the footer's inner area.
        let hints = match self.focus {
            Focus::Editor => self.editor.hint_shortcuts(),
            Focus::Sidebar => self.sidebar.hint_shortcuts(),
            Focus::NoteBrowser => self
                .note_browser
                .as_ref()
                .map(|m| m.hint_shortcuts())
                .unwrap_or_default(),
            Focus::Dialog => vec![],
        };
        // Build the hints line with the nvim mode label (empty key) styled
        // distinctly from the regular shortcut hints.
        let secondary = Style::default().fg(theme.fg_secondary.to_ratatui());
        let sep = Span::styled("  │  ", secondary);
        let mut spans = vec![Span::styled(format!(" {} ", self.icons.info), secondary)];
        for (i, (key, label)) in hints.iter().enumerate() {
            if i > 0 {
                spans.push(sep.clone());
            }
            if key.is_empty() {
                // Mode / command-line label from the nvim backend — make it pop.
                spans.push(Span::styled(
                    format!(" {label} "),
                    Style::default()
                        .fg(theme.accent.to_ratatui())
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(format!("{key}: {label}"), secondary));
            }
        }
        f.render_widget(Paragraph::new(Line::from(spans)), footer_inner);

        // Modal overlay — rendered last so it appears on top of everything.
        if let Some(modal) = &mut self.note_browser {
            modal.render(f, f.area(), &self.theme, true);
        }

        // Dialog overlay — rendered after the note browser so it appears on top.
        if let Some(dialog) = &mut self.active_dialog {
            dialog.render(f, f.area(), &self.theme, true);
        }
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) -> Option<AppEvent> {
        match msg {
            AppEvent::Autosave => {
                self.try_save().await;
                None
            }
            AppEvent::OpenPath(path) => {
                if self.active_dialog.is_some() {
                    self.restore_focus(); // dismiss any active dialog (e.g. CreateNote) before loading
                }
                if path.is_note() {
                    self.open_path(path, tx).await;
                    self.focus_editor();
                } else {
                    self.navigate_sidebar(path, tx).await;
                }
                None
            }
            AppEvent::FocusEditor => {
                self.focus_editor();
                None
            }
            AppEvent::FocusSidebar => {
                self.focus_sidebar();
                None
            }
            AppEvent::OpenJournal => {
                if let Ok((details, _)) = self.vault.journal_entry().await {
                    let path = details.path;
                    self.open_path(path.clone(), tx).await;
                    // The journal note may have just been created; refresh the
                    // sidebar so today's entry appears if it wasn't there yet.
                    let note_parent = path.get_parent_path().0;
                    if note_parent.is_like(self.sidebar.current_dir()) {
                        let dir = self.sidebar.current_dir().clone();
                        self.navigate_sidebar(dir, tx).await;
                    }
                }
                None
            }
            AppEvent::CloseNoteBrowser => {
                self.note_browser = None;
                if matches!(self.focus, Focus::NoteBrowser) {
                    self.focus = Focus::Editor;
                }
                None
            }
            AppEvent::FollowLink(target) => {
                self.follow_link(target, tx).await;
                None
            }
            AppEvent::ShowFileOpsMenu(path) => {
                self.pre_dialog_focus = Some(self.focus);
                self.active_dialog = Some(ActiveDialog::Menu(FileOpsMenuDialog::new(path)));
                self.focus = Focus::Dialog;
                None
            }
            AppEvent::ShowDeleteDialog(path) => {
                self.active_dialog = Some(ActiveDialog::Delete(DeleteConfirmDialog::new(
                    path,
                    self.vault.clone(),
                )));
                self.focus = Focus::Dialog;
                None
            }
            AppEvent::ShowRenameDialog(path) => {
                self.active_dialog = Some(ActiveDialog::Rename(RenameDialog::new(
                    path,
                    self.vault.clone(),
                )));
                self.focus = Focus::Dialog;
                None
            }
            AppEvent::ShowMoveDialog(path) => {
                self.active_dialog = Some(ActiveDialog::Move(MoveDialog::new(
                    path,
                    self.vault.clone(),
                    tx,
                )));
                self.focus = Focus::Dialog;
                None
            }
            AppEvent::RenameValidation { available } => {
                if let Some(ActiveDialog::Rename(d)) = &mut self.active_dialog {
                    d.validation_state = if available {
                        ValidationState::Available
                    } else {
                        ValidationState::Taken
                    };
                    d.validation_task = None;
                }
                None
            }
            AppEvent::MoveDirectoriesLoaded(paths) => {
                if let Some(ActiveDialog::Move(d)) = &mut self.active_dialog {
                    d.all_dirs = paths;
                    d.filtered = None;
                    d.load_task = None;
                    if d.list_state.selected().is_none() && !d.results().is_empty() {
                        d.list_state.select(Some(0));
                    }
                    d.spawn_validation(tx);
                }
                None
            }
            AppEvent::MoveFilterResults(paths) => {
                if let Some(ActiveDialog::Move(d)) = &mut self.active_dialog {
                    d.filter_task = None;
                    d.filtered = Some(paths);
                    if !d.results().is_empty() {
                        d.list_state.select(Some(0));
                    } else {
                        d.list_state.select(None);
                    }
                    d.spawn_validation(tx);
                }
                None
            }
            AppEvent::MoveDestValidation { available } => {
                if let Some(ActiveDialog::Move(d)) = &mut self.active_dialog {
                    d.dest_validation = if available {
                        ValidationState::Available
                    } else {
                        ValidationState::Taken
                    };
                    d.validation_task = None;
                }
                None
            }
            AppEvent::EntryCreated(path) => {
                self.restore_focus();
                self.open_path(path.clone(), tx).await;
                self.focus_editor();
                // Refresh the sidebar so the new note appears in the list.
                let note_parent = path.get_parent_path().0;
                if note_parent.is_like(self.sidebar.current_dir()) {
                    let dir = self.sidebar.current_dir().clone();
                    self.navigate_sidebar(dir, tx).await;
                }
                None
            }
            AppEvent::EntryDeleted(path) => {
                self.on_entry_op(path, tx).await;
                None
            }
            AppEvent::EntryRenamed { from, .. } => {
                self.on_entry_op(from, tx).await;
                None
            }
            AppEvent::EntryMoved { from, .. } => {
                self.on_entry_op(from, tx).await;
                None
            }
            AppEvent::DialogError(msg) => {
                if let Some(dialog) = &mut self.active_dialog {
                    dialog.set_error(msg);
                }
                None
            }
            AppEvent::CloseDialog => {
                self.restore_focus();
                None
            }
            other => Some(other),
        }
    }

    async fn on_exit(&mut self, _tx: &AppTx) {
        self.try_save().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time test: verifies that `Focus::Dialog` variant exists and that
    /// `ActiveDialog` is importable in this module.  No runtime setup needed.
    #[test]
    fn focus_dialog_variant_and_active_dialog_compile() {
        // If `Focus::Dialog` or `ActiveDialog` variants are missing this will
        // fail to compile, catching regressions at test time.
        let focus = Focus::Dialog;
        let label = match focus {
            Focus::Editor => "EDITOR",
            Focus::Sidebar => "SIDEBAR",
            Focus::NoteBrowser => "NOTE BROWSER",
            Focus::Dialog => "DIALOG",
        };
        assert_eq!(label, "DIALOG");

        // Verify ActiveDialog variants are accessible.
        fn _accepts_active_dialog(_d: Option<ActiveDialog>) {}
    }
}
