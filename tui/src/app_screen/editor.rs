use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::error::{FSError, VaultError};
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::autosave_timer::AutosaveTimer;
use crate::components::dialog_manager::DialogManager;
use crate::components::footer_bar::FooterBar;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent};
use crate::components::note_browser::NoteBrowserModal;
use crate::components::note_browser::file_finder_provider::FileFinderProvider;
use crate::components::note_browser::search_provider::SearchNotesProvider;
use crate::components::sidebar::SidebarComponent;
use crate::components::backlinks_panel::BacklinksPanel;
use crate::components::text_editor::TextEditorComponent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::keys::key_strike::KeyStrike;
use crate::settings::SharedSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

#[derive(Clone, Copy)]
enum Focus {
    Sidebar,
    Editor,
    NoteBrowser,
    Dialog,
    Backlinks,
}

pub struct EditorScreen {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    icons: Icons,
    theme: Theme,
    editor: TextEditorComponent,
    sidebar: SidebarComponent,
    path: VaultPath,
    focus: Focus,
    sidebar_visible: bool,
    footer: FooterBar,
    autosave: AutosaveTimer,
    note_browser: Option<NoteBrowserModal>,
    dialogs: DialogManager,
    backlinks_panel: BacklinksPanel,
    backlinks_visible: bool,
}

impl EditorScreen {
    pub fn new(vault: Arc<NoteVault>, path: VaultPath, settings: SharedSettings) -> Self {
        let s = settings.read().unwrap();
        let kb = s.key_bindings.clone();
        let theme = s.get_theme();
        let kb_map = kb.to_hashmap();
        let first_key = |action: &ActionShortcuts| {
            kb_map
                .get(action)
                .and_then(|v| v.first().cloned())
                .map(|c| c.to_string())
                .unwrap_or_default()
        };
        let footer = FooterBar::new(
            first_key(&ActionShortcuts::OpenSettings),
            first_key(&ActionShortcuts::Quit),
            first_key(&ActionShortcuts::ToggleSidebar),
        );
        let icons = s.icons();
        let sidebar = SidebarComponent::new(kb.clone(), vault.clone(), icons.clone(), &s);
        let backlinks_panel = BacklinksPanel::new(vault.clone(), kb.clone());
        let editor = TextEditorComponent::new(kb, &s);
        drop(s);
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
            footer,
            autosave: AutosaveTimer::new(),
            note_browser: None,
            dialogs: DialogManager::new(),
            backlinks_panel,
            backlinks_visible: false,
        }
    }
}

