use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::error::{FSError, VaultError};
use kimun_core::nfs::VaultPath;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app_screen::overlay_host::OverlayHost;
use crate::app_screen::panel_set::PanelSet;
use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::Component;
use crate::components::autosave_timer::AutosaveTimer;
use crate::components::backlinks_panel::QueryPanel;
use crate::components::dialogs::ActiveDialog;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent, SortTarget};
use crate::components::footer_bar::FooterBar;
use crate::components::note_browser::NoteBrowserModal;
use crate::components::note_browser::file_finder_provider::FileFinderProvider;
use crate::components::note_browser::search_provider::SearchNotesProvider;
use crate::components::overlay::{Overlay, OverlayKind, OverlayMsg};
use crate::components::panel::PanelKind;
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

pub struct EditorScreen {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    icons: Icons,
    theme: Theme,
    /// The persistent panels (sidebar, editor, Query panel) plus their order,
    /// visibility, and focus. The host reaches a specific panel through the
    /// typed accessors (`panels.editor_mut()`, …) for panel-specific calls.
    panels: PanelSet,
    path: VaultPath,
    footer: FooterBar,
    autosave: AutosaveTimer,
    /// The active overlay, if any. An open overlay intercepts input ahead of
    /// the panels; closing it restores focus to the panel that opened it.
    overlays: OverlayHost<PanelKind>,
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
        let sidebar = SidebarComponent::from_settings(vault.clone(), &s);
        let backlinks_panel = QueryPanel::new(vault.clone(), kb.clone());
        let mut editor = TextEditorComponent::new(kb, &s);
        editor.set_vault(vault.clone());
        drop(s);
        Self {
            settings,
            icons,
            theme,
            panels: PanelSet::from_panels(sidebar, editor, backlinks_panel),
            vault,
            path,
            footer,
            autosave: AutosaveTimer::new(),
            overlays: OverlayHost::new(),
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
        let img = match self.panels.editor_mut().take_clipboard_image() {
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
                self.present_overlay(Box::new(ActiveDialog::create_note(
                    path,
                    self.vault.clone(),
                )));
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
                self.present_overlay(Box::new(modal));
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
                self.panels.editor_mut().set_text(content);
                self.panels.editor_mut().set_redraw_tx(tx);
                tx.send(AppEvent::Redraw).ok();
                if self.panels.is_visible(PanelKind::Query) {
                    self.panels.query_mut().set_note(path.clone(), tx.clone());
                }
            }
            Err(e) => {
                if matches!(e, VaultError::FSError(FSError::VaultPathNotFound { .. })) {
                    self.present_overlay(Box::new(ActiveDialog::create_note(
                        self.path.clone(),
                        self.vault.clone(),
                    )));
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
        if self.panels.sidebar().is_empty() {
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
        self.panels.sidebar_mut().navigate(dir, tx);
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
        if self.panels.editor().is_dirty() {
            let text = self.panels.editor().get_text();
            // Same cap on our own save so quit cannot hang on a stuck
            // disk. A timeout returns Err(_); we skip mark_saved so the
            // editor stays dirty for any subsequent retry.
            let save = self.vault.save_note(&self.path, &text);
            if matches!(tokio::time::timeout(SAVE_TIMEOUT, save).await, Ok(Ok(_))) {
                self.panels.editor_mut().mark_saved(text);
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
        if !self.panels.editor().is_dirty() {
            return;
        }
        let text = self.panels.editor().get_text();
        let revision = self.panels.editor().content_revision();
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

    /// The panel to restore focus to when the active overlay closes — the
    /// panel that was focused when the overlay opened.
    fn opener_focus(&self) -> PanelKind {
        self.panels.focused()
    }

    /// Present `overlay`, recording the currently focused panel as its opener so
    /// `dismiss_overlay` can return there on close. The single way the editor
    /// opens an overlay — the focus contract lives here, not at each call site.
    fn present_overlay(&mut self, overlay: Box<dyn Overlay>) {
        let opener = self.opener_focus();
        self.overlays.open(overlay, opener);
    }

    /// Close the active overlay and restore focus to the panel that opened it.
    /// The close-side mirror of `present_overlay`. `OverlayHost::close` returns
    /// `None` when nothing is open, so this is a no-op then — which is also why
    /// a selection that closed the overlay itself (and chose its own focus) is
    /// not re-restored by a trailing `CloseOverlay`.
    fn dismiss_overlay(&mut self) {
        if let Some(opener) = self.overlays.close() {
            self.panels.focus(opener);
        }
    }

    async fn on_entry_op(&mut self, from: VaultPath, tx: &AppTx) {
        self.dismiss_overlay();
        if from == self.path {
            self.autosave.stop();
            self.try_save().await;
            let parent = self.path.get_parent_path().0;
            tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(
                self.vault.clone(),
                parent,
            )))
            .ok();
        } else if from
            .get_parent_path()
            .0
            .is_like(self.panels.sidebar().current_dir())
        {
            let dir = self.panels.sidebar().current_dir().clone();
            self.navigate_sidebar(dir, tx).await;
        }
    }
}

impl EditorScreen {
    /// The editor owns key input: it is focused and no overlay sits over it.
    /// Editor-only shortcuts (file ops, follow-link, find, text styling) and
    /// the paste intercept gate on this.
    fn editor_active(&self) -> bool {
        self.panels.focused() == PanelKind::Editor && !self.overlays.is_open()
    }

    pub fn focus_editor(&mut self) {
        self.panels.focus(PanelKind::Editor);
    }

    pub fn focus_sidebar(&mut self) {
        self.panels.show(PanelKind::Sidebar);
        self.panels.focus(PanelKind::Sidebar);
    }

    /// Move focus to `kind`, revealing it first. Revealing the Query panel
    /// loads it for the current note (the heavy side effect that keeps the
    /// reveal in the host rather than in `PanelSet`).
    fn move_focus_to(&mut self, kind: PanelKind, tx: &AppTx) {
        let newly_shown = !self.panels.is_visible(kind);
        self.panels.show(kind);
        if kind == PanelKind::Query && newly_shown {
            self.panels
                .query_mut()
                .set_note(self.path.clone(), tx.clone());
        }
        self.panels.focus(kind);
    }

    /// Move focus one panel left in the current order (revealing it).
    fn focus_left(&mut self, tx: &AppTx) {
        if let Some(kind) = self.panels.prev_kind() {
            self.move_focus_to(kind, tx);
        }
    }

    /// Move focus one panel right in the current order (revealing it).
    fn focus_right(&mut self, tx: &AppTx) {
        if let Some(kind) = self.panels.next_kind() {
            self.move_focus_to(kind, tx);
        }
    }

    fn toggle_sidebar(&mut self) {
        if self.panels.is_visible(PanelKind::Sidebar) {
            self.panels.hide(PanelKind::Sidebar);
        } else {
            self.panels.show(PanelKind::Sidebar);
        }
    }

    fn apply_saved_search(&mut self, query: String, name: String, tx: &AppTx) {
        self.panels.show(PanelKind::Query);
        // The virtual backlinks entry's name should not override the
        // default "Backlinks" title — but the panel's title logic already
        // shows "Backlinks" whenever the active query is `<{note}`, so it's
        // safe to always pass the name through.
        self.panels
            .query_mut()
            .apply_query(query, Some(name), tx.clone());
        self.panels.focus(PanelKind::Query);
    }

    fn toggle_backlinks(&mut self, tx: &AppTx) {
        if self.panels.is_visible(PanelKind::Query) {
            self.panels.hide(PanelKind::Query);
        } else {
            self.panels.show(PanelKind::Query);
            self.panels
                .query_mut()
                .set_note(self.path.clone(), tx.clone());
            self.panels.focus(PanelKind::Query);
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
        if self.editor_active()
            && let InputEvent::Paste(text) = event
        {
            if !self.try_paste_image(tx) && !text.is_empty() {
                self.panels.editor_mut().paste_text(text, tx);
            }
            return EventState::Consumed;
        }
        // Intercept Ctrl+V to handle image paste before the editor consumes it
        // for a regular text paste. Falls through if the clipboard is not an image.
        if self.editor_active()
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
                    // No-op while an overlay owns input, but still consume the key.
                    if !self.overlays.is_open() {
                        self.focus_left(tx);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FocusEditor) => {
                    if !self.overlays.is_open() {
                        self.focus_right(tx);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::NewJournal) => {
                    tx.send(AppEvent::OpenJournal).ok();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::SearchNotes) => {
                    if self.overlays.active_kind() == Some(OverlayKind::NoteBrowser) {
                        self.dismiss_overlay();
                    } else if !self.overlays.is_open() {
                        let s = self.settings.read().unwrap();
                        let provider = SearchNotesProvider::new(
                            self.vault.clone(),
                            s.current_last_paths(),
                            Some(self.path.clone()),
                        );
                        let modal = NoteBrowserModal::new(
                            "Note Browser",
                            provider,
                            self.vault.clone(),
                            s.key_bindings.clone(),
                            s.icons(),
                            tx.clone(),
                        );
                        drop(s);
                        self.present_overlay(Box::new(modal));
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::OpenNote) => {
                    if self.overlays.active_kind() == Some(OverlayKind::NoteBrowser) {
                        self.dismiss_overlay();
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
                        self.present_overlay(Box::new(modal));
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FileOperations) if self.editor_active() => {
                    tx.send(AppEvent::ShowFileOpsMenu(self.path.clone())).ok();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FollowLink) if self.editor_active() => {
                    use crate::components::text_editor::LinkTarget;
                    match self.panels.editor_mut().link_at_cursor() {
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
                        self.dismiss_overlay();
                    } else if !self.overlays.is_open() {
                        let s = self.settings.read().unwrap();
                        let modal = SavedSearchesModal::new(
                            self.vault.clone(),
                            s.key_bindings.clone(),
                            s.icons(),
                            tx.clone(),
                        );
                        drop(s);
                        self.present_overlay(Box::new(modal));
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::OpenSortDialog) => {
                    if !self.overlays.is_open() {
                        let target = match self.panels.focused() {
                            PanelKind::Query => Some(SortTarget::Query),
                            PanelKind::Sidebar => Some(SortTarget::Sidebar),
                            _ if self.panels.is_visible(PanelKind::Sidebar) => {
                                Some(SortTarget::Sidebar)
                            }
                            _ => None,
                        };
                        if let Some(target) = target {
                            let dialog = match target {
                                SortTarget::Sidebar => {
                                    let (f, o) = self.panels.sidebar().current_sort();
                                    ActiveDialog::sort(
                                        target,
                                        f,
                                        o,
                                        self.panels.sidebar().group_dirs(),
                                    )
                                }
                                SortTarget::Query => {
                                    let (f, o) = self.panels.query().current_order();
                                    ActiveDialog::sort(target, f, o, false)
                                }
                            };
                            self.present_overlay(Box::new(dialog));
                        }
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
                        None => self.panels.query().active_query().to_string(),
                        Some(_) => String::new(),
                    };
                    if !query.trim().is_empty() {
                        // Opening the save dialog replaces the note browser (if
                        // any); the chained-open guard preserves the original
                        // opener focus.
                        self.present_overlay(Box::new(ActiveDialog::save_search(query)));
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::SwitchWorkspace) => {
                    if !self.overlays.is_open() {
                        let s = self.settings.read().unwrap();
                        let dialog = ActiveDialog::workspace_switcher(&s);
                        drop(s);
                        self.present_overlay(Box::new(dialog));
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::QuickNote) => {
                    if !self.overlays.is_open() {
                        self.present_overlay(Box::new(ActiveDialog::quick_note(
                            self.vault.clone(),
                        )));
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FindInBuffer) if self.editor_active() => {
                    self.panels.editor_mut().open_or_advance_search();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::Text(
                    action @ (TextAction::Bold | TextAction::Italic | TextAction::Strikethrough),
                )) if self.editor_active() => {
                    self.panels.editor_mut().apply_text_action(action);
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
                            self.present_overlay(Box::new(dialog));
                        }
                        // All F-keys (including F1 when a dialog is already open) are consumed
                        // and never forwarded to the embedded editor.
                        return EventState::Consumed;
                    }
                }
            }
        }

        // An open overlay intercepts all remaining input ahead of the panels.
        if self.overlays.is_open() {
            return self.overlays.handle_input(event, tx);
        }

        if matches!(event, InputEvent::Mouse(_)) {
            // The sidebar gets first crack at clicks even when another panel is
            // focused (it consumes only clicks landing in its own area).
            if self.panels.is_visible(PanelKind::Sidebar)
                && self
                    .panels
                    .sidebar_mut()
                    .handle_input(event, tx)
                    .is_consumed()
            {
                return EventState::Consumed;
            }
            // The Query panel swallows mouse events while focused so clicks
            // don't fall through to the editor.
            if self.panels.focused() == PanelKind::Query {
                return EventState::Consumed;
            }
            return self.panels.editor_mut().handle_input(event, tx);
        }

        // Keyboard → the focused panel. The Query panel gets first crack (its
        // autocomplete popup may consume Esc); on an unhandled Esc it yields
        // focus back to the editor.
        if self.panels.focused() == PanelKind::Query
            && let InputEvent::Key(key) = event
        {
            let state = self.panels.handle_input(event, tx);
            if state == EventState::NotConsumed
                && key.code == ratatui::crossterm::event::KeyCode::Esc
            {
                self.focus_editor();
                return EventState::Consumed;
            }
            return state;
        }

        self.panels.handle_input(event, tx)
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

        // The panels lay themselves out (in config order) and render. No panel
        // shows its focused highlight while an overlay sits over them.
        self.panels
            .render(f, rows[1], theme, !self.overlays.is_open());

        // Footer reflects the overlay if one is open, otherwise the focused panel.
        let (focus_label, hints) = if let Some(kind) = self.overlays.active_kind() {
            (kind.label(), self.overlays.hint_shortcuts())
        } else {
            (self.panels.focused_label(), self.panels.focused_hints())
        };
        self.footer
            .render(f, rows[2], theme, focus_label, &hints, &self.icons);

        // Overlay — rendered last so it appears on top of everything.
        self.overlays.render(f, f.area(), &self.theme);
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) {
        // Route validation / async-result messages to the active overlay first,
        // so an open dialog still receives its events. Show*/CloseOverlay are
        // NotConsumed by overlays and fall through to the owned match below.
        match self.overlays.handle_app_message(&msg, &self.vault, tx) {
            OverlayMsg::Consumed => return,
            OverlayMsg::NotConsumed => {}
        }

        match msg {
            AppEvent::ShowFileOpsMenu(path) => {
                self.present_overlay(Box::new(ActiveDialog::file_ops_menu(path)));
            }
            AppEvent::ShowDeleteDialog(path) => {
                self.present_overlay(Box::new(ActiveDialog::delete(path, self.vault.clone())));
            }
            AppEvent::ShowRenameDialog(path) => {
                self.present_overlay(Box::new(ActiveDialog::rename(path, self.vault.clone())));
            }
            AppEvent::ShowMoveDialog(path) => {
                self.present_overlay(Box::new(ActiveDialog::move_to(
                    path,
                    self.vault.clone(),
                    tx,
                )));
            }
            AppEvent::CloseOverlay => {
                // Dismiss-to-opener. Guarded by is_open() on purpose: a
                // selection that wants a specific post-close focus
                // (OpenPath -> editor, SavedSearchSelected -> Query panel)
                // closes the overlay itself first, so a later/!dialog
                // CloseOverlay must not re-restore and clobber that focus.
                self.dismiss_overlay();
            }
            AppEvent::SortChanged {
                target,
                field,
                order,
                group_directories,
                persist,
            } => {
                match target {
                    SortTarget::Sidebar if persist => {
                        // Update the sidebar's in-session per-context default AND
                        // apply live. `is_current_journal()` is the single source
                        // of truth for which context this save targets — reused
                        // for the on-disk settings write below.
                        let is_journal = self.panels.sidebar().is_current_journal();
                        self.panels
                            .sidebar_mut()
                            .save_default(field, order, group_directories);
                        {
                            let mut s = self.settings.write().unwrap();
                            if is_journal {
                                s.journal_sort_field =
                                    crate::settings::SortFieldSetting::from(field);
                                s.journal_sort_order =
                                    crate::settings::SortOrderSetting::from(order);
                            } else {
                                s.default_sort_field =
                                    crate::settings::SortFieldSetting::from(field);
                                s.default_sort_order =
                                    crate::settings::SortOrderSetting::from(order);
                            }
                            s.group_directories = group_directories;
                        }
                        let snapshot = self.settings.read().unwrap().clone();
                        tokio::spawn(async move {
                            snapshot.save_to_disk().ok();
                        });
                    }
                    SortTarget::Sidebar => {
                        self.panels
                            .sidebar_mut()
                            .apply_sort(field, order, group_directories)
                    }
                    // The query panel has no persisted default (the order lives
                    // in the query string); `persist` is always false here.
                    SortTarget::Query => self.panels.query_mut().apply_sort(field, order, tx),
                }
            }
            AppEvent::Autosave => {
                self.spawn_autosave(tx);
            }
            AppEvent::AutosaveCompleted {
                path,
                saved_revision,
            } => {
                if path == self.path
                    && let Some(rev) = saved_revision
                {
                    self.panels.editor_mut().mark_saved_at_revision(rev);
                }
                // `SingleSlotTask::is_in_flight()` flips to false the
                // moment the spawned future returns (success or panic),
                // so we don't have to clear the slot manually here —
                // the next `spawn_autosave` tick will overwrite it.
                // Skip explicit cleanup; was previously racy because a
                // stale completion arriving after `try_save` had
                // already cleared and respawned could wipe the fresh
                // handle.
            }
            AppEvent::FocusEditor => {
                self.focus_editor();
            }
            AppEvent::FocusSidebar => {
                self.focus_sidebar();
            }
            AppEvent::OpenJournal => {
                // Dismiss any open overlay so the journal note isn't loaded
                // behind it (mirrors OpenPath / EntryCreated).
                self.dismiss_overlay();
                if let Ok((details, _)) = self.vault.journal_entry().await {
                    let path = details.path;
                    self.open_path(path.clone(), tx).await;
                    let note_parent = path.get_parent_path().0;
                    if note_parent.is_like(self.panels.sidebar().current_dir()) {
                        let dir = self.panels.sidebar().current_dir().clone();
                        self.navigate_sidebar(dir, tx).await;
                    }
                }
            }
            AppEvent::SavedSearchSelected { query, name } => {
                // Deliberate non-restoring close: the selection lands in the
                // Query panel (set by apply_saved_search), not back on the
                // overlay's opener — so close the overlay without dismiss_overlay.
                self.overlays.close();
                self.apply_saved_search(query, name, tx);
            }
            AppEvent::FollowLink(target) => {
                self.follow_link(target, tx).await;
            }
            AppEvent::FollowLabel(name) => {
                let initial = format!("#{name}");
                let s = self.settings.read().unwrap();
                let provider = SearchNotesProvider::new(
                    self.vault.clone(),
                    s.current_last_paths(),
                    Some(self.path.clone()),
                );
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
                self.present_overlay(Box::new(modal));
            }
            AppEvent::EntryCreated(path) => {
                self.dismiss_overlay();
                self.open_path(path.clone(), tx).await;
                self.focus_editor();
                let note_parent = path.get_parent_path().0;
                if note_parent.is_like(self.panels.sidebar().current_dir()) {
                    let dir = self.panels.sidebar().current_dir().clone();
                    self.navigate_sidebar(dir, tx).await;
                }
            }
            AppEvent::EntryDeleted(path) => {
                self.on_entry_op(path, tx).await;
            }
            AppEvent::EntryRenamed { from, .. } => {
                self.on_entry_op(from, tx).await;
            }
            AppEvent::EntryMoved { from, .. } => {
                self.on_entry_op(from, tx).await;
            }
            AppEvent::SaveSearchConfirmed { name, query } => {
                let vault = self.vault.clone();
                tokio::spawn(async move {
                    if let Err(e) = vault.save_search(&name, &query).await {
                        tracing::warn!("failed to save search '{}': {}", name, e);
                    }
                });
            }
            AppEvent::InsertAtCursor(text) => {
                if self.panels.focused() == PanelKind::Editor {
                    self.panels.editor_mut().insert_at_cursor(&text, tx);
                }
            }
            _ => {}
        }
    }

    /// The editor handles every path itself: notes open in the buffer,
    /// directories navigate the sidebar. Always consumes.
    async fn try_open_path(&mut self, path: VaultPath, tx: &AppTx) -> Option<VaultPath> {
        self.dismiss_overlay();
        if path.is_note() {
            self.open_path(path, tx).await;
            self.focus_editor();
        } else {
            self.navigate_sidebar(path, tx).await;
        }
        None
    }

    async fn on_exit(&mut self, _tx: &AppTx) {
        self.try_save().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time test: `PanelKind` and `OverlayKind` are usable here.
    #[test]
    fn panel_kind_labels_and_overlay_kind_compile() {
        assert_eq!(PanelKind::Editor.label(), "EDITOR");
        assert_eq!(PanelKind::Sidebar.label(), "SIDEBAR");
        assert_eq!(PanelKind::Query.label(), "BACKLINKS");
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
        assert!(screen.panels.is_visible(PanelKind::Query));
        assert_eq!(screen.panels.query().active_query(), "<{note}");
        assert_eq!(screen.panels.focused(), PanelKind::Query);
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
        screen.handle_app_message(AppEvent::CloseOverlay, &tx).await;

        assert!(
            screen.panels.focused() == PanelKind::Query,
            "focus should remain on the Query panel after select + close"
        );
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
            screen.overlays.open(Box::new(modal), screen.opener_focus());
        }
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

    /// Focus actions are inert while an overlay owns input: pressing
    /// FocusEditor (Ctrl+L) with a dialog open must not reveal or move the
    /// panels underneath, so closing the dialog leaves the layout unchanged.
    #[tokio::test]
    async fn focus_action_is_noop_while_overlay_open() {
        use crate::settings::AppSettings;
        use kimun_core::VaultConfig;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use std::sync::RwLock;

        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(NoteVault::new(VaultConfig::new(dir.path())).await.unwrap());
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        let mut screen = EditorScreen::new(vault.clone(), VaultPath::root(), settings.clone());

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Open a SavedSearches overlay (Query panel starts hidden).
        {
            let s = settings.read().unwrap();
            let modal = SavedSearchesModal::new(
                vault.clone(),
                s.key_bindings.clone(),
                s.icons(),
                tx.clone(),
            );
            drop(s);
            screen.overlays.open(Box::new(modal), screen.opener_focus());
        }
        assert!(!screen.panels.is_visible(PanelKind::Query));

        // Ctrl+L (FocusEditor / focus right) must be consumed but do nothing.
        let focus_right = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        {
            let s = settings.read().unwrap();
            let combo = key_event_to_combo(&focus_right).expect("Ctrl+L maps to a combo");
            assert_eq!(
                s.key_bindings.get_action(&combo),
                Some(ActionShortcuts::FocusEditor),
                "test assumes Ctrl+L is bound to FocusEditor"
            );
        }
        screen.handle_input(&InputEvent::Key(focus_right), &tx);

        assert!(
            !screen.panels.is_visible(PanelKind::Query),
            "focus action must not reveal a panel while an overlay is open"
        );
        assert_eq!(
            screen.overlays.active_kind(),
            Some(OverlayKind::SavedSearches),
            "overlay stays active"
        );
    }

    /// Opening the journal while an overlay is up dismisses the overlay, so the
    /// journal note isn't loaded behind it (regression for the OpenJournal arm,
    /// which used to skip the restore that OpenPath/EntryCreated do).
    #[tokio::test(flavor = "multi_thread")]
    async fn open_journal_dismisses_open_overlay() {
        let vault = crate::test_support::temp_vault("editor-journal").await;
        vault.validate_and_init().await.unwrap();
        let settings = std::sync::Arc::new(std::sync::RwLock::new(
            crate::settings::AppSettings::default(),
        ));
        let mut screen = EditorScreen::new(vault.clone(), VaultPath::root(), settings.clone());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        {
            let s = settings.read().unwrap();
            let modal = SavedSearchesModal::new(
                vault.clone(),
                s.key_bindings.clone(),
                s.icons(),
                tx.clone(),
            );
            drop(s);
            screen.present_overlay(Box::new(modal));
        }
        assert!(screen.overlays.is_open(), "precondition: overlay open");

        screen.handle_app_message(AppEvent::OpenJournal, &tx).await;

        assert!(
            !screen.overlays.is_open(),
            "OpenJournal must dismiss the overlay before loading the note"
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
            let provider = SearchNotesProvider::new(vault.clone(), s.current_last_paths(), None);
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

#[cfg(test)]
mod sort_routing_tests {
    use super::*;
    use crate::app_screen::AppScreen;
    use crate::components::events::SortTarget;
    use crate::components::file_list::{SortField, SortOrder};

    async fn make_editor() -> (
        EditorScreen,
        AppTx,
        tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    ) {
        let vault = crate::test_support::temp_vault("editor-sort").await;
        vault.validate_and_init().await.unwrap();
        let settings = std::sync::Arc::new(std::sync::RwLock::new(
            crate::settings::AppSettings::default(),
        ));
        let screen = EditorScreen::new(vault, VaultPath::root(), settings);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (screen, tx, rx)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sort_save_default_persists_to_settings() {
        let (mut screen, tx, _rx) = make_editor().await;
        screen
            .handle_app_message(
                AppEvent::SortChanged {
                    target: SortTarget::Sidebar,
                    field: SortField::Title,
                    order: SortOrder::Descending,
                    group_directories: true,
                    persist: true,
                },
                &tx,
            )
            .await;
        let s = screen.settings.read().unwrap();
        assert_eq!(
            s.default_sort_field,
            crate::settings::SortFieldSetting::Title
        );
        assert_eq!(
            s.default_sort_order,
            crate::settings::SortOrderSetting::Descending
        );
        assert!(s.group_directories);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sort_save_default_journal_dir_writes_journal_settings() {
        let (mut screen, tx, _rx) = make_editor().await;
        // Point the sidebar at the journal directory (current_dir set synchronously).
        let journal = screen.vault.journal_path().clone();
        screen.panels.sidebar_mut().navigate(journal, &tx);

        screen
            .handle_app_message(
                AppEvent::SortChanged {
                    target: SortTarget::Sidebar,
                    field: SortField::Title,
                    order: SortOrder::Ascending,
                    group_directories: false,
                    persist: true,
                },
                &tx,
            )
            .await;

        let s = screen.settings.read().unwrap();
        assert_eq!(
            s.journal_sort_field,
            crate::settings::SortFieldSetting::Title
        );
        assert_eq!(
            s.journal_sort_order,
            crate::settings::SortOrderSetting::Ascending
        );
        // Default (non-journal) settings must be untouched from their defaults.
        assert_eq!(
            s.default_sort_field,
            crate::settings::SortFieldSetting::Name
        );
    }

    /// A non-persisting SortChanged (a plain dialog toggle) applies live but
    /// must NOT write settings.
    #[tokio::test(flavor = "multi_thread")]
    async fn sort_changed_without_persist_leaves_settings() {
        let (mut screen, tx, _rx) = make_editor().await;
        screen
            .handle_app_message(
                AppEvent::SortChanged {
                    target: SortTarget::Sidebar,
                    field: SortField::Title,
                    order: SortOrder::Descending,
                    group_directories: true,
                    persist: false,
                },
                &tx,
            )
            .await;
        let s = screen.settings.read().unwrap();
        assert_eq!(
            s.default_sort_field,
            crate::settings::SortFieldSetting::Name
        );
        assert_eq!(
            s.default_sort_order,
            crate::settings::SortOrderSetting::Ascending
        );
        assert!(!s.group_directories);
    }
}
