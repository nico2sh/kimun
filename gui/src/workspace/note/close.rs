//! Closing the window and quitting the app: a flush of every group, a
//! warning before edits are discarded, and the end of the app or of the
//! window only.
//! It does not save during normal editing. [`mod@super::save`] does that.

use gpui_kit::{App, AppContext, Context, Window};
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::session::{FlushResult, SaveOutcome};
use tokio::task::JoinError;

use super::{Closing, DISCARD_WINDOW, EditorRevisions, NoteHost, flush_problem, rewritable};
use crate::workspace::Workspace;

impl Workspace {
    /// Where the text of every group stands ([`EditorRevisions`]).
    pub(super) fn editors_revision(&self, cx: &App) -> EditorRevisions {
        EditorRevisions(
            self.group_ids()
                .into_iter()
                .filter_map(|group| {
                    self.state(group)
                        .map(|s| (group, s.editor.read(cx).editor.content_revision()))
                })
                .collect(),
        )
    }

    /// The user was just told the edits cannot be saved: the next close
    /// discards them, if it comes before any new edit (in any group) and
    /// within [`DISCARD_WINDOW`].
    pub(super) fn arm_discard(&mut self, cx: &mut Context<Self>) {
        let revision = self.editors_revision(cx);
        self.closing = Closing::Discard(revision.clone());
        let expiry = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DISCARD_WINDOW).await;
            this.update(cx, |workspace, _| {
                if workspace.closing == Closing::Discard(revision) {
                    workspace.closing = Closing::Open;
                }
            })
            .ok();
        });
        if let Some(host) = self.active_state_mut().notes.as_mut() {
            host.discard_expiry = Some(expiry);
        }
    }

    /// Tell the user that the edits in `group` cannot be saved. The next
    /// close discards them ([`Self::arm_discard`]). A caller that gives up
    /// a close sets `closing` and restarts the ticks before this call.
    fn warn_unsaveable(&mut self, group: GroupId, cx: &mut Context<Self>) {
        let text = format!("{} Close again to discard them.", self.unsaveable_in(group));
        self.arm_discard(cx);
        self.flash_error(text, cx);
    }

    /// Ask the window to check with the workspace before it closes (see the
    /// module docs), and warn in the log when the app quits another way with
    /// unsaved edits.
    pub(crate) fn guard_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window = Some(window.window_handle());
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            // The operating system's close button (or Cmd+W): the window, not the app.
            this.update(cx, |workspace, cx| {
                workspace.set_close_intent(false);
                let close = workspace.should_close(cx);
                // Off macOS the app ends with its window. The window goes by itself
                // now that the guard said yes: only the app has to be asked to end.
                if close && workspace.closes_app() {
                    workspace.end_app(cx);
                } else if close {
                    workspace.flush_layout(cx);
                }
                close
            })
            .unwrap_or(true)
        });
        self._on_quit = Some(cx.on_app_quit(|workspace, cx| {
            for group in workspace.group_ids_active_first() {
                if !workspace.editor_dirty_in(group, cx) {
                    continue;
                }
                let note = workspace
                    .host_in(group)
                    .and_then(|h| h.note.as_ref())
                    .map_or_else(|| "the editor".to_string(), ToString::to_string);
                tracing::warn!("quitting with unsaved edits to {note}");
                eprintln!("kimun-gui: quitting with unsaved edits to {note}");
            }
            // The session's and the layout's last writes land before the
            // app goes.
            workspace.write_session_now(cx);
            let layout_writer = workspace.layout_writer.clone();
            async move { layout_writer.flush() }
        }));
    }

    /// Close the window as its close button does: through [`Self::should_close`],
    /// so the open note is flushed first and the flush closes the window once
    /// written. Whether the app ends with it is [`Self::closes_app`].
    pub(crate) fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_close_intent(false);
        self.begin_close(window, cx);
    }

    /// Record what the close about to start is for: a window close (`false`) or a
    /// quit (`true`). Every way in states its own intent, so a quit that was refused
    /// cannot turn a later window close into a quit. While a close is already under
    /// way (waiting on a switch, or on its flush) only a quit can change it, to a
    /// quit: a window close arriving then must not downgrade a quit, or the flush
    /// would end with only the window gone and the app left in the dock.
    pub(super) fn set_close_intent(&mut self, quit: bool) {
        let under_way = self.window_closing();
        self.quit_on_close = if under_way {
            self.quit_on_close || quit
        } else {
            quit
        };
    }

    pub(super) fn begin_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.should_close(cx) {
            self.finish_close(window, cx);
        }
    }

    /// Quit the app (the Quit chord, the palette's quit, the leader's quit): the
    /// same close, and the app ends with the window on every platform.
    pub(crate) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_close_intent(true);
        self.begin_close(window, cx);
    }

    /// Whether the close in progress ends the app: a quit always does, a window
    /// close only off macOS (where the app stays in the dock to open again).
    pub(crate) fn closes_app(&self) -> bool {
        self.quit_on_close || !self.stays_in_dock
    }

    /// The close is allowed and nothing is left to write: end the app, or just
    /// the window. Quitting rather than removing the window keeps this workspace
    /// alive for the app's quit hook (the layout's last write).
    pub(super) fn finish_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closes_app() {
            self.end_app(cx);
        } else {
            self.flush_layout(cx);
            window.remove_window();
        }
    }

    /// Wait for the layout's last write before this window goes. Removing the window
    /// drops the workspace, and with it the app-quit hook that flushes the writer: the
    /// window the app opens next from the dock reads the config file, and it must find
    /// the layout this one left.
    pub(super) fn flush_layout(&mut self, cx: &App) {
        #[cfg(test)]
        {
            self.layout_flushes += 1;
        }
        // The tabs and groups as the window leaves them, at once.
        self.write_session_now(cx);
        self.layout_writer.flush();
    }

    /// End the app. The one place that asks the platform to quit, so the tests
    /// can count the asks (the test platform's quit does nothing).
    pub(super) fn end_app(&mut self, cx: &mut App) {
        #[cfg(test)]
        {
            self.quits += 1;
        }
        // The tabs and groups as the app leaves them, at once (the quit
        // hook writes them again, for a quit that does not come by here).
        self.write_session_now(cx);
        // An update replaced this app: start it again as this one ends. A
        // failure only costs the user the manual start.
        if std::mem::take(&mut self.updates.restart_pending)
            && let Err(e) = self.updates.relauncher.relaunch_after_exit()
        {
            tracing::warn!("could not restart after the update: {e}");
        }
        cx.quit();
    }

    /// Whether the window may close now. With edits to write, in any group,
    /// it starts the close flush and says no; the flush closes the window
    /// when done.
    pub(crate) fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        // An update installing (or the note's flush before it): refused and
        // said, not queued, as Preferences refuse it while they index.
        if self.update_busy() {
            self.flash_error(kimun_notes::logic::update::CLOSE_REFUSED_TEXT, cx);
            return false;
        }
        // Unsaved preferences ask first, as their Cancel does.
        if !self.preferences_allow_close(cx) {
            return false;
        }
        match &self.closing {
            Closing::Requested | Closing::Flushing => return false,
            Closing::Closed => return true,
            // Warned, and nothing typed since: the edits go.
            Closing::Discard(revision) if *revision == self.editors_revision(cx) => return true,
            // New edits since the warning: start over.
            Closing::Discard(_) => self.closing = Closing::Open,
            Closing::Open => {}
        }
        // No vault: the editor is not even shown.
        if self.active_state().notes.is_none() {
            return true;
        }
        let groups = self.group_ids_active_first();
        // A switch under way in a group that saves: the close waits for it.
        if groups
            .iter()
            .any(|g| self.can_save_in(*g) && self.host_in(*g).is_some_and(NoteHost::is_switching))
        {
            self.stop_ticks();
            self.closing = Closing::Requested;
            return false;
        }
        let to_write = groups.iter().any(|g| {
            self.can_save_in(*g)
                && (self.editor_dirty_in(*g, cx)
                    || self
                        .host_in(*g)
                        .is_some_and(|h| h.session.is_autosave_in_flight()))
        });
        if to_write {
            self.close_flush(cx);
            return false;
        }
        // Nothing can be written: edits that cannot be saved warn first.
        if let Some(group) = groups
            .iter()
            .copied()
            .find(|g| !self.can_save_in(*g) && self.editor_dirty_in(*g, cx))
        {
            self.warn_unsaveable(group, cx);
            return false;
        }
        for group in groups {
            if self.can_save_in(group)
                && let Some(host) = self.host_in_mut(group)
            {
                host.session.stop_autosave();
            }
        }
        true
    }

    /// Stop the autosave tick of every group.
    pub(super) fn stop_ticks(&mut self) {
        for state in self.groups.values_mut() {
            if let Some(host) = state.notes.as_mut() {
                host.session.stop_autosave();
            }
        }
    }

    /// Start the autosave tick of every group again.
    pub(super) fn restart_ticks(&mut self) {
        for group in self.group_ids() {
            self.restart_tick_in(group);
        }
    }

    /// A switch ended while a close waited on it, without leaving the open
    /// note clean to close on: give up this close (the flash says why). The
    /// next close tries the flush again, and warns before it discards.
    pub(super) fn continue_close(&mut self, cx: &mut Context<Self>) {
        if self.closing == Closing::Requested {
            self.closing = Closing::Open;
            self.restart_ticks();
            cx.notify();
        }
    }

    /// Flush the note of every group that saves before the window closes
    /// (all at once); close once everything is written. A switch still
    /// under way in one of them is waited for first.
    pub(super) fn close_flush(&mut self, cx: &mut Context<Self>) {
        let groups: Vec<GroupId> = self
            .group_ids_active_first()
            .into_iter()
            .filter(|g| self.can_save_in(*g))
            .collect();
        if groups
            .iter()
            .any(|g| self.host_in(*g).is_some_and(NoteHost::is_switching))
        {
            self.closing = Closing::Requested;
            return;
        }
        self.stop_ticks();
        let mut flushes = Vec::new();
        for group in groups {
            let Some((editor, host)) = self.editor_and_host_in(group) else {
                continue;
            };
            match host.session.prepare_flush(Some(&editor.read(cx).editor)) {
                Ok(flush) => flushes.push((group, flush)),
                Err(_) => {
                    // The flushes prepared before it go unrun, given back
                    // so their background saves finish; every group gets
                    // its tick back, and the flash says why the window
                    // stays.
                    tracing::warn!("close: a flush is still outstanding");
                    for (begun, flush) in flushes {
                        self.give_back_flush_in(begun, flush);
                    }
                    self.closing = Closing::Open;
                    self.restart_ticks();
                    self.flash_error(
                        "Another save is still running. The window stays open.".to_string(),
                        cx,
                    );
                    return;
                }
            }
        }
        self.closing = Closing::Flushing;
        if let Some(note) = flushes
            .first()
            .and_then(|(group, _)| self.host_in(*group))
            .and_then(|h| h.note.clone())
        {
            self.flash(format!("Saving {note}…"), cx);
        }
        let window = self.window;
        // Detached: removing the window drops the workspace, which must not
        // cancel the task that removes it.
        cx.spawn(async move |this, cx| {
            let results = crate::workspace::file_ops::run_all(flushes).await;
            // `Some(true)`: the app was asked to end; `Some(false)`: only the window
            // goes; `None`: the close did not go through.
            let ended = this
                .update(cx, |workspace, cx| {
                    if !workspace.close_flushed(results, cx) {
                        return None;
                    }
                    let quit = workspace.closes_app();
                    if quit {
                        workspace.end_app(cx);
                    } else {
                        workspace.flush_layout(cx);
                    }
                    Some(quit)
                })
                .unwrap_or(None);
            if ended == Some(false)
                && let Some(window) = window
            {
                cx.update_window(window, |_, window, _| window.remove_window())
                    .ok();
            }
        })
        .detach();
    }

    /// The close flush ended: close when everything is written; otherwise
    /// stay open and say why (closing again discards).
    pub(super) fn close_flushed(
        &mut self,
        results: Vec<(GroupId, Result<FlushResult, JoinError>)>,
        cx: &mut Context<Self>,
    ) -> bool {
        let outcomes: Vec<(GroupId, SaveOutcome)> = results
            .into_iter()
            .map(|(group, result)| (group, self.apply_flush(group, result, cx)))
            .collect();
        if let Some((group, problem)) = outcomes
            .iter()
            .find_map(|(group, outcome)| flush_problem(outcome).map(|p| (*group, p)))
        {
            let note = self.note_name_in(group);
            self.closing = Closing::Open;
            self.restart_ticks();
            self.arm_discard(cx);
            self.flash_error(
                format!("Couldn't save {note}: {problem}. Close again to discard your edits."),
                cx,
            );
            return false;
        }
        let dirty: Vec<GroupId> = self
            .group_ids_active_first()
            .into_iter()
            .filter(|g| self.editor_dirty_in(*g, cx))
            .collect();
        if !dirty.is_empty() {
            // Typed while the flush ran: write that too. Only when that flush
            // could capture the buffer (never on a pending open, which would
            // spin): otherwise say so and let the next close discard.
            let stuck = dirty.iter().copied().find(|group| {
                let flushed = outcomes
                    .iter()
                    .find(|(g, _)| g == group)
                    .map(|(_, outcome)| outcome);
                let pending = self
                    .host_in(*group)
                    .is_some_and(|h| h.session.is_open_pending());
                !rewritable(flushed) || pending
            });
            match stuck {
                None => self.close_flush(cx),
                Some(group) => {
                    self.closing = Closing::Open;
                    self.restart_ticks();
                    self.warn_unsaveable(group, cx);
                }
            }
            return false;
        }
        self.closing = Closing::Closed;
        true
    }
}
