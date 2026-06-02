use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::error::{FSError, VaultError};
use kimun_core::nfs::VaultPath;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app_screen::{AppScreen, ScreenKind};
use crate::app_screen::overlay_host::OverlayHost;
use crate::components::Component;
use crate::components::overlay::{OverlayKind, OverlayMsg};
use crate::components::autosave_timer::AutosaveTimer;
use crate::components::backlinks_panel::QueryPanel;
use crate::components::dialogs::ActiveDialog;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent};
use crate::components::footer_bar::FooterBar;
use crate::components::note_browser::NoteBrowserModal;
use crate::components::note_browser::file_finder_provider::FileFinderProvider;
use crate::components::note_browser::search_provider::SearchNotesProvider;
use crate::components::saved_searches_modal::SavedSearchesModal;
use crate::components::sidebar::SidebarComponent;
use crate::components::text_editor::TextEditorComponent;
use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
use crate::keys::key_event_to_combo;
use crate::keys::key_strike::KeyStrike;
use crate::settings::SharedSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;
use crate::util::single_slot_task::SingleSlotTask;

/// Hard cap on every blocking save path so a stuck disk (NFS hang,
/// fsync stall) cannot freeze quit, navigation, or the next autosave
/// tick. The same value is used both to wait for an in-flight
/// background autosave and to bound our own synchronous save call —
/// the worst-case quit time is therefore one cap plus the
/// observability of `abort()` on a syscall in progress.
const SAVE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Sidebar,
    Editor,
    Overlay,
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
    overlays: OverlayHost<Focus>,
    backlinks_panel: QueryPanel,
    backlinks_visible: bool,
    /// Handle to the most recently spawned background autosave task.
    /// `is_in_flight()` is the source of truth for "is a save still in
    /// flight"; both successful completion AND panic flip it to false
    /// so the next periodic tick can spawn fresh. The synchronous save
    /// paths (`open_path` / `on_entry_op` / `on_exit`) await this slot
    /// before issuing their own `vault.save_note`, so two concurrent
    /// writes for the same path can never collide. Drop aborts the
    /// in-flight task so the spawned future cannot outlive the screen.
    autosave_task: SingleSlotTask<()>,
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
            first_key(&ActionShortcuts::ToggleQueryPanel),
        );
        let icons = s.icons();
        let sidebar = SidebarComponent::new(kb.clone(), vault.clone(), icons.clone(), &s);
        let backlinks_panel = QueryPanel::new(vault.clone(), kb.clone());
        let mut editor = TextEditorComponent::new(kb, &s);
        editor.set_vault(vault.clone());
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
            overlays: OverlayHost::new(),
            backlinks_panel,
            backlinks_visible: false,
            autosave_task: SingleSlotTask::empty(),
        }
    }
}

/// Encodes raw RGBA pixels as a PNG byte stream.
fn encode_rgba_to_png(width: u32, height: u32, rgba: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    }
    Ok(buf)
}

impl EditorScreen {
    /// Pulls an image off the system clipboard, encodes it to PNG, saves it as
    /// an attachment under the vault's `/assets` directory, and inserts a
    /// markdown image link (relative to the current note) at the cursor.
    ///
    /// Returns `true` if a clipboard image was found and the paste was
    /// dispatched — even if the encode/save is still in flight. Returns
    /// `false` if the clipboard contained no image, so the caller can fall
    /// through to a regular text paste.
    fn try_paste_image(&mut self, tx: &AppTx) -> bool {
        let img = match self.editor.take_clipboard_image() {
            Some(i) if !i.rgba.is_empty() && i.width > 0 && i.height > 0 => i,
            _ => return false,
        };
        // arboard contract: rgba length == width * height * 4. A mismatch means
        // a misbehaving clipboard provider — refuse rather than encode garbage.
        let expected = img
            .width
            .checked_mul(img.height)
            .and_then(|n| n.checked_mul(4));
        if expected != Some(img.rgba.len()) {
            self.footer
                .flash("Clipboard image size mismatch".to_string(), tx);
            return true;
        }
        let asset_path = self.vault.generate_attachment_path("image", "png");
        let link_path = asset_path.relative_link_from_note(&self.path);
        let markdown = format!("![]({link_path})");
        let vault = self.vault.clone();
        let tx2 = tx.clone();
        let width = img.width as u32;
        let height = img.height as u32;
        let rgba = img.rgba;
        tokio::spawn(async move {
            // PNG encoding is CPU-bound — keep it off the runtime worker threads.
            let png_bytes =
                match tokio::task::spawn_blocking(move || encode_rgba_to_png(width, height, &rgba))
                    .await
                {
                    Ok(Ok(b)) => b,
                    Ok(Err(e)) => {
                        tx2.send(AppEvent::DialogError(format!("Image encode failed: {e}")))
                            .ok();
                        return;
                    }
                    Err(e) => {
                        tx2.send(AppEvent::DialogError(format!(
                            "Image encode task failed: {e}"
                        )))
                        .ok();
                        return;
                    }
                };
            match vault.save_attachment(&asset_path, &png_bytes).await {
                Ok(()) => {
                    tx2.send(AppEvent::InsertAtCursor(markdown)).ok();
                }
                Err(e) => {
                    tx2.send(AppEvent::DialogError(format!("Image save failed: {e}")))
                        .ok();
                }
            }
        });
        true
    }

