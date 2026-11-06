//! The **kimün server** seen from the window (M7c G9): the background sync
//! the TUI runs, started here too when a server is configured, and what its
//! status reports turn into — the SEM rail cell, the SEM view's empty
//! line, the Thread's client (Ask), and the Footer's "rag: …" segment.
//!
//! **Lifecycle.** With `kimun_server_url` set, opening the vault starts
//! `rag::spawn_rag_sync` on the window's tokio runtime (decision 1b: the
//! same sync as the TUI: it probes the server every ten seconds, reports
//! `AppMessage::RagStatus`, and pushes the vault's notes). Without one,
//! nothing starts and the status stays `Disabled`: no SEM cell, no Footer
//! segment. The window holds exactly one sync task ([`TaskSlot`]): a new
//! start aborts the one before it, and the workspace going away aborts it.
//!
//! **Respawn (M8b).** A vault change ([`reopen`](super::reopen)) stops the
//! sync with the vault it leaves and resets the status to `Disabled`; the
//! entry into the new vault starts [`Workspace::start_rag_sync`] again.
//!
//! **Not handled here.** `AppMessage::ServerUpdate` (the server's update
//! notice) waits for M10's update surfaces; the window ignores it.
//!
//! [`TaskSlot`]: kimun_notes::logic::task_slot::TaskSlot

use std::sync::Arc;

use gpui_kit::Context;
use kimun_core::NoteVault;
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::rag_caps::{self, Caps};
use kimun_notes::rag::{self, RagStatus};
use kimun_notes::settings::SharedSettings;

use super::Workspace;

impl Workspace {
    /// Start the server sync for `vault` when `settings` name a server,
    /// replacing (aborting) any sync running; with none configured, stop
    /// any and start nothing. Must run with the tokio runtime entered.
    pub(super) fn start_rag_sync(
        &mut self,
        vault: Arc<NoteVault>,
        settings: &SharedSettings,
        tx: AppTx,
    ) {
        // Before anything new: one sync per vault per process. Not left to
        // `replace`, which aborts the old task only after its argument has
        // spawned the new one (and the unconfigured branch replaces nothing).
        self.rag_sync.stop();
        if rag::rag_configured(settings) {
            self.rag_sync
                .replace(rag::spawn_rag_sync(vault, settings, tx));
        }
    }

    /// What the server can do changed (`AppMessage::RagStatus`): the Footer
    /// words it; a capability that flipped rebuilds the rail (a view placed
    /// in a drawer stays put, only its cell comes or goes; the rail cursor
    /// stays on its view when it can); SEM learns whether it can search.
    pub(crate) fn on_rag_status(&mut self, status: RagStatus, cx: &mut Context<Self>) {
        self.rag_status = status;
        self.status.rag = status.label();
        let now = Caps::from_status(status);
        let flip = Caps::changed(self.rag_caps, now);
        if flip.any() {
            let cursor = self.rail_cursor_view();
            self.rag_caps = now;
            self.place_rail_cursor(cursor);
        }
        // The Thread's client follows ASK ([`ask`](super::ask)).
        if flip.ask {
            self.refresh_ask_client(cx);
        }
        let unavailable = self.semantic_block();
        if let Some(semantic) = self.semantic.clone() {
            semantic.update(cx, |view, cx| view.set_unavailable(unavailable, cx));
        }
        cx.notify();
    }

    /// Why SEM cannot be opened now, in the TUI's words, or `None` when the
    /// server can search.
    pub(crate) fn semantic_block(&self) -> Option<&'static str> {
        let configured = self.settings.as_ref().is_some_and(rag::rag_configured);
        rag_caps::semantic_unavailable_message(self.rag_status, configured)
    }

    /// What the server can do now: SEM and ASK.
    #[cfg(test)]
    pub(crate) fn rag_caps(&self) -> Caps {
        self.rag_caps
    }

    /// Whether the server sync task is running (tests).
    #[cfg(test)]
    pub(crate) fn rag_sync_running(&self) -> bool {
        self.rag_sync.is_running()
    }
}