impl EditorScreen {
    async fn follow_link(&mut self, target: String, tx: &AppTx) {
        // External URL — hand off to the OS browser/handler.
        if target.starts_with("http://") || target.starts_with("https://") {
            if let Err(e) = open::that_detached(&target) {
                self.footer.flash(format!("Cannot open URL: {e}"));
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
                self.dialogs
                    .open_create_note(path, self.vault.clone(), self.focus_index());
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
                let s = self.settings.read().unwrap();
                self.note_browser = Some(NoteBrowserModal::new(
                    format!("Follow: {target}"),
                    provider,
                    self.vault.clone(),
                    s.key_bindings.clone(),
                    s.icons(),
                    tx.clone(),
                ));
                drop(s);
                self.focus = Focus::NoteBrowser;
            }
            Err(e) => {
                self.footer.flash(format!("Link error: {e}"));
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

        {
            let mut s = self.settings.write().unwrap();
            s.add_path_history(&path);
        }
        let settings_snapshot = self.settings.read().unwrap().clone();
        tokio::spawn(async move {
            settings_snapshot.save_to_disk().ok();
        });

        self.path = path.clone();
        match self.vault.get_note_text(&self.path).await {
            Ok(content) => {
                self.editor.set_text(content);
                tx.send(AppEvent::Redraw).ok();
                if self.backlinks_visible {
                    self.backlinks_panel.load(path.clone(), tx.clone());
                }
            }
            Err(e) => {
                if matches!(e, VaultError::FSError(FSError::VaultPathNotFound { .. })) {
                    self.dialogs.open_create_note(
                        self.path.clone(),
                        self.vault.clone(),
                        self.focus_index(),
                    );
                    self.focus = Focus::Dialog;
                } else {
                    tracing::error!("Failed to read note {}: {e}", self.path);
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
        let interval = self.settings.read().unwrap().autosave_interval_secs;
        self.autosave.restart(interval, tx.clone());
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
                tracing::error!("browse_vault failed: {e}");
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

    fn focus_index(&self) -> u8 {
        match self.focus {
            Focus::Sidebar => 0,
            Focus::Editor => 1,
            Focus::NoteBrowser => 2,
            Focus::Dialog => 3,
            Focus::Backlinks => 4,
        }
    }

    fn focus_from_index(idx: u8) -> Focus {
        match idx {
            0 => Focus::Sidebar,
            2 => Focus::NoteBrowser,
            3 => Focus::Dialog,
            4 => Focus::Backlinks,
            _ => Focus::Editor,
        }
    }

    fn restore_focus(&mut self) {
        self.focus = self
            .dialogs
            .close()
            .map(Self::focus_from_index)
            .unwrap_or(Focus::Editor);
    }

    async fn on_entry_op(&mut self, from: VaultPath, tx: &AppTx) {
        self.restore_focus();
        if from == self.path {
            self.autosave.stop();
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

    /// Move focus one step to the left: Backlinks → Editor → Sidebar.
    /// Opens the sidebar if moving left from the editor and it's hidden.
    fn focus_left(&mut self) {
        match self.focus {
            Focus::Backlinks => self.focus_editor(),
            Focus::Editor => self.focus_sidebar(),
            _ => {}
        }
    }

    /// Move focus one step to the right: Sidebar → Editor → Backlinks.
    /// Opens and loads backlinks if moving right from the editor and the panel is hidden.
    fn focus_right(&mut self, tx: &AppTx) {
        match self.focus {
            Focus::Sidebar => self.focus_editor(),
            Focus::Editor => {
                if !self.backlinks_visible {
                    self.backlinks_visible = true;
                    self.backlinks_panel.load(self.path.clone(), tx.clone());
                }
                self.focus = Focus::Backlinks;
            }
            _ => {}
        }
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        if !self.sidebar_visible {
            self.focus_editor();
        }
    }

    fn toggle_backlinks(&mut self, tx: &AppTx) {
        self.backlinks_visible = !self.backlinks_visible;
        if self.backlinks_visible {
            self.backlinks_panel.load(self.path.clone(), tx.clone());
            self.focus = Focus::Backlinks;
        } else if matches!(self.focus, Focus::Backlinks) {
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
            && let Some(combo) = key_event_to_combo(key)
        {
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
                // We display for two seconds the key combination pressed in the footer
                self.footer.flash(combo.to_string());
                let tx2 = tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    tx2.send(AppEvent::Redraw).ok();
                });
            }
            let action = {
                let s = self.settings.read().unwrap();
                s.key_bindings.get_action(&combo)
            };
            match action {
                Some(ActionShortcuts::ToggleSidebar) => {
                    self.toggle_sidebar();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FocusSidebar) => {
                    self.focus_left();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FocusEditor) => {
                    self.focus_right(tx);
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::NewJournal) => {
                    tx.send(AppEvent::OpenJournal).ok();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::SearchNotes) => {
                    if self.note_browser.is_some() {
                        self.note_browser = None;
                        if matches!(self.focus, Focus::NoteBrowser) {
                            self.focus = Focus::Editor;
                        }
                    } else {
                        let s = self.settings.read().unwrap();
                        let provider = SearchNotesProvider::new(
                            self.vault.clone(),
                            s.current_last_paths(),
                        );
                        self.note_browser = Some(NoteBrowserModal::new(
                            "Note Browser",
                            provider,
                            self.vault.clone(),
                            s.key_bindings.clone(),
                            s.icons(),
                            tx.clone(),
                        ));
                        drop(s);
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
                        let s = self.settings.read().unwrap();
                        self.note_browser = Some(NoteBrowserModal::new(
                            "Find Note",
                            provider,
                            self.vault.clone(),
                            s.key_bindings.clone(),
                            s.icons(),
                            tx.clone(),
                        ));
                        drop(s);
                        self.focus = Focus::NoteBrowser;
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FileOperations) if matches!(self.focus, Focus::Editor) => {
                    tx.send(AppEvent::ShowFileOpsMenu(self.path.clone())).ok();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FollowLink) if matches!(self.focus, Focus::Editor) => {
                    if let Some(target) = self.editor.link_at_cursor() {
                        tx.send(AppEvent::FollowLink(target)).ok();
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::ToggleBacklinks) => {
                    self.toggle_backlinks(tx);
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::SwitchWorkspace) => {
                    let s = self.settings.read().unwrap();
                    self.dialogs
                        .open_workspace_switcher(&s, self.focus_index());
                    drop(s);
                    self.focus = Focus::Dialog;
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::QuickNote) => {
                    self.dialogs
                        .open_quick_note(self.vault.clone(), self.focus_index());
                    self.focus = Focus::Dialog;
                    return EventState::Consumed;
                }
                _ => {
                    if is_fkey {
                        // F1 opens the help modal (only when no other dialog is active).
                        if combo.key == KeyStrike::F1
                            && combo.modifiers.is_empty()
                            && !self.dialogs.is_open()
                        {
                            let s = self.settings.read().unwrap();
                            self.dialogs
                                .open_help(&s.key_bindings, self.focus_index());
                            drop(s);
                            self.focus = Focus::Dialog;
                        }
                        // All F-keys (including F1 when a dialog is already open) are consumed
                        // and never forwarded to the embedded editor.
                        return EventState::Consumed;
                    }
                }
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
                && let Some(modal) = &mut self.note_browser
            {
                return modal.handle_input(event, tx);
            }
            if self.sidebar_visible && self.sidebar.handle_input(event, tx).is_consumed() {
                return EventState::Consumed;
            }
            // Backlinks panel consumes mouse events in its focus to prevent
            // clicks from falling through to the editor.
            if matches!(self.focus, Focus::Backlinks) {
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
            Focus::Dialog => self.dialogs.handle_input(event, tx),
            Focus::Backlinks => {
                if let InputEvent::Key(key) = event {
                    // Esc goes directly to editor (not through directional navigation).
                    if key.code == ratatui::crossterm::event::KeyCode::Esc {
                        self.focus_editor();
                        return EventState::Consumed;
                    }
                    self.backlinks_panel.handle_key(key, tx)
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

        let mut constraints = Vec::new();
        if self.sidebar_visible {
            constraints.push(Constraint::Length(30));
        }
        constraints.push(Constraint::Min(0));
        if self.backlinks_visible {
            constraints.push(Constraint::Length(40));
        }
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(rows[1]);

        let editor_focused = matches!(self.focus, Focus::Editor);
        let sidebar_focused = matches!(self.focus, Focus::Sidebar);
        let backlinks_focused = matches!(self.focus, Focus::Backlinks);

        let mut col_idx = 0;
        if self.sidebar_visible {
            self.sidebar.render(f, columns[col_idx], theme, sidebar_focused);
            col_idx += 1;
        }
        let editor_area = columns[col_idx];
        col_idx += 1;

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

        if self.backlinks_visible {
            self.backlinks_panel.render(f, columns[col_idx], theme, backlinks_focused);
        }

        let focus_label = match self.focus {
            Focus::Editor => "EDITOR",
            Focus::Sidebar => "SIDEBAR",
            Focus::NoteBrowser => "NOTE BROWSER",
            Focus::Dialog => "DIALOG",
            Focus::Backlinks => "BACKLINKS",
        };
        let hints = match self.focus {
            Focus::Editor => self.editor.hint_shortcuts(),
            Focus::Sidebar => self.sidebar.hint_shortcuts(),
            Focus::NoteBrowser => self
                .note_browser
                .as_ref()
                .map(|m| m.hint_shortcuts())
                .unwrap_or_default(),
            Focus::Dialog => vec![],
            Focus::Backlinks => self.backlinks_panel.hint_shortcuts(),
        };
        self.footer
            .render(f, rows[2], theme, focus_label, &hints, &self.icons);

        // Modal overlay — rendered last so it appears on top of everything.
        if let Some(modal) = &mut self.note_browser {
            modal.render(f, f.area(), &self.theme, true);
        }

        // Dialog overlay — rendered after the note browser so it appears on top.
        self.dialogs.render(f, f.area(), &self.theme);
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) -> Option<AppEvent> {
        // Let the dialog manager handle dialog-related events first.
        if self
            .dialogs
            .handle_app_message(&msg, &self.vault, tx, self.focus_index())
        {
            // CloseDialog was handled inside the manager; restore our focus.
            if matches!(msg, AppEvent::CloseDialog) {
                self.restore_focus();
            }
            // Show* events switch focus to the dialog.
            if matches!(
                msg,
                AppEvent::ShowFileOpsMenu(_)
                    | AppEvent::ShowDeleteDialog(_)
                    | AppEvent::ShowRenameDialog(_)
                    | AppEvent::ShowMoveDialog(_)
            ) {
                self.focus = Focus::Dialog;
            }
            return None;
        }

        match msg {
            AppEvent::Autosave => {
                self.try_save().await;
                None
            }
            AppEvent::OpenPath(path) => {
                if self.dialogs.is_open() {
                    self.restore_focus();
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
            AppEvent::EntryCreated(path) => {
                self.restore_focus();
                self.open_path(path.clone(), tx).await;
                self.focus_editor();
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
            AppEvent::BacklinksLoaded(entries) => {
                self.backlinks_panel.on_loaded(entries);
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
    /// `DialogManager` is importable in this module.  No runtime setup needed.
    #[test]
    fn focus_dialog_variant_and_dialog_manager_compile() {
        let focus = Focus::Dialog;
        let label = match focus {
            Focus::Editor => "EDITOR",
            Focus::Sidebar => "SIDEBAR",
            Focus::NoteBrowser => "NOTE BROWSER",
            Focus::Dialog => "DIALOG",
            Focus::Backlinks => "BACKLINKS",
        };
        assert_eq!(label, "DIALOG");

        // Verify DialogManager is accessible.
        fn _accepts_dialog_manager(_d: DialogManager) {}
    }
}
