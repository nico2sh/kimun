//! The **vault change at runtime** (M8b): leave the vault the window is on
//! (or none) and enter the one the settings name now (or none), with nothing
//! of the old one left. What the TUI does by rebuilding the app on
//! `OnboardingFinished`, `PreferencesSaved` and `WorkspaceSwitched`.
//!
//! **The flow** ([`Workspace::reopen`]):
//!
//! 1. The open note is flushed, as before any other note opens. A flush that
//!    fails keeps the window on the vault it is on, every part of it still
//!    working, with an error flash. A change still being written, a note
//!    still opening or the window closing refuse the change the same way.
//!    Edits that can never be written (a read-only vault) are not kept, as a
//!    note switch does not keep them.
//! 2. The new vault is opened off the UI thread, from the settings as they
//!    are then (`try_open_vault`, then the startup's decision table, never
//!    with the command line's note: the first note is the restore).
//! 3. Everything of the old vault goes ([`Workspace::leave_vault`]), the
//!    window gets a fresh app channel, and the new vault is entered as at
//!    startup ([`Workspace::enter`]): indexed in the background, the session
//!    restored, the drawer views rebuilt over it, the server sync restarted
//!    when one is configured, the window title renamed. The layout, the
//!    theme, the keymap and the focus handles stay.
//!
//! **No stale messages.** Each vault entry gets its own app channel: the
//! listener of the old one is dropped with the messages still queued on it,
//! and every background job of the old vault holds a sender nobody reads.
//! The editor's sender moves to the new channel. The window's own tasks over
//! the old vault are dropped (cancelled) with it; the two detached ones that
//! call back into the window (today's journal, a saved search's delete)
//! check that their vault is still the window's ([`Workspace::is_vault`]).
//!
//! **The settings write** ([`Workspace::persist_settings`]): the one way the
//! window writes the shared settings (not its `[gui]` table, which the
//! layout store owns). Off the UI thread, one write at a time, each writing
//! the settings as they are when it runs; the `[gui]` table on disk is kept
//! and a config that does not parse is never overwritten. A failure is an
//! error flash.

use std::sync::{Arc, Mutex};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, Context, Window};
use kimun_core::NoteVault;
use kimun_notes::logic::ask_flow::AskFlow;
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::rag_caps::Caps;
use kimun_notes::logic::session::FlushResult;
use kimun_notes::logic::startup::{VaultOpen, try_open_vault};
use kimun_notes::rag::RagStatus;
use kimun_notes::settings::AppSettings;
use tokio::task::JoinError;

use super::note::Closing;
use super::note_flush::FlushBlock;
use super::{LayoutStore, Workspace};
use crate::startup;
use crate::status_row::Status;

/// One settings write at a time, process-wide: each reads the settings as
/// they are when it runs, so the last one to run writes the newest.
static SETTINGS_WRITE: Mutex<()> = Mutex::new(());

impl Workspace {
    /// Whether a vault change is under way.
    pub(crate) fn reopen_running(&self) -> bool {
        self.reopening.is_some()
    }

    /// Whether `vault` is the one the window is on: a task started over
    /// another vault has nothing to apply here.
    pub(crate) fn is_vault(&self, vault: &Arc<NoteVault>) -> bool {
        self.vault().is_some_and(|v| Arc::ptr_eq(&v, vault))
    }

