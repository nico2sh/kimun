use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::error::{FSError, VaultError};
use kimun_core::nfs::VaultPath;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

use crate::app_screen::overlay_host::OverlayHost;
use crate::app_screen::panel_set::PanelSet;
use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::attachment_view::AttachmentView;
use crate::components::autosave_timer::AutosaveTimer;
use crate::components::dialogs::ActiveDialog;
use crate::components::drawer::{DrawerHost, DrawerView};
use crate::components::drawer_views::{LinksPanel, OutlinePanel, TagsPanel};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, SaveSource, ScreenEvent, SortTarget};
use crate::components::file_list::FileListEntry;
use crate::components::footer_bar::FooterBar;
use crate::components::note_browser::file_finder_provider::FileFinderProvider;
use crate::components::note_browser::search_provider::resolving_search_source;
use crate::components::note_browser::{BrowserScope, NoteBrowserModal};
use crate::components::overlay::{Overlay, OverlayKind, OverlayMsg};
use crate::components::panel::PanelKind;
use crate::components::query_panel::QueryPanel;
use crate::components::saved_searches_modal::SavedSearchesModal;
use crate::components::sidebar::SidebarComponent;
use crate::components::text_editor::TextEditorComponent;
use crate::keys::action_shortcuts::{ActionShortcuts, TextAction};
use crate::keys::key_event_to_combo;
use crate::keys::key_strike::KeyStrike;
use crate::keys::leader::{LeaderAction, LeaderEngine, LeaderOutcome};
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
    /// Async document/status state: backlink count, git summary, link
    /// affordance cache, pending emphasis needles (see `doc_meta.rs`).
    doc_meta: crate::app_screen::doc_meta::DocMeta,
    /// App-global update notice, seeded by `AppEvent::UpdateAvailable`. Drives
    /// the footer indicator; `None` when up to date or the check found nothing.
    update: Option<crate::update::UpdateStatus>,
    /// The leader-key sequence state machine (Ctrl-G gateway, spec §8a).
    leader: LeaderEngine,
    /// App event sender, captured on enter — render-side async kicks (the
    /// link-affordance backlink fetch) need it where no `tx` is threaded.
    app_tx: Option<AppTx>,
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
        let footer = FooterBar::new();
        let icons = s.icons();
        let sidebar = SidebarComponent::from_settings(vault.clone(), &s);
        let query_panel = QueryPanel::new(vault.clone(), kb.clone(), icons.clone());
        let tags = TagsPanel::new(vault.clone(), s.icons());
        let links = LinksPanel::new(vault.clone(), s.icons());
        let outline = OutlinePanel::new(vault.clone(), s.icons());
        let drawer = DrawerHost::new(sidebar, query_panel, tags, links, outline);
        let rail_kb = kb.clone();
        let mut editor = TextEditorComponent::new(kb, &s);
        editor.set_vault(vault.clone());
        let leader_engine = LeaderEngine::with_tree(s.leader_tree());
        drop(s);
        let rail_icons = icons.clone();
        Self {
            settings,
            icons,
            theme,
            panels: PanelSet::from_panels(drawer, editor, rail_icons, rail_kb),
            doc_meta: crate::app_screen::doc_meta::DocMeta::new(vault.clone()),
            update: None,
            vault,
            path,
            footer,
            leader: leader_engine,
            app_tx: None,
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
        let Some(editor) = self.panels.editor_mut() else {
            return false;
        };
        let img = match editor.take_clipboard_image() {
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
        if kimun_core::note::scan::is_remote_url(&target) {
            match open::that_detached(&target) {
                Ok(()) => self.footer.flash(format!("Opening {target}"), tx),
                Err(e) => self.footer.flash(format!("Cannot open URL: {e}"), tx),
            }
            return;
        }

        // Image attachment — resolve the (potentially relative) path against
        // the current note's directory, convert to an OS path, hand off to the
        // OS default handler. Images are not notes, so skip the note lookup.
        if kimun_core::note::scan::target_looks_like_image(&target) {
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
        // Resolve the (possibly relative, e.g. `../work/anton.md`) target
        // against this note's directory so the existence lookup uses the same
        // absolute path the note is stored under. Bare names stay name-lookups.
        let path = kimun_core::nfs::VaultPath::note_path_from(target_clean)
            .resolve_link_in_note(&self.path);
        match self.vault.open_or_search(&path).await {
            Ok(results) if results.is_empty() => {
                self.present_overlay(Box::new(ActiveDialog::create_note(
                    path,
                    self.vault.clone(),
                )));
            }
            Ok(mut results) if results.len() == 1 => {
                let (entry, _) = results.remove(0);
                self.open_path(entry.path, None, tx).await;
            }
            Ok(results) => {
                use crate::components::note_browser::link_results_provider::LinkResultsProvider;
                let provider = LinkResultsProvider::from_results(results);
                let s = self.settings.read().unwrap();
                let modal = NoteBrowserModal::new(
                    format!("Follow: {target}"),
                    BrowserScope::Files,
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

    pub async fn open_path(&mut self, path: VaultPath, emphasis: Option<Vec<String>>, tx: &AppTx) {
        if !path.is_note() {
            // A non-note path is either an attachment (show it in place of the
            // editor) or a directory (browse it). Classify so a stray
            // attachment open here still lands in the attachment view rather
            // than the directory browser (ADR-0017).
            if let Ok(kimun_core::EntryKind::Attachment) = self.vault.entry_kind(&path).await {
                self.open_attachment(path, tx).await;
                return;
            }
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
        // Returning to a note swaps the editor area back from any attachment.
        self.panels.clear_attachment();
        // Mark this note's row in the sidebar (clears the previous one).
        self.panels
            .sidebar_mut()
            .set_open_note(Some(self.path.clone()));
        match self.vault.get_note_text(&self.path).await {
            Ok(content) => {
                self.doc_meta.note_opened(&self.path, tx);
                if let Some(ed) = self.panels.editor_mut() {
                    ed.set_text(content);
                    // Arrive-from-query emphasis: apply after the load so the
                    // buffer's new revision owns the needles.
                    if let Some(needles) = emphasis {
                        ed.set_search_needles(needles);
                    }
                    ed.set_redraw_tx(tx);
                }
                tx.send(AppEvent::Redraw).ok();
                // FIND / LINKS / OUTLINE reflect the open note; keep them in
                // step. Shared with `on_note_renamed` via the helper.
                self.reflect_open_note_in_drawers(tx);
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
            self.navigate_sidebar(note_parent, tx);
        }

        // Abort any existing timer and spawn a fresh one for the new note.
        let interval = self.settings.read().unwrap().autosave_interval_secs;
        self.autosave.restart(interval, tx.clone());
    }

    /// Show the attachment at `path` in the editor area's read-only attachment
    /// view (ADR-0017). Saves and unmounts the open note first; while an
    /// attachment is shown there is no open note, so the autosave task is
    /// aborted and the sidebar's open-note marker cleared. The next periodic
    /// autosave tick no-ops because the note editor is absent.
    async fn open_attachment(&mut self, path: VaultPath, tx: &AppTx) {
        // Persist the current note before swapping the editor area away from it.
        self.try_save().await;
        self.autosave_task.abort();

        match self.vault.get_attachment_details(&path).await {
            Ok(details) => {
                let (icons, kb) = {
                    let s = self.settings.read().unwrap();
                    (s.icons(), s.key_bindings.clone())
                };
                let view = AttachmentView::new(details, icons, kb);
                self.path = path;
                self.panels.show_attachment(view);
                self.panels.sidebar_mut().set_open_note(None);
                tx.send(AppEvent::Redraw).ok();
            }
            Err(e) => {
                self.footer
                    .flash(format!("Cannot open attachment: {e}"), tx);
            }
        }
    }

    fn navigate_sidebar(&mut self, dir: VaultPath, tx: &AppTx) {
        // The sidebar hosts a streamed `SearchList`; (re)building its engine for
        // `dir` runs `browse_vault` inside the source and emits rows as they
        // arrive (with a redraw on each).
        self.panels.sidebar_mut().navigate(dir, tx);
    }

    /// Rebuild the sidebar listing only when it is currently showing `dir` —
    /// used after entry create/rename/move ops so the change appears without
    /// yanking the user away from an unrelated directory they browsed to.
    /// Deliberately the inverse of `reveal_note_dir_in_sidebar`, which
    /// navigates when the sidebar is NOT on the target dir.
    fn refresh_sidebar_if_showing(&mut self, dir: &VaultPath, tx: &AppTx) {
        self.panels.sidebar_mut().refresh_if_showing(dir, tx);
    }

    /// A note at `path` was just saved with raw title `raw_title`; update its
    /// sidebar row in place. Keyed by the saved path, not the open note, so a
    /// just-saved-then-deselected note's row updates too.
    fn note_saved(&mut self, path: &VaultPath, raw_title: String) {
        let title = FileListEntry::display_title(raw_title);
        self.panels.sidebar_mut().update_note_row(path, &title);
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
        // No note editor mounted (an attachment is shown) → nothing to save.
        let Some(text) = self
            .panels
            .editor()
            .filter(|e| e.is_dirty())
            .map(|e| e.get_text())
        else {
            return;
        };
        // Same cap on our own save so quit cannot hang on a stuck
        // disk. A timeout returns Err(_); we skip mark_saved so the
        // editor stays dirty for any subsequent retry.
        let save = self.vault.save_note(&self.path, &text);
        if let Ok(Ok((_, content))) = tokio::time::timeout(SAVE_TIMEOUT, save).await {
            if let Some(ed) = self.panels.editor_mut() {
                ed.mark_saved(text);
            }
            let path = self.path.clone();
            self.note_saved(&path, content.title);
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
        let Some(ed) = self.panels.editor() else {
            return;
        };
        if !ed.is_dirty() {
            return;
        }
        let text = ed.get_text();
        let revision = ed.content_revision();
        let vault = self.vault.clone();
        let path = self.path.clone();
        let tx = tx.clone();
        self.autosave_task.spawn(async move {
            let (saved_revision, title) = match vault.save_note(&path, &text).await {
                Ok((_, content)) => (Some(revision), Some(content.title)),
                Err(_) => (None, None),
            };
            let _ = tx.send(AppEvent::AutosaveCompleted {
                path,
                saved_revision,
                title,
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
        // An overlay taking input must never leave a leader sequence armed —
        // its keys would be eaten by the leader intercept.
        self.leader.cancel();
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
        } else {
            self.refresh_sidebar_if_showing(&from.get_parent_path().0, tx);
        }
    }

    /// A note was renamed. Update its sidebar row in place; if it is the note
    /// currently open, retarget the editor to the new path and reload the body
    /// from disk so any self-link rewrites from the rename land in the buffer
    /// (the in-memory text still holds the pre-rename self-links). We
    /// deliberately do NOT `try_save` — the old path no longer exists on disk.
    async fn on_note_renamed(&mut self, from: VaultPath, to: VaultPath, tx: &AppTx) {
        self.dismiss_overlay();
        self.panels.sidebar_mut().rename_note_row(&from, &to);
        if from == self.path {
            // The open note was renamed. Kill any in-flight autosave still
            // targeting the OLD path before retargeting (spawn_autosave bakes
            // the path in; vault.save_note writes unconditionally, so a stale
            // save would recreate the renamed-away file). abort() is
            // best-effort (can't unwind a syscall already in progress).
            self.autosave_task.abort();
            match self.vault.get_note_text(&to).await {
                Ok(text) => {
                    self.path = to.clone();
                    if let Some(ed) = self.panels.editor_mut() {
                        ed.set_text(text.clone());
                        ed.mark_saved(text);
                    }
                    self.panels
                        .sidebar_mut()
                        .set_open_note(Some(self.path.clone()));
                    self.doc_meta.note_opened(&self.path, tx);
                    self.reflect_open_note_in_drawers(tx);
                    // Fresh autosave timer for the new path (mirrors open_path).
                    let interval = self.settings.read().unwrap().autosave_interval_secs;
                    self.autosave.restart(interval, tx.clone());
                }
                Err(_) => {
                    // Couldn't load the renamed note — do NOT keep a dirty
                    // buffer pointed at it (autosave would clobber the
                    // on-disk rewrite). Fall back to Browse on the new
                    // parent dir.
                    self.autosave.stop();
                    let parent = to.get_parent_path().0;
                    tx.send(AppEvent::OpenScreen(ScreenEvent::OpenBrowse(
                        self.vault.clone(),
                        parent,
                    )))
                    .ok();
                }
            }
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

    /// Whether the drawer is open showing `view`.
    fn drawer_open_on(&self, view: DrawerView) -> bool {
        self.panels.is_visible(PanelKind::Drawer) && self.panels.active_drawer_view() == view
    }

    /// Reflect the currently-open note (`self.path`) into whichever drawer view
    /// is visible (FIND/LINKS/OUTLINE). Shared by `open_path` and
    /// `on_note_renamed` so a rename keeps the drawers in step, not just a
    /// fresh open.
    fn reflect_open_note_in_drawers(&mut self, tx: &AppTx) {
        let path = self.path.clone();
        if self.drawer_open_on(DrawerView::Find) {
            self.panels.query_mut().set_note(path.clone(), tx.clone());
        }
        if self.panels.is_visible(PanelKind::Drawer) {
            match self.panels.active_drawer_view() {
                DrawerView::Links => self.panels.links_mut().set_note(path.clone(), tx),
                DrawerView::Outline => self.panels.outline_mut().set_note(path, tx),
                _ => {}
            }
        }
    }

    /// Point the sidebar at the current note's directory. Skips the engine
    /// rebuild when the sidebar is already there so an in-progress filter
    /// and selection survive a re-open.
    fn reveal_note_dir_in_sidebar(&mut self, tx: &AppTx) {
        let note_parent = self.path.get_parent_path().0;
        if self.panels.sidebar().is_empty()
            || !note_parent.is_like(self.panels.sidebar().current_dir())
        {
            self.navigate_sidebar(note_parent, tx);
        }
    }

    /// Focus the drawer, revealing it on FILES if hidden — but never clobber
    /// the view the user already has open (e.g. a FIND query in progress).
    /// Sent by the nvim backend's leave-editor motions.
    pub fn focus_sidebar(&mut self, tx: &AppTx) {
        if !self.panels.is_visible(PanelKind::Drawer) {
            // Routed through the host opener so the FILES view reveals the
            // current note's directory like any other drawer open (it also
            // focuses the drawer).
            self.open_drawer_view(DrawerView::Files, tx);
        } else {
            self.panels.focus(PanelKind::Drawer);
        }
    }

    /// Switch the drawer to `view`, reveal it, and focus it. The per-view
    /// reveal side effects live in `drawer_view_revealed` (the heavy work
    /// that keeps the reveal in the host rather than in `PanelSet`).
    fn open_drawer_view(&mut self, view: DrawerView, tx: &AppTx) {
        let newly_shown = !self.drawer_open_on(view);
        self.panels.open_drawer_view(view);
        self.drawer_view_revealed(view, newly_shown, tx);
        self.panels.focus(PanelKind::Drawer);
    }

    /// Per-view side effects to run when `view` becomes visible in the
    /// drawer — the single table shared by every reveal path (rail/leader
    /// opens, the Ctrl-T toggle restore) so a view cannot go stale on one
    /// path and refresh on another. `newly_shown` gates the effects that
    /// must not clobber state the user already has on screen (an
    /// in-progress FIND query, a browsed sidebar directory); the rest
    /// refresh unconditionally. Never touches focus — callers decide that.
    fn drawer_view_revealed(&mut self, view: DrawerView, newly_shown: bool, tx: &AppTx) {
        match view {
            DrawerView::Find if newly_shown => {
                self.panels
                    .query_mut()
                    .set_note(self.path.clone(), tx.clone());
            }
            DrawerView::Files if newly_shown => {
                self.reveal_note_dir_in_sidebar(tx);
            }
            DrawerView::Config => {
                let info = {
                    let s = self.settings.read().unwrap();
                    let key_of = |a: &ActionShortcuts| {
                        s.key_bindings
                            .first_combo_for(a)
                            .unwrap_or_else(|| "unbound".to_string())
                    };
                    crate::components::drawer::ConfigInfo {
                        theme_name: s.get_theme().name,
                        leader_key: key_of(&ActionShortcuts::Leader),
                        preferences_key: key_of(&ActionShortcuts::OpenPreferences),
                        leader_timeout_ms: s.leader_timeout_ms,
                        config_path: s
                            .config_file
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "default location".to_string()),
                    }
                };
                self.panels.drawer_set_config_info(info);
            }
            DrawerView::Tags => self.panels.tags_mut().refresh(tx),
            DrawerView::Links => self.panels.links_mut().set_note(self.path.clone(), tx),
            DrawerView::Outline => self.panels.outline_mut().set_note(self.path.clone(), tx),
            _ => {}
        }
    }

    /// Move focus one visible panel left, wrapping at the end.
    fn focus_left(&mut self, _tx: &AppTx) {
        if let Some(kind) = self.panels.prev_kind() {
            self.panels.focus(kind);
        }
    }

    /// Move focus one visible panel right, wrapping at the end.
    fn focus_right(&mut self, _tx: &AppTx) {
        if let Some(kind) = self.panels.next_kind() {
            self.panels.focus(kind);
        }
    }

    /// Toggle the drawer (Ctrl-T): hiding it gives the full remaining width
    /// to the editor; showing it restores the last view. Restoring is a
    /// fresh reveal: the restored view re-targets/refreshes via the shared
    /// `drawer_view_revealed` table — the note (or its data) may have
    /// changed while hidden. Unlike an explicit open, toggling never moves
    /// focus into the drawer.
    fn toggle_drawer(&mut self, tx: &AppTx) {
        if self.panels.is_visible(PanelKind::Drawer) {
            self.panels.hide(PanelKind::Drawer);
        } else {
            self.panels.show(PanelKind::Drawer);
            self.drawer_view_revealed(self.panels.active_drawer_view(), true, tx);
        }
    }

    /// The query the save-current-query action would save, with its
    /// saved-search provenance (breadcrumb name) for the dialog's name
    /// pre-fill. Sourced from the active note browser if one is open (Ctrl+K
    /// modal), otherwise from the Query panel. `None` when there is nothing
    /// to save: a blank query, or another overlay is open.
    fn save_query_source(&self) -> Option<(String, Option<String>, SaveSource)> {
        let (query, provenance, source) = match self.overlays.active_kind() {
            Some(OverlayKind::NoteBrowser) => (
                self.overlays.active_query().unwrap_or_default().to_string(),
                self.overlays
                    .active_saved_search_provenance()
                    .map(str::to_string),
                SaveSource::NoteBrowser,
            ),
            None => (
                self.panels.query().active_query().to_string(),
                self.panels.query().saved_search_name().map(str::to_string),
                SaveSource::QueryPanel,
            ),
            Some(_) => return None,
        };
        if query.trim().is_empty() {
            None
        } else {
            Some((query, provenance, source))
        }
    }

    /// Schedule a redraw for when the which-key overlay should reveal: the
    /// hesitation timeout after the sequence (re)advanced. Fluent typing
    /// never sees the overlay; the timer redraw simply finds the sequence
    /// already gone.
    fn schedule_whichkey_reveal(&self, tx: &AppTx) {
        let timeout = {
            let s = self.settings.read().unwrap();
            std::time::Duration::from_millis(s.leader_timeout_ms)
        };
        let tx2 = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(timeout + std::time::Duration::from_millis(10)).await;
            let _ = tx2.send(AppEvent::Redraw);
        });
    }

    /// Open the workspace switcher (SwitchWorkspace action and leader `v s`).
    /// No-op while an overlay is open.
    fn open_workspace_switcher(&mut self) {
        if self.overlays.is_open() {
            return;
        }
        let s = self.settings.read().unwrap();
        let dialog = ActiveDialog::workspace_switcher(&s);
        drop(s);
        self.present_overlay(Box::new(dialog));
    }

    /// Open the command palette (Ctrl+Shift+P and leader `p`). No-op while
    /// an overlay is open.
    fn open_command_palette(&mut self, tx: &AppTx) {
        if self.overlays.is_open() {
            return;
        }
        let (gateway, icons) = {
            let s = self.settings.read().unwrap();
            (
                s.key_bindings
                    .first_combo_for(&ActionShortcuts::Leader)
                    .unwrap_or_else(|| "leader".to_string()),
                s.icons(),
            )
        };
        let tree = {
            let s = self.settings.read().unwrap();
            s.leader_tree()
        };
        let modal = crate::components::command_palette::CommandPaletteModal::new(
            &tree,
            &gateway,
            icons,
            tx.clone(),
        );
        self.present_overlay(Box::new(modal));
    }

    /// Open the Saved Searches modal (F3 and leader `f s`). No-op while an
    /// overlay is open.
    fn open_saved_searches(&mut self, tx: &AppTx) {
        if self.overlays.is_open() {
            return;
        }
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

    /// Open the live theme picker (leader `v c` and the CFG rail item). The
    /// full settings screen stays on the OpenSettings binding. No-op while an
    /// overlay is open.
    fn open_theme_picker(&mut self) {
        if self.overlays.is_open() {
            return;
        }
        let s = self.settings.read().unwrap();
        let dialog = ActiveDialog::theme_picker(&s);
        drop(s);
        self.present_overlay(Box::new(dialog));
    }

    /// Open the flat key-bindings help (F1). No-op while an overlay is open.
    fn open_help(&mut self) {
        if self.overlays.is_open() {
            return;
        }
        let s = self.settings.read().unwrap();
        let dialog = ActiveDialog::help(&s.key_bindings);
        drop(s);
        self.present_overlay(Box::new(dialog));
    }

    /// True when the Find drawer view (the query panel) holds focus — the
    /// context in which F1 surfaces query syntax help instead of the flat
    /// key-bindings panel.
    fn find_panel_focused(&self) -> bool {
        self.panels.focused() == PanelKind::Drawer
            && self.panels.active_drawer_view() == DrawerView::Find
    }

    /// Open the search query syntax reference (F1 while the Find panel is
    /// focused). No-op while an overlay is open.
    fn open_query_help(&mut self) {
        if self.overlays.is_open() {
            return;
        }
        self.present_overlay(Box::new(ActiveDialog::query_syntax()));
    }

    /// Open the full leader-tree cheatsheet (leader `?`). No-op while an
    /// overlay is open.
    fn open_cheatsheet(&mut self) {
        if self.overlays.is_open() {
            return;
        }
        let s = self.settings.read().unwrap();
        let dialog = ActiveDialog::cheatsheet(&s);
        drop(s);
        self.present_overlay(Box::new(dialog));
    }

    /// Open the note-browser modal over the full-text search provider
    /// (Ctrl-K and the leader's find paths). No-op while an overlay is open.
    fn open_search_browser(&mut self, tx: &AppTx) {
        if self.overlays.is_open() {
            return;
        }
        let s = self.settings.read().unwrap();
        let provider = resolving_search_source(
            self.vault.clone(),
            s.current_last_paths(),
            Some(self.path.clone()),
        );
        let modal = NoteBrowserModal::new(
            "Note Browser",
            BrowserScope::Query,
            provider,
            self.vault.clone(),
            s.key_bindings.clone(),
            s.icons(),
            tx.clone(),
        );
        drop(s);
        self.present_overlay(Box::new(modal));
    }

    /// Open the note-browser modal over the fuzzy file finder (Ctrl-O and
    /// the leader's `f f`). No-op while an overlay is open.
    fn open_file_finder(&mut self, tx: &AppTx) {
        if self.overlays.is_open() {
            return;
        }
        let current_dir = self.path.get_parent_path().0;
        let provider = FileFinderProvider::new(self.vault.clone(), current_dir);
        let s = self.settings.read().unwrap();
        let modal = NoteBrowserModal::new(
            "Find Note",
            BrowserScope::Files,
            provider,
            self.vault.clone(),
            s.key_bindings.clone(),
            s.icons(),
            tx.clone(),
        );
        drop(s);
        self.present_overlay(Box::new(modal));
    }

    /// Follow the wikilink / tag under the editor cursor (FollowLink action,
    /// Ctrl+Enter on kitty-protocol terminals).
    fn follow_link_at_cursor(&mut self, tx: &AppTx) {
        use crate::components::text_editor::LinkTarget;
        // In the attachment view, FollowLink (Ctrl+N) opens the attachment with
        // the OS default program rather than following a link (there is none).
        if self.panels.is_showing_attachment() {
            self.open_attachment_externally(tx);
            return;
        }
        let Some(editor) = self.panels.editor_mut() else {
            return;
        };
        match editor.link_at_cursor() {
            Some(LinkTarget::Note(target)) => {
                tx.send(AppEvent::FollowLink(target)).ok();
            }
            Some(LinkTarget::Label(name)) => {
                tx.send(AppEvent::FollowLabel(name)).ok();
            }
            None => {}
        }
    }

    /// Opens the attachment currently shown in the editor area with the OS
    /// default program (the same handoff `follow_link` uses for image links).
    fn open_attachment_externally(&mut self, tx: &AppTx) {
        let Some(path) = self.panels.attachment_path() else {
            return;
        };
        let os_path = self.vault.path_to_pathbuf(path);
        match open::that_detached(&os_path) {
            Ok(()) => self
                .footer
                .flash(format!("Opening {}", os_path.display()), tx),
            Err(e) => self.footer.flash(format!("Cannot open: {e}"), tx),
        }
    }

    /// One key of a pending leader sequence. Esc cancels (focus returns to
    /// the editor), Backspace steps up, chars walk the tree, a fired leaf
    /// executes. Everything is consumed — a pending sequence owns the
    /// keyboard.
    fn handle_leader_key(
        &mut self,
        key: &ratatui::crossterm::event::KeyEvent,
        tx: &AppTx,
    ) -> EventState {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        match key.code {
            KeyCode::Esc => {
                self.leader.cancel();
                self.focus_editor();
            }
            KeyCode::Backspace => {
                let outcome = self.leader.step_up();
                // Stepping past the root cancels — no reveal to re-arm then.
                if outcome == LeaderOutcome::SteppedUp {
                    self.schedule_whichkey_reveal(tx);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                match self.leader.feed(c) {
                    LeaderOutcome::Fired(action) => self.execute_leader_action(action, tx),
                    LeaderOutcome::Invalid => {
                        // Gentle feedback; the sequence stays pending.
                        self.footer.flash(format!("leader: no entry for '{c}'"), tx);
                        self.schedule_whichkey_reveal(tx);
                    }
                    LeaderOutcome::Descended => self.schedule_whichkey_reveal(tx),
                    _ => {}
                }
            }
            // Other keys (arrows, function keys, …) are swallowed; the
            // sequence stays pending. Ctrl-chords never reach here — the
            // intercept in handle_input cancels and re-dispatches them.
            _ => {}
        }
        tx.send(AppEvent::Redraw).ok();
        EventState::Consumed
    }

    /// Put `text` on the system clipboard, flashing `done` on success.
    fn copy_to_clipboard(&mut self, text: String, done: &str, tx: &AppTx) {
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            Ok(()) => self.footer.flash(done.to_string(), tx),
            Err(e) => self.footer.flash(format!("clipboard: {e}"), tx),
        }
    }

    /// Execute a fired leader leaf. Stubs for surfaces that land in later
    /// phases flash a "coming soon" notice instead of silently doing nothing.
    fn execute_leader_action(&mut self, action: LeaderAction, tx: &AppTx) {
        match action {
            LeaderAction::OpenDrawer(view) => self.open_drawer_view(view, tx),

            LeaderAction::AppCheckUpdates => {
                if let Some(status) = self.update.clone() {
                    self.present_overlay(Box::new(ActiveDialog::update(&status)));
                } else {
                    // No cached notice — run a forced check and report the result.
                    let tx2 = tx.clone();
                    tx.send(AppEvent::FlashMessage("Checking for updates…".into()))
                        .ok();
                    tokio::spawn(async move {
                        let Ok(config_dir) = crate::settings::config_dir() else {
                            return;
                        };
                        match crate::update::check_now(config_dir, true).await {
                            Ok(Some(status)) if status.update_available => {
                                // Manual check: surface the notice AND open the
                                // dialog the user explicitly asked for (shown even
                                // if previously skipped — they asked).
                                tx2.send(AppEvent::UpdateAvailable(status)).ok();
                                tx2.send(AppEvent::ShowUpdateDialog).ok();
                            }
                            Ok(_) => {
                                tx2.send(AppEvent::FlashMessage("kimün is up to date".into()))
                                    .ok();
                            }
                            Err(e) => {
                                tx2.send(AppEvent::FlashMessage(format!(
                                    "Update check failed: {e}"
                                )))
                                .ok();
                            }
                        }
                    });
                }
            }

            // +find — list-style leaves route to today's pickers; the
            // telescope modal takes them over in phase 08.
            LeaderAction::FindFiles => self.open_file_finder(tx),
            LeaderAction::FindGrep => self.open_search_browser(tx),
            LeaderAction::FindTags => self.open_drawer_view(DrawerView::Tags, tx),
            LeaderAction::FindBacklinks => {
                self.open_find_with_query("<{note}".to_string(), None, tx)
            }
            LeaderAction::FindSaved => self.open_saved_searches(tx),
            LeaderAction::FindRecent => self.open_search_browser(tx),
            LeaderAction::FindHeadings => self.open_drawer_view(DrawerView::Outline, tx),

            // +note
            LeaderAction::NoteNew => {
                // The FILES filter doubles as the create field (typing a new
                // name offers "Create: …"); the telescope picker (08) gives
                // this a dedicated door.
                self.open_drawer_view(DrawerView::Files, tx);
                self.footer
                    .flash("type a name — Enter creates".to_string(), tx);
            }
            LeaderAction::NoteDaily => {
                tx.send(AppEvent::OpenJournal).ok();
            }
            LeaderAction::NoteFromTemplate => {
                self.footer.flash("templates — coming soon".to_string(), tx);
            }
            LeaderAction::NoteRename => {
                tx.send(AppEvent::ShowRenameDialog(self.path.clone())).ok();
            }
            LeaderAction::NoteMove => {
                tx.send(AppEvent::ShowMoveDialog(self.path.clone())).ok();
            }
            LeaderAction::NoteDelete => {
                tx.send(AppEvent::ShowDeleteDialog(self.path.clone())).ok();
            }

            // +links
            LeaderAction::LinksTab(tab) => {
                self.open_drawer_view(DrawerView::Links, tx);
                self.panels.links_mut().show_tab(tab, tx);
            }
            LeaderAction::LinksGraph => {
                self.footer
                    .flash("local graph — coming soon".to_string(), tx);
            }

            // +git/sync — status is live; the rest are display-only stubs
            // (spec §12 keeps git interactions out of scope).
            LeaderAction::GitStatus => {
                self.doc_meta.refresh_git(tx);
                let msg = self
                    .doc_meta
                    .git()
                    .cloned()
                    .unwrap_or_else(|| "not a git repository".to_string());
                self.footer.flash(msg, tx);
            }
            LeaderAction::GitSync | LeaderAction::GitLog | LeaderAction::GitDiff => {
                self.footer
                    .flash("git is display-only for now".to_string(), tx);
            }

            // +vault
            LeaderAction::VaultSwitch => self.open_workspace_switcher(),
            LeaderAction::VaultReindex => {
                // Fast reindex right here — the same pipeline the Settings
                // screen runs, result surfaced as a footer flash.
                let vault = self.vault.clone();
                let tx2 = tx.clone();
                self.footer.flash("reindexing…".to_string(), tx);
                tokio::spawn(async move {
                    let started = std::time::Instant::now();
                    let result = vault.index_notes(kimun_core::NotesValidation::Fast).await;
                    let msg = match result {
                        Ok(_) => format!("reindexed in {:.1?}", started.elapsed()),
                        Err(e) => format!("reindex failed: {e}"),
                    };
                    tx2.send(AppEvent::FlashMessage(msg)).ok();
                });
            }
            // `v c` opens the config panel (the CFG drawer); `v t` opens the
            // theme picker directly (also reachable inside CFG via `t`).
            LeaderAction::VaultConfig => self.open_drawer_view(DrawerView::Config, tx),
            LeaderAction::VaultTheme => self.open_theme_picker(),
            LeaderAction::VaultPreferences => {
                tx.send(AppEvent::OpenScreen(ScreenEvent::OpenPreferences))
                    .ok();
            }
            LeaderAction::AppOnboarding => {
                tx.send(AppEvent::OpenScreen(ScreenEvent::OpenOnboarding))
                    .ok();
            }

            // +window
            LeaderAction::WindowZen => {
                self.panels.hide(PanelKind::Drawer);
                self.focus_editor();
            }
            LeaderAction::WindowSplit => {
                self.footer
                    .flash("editor splits — coming soon".to_string(), tx);
            }
            LeaderAction::WindowGrowDrawer => self.panels.adjust_drawer_width(4),
            LeaderAction::WindowShrinkDrawer => self.panels.adjust_drawer_width(-4),

            // +this note
            LeaderAction::NoteToggleTodo => {
                self.footer
                    .flash("toggle todo — coming soon".to_string(), tx);
            }
            LeaderAction::NotePreview => {
                self.footer
                    .flash("preview — lands with phase 09".to_string(), tx);
            }
            LeaderAction::NoteCopyWikilink => {
                let link = format!("[[{}]]", self.path.get_clean_name());
                self.copy_to_clipboard(link, "wikilink copied", tx);
            }
            LeaderAction::NoteExport => {
                self.footer.flash("export — coming soon".to_string(), tx);
            }
            LeaderAction::NoteYankPath => {
                let path = self.path.to_string();
                self.copy_to_clipboard(path, "note path copied", tx);
            }

            LeaderAction::Palette => self.open_command_palette(tx),
            LeaderAction::Help => self.open_cheatsheet(),

            LeaderAction::NoteSave => {
                // Flush the periodic autosave immediately (no manual-save
                // concept; this force-persists the current buffer if dirty).
                self.spawn_autosave(tx);
            }
            LeaderAction::AppQuit => {
                tx.send(AppEvent::Quit).ok();
            }
        }
    }

    /// Kick off the async loads behind status line 2: the open note's
    /// backlink count and the workspace git summary. Results return as
    /// `BacklinkCountLoaded` / `GitStatusLoaded`; stale backlink completions
    /// are dropped by path. Contract: this runs when a note is *opened* (and
    /// the git half on autosave) — counts can go stale while a note stays
    /// open and another note adds a link to it; accepted for now.
    /// The one path that reveals FIND with a concrete query: used by saved
    /// searches and tag queries so they cannot drift on what "open FIND"
    /// means.
    fn open_find_with_query(&mut self, query: String, name: Option<String>, tx: &AppTx) {
        self.panels.open_drawer_view(DrawerView::Find);
        self.panels.query_mut().apply_query(query, name, tx.clone());
        self.panels.focus(PanelKind::Drawer);
    }

    fn apply_saved_search(&mut self, query: String, name: String, tx: &AppTx) {
        // The virtual backlinks entry's name should not override the
        // default "Backlinks" title — but the panel's title logic already
        // shows "Backlinks" whenever the active query is `<{note}`, so it's
        // safe to always pass the name through.
        self.open_find_with_query(query, Some(name), tx);
    }

    fn toggle_backlinks(&mut self, tx: &AppTx) {
        if self.drawer_open_on(DrawerView::Find) {
            self.panels.hide(PanelKind::Drawer);
        } else {
            self.open_drawer_view(DrawerView::Find, tx);
        }
    }
}

#[async_trait(?Send)]
impl AppScreen for EditorScreen {
    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Editor
    }

    async fn on_enter(&mut self, tx: &AppTx) {
        self.app_tx = Some(tx.clone());
        self.open_path(self.path.clone(), None, tx).await;
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        // Bracketed paste (terminal-level) — fired by Cmd+V on macOS and by
        // terminal-paste shortcuts on every platform. Try image first; if the
        // clipboard does not hold an image, fall back to the pasted text. The
        // payload string may be empty (e.g. clipboard contains image only).
        if self.editor_active()
            && let InputEvent::Paste(text) = event
        {
            if !self.try_paste_image(tx)
                && !text.is_empty()
                && let Some(ed) = self.panels.editor_mut()
            {
                ed.paste_text(text, tx);
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
        // Ctrl+Enter follows the link under the cursor on kitty-protocol
        // terminals (legacy terminals can't tell it from Enter; Ctrl+N is the
        // always-works binding).
        if self.editor_active()
            && let InputEvent::Key(key) = event
            && key.code == ratatui::crossterm::event::KeyCode::Enter
            && key
                .modifiers
                .contains(ratatui::crossterm::event::KeyModifiers::CONTROL)
        {
            self.follow_link_at_cursor(tx);
            return EventState::Consumed;
        }

        // A pending leader sequence owns the keyboard ahead of everything
        // else (spec §8a) — every key is consumed until it fires or cancels.
        // Exceptions: an overlay that opened underneath (async paths) wins,
        // and a Ctrl-chord cancels the sequence then dispatches normally, so
        // Ctrl-G restarts the leader and Ctrl-Q still quits mid-sequence.
        if self.leader.is_pending()
            && let InputEvent::Key(key) = event
        {
            if self.overlays.is_open() {
                self.leader.cancel();
            } else if matches!(key.code, ratatui::crossterm::event::KeyCode::Char(_))
                && key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL)
            {
                self.leader.cancel();
                // fall through to normal dispatch below
            } else {
                return self.handle_leader_key(key, tx);
            }
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
            let action = {
                let s = self.settings.read().unwrap();
                s.key_bindings.get_action(&combo)
            };
            // Flash the raw chord — except for the leader gateway, whose
            // affordance is the pending sequence (and the which-key overlay).
            if action != Some(ActionShortcuts::Leader)
                && (is_fkey
                    || ((combo.modifiers.is_ctrl() || combo.modifiers.is_alt())
                        && combo.key >= KeyStrike::KeyA
                        && combo.key <= KeyStrike::KeyZ))
            {
                self.footer.flash(combo.to_string(), tx);
            }
            match action {
                Some(ActionShortcuts::OpenCommandPalette) => {
                    if self.overlays.active_kind() == Some(OverlayKind::CommandPalette) {
                        self.dismiss_overlay();
                    } else {
                        self.open_command_palette(tx);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::Leader) => {
                    // The gateway works in every context, including
                    // mid-typing — but not while an overlay owns input.
                    if !self.overlays.is_open() {
                        self.leader.start();
                        self.schedule_whichkey_reveal(tx);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::ToggleSidebar) => {
                    self.toggle_drawer(tx);
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
                    } else {
                        self.open_search_browser(tx);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::OpenNote) => {
                    if self.overlays.active_kind() == Some(OverlayKind::NoteBrowser) {
                        self.dismiss_overlay();
                    } else {
                        self.open_file_finder(tx);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FileOperations) if self.editor_active() => {
                    tx.send(AppEvent::ShowFileOpsMenu(self.path.clone())).ok();
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::FollowLink) if self.editor_active() => {
                    self.follow_link_at_cursor(tx);
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::ToggleQueryPanel) => {
                    self.toggle_backlinks(tx);
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::OpenFileBrowser) => {
                    // Open (or switch to) the FILES view — never hides the
                    // drawer; Ctrl-T (ToggleSidebar) is the on/off switch.
                    // Always reveal: with FILES already open this is the
                    // "where is my note" gesture, snapping a browsed-away
                    // sidebar back to the current note's directory.
                    self.open_drawer_view(DrawerView::Files, tx);
                    self.reveal_note_dir_in_sidebar(tx);
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::OpenSavedSearches) => {
                    if self.overlays.active_kind() == Some(OverlayKind::SavedSearches) {
                        self.dismiss_overlay();
                    } else {
                        self.open_saved_searches(tx);
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::OpenSortDialog) => {
                    // Sort applies only when a list is focused (the drawer's
                    // Find / Files views). When the editor is focused, do NOT
                    // consume — fall through so the key reaches it (e.g. Ctrl+R
                    // is redo in the nvim editor).
                    if matches!(self.panels.focused(), PanelKind::Drawer)
                        && !self.overlays.is_open()
                    {
                        let target = match self.panels.active_drawer_view() {
                            DrawerView::Find => Some(SortTarget::Query),
                            DrawerView::Files => Some(SortTarget::Sidebar),
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
                        return EventState::Consumed;
                    }
                }
                Some(ActionShortcuts::SaveCurrentQuery) => {
                    if let Some((query, provenance, source)) = self.save_query_source() {
                        // Opening the save dialog replaces the note browser (if
                        // any); the chained-open guard preserves the original
                        // opener focus.
                        self.present_overlay(Box::new(ActiveDialog::save_search(
                            query,
                            provenance,
                            source,
                            self.vault.clone(),
                            tx,
                        )));
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::SwitchWorkspace) => {
                    self.open_workspace_switcher();
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
                    if let Some(ed) = self.panels.editor_mut() {
                        ed.open_or_advance_search();
                    }
                    return EventState::Consumed;
                }
                Some(ActionShortcuts::Text(
                    action @ (TextAction::Bold | TextAction::Italic | TextAction::Strikethrough),
                )) if self.editor_active() => {
                    if let Some(ed) = self.panels.editor_mut() {
                        ed.apply_text_action(action);
                    }
                    return EventState::Consumed;
                }
                _ => {
                    if is_fkey {
                        // F1 opens the help modal (only when no other dialog is active).
                        // Over the Find panel it surfaces query syntax instead of
                        // the flat key-bindings help.
                        if combo.key == KeyStrike::F1 && combo.modifiers.is_empty() {
                            if self.find_panel_focused() {
                                self.open_query_help();
                            } else {
                                self.open_help();
                            }
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
            // `PanelSet` hit-tests the panel columns: a click focuses the
            // panel under the cursor (one rule for every panel) and the event
            // is forwarded to that panel for its internal behavior.
            let state = self.panels.handle_mouse(event, tx);
            // A selectionless right-click in the editor asks for the note's
            // context menu — the screen owns the path, so it opens it here.
            if self.panels.editor().is_some_and(|e| e.wants_context_menu) {
                if let Some(ed) = self.panels.editor_mut() {
                    ed.wants_context_menu = false;
                }
                tx.send(AppEvent::ShowFileOpsMenu(self.path.clone())).ok();
            }
            return state;
        }

        // Bare Space falls through to the focused panel (types a space in text
        // inputs, no-op in lists) — EXCEPT in vim Normal mode with empty pending
        // state, where Space is a second leader gateway (in addition to Ctrl-G).
        // Insert/Visual/other backends keep Space typing a space because
        // `vim_space_leads()` returns false for those states.

        // Vim Normal mode: bare Space is the leader (in addition to Ctrl-G),
        // but only with an empty pending state so it never shadows Space as a
        // motion/operator argument. Insert/Visual and the other backends keep
        // Space typing a space (the rule below the Tab handling).
        if self.editor_active()
            && !self.overlays.is_open()
            && !self.leader.is_pending()
            && let InputEvent::Key(key) = event
            && key.code == ratatui::crossterm::event::KeyCode::Char(' ')
            && key.modifiers.is_empty()
            && self.panels.editor().is_some_and(|e| e.vim_space_leads())
        {
            self.leader.start();
            self.schedule_whichkey_reveal(tx);
            return EventState::Consumed;
        }

        // Tab / Shift-Tab cycle panel focus (spec §2). The focused panel gets
        // first crack — the Query panel's autocomplete accepts on Tab — and
        // the editor keeps Tab for indentation (it is a text field, the same
        // rule that keeps Space typing a space there).
        if self.panels.focused() != PanelKind::Editor
            && let InputEvent::Key(key) = event
        {
            use ratatui::crossterm::event::KeyCode;
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                let state = self.panels.handle_input(event, tx);
                if state == EventState::NotConsumed {
                    if key.code == KeyCode::Tab {
                        self.focus_right(tx);
                    } else {
                        self.focus_left(tx);
                    }
                }
                return EventState::Consumed;
            }
        }

        // Keyboard → the focused panel. The drawer's FIND view gets first
        // crack (its autocomplete popup may consume Esc); on an unhandled Esc
        // it yields focus back to the editor.
        if self.panels.focused() == PanelKind::Drawer
            && self.panels.active_drawer_view() == DrawerView::Find
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
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(crate::components::footer_bar::STATUS_BAR_HEIGHT),
            ])
            .split(f.area());

        // ── Title bar (1 line): Kimün · note breadcrumb · workspace badge ──
        // Build the badge line once; its column width comes from the same
        // value that gets rendered, so glyph/separator tweaks can't drift.
        let workspace_badge = {
            let s = self.settings.read().unwrap();
            s.workspace_config.as_ref().map(|wc| {
                ratatui::text::Line::from(vec![
                    ratatui::text::Span::styled(
                        self.icons.workspace,
                        Style::default().fg(theme.accent.to_ratatui()),
                    ),
                    ratatui::text::Span::styled(
                        format!("  {}", wc.global.current_workspace),
                        Style::default().fg(theme.gray.to_ratatui()),
                    ),
                ])
            })
        };
        let workspace_label_width = workspace_badge
            .as_ref()
            .map(ratatui::text::Line::width)
            .unwrap_or_default();
        let breadcrumb = self
            .path
            .to_string()
            .trim_start_matches('/')
            .replace('/', " / ");
        let title_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(workspace_label_width as u16 + 2),
            ])
            .split(rows[0]);
        f.render_widget(
            Paragraph::new(ratatui::text::Line::from(vec![
                ratatui::text::Span::styled(
                    " Kimün ",
                    Style::default()
                        .fg(theme.accent.to_ratatui())
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                ratatui::text::Span::styled(
                    format!("─  {breadcrumb}"),
                    Style::default().fg(theme.fg_secondary.to_ratatui()),
                ),
            ])),
            title_cols[0],
        );
        if let Some(badge) = workspace_badge {
            f.render_widget(
                Paragraph::new(badge).alignment(ratatui::layout::Alignment::Right),
                title_cols[1],
            );
        }

        // The panels lay themselves out and render. No panel shows its
        // focused highlight while an overlay sits over them.
        self.panels
            .render(f, rows[1], theme, !self.overlays.is_open());

        // Status bar reflects the overlay if one is open, otherwise the
        // focused panel. `editing` drives the ⌨/≣ focus-context indicator —
        // true when a text field holds the cursor.
        let (focus_label, hints) = if let Some(kind) = self.overlays.active_kind() {
            (kind.label(), self.overlays.hint_shortcuts())
        } else {
            (self.panels.focused_label(), self.panels.focused_hints())
        };
        let editing = if let Some(kind) = self.overlays.active_kind() {
            // Browsers and the saved-searches modal host a query input;
            // dialogs are button/list selections.
            !matches!(kind, OverlayKind::Dialog)
        } else {
            match self.panels.focused() {
                PanelKind::Editor => true,
                PanelKind::Drawer => self.panels.drawer_is_text_input(),
                PanelKind::Rail => false,
            }
        };
        let path_str = self.path.to_string();
        // Link-under-cursor affordance (spec §5.2): `→ target · N backlinks`.
        // The backlink count loads async, cached per target.
        let link_segment = if self.panels.focused() == PanelKind::Editor && !self.overlays.is_open()
        {
            let link = self.panels.editor().and_then(|e| e.link_at_cursor());
            self.doc_meta
                .link_segment(link.as_ref(), &self.path, self.app_tx.as_ref())
        } else {
            None
        };
        // ln/col only when the editor buffer holds the cursor (not the
        // attachment view, which has none).
        let ln_col = (self.panels.focused() == PanelKind::Editor && !self.overlays.is_open())
            .then(|| {
                self.panels.editor().map(|e| {
                    let (row, col) = e.cursor_pos();
                    (row + 1, col + 1)
                })
            })
            .flatten();
        // Match count when the FIND drawer is the focused query context.
        let matches = (self.panels.focused() == PanelKind::Drawer
            && self.panels.active_drawer_view() == DrawerView::Find)
            .then(|| self.panels.query().result_count());
        let (global_hints, leader_timeout, gateway_label) = {
            let s = self.settings.read().unwrap();
            (
                crate::components::hints::global_hints(&s.key_bindings),
                std::time::Duration::from_millis(s.leader_timeout_ms),
                s.key_bindings
                    .first_combo_for(&ActionShortcuts::Leader)
                    .unwrap_or_else(|| "leader".to_string()),
            )
        };
        let ctx = crate::components::footer_bar::StatusContext {
            focus_label,
            editing,
            hints: &hints,
            global_hints: &global_hints,
            doc: crate::components::footer_bar::DocState {
                path: &path_str,
                dirty: self.panels.editor().is_some_and(|e| e.is_dirty()),
                ln_col,
                backlinks: self.doc_meta.backlinks(),
                git: self.doc_meta.git().cloned(),
                matches,
                link: link_segment,
                update: self
                    .update
                    .as_ref()
                    .map(|u| format!("⬆ {} available", u.latest)),
            },
        };
        self.footer.render(f, rows[2], theme, &ctx);

        // which-key overlay — docked above the status bar once the user
        // hesitates mid-sequence (spec §8b).
        let whichkey_visible = self
            .leader
            .pending_since()
            .is_some_and(|since| since.elapsed() >= leader_timeout);
        if whichkey_visible {
            let gateway = gateway_label;
            let area = f.area();
            let h = crate::components::which_key::desired_height(&self.leader, area.width)
                .min(rows[1].height);
            let rect =
                ratatui::layout::Rect::new(area.x, rows[2].y.saturating_sub(h), area.width, h);
            crate::components::which_key::render(f, rect, theme, &self.leader, &gateway);
        }

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

        // Async status results (backlink count, git, link meta) are
        // DocMeta's; everything else comes back for the owned match.
        let Some(msg) = self.doc_meta.handle(msg, &self.path) else {
            return;
        };

        match msg {
            AppEvent::UpdateAvailable(status) => {
                self.update = Some(status);
            }
            AppEvent::ShowUpdateDialog => {
                if let Some(status) = self.update.clone() {
                    self.present_overlay(Box::new(ActiveDialog::update(&status)));
                }
            }
            AppEvent::DismissUpdate(_) => {
                // Persistence happens in main; here we just drop the indicator.
                self.update = None;
            }
            AppEvent::UpdateApplied => {
                // Installed — drop the notice so the footer/dialog stop offering
                // the version we just wrote (restart still required to run it).
                self.update = None;
            }
            AppEvent::ApplyUpdate => {
                let tx2 = tx.clone();
                tx.send(AppEvent::FlashMessage("Downloading update…".into()))
                    .ok();
                tokio::spawn(async move {
                    let result = async {
                        let latest = crate::update::latest_release().await?;
                        crate::update::install(latest).await
                    }
                    .await;
                    let msg = match result {
                        Ok(()) => {
                            tx2.send(AppEvent::UpdateApplied).ok();
                            "Update installed — restart kimün to apply".to_string()
                        }
                        Err(e) => format!("Update failed: {e}"),
                    };
                    tx2.send(AppEvent::FlashMessage(msg)).ok();
                });
            }
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
            // Stale completions (for notes we've navigated away from) fall
            // through to the catch-all and are dropped.
            AppEvent::FlashMessage(msg) => {
                self.footer.flash(msg, tx);
            }
            AppEvent::ExecuteLeaderAction(action) => {
                if self.overlays.is_open() {
                    // The palette closes itself before sending, so this is
                    // unreachable from it — but never drop an action silently.
                    tracing::warn!("ExecuteLeaderAction({action:?}) dropped: overlay open");
                } else {
                    self.execute_leader_action(action, tx);
                }
            }
            AppEvent::ApplyTheme { theme, persist } => {
                // The picker resolved the theme already — no disk re-read,
                // just adapt to the terminal and swap.
                {
                    let mut s = self.settings.write().unwrap();
                    s.set_theme(theme.name.clone());
                }
                self.theme = (*theme).adapt_to_terminal();
                if persist {
                    let snapshot = self.settings.read().unwrap().clone();
                    tokio::spawn(async move {
                        snapshot.save_to_disk().ok();
                    });
                }
                tx.send(AppEvent::Redraw).ok();
            }
            // Drawer panels can't emit these under an overlay, but guard
            // anyway: never mutate panels while an overlay owns input.
            AppEvent::RunTagQuery(label) if !self.overlays.is_open() => {
                self.open_find_with_query(format!("#{label}"), None, tx);
            }
            AppEvent::JumpToHeading(heading) if !self.overlays.is_open() => {
                if let Some(ed) = self.panels.editor_mut() {
                    ed.jump_to_heading(&heading);
                }
                self.focus_editor();
            }
            AppEvent::OpenDrawerView(view) => {
                // Selecting the already-active view toggles the drawer closed
                // (spec §3: clicking the active rail item toggles).
                if self.panels.is_visible(PanelKind::Drawer)
                    && self.panels.active_drawer_view() == view
                {
                    self.panels.hide(PanelKind::Drawer);
                } else {
                    self.open_drawer_view(view, tx);
                }
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
                title,
            } => {
                if path == self.path
                    && let Some(rev) = saved_revision
                    && let Some(ed) = self.panels.editor_mut()
                {
                    ed.mark_saved_at_revision(rev);
                }
                if let Some(raw_title) = title {
                    self.note_saved(&path, raw_title);
                }
                // The write changed the working tree — refresh the git
                // segment (throttled).
                self.doc_meta.refresh_git(tx);
                // `SingleSlotTask::is_in_flight()` flips to false the
                // moment the spawned future returns (success or panic),
                // so we don't have to clear the slot manually here —
                // the next `spawn_autosave` tick will overwrite it.
                // Skip explicit cleanup; was previously racy because a
                // stale completion arriving after `try_save` had
                // already cleared and respawned could wipe the fresh
                // handle.
            }
            AppEvent::FocusSidebar => {
                self.focus_sidebar(tx);
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
                let provider = resolving_search_source(
                    self.vault.clone(),
                    s.current_last_paths(),
                    Some(self.path.clone()),
                );
                let modal = NoteBrowserModal::with_initial_query(
                    "Note Browser",
                    BrowserScope::Query,
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
                // Pure notification: a note now exists at `path`. Opening is the
                // creator's job (via OpenPath); here we only keep the sidebar in
                // step when it is browsing the new note's directory.
                self.refresh_sidebar_if_showing(&path.get_parent_path().0, tx);
            }
            AppEvent::EntryDeleted(path) => {
                self.on_entry_op(path, tx).await;
            }
            AppEvent::EntryRenamed { from, to } => {
                // Note rename → targeted row update (and retarget the editor if
                // it is the open note). Directory rename keeps the full reload.
                if from.is_note() {
                    self.on_note_renamed(from, to, tx).await;
                } else {
                    self.on_entry_op(from, tx).await;
                }
            }
            AppEvent::EntryMoved { from, .. } => {
                self.on_entry_op(from, tx).await;
            }
            AppEvent::SaveSearchConfirmed {
                name,
                query,
                source,
            } => {
                // Write in the background; the breadcrumb re-pin waits for
                // the success event so the UI never claims an unpersisted
                // save (see SavedSearchPersisted below).
                let vault = self.vault.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    match vault.save_search(&name, &query).await {
                        Ok(()) => {
                            tx.send(AppEvent::SavedSearchPersisted {
                                name,
                                query,
                                source,
                            })
                            .ok();
                        }
                        Err(e) => {
                            tracing::warn!("failed to save search '{}': {}", name, e);
                            tx.send(AppEvent::SavedSearchSaveFailed { name }).ok();
                        }
                    }
                });
            }
            // Re-pin the panel breadcrumb to the saved identity: the edited
            // marker drops on an update, the name switches on a save-as-new.
            // Only for panel-sourced saves (a note-browser save must not
            // steal the panel's provenance, even when the query text
            // coincides), and not for the query-as-name fallback (a
            // breadcrumb that echoes the query is noise).
            AppEvent::SavedSearchPersisted {
                name,
                query,
                source: SaveSource::QueryPanel,
            } if name.trim() != query.trim() => {
                self.panels.query_mut().repin_saved_search(name, &query);
            }
            AppEvent::SavedSearchSaveFailed { name } => {
                self.footer
                    .flash(format!("Failed to save search '{name}'"), tx);
            }
            AppEvent::InsertAtCursor(text) if self.panels.focused() == PanelKind::Editor => {
                if let Some(ed) = self.panels.editor_mut() {
                    ed.insert_at_cursor(&text, tx);
                }
            }
            _ => {}
        }
    }

    /// The editor handles every path itself: notes open in the buffer,
    /// directories navigate the sidebar. Always consumes.
    async fn try_open_path(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        tx: &AppTx,
    ) -> Option<VaultPath> {
        self.dismiss_overlay();
        if path.is_note() {
            self.open_path(path, emphasis, tx).await;
            self.focus_editor();
        } else {
            self.navigate_sidebar(path, tx);
        }
        None
    }

    async fn try_open_attachment(&mut self, path: VaultPath, tx: &AppTx) -> Option<VaultPath> {
        self.dismiss_overlay();
        self.open_attachment(path, tx).await;
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
        assert_eq!(PanelKind::Rail.label(), "RAIL");
        assert_eq!(DrawerView::Files.label(), "FILES");
        assert_eq!(DrawerView::Find.label(), "FIND");
        let _kind = OverlayKind::Dialog;
    }

    /// One screen over a fresh temp vault. Returns the `TempDir` so the vault
    /// directory outlives the test body.
    async fn test_screen() -> (
        EditorScreen,
        Arc<NoteVault>,
        SharedSettings,
        tempfile::TempDir,
    ) {
        use crate::settings::AppSettings;
        use kimun_core::VaultConfig;
        use std::sync::RwLock;

        let dir = tempfile::TempDir::new().unwrap();
        let vault = Arc::new(NoteVault::new(VaultConfig::new(dir.path())).await.unwrap());
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        let screen = EditorScreen::new(vault.clone(), VaultPath::root(), settings.clone());
        (screen, vault, settings, dir)
    }

    fn key_event(code: ratatui::crossterm::event::KeyCode) -> InputEvent {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
        InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl_key(c: char) -> InputEvent {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        InputEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn chr(c: char) -> InputEvent {
        key_event(ratatui::crossterm::event::KeyCode::Char(c))
    }

    /// Ctrl-G (leader) then `o` `f` opens the FILES drawer — the full
    /// sequence fires with no menu drawn and no timeout wait.
    #[tokio::test]
    async fn leader_sequence_opens_drawer_view() {
        let (mut screen, _, _, _dir) = test_screen().await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Start from a non-Files view so the switch is observable.
        screen.panels.open_drawer_view(DrawerView::Tags);

        screen.handle_input(&ctrl_key('g'), &tx);
        assert!(screen.leader.is_pending());
        screen.handle_input(&chr('o'), &tx);
        screen.handle_input(&chr('f'), &tx);

        assert!(!screen.leader.is_pending());
        assert_eq!(screen.panels.active_drawer_view(), DrawerView::Files);
        assert_eq!(screen.panels.focused(), PanelKind::Drawer);
    }

    /// Opening the FILES drawer points the sidebar at the current note's
    /// directory, not the stale dir it was last left on.
    #[tokio::test]
    async fn opening_files_drawer_reveals_current_note_dir() {
        let (mut screen, _, _, _dir) = test_screen().await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        screen.path = VaultPath::new("projects").append(&VaultPath::note_path_from("plan"));
        // Sidebar left on an unrelated directory, drawer hidden.
        screen
            .panels
            .sidebar_mut()
            .navigate(VaultPath::new("other"), &tx);
        screen.panels.hide(PanelKind::Drawer);

        screen.open_drawer_view(DrawerView::Files, &tx);

        assert!(
            screen
                .panels
                .sidebar()
                .current_dir()
                .is_like(&VaultPath::new("projects"))
        );
    }

    /// With FILES already open but browsed elsewhere, the open-file-browser
    /// shortcut is the "where is my note" gesture: it re-reveals the current
    /// note's directory.
    #[tokio::test]
    async fn file_browser_shortcut_rereveals_note_dir_when_already_open() {
        let (mut screen, _, _, _dir) = test_screen().await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        screen.path = VaultPath::new("projects").append(&VaultPath::note_path_from("plan"));
        screen.open_drawer_view(DrawerView::Files, &tx);
        // User browses away while FILES stays open.
        screen
            .panels
            .sidebar_mut()
            .navigate(VaultPath::new("other"), &tx);

        screen.handle_input(&ctrl_key('e'), &tx);

        assert!(
            screen
                .panels
                .sidebar()
                .current_dir()
                .is_like(&VaultPath::new("projects"))
        );
    }

    /// The gateway works mid-typing: with the editor focused, Ctrl-G arms
    /// the sequence and the next chars are consumed, not inserted.
    #[tokio::test]
    async fn leader_consumes_keys_while_editor_focused() {
        let (mut screen, _, _, _dir) = test_screen().await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        screen.panels.editor_mut().unwrap().set_text(String::new());
        assert_eq!(screen.panels.focused(), PanelKind::Editor);

        screen.handle_input(&ctrl_key('g'), &tx);
        assert!(screen.leader.is_pending());
        // 'w' is a group key; it must not land in the buffer.
        screen.handle_input(&chr('w'), &tx);
        screen.handle_input(&chr('z'), &tx); // zen: hides the drawer
        assert_eq!(screen.panels.editor().unwrap().get_text(), "");
        assert!(!screen.panels.is_visible(PanelKind::Drawer));
    }

    /// Esc cancels a pending sequence and returns focus to the editor.
    #[tokio::test]
    async fn leader_esc_cancels_and_focuses_editor() {
        let (mut screen, _, _, _dir) = test_screen().await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        screen.panels.focus(PanelKind::Rail);
        screen.handle_input(&ctrl_key('g'), &tx);
        screen.handle_input(&chr('f'), &tx);
        screen.handle_input(&key_event(ratatui::crossterm::event::KeyCode::Esc), &tx);

        assert!(!screen.leader.is_pending());
        assert_eq!(screen.panels.focused(), PanelKind::Editor);
    }

    /// Bare Space never leads — the leader is only the configured gateway.
    /// Space types a space in the editor and never arms the sequence, whatever
    /// panel is focused (rail, a list drawer, or a text-input drawer).
    #[tokio::test]
    async fn space_never_leads() {
        let (mut screen, _, _, _dir) = test_screen().await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Editor focused: Space must insert a space.
        screen.panels.editor_mut().unwrap().set_text(String::new());
        screen.handle_input(&chr(' '), &tx);
        assert!(!screen.leader.is_pending());
        assert_eq!(screen.panels.editor().unwrap().get_text(), " ");

        // Rail focused: Space must NOT lead.
        screen.panels.focus(PanelKind::Rail);
        screen.handle_input(&chr(' '), &tx);
        assert!(!screen.leader.is_pending());

        // FIND drawer (a text input): Space must NOT lead.
        screen.panels.open_drawer_view(DrawerView::Find);
        screen.panels.focus(PanelKind::Drawer);
        screen.handle_input(&chr(' '), &tx);
        assert!(!screen.leader.is_pending());
    }

    #[tokio::test]
    async fn persist_saved_search_writes_via_core() {
        let (screen, _, _, _dir) = test_screen().await;

        screen.persist_saved_search("t", "#todo").await.unwrap();

        let all = screen.vault.list_saved_searches().await.unwrap();
        assert!(all.iter().any(|s| s.name == "t" && s.query == "#todo"));
    }

    #[tokio::test]
    async fn applying_saved_search_sets_panel_query_and_focuses_it() {
        let (mut screen, _, _, _dir) = test_screen().await;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.apply_saved_search(
            "<{note}".to_string(),
            "Backlinks (current note)".to_string(),
            &tx,
        );
        assert!(screen.panels.is_visible(PanelKind::Drawer));
        assert_eq!(screen.panels.active_drawer_view(), DrawerView::Find);
        assert_eq!(screen.panels.query().active_query(), "<{note}");
        assert_eq!(screen.panels.focused(), PanelKind::Drawer);
    }

    #[tokio::test]
    async fn saved_search_persisted_repins_panel_breadcrumb() {
        let (mut screen, _, _, _dir) = test_screen().await;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.apply_saved_search("#todo".to_string(), "todo".to_string(), &tx);
        screen
            .panels
            .query_mut()
            .set_active_query("#todo and #urgent".to_string());

        screen
            .handle_app_message(
                AppEvent::SavedSearchPersisted {
                    name: "urgent-todos".to_string(),
                    query: "#todo and #urgent".to_string(),
                    source: SaveSource::QueryPanel,
                },
                &tx,
            )
            .await;

        assert_eq!(
            screen.panels.query().saved_search_breadcrumb().as_deref(),
            Some("urgent-todos"),
            "a persisted panel-sourced save re-pins the breadcrumb"
        );
    }

    #[tokio::test]
    async fn persisted_note_browser_save_does_not_repin_even_on_equal_query() {
        let (mut screen, _, _, _dir) = test_screen().await;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.apply_saved_search("#todo".to_string(), "todo".to_string(), &tx);

        // A note-browser-sourced save whose query text happens to equal the
        // panel's live query: source identity, not text equality, decides.
        screen
            .handle_app_message(
                AppEvent::SavedSearchPersisted {
                    name: "inbox".to_string(),
                    query: "#todo".to_string(),
                    source: SaveSource::NoteBrowser,
                },
                &tx,
            )
            .await;

        assert_eq!(
            screen.panels.query().saved_search_breadcrumb().as_deref(),
            Some("todo"),
            "a note-browser save must not steal the panel's provenance"
        );
    }

    #[tokio::test]
    async fn persisted_query_as_name_save_skips_repin() {
        let (mut screen, _, _, _dir) = test_screen().await;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.apply_saved_search("#todo".to_string(), "todo".to_string(), &tx);

        // The empty-name fallback saved the query under its own text. Pinning
        // that as the breadcrumb name would just echo the query (CONTEXT.md:
        // the breadcrumb is a distinct provenance tag, not the query).
        screen
            .handle_app_message(
                AppEvent::SavedSearchPersisted {
                    name: "#todo".to_string(),
                    query: "#todo".to_string(),
                    source: SaveSource::QueryPanel,
                },
                &tx,
            )
            .await;

        assert_eq!(
            screen.panels.query().saved_search_breadcrumb().as_deref(),
            Some("todo"),
            "a query-as-name save leaves the breadcrumb alone"
        );
    }

    #[tokio::test]
    async fn save_search_confirmed_persists_then_emits_persisted() {
        let (mut screen, vault, _, _dir) = test_screen().await;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        screen
            .handle_app_message(
                AppEvent::SaveSearchConfirmed {
                    name: "mine".to_string(),
                    query: "#todo".to_string(),
                    source: SaveSource::QueryPanel,
                },
                &tx,
            )
            .await;

        // The write runs in a spawned task; Persisted arrives only on success.
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match rx.recv().await {
                    Some(e @ AppEvent::SavedSearchPersisted { .. }) => break e,
                    Some(_) => continue,
                    None => panic!("channel closed before SavedSearchPersisted"),
                }
            }
        })
        .await
        .expect("SavedSearchPersisted within timeout");

        let AppEvent::SavedSearchPersisted {
            name,
            query,
            source,
        } = event
        else {
            unreachable!()
        };
        assert_eq!((name.as_str(), query.as_str()), ("mine", "#todo"));
        assert_eq!(source, SaveSource::QueryPanel);
        let all = vault.list_saved_searches().await.unwrap();
        assert!(all.iter().any(|s| s.name == "mine" && s.query == "#todo"));
    }

    #[tokio::test]
    async fn save_query_source_carries_panel_provenance() {
        let (mut screen, _, _, _dir) = test_screen().await;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.apply_saved_search("#todo".to_string(), "todo".to_string(), &tx);

        assert_eq!(
            screen.save_query_source(),
            Some((
                "#todo".to_string(),
                Some("todo".to_string()),
                SaveSource::QueryPanel
            )),
            "the save dialog opens pre-filled with the breadcrumb provenance"
        );
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
            screen.panels.focused() == PanelKind::Drawer
                && screen.panels.active_drawer_view() == DrawerView::Find,
            "focus should remain on the FIND drawer after select + close"
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
        assert_ne!(screen.panels.active_drawer_view(), DrawerView::Find);
        let focused_before = screen.panels.focused();

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

        assert_eq!(
            screen.panels.focused(),
            focused_before,
            "focus action must not move focus while an overlay is open"
        );
        assert_ne!(
            screen.panels.active_drawer_view(),
            DrawerView::Find,
            "focus action must not switch the drawer view while an overlay is open"
        );
        assert_eq!(
            screen.overlays.active_kind(),
            Some(OverlayKind::SavedSearches),
            "overlay stays active"
        );
    }

    /// Opening the journal while an overlay is up dismisses the overlay, so the
    /// journal note isn't loaded behind it. OpenJournal is now resolved at the
    /// app level into an OpenPath, which lands here in `try_open_path` — that's
    /// the door that must dismiss the overlay.
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

        let (details, _, _) = vault.journal_entry().await.unwrap();
        screen.try_open_path(details.path, None, &tx).await;

        assert!(
            !screen.overlays.is_open(),
            "opening the journal must dismiss the overlay before loading the note"
        );
    }

    /// Opening an attachment swaps the editor area to the read-only attachment
    /// view: the note editor accessor reports absent, and FollowLink in that
    /// state opens externally rather than touching the (absent) editor.
    #[tokio::test(flavor = "multi_thread")]
    async fn opening_attachment_shows_attachment_view() {
        let vault = crate::test_support::temp_vault("editor-attachment").await;
        vault.validate_and_init().await.unwrap();
        vault
            .save_attachment(&VaultPath::new("assets/diagram.png"), &[1, 2, 3])
            .await
            .unwrap();
        let settings = std::sync::Arc::new(std::sync::RwLock::new(
            crate::settings::AppSettings::default(),
        ));
        let mut screen = EditorScreen::new(vault.clone(), VaultPath::root(), settings);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Precondition: a note editor is mounted.
        assert!(screen.panels.editor().is_some());

        screen
            .try_open_attachment(VaultPath::new("assets/diagram.png"), &tx)
            .await;

        assert!(
            screen.panels.is_showing_attachment(),
            "the editor area shows the attachment view"
        );
        assert!(
            screen.panels.editor().is_none(),
            "no note editor is mounted while an attachment is shown"
        );
        assert_eq!(
            screen.panels.attachment_path(),
            Some(&VaultPath::new("assets/diagram.png"))
        );

        // Returning to a note swaps the editor area back.
        vault
            .create_note(&VaultPath::new("note.md"), "hi")
            .await
            .unwrap();
        screen.open_path(VaultPath::new("note.md"), None, &tx).await;
        assert!(!screen.panels.is_showing_attachment());
        assert!(screen.panels.editor().is_some());
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
            let provider = resolving_search_source(vault.clone(), s.current_last_paths(), None);
            let modal = NoteBrowserModal::with_initial_query(
                "Note Browser",
                BrowserScope::Query,
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

    /// Renaming the open note keeps it open under the new path (retarget in
    /// place) instead of navigating away, and reloads the buffer clean.
    #[tokio::test(flavor = "multi_thread")]
    async fn renaming_open_note_retargets_in_place() {
        let vault = crate::test_support::temp_vault("editor-rename").await;
        vault.validate_and_init().await.unwrap();
        let from = VaultPath::note_path_from("old");
        vault.create_note(&from, "# Old\n\nbody").await.unwrap();
        let settings = std::sync::Arc::new(std::sync::RwLock::new(
            crate::settings::AppSettings::default(),
        ));
        let mut screen = EditorScreen::new(vault.clone(), from.clone(), settings);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.on_enter(&tx).await;

        let to = VaultPath::note_path_from("new");
        vault.rename_note(&from, &to).await.unwrap();
        screen
            .handle_app_message(
                AppEvent::EntryRenamed {
                    from: from.clone(),
                    to: to.clone(),
                },
                &tx,
            )
            .await;

        assert_eq!(screen.path, to, "editor retargets to the new path");
        assert!(
            !screen.panels.editor().unwrap().is_dirty(),
            "reloaded buffer is clean (won't clobber the renamed file)"
        );
    }

    /// Opening a note marks its sidebar row; saving it (AutosaveCompleted with a
    /// new title) updates that row's title in place.
    #[tokio::test(flavor = "multi_thread")]
    async fn open_then_save_marks_and_retitles_sidebar_row() {
        let vault = crate::test_support::temp_vault("editor-marksave").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("alpha"), "# Alpha\n\nbody")
            .await
            .unwrap();
        let settings = std::sync::Arc::new(std::sync::RwLock::new(
            crate::settings::AppSettings::default(),
        ));
        let path = VaultPath::note_path_from("alpha");
        let mut screen = EditorScreen::new(vault.clone(), path.clone(), settings);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.on_enter(&tx).await;
        for _ in 0..50 {
            screen.panels.sidebar_mut().poll_for_test();
            if !screen.panels.sidebar().is_loading_for_test() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        assert!(
            screen
                .panels
                .sidebar()
                .note_row_is_open_for_test("alpha.md"),
            "the open note's row is marked"
        );

        screen
            .handle_app_message(
                AppEvent::AutosaveCompleted {
                    path: path.clone(),
                    saved_revision: None,
                    title: Some("New First Line".to_string()),
                },
                &tx,
            )
            .await;

        assert_eq!(
            screen.panels.sidebar().note_row_title_for_test("alpha.md"),
            Some("New First Line".to_string()),
            "the saved note's row title updated in place"
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