    /// Persist a saved search via core. Used by the SaveSearchConfirmed handler
    /// and unit tests.
    #[cfg(test)]
    async fn persist_saved_search(&self, name: &str, query: &str) -> Result<(), VaultError> {
        self.vault.save_search(name, query).await
    }

    async fn follow_link(&mut self, target: String, tx: &AppTx) {
        // External URL — hand off to the OS browser/handler.
        if kimun_core::note::is_remote_url(&target) {
            match open::that_detached(&target) {
                Ok(()) => self.footer.flash(format!("Opening {target}"), tx),
                Err(e) => self.footer.flash(format!("Cannot open URL: {e}"), tx),
            }
            return;
        }

        // Image attachment — resolve the (potentially relative) path against
        // the current note's directory, convert to an OS path, hand off to the
        // OS default handler. Images are not notes, so skip the note lookup.
        if kimun_core::note::target_looks_like_image(&target) {
            let parent = self.path.get_parent_path().0;
            let resolved = parent.append(&VaultPath::new(target.trim()));
            let os_path = self.vault.path_to_pathbuf(&resolved);
            match open::that_detached(&os_path) {
                Ok(()) => self.footer.flash(format!("Opening {target}"), tx),
                Err(e) => self.footer.flash(format!("Cannot open image: {e}"), tx),
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
                self.overlays.open(
                    Box::new(ActiveDialog::create_note(path, self.vault.clone())),
                    self.opener_focus(),
                );
                self.set_focus(Focus::Overlay);
            }
            Ok(mut results) if results.len() == 1 => {
                let (entry, _) = results.remove(0);
                self.open_path(entry.path, tx).await;
            }
            Ok(results) => {
                use crate::components::note_browser::link_results_provider::LinkResultsProvider;
                let provider = LinkResultsProvider::from_results(results);
                let s = self.settings.read().unwrap();
                let modal = NoteBrowserModal::new(
                    format!("Follow: {target}"),
                    provider,
                    self.vault.clone(),
                    s.key_bindings.clone(),
                    s.icons(),
                    tx.clone(),
                );
                drop(s);
                self.overlays.open(Box::new(modal), self.opener_focus());
                self.set_focus(Focus::Overlay);
            }
            Err(e) => {
                self.footer.flash(format!("Link error: {e}"), tx);
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
                self.editor.set_redraw_tx(tx);
                tx.send(AppEvent::Redraw).ok();
                if self.backlinks_visible {
                    self.backlinks_panel.set_note(path.clone(), tx.clone());
                }
            }
            Err(e) => {
                if matches!(e, VaultError::FSError(FSError::VaultPathNotFound { .. })) {
                    self.overlays.open(
                        Box::new(ActiveDialog::create_note(self.path.clone(), self.vault.clone())),
                        self.opener_focus(),
                    );
                    self.set_focus(Focus::Overlay);
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
        // The sidebar hosts a streamed `SearchList`; (re)building its engine for
        // `dir` runs `browse_vault` inside the source and emits rows as they
        // arrive (with a redraw on each).
        self.sidebar.navigate(dir, tx);
    }

    async fn try_save(&mut self) {
        // Wait out any background autosave so two concurrent `vault.save_note`
        // calls cannot race on the same path. Capped at 5s so a wedged
        // filesystem (NFS hang, fsync stall, SQLite lock contention) does
        // not freeze app-quit indefinitely.
        //
        // If the timeout fires we MUST abort the prior task and bail
        // without issuing our own save: dropping a JoinHandle detaches
        // the tokio task rather than cancelling it, so the spawned
        // vault.save_note keeps running. Calling our own vault.save_note
        // on the same path on top of that is the exact two-writer race
        // the in-flight serialisation is meant to prevent. abort() is
        // best-effort (will not unwind an in-progress syscall) but it
        // stops any further await points in the spawned task. The editor
        // stays dirty so the next session retries; the spawned task
        // either finishes against the disk on its own or is killed when
        // the process exits.
        if self.autosave_task.is_in_flight() {
            match self.autosave_task.await_with_timeout(SAVE_TIMEOUT).await {
                Some(_) => {} // completed (success or panic) — slot already cleared
                None => {
                    // Timeout: abort the spawned task and bail.
                    self.autosave_task.abort();
                    return;
                }
            }
        }
        if self.editor.is_dirty() {
            let text = self.editor.get_text();
            // Same cap on our own save so quit cannot hang on a stuck
            // disk. A timeout returns Err(_); we skip mark_saved so the
            // editor stays dirty for any subsequent retry.
            let save = self.vault.save_note(&self.path, &text);
            if matches!(tokio::time::timeout(SAVE_TIMEOUT, save).await, Ok(Ok(_))) {
                self.editor.mark_saved(text);
            }
        }
    }

    /// Fire-and-forget autosave used by the periodic timer. The save runs in
    /// a spawned tokio task so the main event loop is never blocked by the
    /// filesystem + SQLite write. Completion is reported back as
    /// `AppEvent::AutosaveCompleted`, which marks the editor clean iff the
    /// editor is still at the revision that was written. `is_in_flight()`
    /// on the `SingleSlotTask` slot is the "is a save in flight" signal;
    /// it flips to false on both successful completion AND panic, so a
    /// single panicked task can never permanently disable autosave.
    fn spawn_autosave(&mut self, tx: &AppTx) {
        // A previous task that hasn't reported completion yet still holds the
        // lock on the file system + SQLite path; let it finish first.
        if self.autosave_task.is_in_flight() {
            return;
        }
        if !self.editor.is_dirty() {
            return;
        }
        let text = self.editor.get_text();
        let revision = self.editor.content_revision();
        let vault = self.vault.clone();
        let path = self.path.clone();
        let tx = tx.clone();
        self.autosave_task.spawn(async move {
            let saved_revision = vault.save_note(&path, &text).await.ok().map(|_| revision);
            let _ = tx.send(AppEvent::AutosaveCompleted {
                path,
                saved_revision,
            });
        });
    }

    /// The panel focus to restore when the active overlay closes. An overlay
    /// is only ever opened from a panel, so Overlay maps to Editor defensively.
    fn opener_focus(&self) -> Focus {
        match self.focus {
            Focus::Sidebar => Focus::Sidebar,
            Focus::Backlinks => Focus::Backlinks,
            Focus::Editor | Focus::Overlay => Focus::Editor,
        }
    }

    /// Close the active overlay (if any) and restore the opener panel focus.
    fn restore_focus(&mut self) {
        let restored = self.overlays.close().unwrap_or(Focus::Editor);
        self.set_focus(restored);
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
    /// Single entry point for changing focus. Any transition AWAY from
    /// `Focus::Editor` closes the autocomplete popup so it doesn't
    /// linger over the editor while another component owns key input.
    fn set_focus(&mut self, focus: Focus) {
        if !matches!(focus, Focus::Editor) {
            self.editor.close_autocomplete();
        }
        self.focus = focus;
    }

    pub fn focus_editor(&mut self) {
        self.set_focus(Focus::Editor);
    }

    pub fn focus_sidebar(&mut self) {
        self.sidebar_visible = true;
        self.set_focus(Focus::Sidebar);
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
                    self.backlinks_panel.set_note(self.path.clone(), tx.clone());
                }
                self.set_focus(Focus::Backlinks);
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

    fn apply_saved_search(&mut self, query: String, name: String, tx: &AppTx) {
        self.backlinks_visible = true;
        // The virtual backlinks entry's name should not override the
        // default "Backlinks" title — but the panel's title logic already
        // shows "Backlinks" whenever the active query is `<{note}`, so it's
        // safe to always pass the name through.
        self.backlinks_panel
            .apply_query(query, Some(name), tx.clone());
        self.set_focus(Focus::Backlinks);
    }

    fn toggle_backlinks(&mut self, tx: &AppTx) {
        self.backlinks_visible = !self.backlinks_visible;
        if self.backlinks_visible {
            self.backlinks_panel.set_note(self.path.clone(), tx.clone());
            self.set_focus(Focus::Backlinks);
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
        // Bracketed paste (terminal-level) — fired by Cmd+V on macOS and by
        // terminal-paste shortcuts on every platform. Try image first; if the
        // clipboard does not hold an image, fall back to the pasted text. The
        // payload string may be empty (e.g. clipboard contains image only).
        if matches!(self.focus, Focus::Editor)
            && let InputEvent::Paste(text) = event
        {
            if !self.try_paste_image(tx) && !text.is_empty() {
                self.editor.paste_text(text, tx);
            }
            return EventState::Consumed;
        }
        // Intercept Ctrl+V to handle image paste before the editor consumes it
        // for a regular text paste. Falls through if the clipboard is not an image.
        if matches!(self.focus, Focus::Editor)
            && let InputEvent::Key(key) = event
            && key.modifiers == ratatui::crossterm::event::KeyModifiers::CONTROL
            && key.code == ratatui::crossterm::event::KeyCode::Char('v')
            && self.try_paste_image(tx)
        {
            return EventState::Consumed;
        }
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
                self.footer.flash(combo.to_string(), tx);
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
                    if self.overlays.active_kind() == Some(OverlayKind::NoteBrowser) {
                        self.restore_focus();
                    } else if !self.overlays.is_open() {
                        let s = self.settings.read().unwrap();
                        let provider =
                            SearchNotesProvider::new(self.vault.clone(), s.current_last_paths());
                        let modal = NoteBrowserModal::new(
                            "Note Browser",
                            provider,
                            self.vault.clone(),
                            s.key_bindings.clone(),
                            s.icons(),
                            tx.clone(),
                        );
                        drop(s);
                        self.overlays.open(Box::new(modal), self.opener_focus());
                        self.set_focus(Focus::Overlay);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::OpenNote) => {
                    if self.overlays.active_kind() == Some(OverlayKind::NoteBrowser) {
                        self.restore_focus();
                    } else if !self.overlays.is_open() {
                        let current_dir = self.path.get_parent_path().0;
                        let provider = FileFinderProvider::new(self.vault.clone(), current_dir);
                        let s = self.settings.read().unwrap();
                        let modal = NoteBrowserModal::new(
                            "Find Note",
                            provider,
                            self.vault.clone(),
                            s.key_bindings.clone(),
                            s.icons(),
                            tx.clone(),
                        );
                        drop(s);
                        self.overlays.open(Box::new(modal), self.opener_focus());
                        self.set_focus(Focus::Overlay);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FileOperations) if matches!(self.focus, Focus::Editor) => {
                    tx.send(AppEvent::ShowFileOpsMenu(self.path.clone())).ok();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FollowLink) if matches!(self.focus, Focus::Editor) => {
                    use crate::components::text_editor::LinkTarget;
                    match self.editor.link_at_cursor() {
                        Some(LinkTarget::Note(target)) => {
                            tx.send(AppEvent::FollowLink(target)).ok();
                        }
                        Some(LinkTarget::Label(name)) => {
                            tx.send(AppEvent::FollowLabel(name)).ok();
                        }
                        None => {}
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::ToggleQueryPanel) => {
                    self.toggle_backlinks(tx);
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::OpenSavedSearches) => {
                    if self.overlays.active_kind() == Some(OverlayKind::SavedSearches) {
                        self.restore_focus();
                    } else if !self.overlays.is_open() {
                        let s = self.settings.read().unwrap();
                        let modal = SavedSearchesModal::new(
                            self.vault.clone(),
                            s.key_bindings.clone(),
                            s.icons(),
                            tx.clone(),
                        );
                        drop(s);
                        self.overlays.open(Box::new(modal), self.opener_focus());
                        self.set_focus(Focus::Overlay);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::SaveCurrentQuery) => {
                    // Source the query from the active note browser if one is
                    // open (Ctrl+K modal), otherwise from the Query panel. Any
                    // other overlay being open suppresses the action.
                    let query = match self.overlays.active_kind() {
                        Some(OverlayKind::NoteBrowser) => {
                            self.overlays.active_query().unwrap_or_default().to_string()
                        }
                        None => self.backlinks_panel.active_query().to_string(),
                        Some(_) => String::new(),
                    };
                    if !query.trim().is_empty() {
                        // Opening the save dialog replaces the note browser (if
                        // any); the chained-open guard preserves the original
                        // opener focus.
                        self.overlays.open(
                            Box::new(ActiveDialog::save_search(query)),
                            self.opener_focus(),
                        );
                        self.set_focus(Focus::Overlay);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::SwitchWorkspace) => {
                    if !self.overlays.is_open() {
                        let s = self.settings.read().unwrap();
                        let dialog = ActiveDialog::workspace_switcher(&s);
                        drop(s);
                        self.overlays.open(Box::new(dialog), self.opener_focus());
                        self.set_focus(Focus::Overlay);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::QuickNote) => {
                    if !self.overlays.is_open() {
                        self.overlays.open(
                            Box::new(ActiveDialog::quick_note(self.vault.clone())),
                            self.opener_focus(),
                        );
                        self.set_focus(Focus::Overlay);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FindInBuffer) if matches!(self.focus, Focus::Editor) => {
                    self.editor.open_or_advance_search();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::Text(
                    action @ (TextAction::Bold | TextAction::Italic | TextAction::Strikethrough),
                )) if matches!(self.focus, Focus::Editor) => {
                    self.editor.apply_text_action(action);
                    return EventState::Consumed;
                }
                _ => {
                    if is_fkey {
                        // F1 opens the help modal (only when no other dialog is active).
                        if combo.key == KeyStrike::F1
                            && combo.modifiers.is_empty()
                            && !self.overlays.is_open()
                        {
                            let s = self.settings.read().unwrap();
                            let dialog = ActiveDialog::help(&s.key_bindings);
                            drop(s);
                            self.overlays.open(Box::new(dialog), self.opener_focus());
                            self.set_focus(Focus::Overlay);
                        }
                        // All F-keys (including F1 when a dialog is already open) are consumed
                        // and never forwarded to the embedded editor.
                        return EventState::Consumed;
                    }
                }
            }
        }

        if matches!(event, InputEvent::Mouse(_)) {
            // An open overlay captures all mouse events.
            if self.overlays.is_open() {
                return self.overlays.handle_input(event, tx);
            }
            if self.sidebar_visible && self.sidebar.handle_input(event, tx).is_consumed() {
                return EventState::Consumed;
            }
            // Query panel consumes mouse events in its focus to prevent
            // clicks falling through to the editor.
            if matches!(self.focus, Focus::Backlinks) {
                return EventState::Consumed;
            }
            return self.editor.handle_input(event, tx);
        }

        match self.focus {
            Focus::Sidebar => self.sidebar.handle_input(event, tx),
            Focus::Editor => self.editor.handle_input(event, tx),
            Focus::Overlay => self.overlays.handle_input(event, tx),
            Focus::Backlinks => {
                if let InputEvent::Key(key) = event {
                    // Give the panel first crack (its autocomplete popup may
                    // consume Esc to close itself). If the panel doesn't
                    // consume it, Esc returns focus to the editor.
                    let state = self.backlinks_panel.handle_key(key, tx);
                    if state == EventState::NotConsumed
                        && key.code == ratatui::crossterm::event::KeyCode::Esc
                    {
                        self.focus_editor();
                        return EventState::Consumed;
                    }
                    state
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

        let workspace_label = {
            let s = self.settings.read().unwrap();
            s.workspace_config
                .as_ref()
                .map(|wc| format!("{} {}", self.icons.workspace, wc.global.current_workspace))
                .unwrap_or_default()
        };
        let header = Block::default()
            .title("Kimün")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border.to_ratatui()))
            .style(theme.base_style())
            .title_style(Style::default().fg(theme.accent.to_ratatui()));
        let header_inner = header.inner(rows[0]);
        f.render_widget(header, rows[0]);

        // Split header inner: note path on left, workspace label on right.
        let header_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(workspace_label.len() as u16 + 1),
            ])
            .split(header_inner);
        f.render_widget(
            Paragraph::new(self.path.to_string())
                .style(Style::default().fg(theme.fg_secondary.to_ratatui())),
            header_cols[0],
        );
        f.render_widget(
            Paragraph::new(workspace_label)
                .alignment(ratatui::layout::Alignment::Right)
                .style(Style::default().fg(theme.fg_muted.to_ratatui())),
            header_cols[1],
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
            self.sidebar
                .render(f, columns[col_idx], theme, sidebar_focused);
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
            self.backlinks_panel
                .render(f, columns[col_idx], theme, backlinks_focused);
        }

        let focus_label = match self.focus {
            Focus::Editor => "EDITOR",
            Focus::Sidebar => "SIDEBAR",
            Focus::Backlinks => "BACKLINKS",
            Focus::Overlay => self
                .overlays
                .active_kind()
                .map(|k| k.label())
                .unwrap_or("EDITOR"),
        };
        let hints = match self.focus {
            Focus::Editor => self.editor.hint_shortcuts(),
            Focus::Sidebar => self.sidebar.hint_shortcuts(),
            Focus::Backlinks => self.backlinks_panel.hint_shortcuts(),
            Focus::Overlay => self.overlays.hint_shortcuts(),
        };
        self.footer
            .render(f, rows[2], theme, focus_label, &hints, &self.icons);

        // Overlay — rendered last so it appears on top of everything.
        self.overlays.render(f, f.area(), &self.theme);
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) -> Option<AppEvent> {
        // Route validation / async-result messages to the active overlay first,
        // so an open dialog still receives its events. Show*/CloseOverlay are
        // NotConsumed by overlays and fall through to the owned match below.
        match self.overlays.handle_app_message(&msg, &self.vault, tx) {
            OverlayMsg::Consumed => return None,
            OverlayMsg::NotConsumed => {}
        }

        match msg {
            AppEvent::ShowFileOpsMenu(path) => {
                self.overlays.open(
                    Box::new(ActiveDialog::file_ops_menu(path)),
                    self.opener_focus(),
                );
                self.set_focus(Focus::Overlay);
                None
            }
            AppEvent::ShowDeleteDialog(path) => {
                self.overlays.open(
                    Box::new(ActiveDialog::delete(path, self.vault.clone())),
                    self.opener_focus(),
                );
                self.set_focus(Focus::Overlay);
                None
            }
            AppEvent::ShowRenameDialog(path) => {
                self.overlays.open(
                    Box::new(ActiveDialog::rename(path, self.vault.clone())),
                    self.opener_focus(),
                );
                self.set_focus(Focus::Overlay);
                None
            }
            AppEvent::ShowMoveDialog(path) => {
                self.overlays.open(
                    Box::new(ActiveDialog::move_to(path, self.vault.clone(), tx)),
                    self.opener_focus(),
                );
                self.set_focus(Focus::Overlay);
                None
            }
            AppEvent::CloseOverlay => {
                // Dismiss-to-opener. Guarded by is_open() on purpose: a
                // selection that wants a specific post-close focus
                // (OpenPath -> editor, SavedSearchSelected -> Query panel)
                // closes the overlay itself first, so a later/!dialog
                // CloseOverlay must not re-restore and clobber that focus.
                if self.overlays.is_open() {
                    self.restore_focus();
                }
                None
            }
            AppEvent::Autosave => {
                self.spawn_autosave(tx);
                None
            }
            AppEvent::AutosaveCompleted {
                path,
                saved_revision,
            } => {
                if path == self.path
                    && let Some(rev) = saved_revision
                {
                    self.editor.mark_saved_at_revision(rev);
                }
                // `SingleSlotTask::is_in_flight()` flips to false the
                // moment the spawned future returns (success or panic),
                // so we don't have to clear the slot manually here —
                // the next `spawn_autosave` tick will overwrite it.
                // Skip explicit cleanup; was previously racy because a
                // stale completion arriving after `try_save` had
                // already cleared and respawned could wipe the fresh
                // handle.
                None
            }
            AppEvent::OpenPath(path) => {
                if self.overlays.is_open() {
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
            AppEvent::SavedSearchSelected { query, name } => {
                self.overlays.close();
                self.apply_saved_search(query, name, tx);
                None
            }
            AppEvent::FollowLink(target) => {
                self.follow_link(target, tx).await;
                None
            }
            AppEvent::FollowLabel(name) => {
                let initial = format!("#{name}");
                let s = self.settings.read().unwrap();
                let provider = SearchNotesProvider::new(self.vault.clone(), s.current_last_paths());
                let modal = NoteBrowserModal::with_initial_query(
                    "Note Browser",
                    provider,
                    self.vault.clone(),
                    s.key_bindings.clone(),
                    s.icons(),
                    tx.clone(),
                    initial,
                );
                drop(s);
                self.overlays.open(Box::new(modal), self.opener_focus());
                self.set_focus(Focus::Overlay);
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
            AppEvent::SaveSearchConfirmed { name, query } => {
                let vault = self.vault.clone();
                tokio::spawn(async move {
                    if let Err(e) = vault.save_search(&name, &query).await {
                        tracing::warn!("failed to save search '{}': {}", name, e);
                    }
                });
                None
            }
            AppEvent::InsertAtCursor(text) => {
                if matches!(self.focus, Focus::Editor) {
                    self.editor.insert_at_cursor(&text, tx);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time test: the collapsed `Focus` enum and `OverlayKind` are
    /// usable from this module.
    #[test]
    fn focus_overlay_variant_and_overlay_kind_compile() {
        let focus = Focus::Overlay;
        let label = match focus {
            Focus::Editor => "EDITOR",
            Focus::Sidebar => "SIDEBAR",
            Focus::Backlinks => "BACKLINKS",
            Focus::Overlay => "OVERLAY",
        };
        assert_eq!(label, "OVERLAY");
        let _kind = OverlayKind::Dialog;
    }

    #[tokio::test]
    async fn persist_saved_search_writes_via_core() {
        use crate::settings::AppSettings;
        use kimun_core::VaultConfig;
        use std::sync::RwLock;

        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(NoteVault::new(VaultConfig::new(dir.path())).await.unwrap());
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        let screen = EditorScreen::new(vault, VaultPath::root(), settings);

        screen.persist_saved_search("t", "#todo").await.unwrap();

        let all = screen.vault.list_saved_searches().await.unwrap();
        assert!(all.iter().any(|s| s.name == "t" && s.query == "#todo"));
    }

    #[tokio::test]
    async fn applying_saved_search_sets_panel_query_and_focuses_it() {
        use crate::settings::AppSettings;
        use kimun_core::VaultConfig;
        use std::sync::RwLock;

        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(NoteVault::new(VaultConfig::new(dir.path())).await.unwrap());
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        let mut screen = EditorScreen::new(vault, VaultPath::root(), settings);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.apply_saved_search(
            "<{note}".to_string(),
            "Backlinks (current note)".to_string(),
            &tx,
        );
        assert!(screen.backlinks_visible);
        assert_eq!(screen.backlinks_panel.active_query(), "<{note}");
        assert!(matches!(screen.focus, Focus::Backlinks));
    }

    // The try_save timeout-abort regression tests (commits 55eb49ed +
    // 5e28b796) previously lived here against `await_or_abort`. The
    // logic now lives in `SingleSlotTask::await_with_timeout` and is
    // covered by `single_slot_task_timeout_returns_none_keeps_handle`
    // in `crate::util::single_slot_task`.

    #[tokio::test]
    async fn saved_search_selected_then_close_overlay_keeps_backlinks_focus() {
        use crate::settings::AppSettings;
        use kimun_core::VaultConfig;
        use std::sync::RwLock;

        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(NoteVault::new(VaultConfig::new(dir.path())).await.unwrap());
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        let mut screen = EditorScreen::new(vault, VaultPath::root(), settings);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // Replay the exact sequence the saved-searches modal emits on select.
        screen
            .handle_app_message(
                AppEvent::SavedSearchSelected {
                    query: "<{note}".to_string(),
                    name: "Backlinks (current note)".to_string(),
                },
                &tx,
            )
            .await;
        screen
            .handle_app_message(AppEvent::CloseOverlay, &tx)
            .await;

        assert!(matches!(screen.focus, Focus::Backlinks), "focus should remain on the Query panel after select + close");
        assert!(!screen.overlays.is_open(), "overlay should be closed");
    }

    /// Capture-all guard: while an overlay is open, an opener action
    /// (QuickNote) must NOT replace it. Drives a real QuickNote keypress
    /// through `handle_input` so the guard in the action arm is exercised
    /// end-to-end.
    #[tokio::test]
    async fn opener_action_does_not_replace_open_overlay() {
        use crate::settings::AppSettings;
        use kimun_core::VaultConfig;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use std::sync::RwLock;

        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(NoteVault::new(VaultConfig::new(dir.path())).await.unwrap());
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        let mut screen = EditorScreen::new(vault.clone(), VaultPath::root(), settings.clone());

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Force a SavedSearches overlay open and focus it, as if the user had
        // opened it via its action.
        {
            let s = settings.read().unwrap();
            let modal = SavedSearchesModal::new(
                vault.clone(),
                s.key_bindings.clone(),
                s.icons(),
                tx.clone(),
            );
            drop(s);
            screen
                .overlays
                .open(Box::new(modal), screen.opener_focus());
        }
        screen.set_focus(Focus::Overlay);
        assert_eq!(
            screen.overlays.active_kind(),
            Some(OverlayKind::SavedSearches),
            "precondition: SavedSearches overlay is active"
        );

        // The QuickNote action key (Ctrl+W by default). Assert the binding
        // resolves to QuickNote so this test fails loudly if the default
        // rebinds, rather than silently exercising the wrong path.
        let quick_note_event = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        {
            let s = settings.read().unwrap();
            let combo = key_event_to_combo(&quick_note_event).expect("Ctrl+W maps to a combo");
            assert_eq!(
                s.key_bindings.get_action(&combo),
                Some(ActionShortcuts::QuickNote),
                "test assumes Ctrl+W is bound to QuickNote"
            );
        }

        screen.handle_input(&InputEvent::Key(quick_note_event), &tx);

        // The guard must have suppressed the open: the SavedSearches overlay
        // is still active, NOT replaced by the QuickNote dialog.
        assert_eq!(
            screen.overlays.active_kind(),
            Some(OverlayKind::SavedSearches),
            "open overlay must not be replaced by a QuickNote opener action"
        );
    }

    #[tokio::test]
    async fn save_query_from_note_browser_opens_save_dialog() {
        use crate::settings::AppSettings;
        use kimun_core::VaultConfig;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use std::sync::RwLock;

        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(NoteVault::new(VaultConfig::new(dir.path())).await.unwrap());
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        let mut screen = EditorScreen::new(vault.clone(), VaultPath::root(), settings.clone());

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Open a note browser carrying a query, as if the user typed "#todo".
        {
            let s = settings.read().unwrap();
            let provider = SearchNotesProvider::new(vault.clone(), s.current_last_paths());
            let modal = NoteBrowserModal::with_initial_query(
                "Note Browser",
                provider,
                vault.clone(),
                s.key_bindings.clone(),
                s.icons(),
                tx.clone(),
                "#todo",
            );
            drop(s);
            screen.overlays.open(Box::new(modal), screen.opener_focus());
        }
        screen.set_focus(Focus::Overlay);
        assert_eq!(
            screen.overlays.active_kind(),
            Some(OverlayKind::NoteBrowser),
            "precondition: note browser is active with a query"
        );
        assert_eq!(screen.overlays.active_query(), Some("#todo"));

        // Ctrl+D (SaveCurrentQuery) while the note browser is active should
        // replace it with the save-search dialog, sourcing the browser's query.
        let save_event = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        {
            let s = settings.read().unwrap();
            let combo = key_event_to_combo(&save_event).expect("Ctrl+D maps to a combo");
            assert_eq!(
                s.key_bindings.get_action(&combo),
                Some(ActionShortcuts::SaveCurrentQuery),
                "test assumes Ctrl+D is bound to SaveCurrentQuery"
            );
        }

        screen.handle_input(&InputEvent::Key(save_event), &tx);

        assert_eq!(
            screen.overlays.active_kind(),
            Some(OverlayKind::Dialog),
            "saving from the note browser opens the save-search dialog"
        );
    }
}
