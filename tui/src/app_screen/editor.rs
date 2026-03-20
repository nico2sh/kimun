use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, VaultBrowseOptionsBuilder};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent};
use crate::components::sidebar::SidebarComponent;
use crate::components::text_editor::TextEditorComponent;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_event_to_combo;
use crate::keys::key_strike::KeyStrike;
use crate::settings::AppSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

enum Focus {
    Sidebar,
    Editor,
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
    quit_key: String,
    toggle_key: String,
    autosave_handle: Option<tokio::task::JoinHandle<()>>,
    key_flash: Option<(String, std::time::Instant)>,
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
        let toggle_key = first_key(&ActionShortcuts::ToggleSidebar);
        let icons = settings.icons();
        Self {
            settings,
            icons: icons.clone(),
            theme,
            editor: TextEditorComponent::new(kb.clone()),
            sidebar: SidebarComponent::new(kb, vault.clone(), icons),
            vault,
            path,
            focus: Focus::Editor,
            sidebar_visible: true,
            quit_key,
            toggle_key,
            autosave_handle: None,
            key_flash: None,
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
        self.settings.save_to_disk().ok();

        self.path = path.clone();
        let content = self.vault.get_note_text(&self.path).await.unwrap();
        self.editor.set_text(content);

        // Only load the sidebar on first open (when it has no entries yet).
        // Selecting a note while browsing should not reload the sidebar.
        if self.sidebar.is_empty() {
            let dir = if path.is_note() {
                path.get_parent_path().0
            } else {
                path
            };
            self.navigate_sidebar(dir, tx).await;
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
            vault.browse_vault(options).await.ok();
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
        if let InputEvent::Key(key) = event {
            if let Some(combo) = key_event_to_combo(key) {
                if (combo.modifiers.is_ctrl() || combo.modifiers.is_alt())
                    && combo.key >= KeyStrike::KeyA
                    && combo.key <= KeyStrike::KeyZ
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
                    _ => {}
                }
            }
        }

        // Mouse events are routed to all components regardless of focus so that
        // clicking anywhere can transfer focus correctly.
        if matches!(event, InputEvent::Mouse(_)) {
            if self.sidebar_visible && self.sidebar.handle_input(event, tx).is_consumed() {
                return EventState::Consumed;
            }
            return self.editor.handle_input(event, tx);
        }

        match self.focus {
            Focus::Sidebar => self.sidebar.handle_input(event, tx),
            Focus::Editor => self.editor.handle_input(event, tx),
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
        f.render_widget(header, rows[0]);

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
        if let Some((_, instant)) = &self.key_flash {
            if instant.elapsed() >= std::time::Duration::from_secs(2) {
                self.key_flash = None;
            }
        }

        let focus_label = if editor_focused { "EDITOR" } else { "SIDEBAR" };
        let mut footer = Block::default()
            .title(format!(
                "[{focus_label}]  {}: Quit  |  {}: Toggle sidebar",
                self.quit_key, self.toggle_key,
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
        };
        let hints_text = hints
            .iter()
            .map(|(key, label)| format!("{key}: {label}"))
            .collect::<Vec<_>>()
            .join("  |  ");
        let hints_text = format!(" {} {hints_text}", self.icons.info);
        f.render_widget(
            Paragraph::new(hints_text).style(Style::default().fg(theme.fg_secondary.to_ratatui())),
            footer_inner,
        );
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) -> Option<AppEvent> {
        match msg {
            AppEvent::Autosave => {
                self.try_save().await;
                None
            }
            AppEvent::OpenPath(path) => {
                if path.is_note() {
                    self.open_path(path, tx).await;
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
                    self.open_path(details.path, tx).await;
                }
                None
            }
            other => Some(other),
        }
    }

    async fn on_exit(&mut self, _tx: &AppTx) {
        self.try_save().await;
    }
}