    /// Why the window cannot change vault now, if it cannot: the reason in
    /// a sentence's middle.
    fn reopen_block(&self, cx: &gpui_kit::App) -> Option<&'static str> {
        if self.closing != Closing::Open {
            return Some(FlushBlock::Closing.reason());
        }
        match self.flush_block(cx) {
            // Edits that can never be written do not hold the window on a
            // vault (a read-only one would keep it there for good).
            None | Some(FlushBlock::Unsaveable) => None,
            Some(block) => Some(block.reason()),
        }
    }

    /// `WorkspaceSwitched(name)`, the workspace switcher: enter `name`
    /// ([`Self::reopen`]), making it the current workspace in the shared
    /// settings and persisting them, as the TUI does. Refused, with nothing
    /// changed, while the window cannot change vault; the settings change
    /// only once the open note is flushed, so a flush that fails leaves them
    /// naming the workspace the window stays on.
    pub(crate) fn switch_workspace(&mut self, name: String, cx: &mut Context<Self>) {
        if !self.reopen_running()
            && let Some(reason) = self.reopen_block(cx)
        {
            self.flash_error(format!("Couldn't switch to {name}: {reason}."), cx);
            return;
        }
        if self.settings.is_none() {
            return;
        }
        self.switch_to = Some(name);
        self.reopen(cx);
        // Refused at once (another save running): nothing changes.
        if !self.reopen_running() {
            self.switch_to = None;
        }
    }

    /// A switch asked for: its workspace becomes the current one, in memory
    /// and on disk. Runs once the flush is done, before the vault opens.
    fn apply_switch(&mut self, cx: &mut Context<Self>) {
        let (Some(name), Some(settings)) = (self.switch_to.take(), self.settings.clone()) else {
            return;
        };
        {
            let mut settings = settings.write().unwrap_or_else(|e| e.into_inner());
            if let Some(config) = settings.workspace_config.as_mut() {
                config.global.current_workspace = name;
            }
        }
        self.persist_settings(cx);
    }

    /// Persist the shared settings ([module docs](self)): off the UI
    /// thread, a failure flashed. Must run with the tokio runtime entered.
    pub(crate) fn persist_settings(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let write = write_settings(self.store.clone(), move || {
            settings.read().unwrap_or_else(|e| e.into_inner()).clone()
        });
        cx.spawn(async move |this, cx| {
            let Err(error) = write.await else {
                return;
            };
            tracing::warn!("could not save the settings: {error}");
            this.update(cx, |workspace, cx| {
                workspace.flash_error(format!("Couldn't save the settings: {error}"), cx)
            })
            .ok();
        })
        .detach();
    }

    /// Leave the vault the window is on (or none) for the one the shared
    /// settings name now (or none): see the [module docs](self). A change
    /// asked for while one runs makes that one open the vault again from
    /// the settings as they are then. Must run with the tokio runtime
    /// entered.
    pub(crate) fn reopen(&mut self, cx: &mut Context<Self>) {
        if self.reopen_running() {
            self.reopen_again = true;
            return;
        }
        if let Some(reason) = self.reopen_block(cx) {
            self.flash_error(format!("Couldn't change the workspace: {reason}."), cx);
            return;
        }
        let (Some(settings), Some(handle)) = (self.settings.clone(), self.window) else {
            return;
        };
        let flush = if self.can_save() {
            // No tick from here: the flush is the note's last write.
            match self.begin_note_flush(cx) {
                Some(flush) => Some(flush),
                None => {
                    self.flash_error(
                        "Couldn't change the workspace: another save is still running.",
                        cx,
                    );
                    return;
                }
            }
        } else {
            if self.notes.is_some() && self.editor_dirty(cx) {
                let text = format!("{} They were not kept.", self.unsaveable());
                self.flash_error(text, cx);
            }
            if let Some(host) = self.notes.as_mut() {
                host.session.stop_autosave();
            }
            // Nothing typed now could be kept.
            self.block_input(true, cx);
            None
        };
        self.reopening = Some(cx.spawn(async move |this, cx| {
            if let Some(flush) = flush {
                let flushed = tokio::spawn(flush.run()).await;
                let go = this
                    .update(cx, |workspace, cx| workspace.reopen_flushed(flushed, cx))
                    .unwrap_or(false);
                if !go {
                    return;
                }
            }
            loop {
                this.update(cx, |workspace, cx| workspace.apply_switch(cx))
                    .ok();
                let settings = settings.clone();
                let opened = tokio::spawn(async move {
                    let name = settings
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .current_workspace_name();
                    (try_open_vault(&settings).await, name)
                })
                .await;
                let again = this
                    .update(cx, |workspace, _| {
                        std::mem::take(&mut workspace.reopen_again)
                    })
                    .unwrap_or(false);
                if again {
                    continue;
                }
                let (open, name) = match opened {
                    Ok(opened) => opened,
                    Err(e) => (VaultOpen::Failed(e.to_string()), None),
                };
                let start = startup::decide(open, name, None, None);
                cx.update_window(handle, |_, window, cx| {
                    this.update(cx, |workspace, cx| {
                        workspace.reopen_landed(start, window, cx)
                    })
                    .ok();
                })
                .ok();
                return;
            }
        }));
        cx.notify();
    }

    /// The flush before a vault change ended: go on (true), or stay on this
    /// vault with its note, its edits and its tick, and say why.
    fn reopen_flushed(
        &mut self,
        flushed: Result<FlushResult, JoinError>,
        cx: &mut Context<Self>,
    ) -> bool {
        let outcome = self.apply_flush(flushed, cx);
        let Some(problem) = self.flush_failure(&outcome, cx) else {
            // Typing stays blocked until the new vault is in.
            return true;
        };
        self.reopening = None;
        self.reopen_again = false;
        self.switch_to = None;
        self.block_input(false, cx);
        self.restart_tick_unless_read_only();
        let note = self
            .notes
            .as_ref()
            .and_then(|h| h.note.clone())
            .map_or_else(|| "the note".to_string(), |n| n.to_string());
        self.flash_error(
            format!("Couldn't save {note}: {problem}. The workspace was not changed."),
            cx,
        );
        // An open that waited for the flush goes now.
        self.file_op_ended(cx);
        false
    }

    /// The new vault is open (or the settings name none): leave the old one
    /// and enter `start` on a fresh app channel.
    fn reopen_landed(
        &mut self,
        start: startup::Start,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_vault(window, cx);
        // The editor engine the settings name now (the setup or the
        // preferences may have changed it): a fresh, empty one of that kind.
        if let Some(settings) = self.settings.as_ref() {
            let vim = crate::uses_vim(
                settings
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .editor_backend,
            );
            self.editor.update(cx, |view, cx| view.set_vim(vim, cx));
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.set_channel(tx, rx, cx);
        self.enter(start, window, cx);
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Let go of everything that belongs to the vault the window is on: the
    /// note session and the editor's text, every drawer view over the vault
    /// and the Thread, the server sync and what it reported, the attachment,
    /// every task under way, the modals, a pending leader sequence and the
    /// Footer's state. Kept: the layout (the views' places and the drawers'
    /// sizes), the theme, the keymap, the focus handles, the window.
    pub(super) fn leave_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Preferences over the old vault: their draft goes, and the theme
        // they were previewing with it.
        if let Some(view) = self.preferences.clone() {
            let original = view.read(cx).original_theme().clone();
            self.drop_preferences();
            self.apply_theme(original, cx);
        }
        // The modals show the old vault's data; the leader would run its
        // route there.
        if super::modal::dialog_open(window, cx) {
            window.close_all_dialogs(cx);
        }
        self.reset_modal_state(cx);
        self.file_status = None;
        self.cancel_leader(cx);
        // The Thread gives the editor Panel back and ASK leaves its slot
        // (FILES takes it), as when a note opens.
        self.hide_ask(cx);
        if self.layout.view_location(&DrawerView::Ask).is_some() {
            self.drawer_off_ask(cx);
        }
        // The open note (already flushed): no tick, no path, an empty clean
        // buffer, no search emphasis or find bar from the old text.
        self.release_note(cx);
        self.editor.update(cx, |view, cx| {
            view.editor.close_find();
            view.editor.set_search_needles(Vec::new());
            cx.notify();
        });
        if let Some(host) = self.notes.as_mut() {
            host.session.stop_autosave();
            host.session.abort_autosave();
        }
        self.notes = None;
        // Every task over the old vault: dropped, so none lands here.
        self.reopening = None;
        self.reopen_again = false;
        self.switch_to = None;
        self.following = None;
        self.file_task = None;
        self.note_task = None;
        self.save_task = None;
        self.search_flush = None;
        self.properties_opening = None;
        self.property_write = None;
        self.note_reload = None;
        self.attachment_loading = None;
        self.attachment_flushing = None;
        self.attachment_wanted = None;
        self.ask_flushing = None;
        self.ask_client_task = None;
        self.ask_saving = None;
        // The attachment view and the views over the vault, with their
        // subscriptions.
        self.attachment = None;
        self._attachment_events = None;
        self.files = None;
        self._files_events = None;
        self.links = None;
        self.outline = None;
        self.find = None;
        self.tags = None;
        self.semantic = None;
        self.ask = None;
        self._ask_events = None;
        self.ask_sources = None;
        self._views_events.clear();
        self.ask_flow = AskFlow::new();
        // The server: its sync stops; nothing is known of the next one's
        // until it reports (the rail drops SEM and ASK meanwhile).
        self.rag_sync.stop();
        self.rag_status = RagStatus::Disabled;
        let cursor = self.rail_cursor_view();
        self.rag_caps = Caps::default();
        self.place_rail_cursor(cursor);
        // What the editor Panel and the Footer said about the old vault.
        self.empty = None;
        self.note_gone = None;
        self.restore_pending = false;
        self.first_open_pending = false;
        self.status = Status::default();
        self.closing = Closing::Open;
    }
}

/// The settings write ([module docs](self)) to `store`: `snapshot` runs when
/// the write's turn comes, off the UI thread, and gives the settings to
/// write. Resolves to the write's failure, if any. Must be called with the
/// tokio runtime entered (the write starts at once).
pub(super) fn write_settings(
    store: LayoutStore,
    snapshot: impl FnOnce() -> AppSettings + Send + 'static,
) -> impl std::future::Future<Output = Result<(), String>> + 'static {
    // The write blocks: off the UI thread, and off the runtime's own
    // thread too (`blocking` starts it at once).
    let write = crate::tasks::blocking(move || {
        let _one_at_a_time = SETTINGS_WRITE.lock().unwrap_or_else(|e| e.into_inner());
        store.save_settings(&snapshot())
    });
    async move { write.await.and_then(|result| result) }
}
