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
//! **The server's own update notice** (`AppMessage::ServerUpdate`, the TUI's
//! footer hint and dialog): one toast per news, saying which release. When the
//! install script put the server on this machine and set up a service for it
//! (`kimun_notes::server_manage`, ADR-0059), the toast carries `Update now`: the
//! app downloads, verifies and installs the release, restarts the service, and
//! rolls back if the new server does not come up. The same update is offered
//! in Preferences > Server, which also opens the server's own settings page.
//! For every other server (remote, Docker, Cargo, installed before the
//! marker) the toast says to update it the way it was installed. All the
//! server work is `server_manage`'s: this file shows a button and some text.
//! The app's own update flow is [`update`](super::update)'s.
//!
//! [`TaskSlot`]: kimun_notes::logic::task_slot::TaskSlot

use std::ffi::OsStr;
use std::sync::Arc;

use gpui_kit::{Context, Task};
use kimun_core::NoteVault;
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::rag_caps::{self, Caps};
use kimun_notes::rag::{self, RagStatus};
use kimun_notes::server_client::sync::ServerUpdate;
use kimun_notes::server_manage::{ServerInstall, ServerUpdates, Unavailable};
use kimun_notes::settings::SharedSettings;

use super::{ToastAction, Workspace};
use crate::tasks::blocking;

/// The toast's button when the app can update the server.
pub(crate) const UPDATE_SERVER_NOW_LABEL: &str = "Update now";
/// The toast's button after a failed update.
pub(crate) const UPDATE_SERVER_RETRY_LABEL: &str = "Retry";
/// What the window says once the update started.
pub(crate) const UPDATING_SERVER_TEXT: &str =
    "Updating the Kimün server. This can take up to two minutes.";
/// What the toast says for a server the app cannot update.
const THE_OLD_WAY: &str = "Update it the way you installed it.";

/// What Preferences shows about the server's update: nothing, an update the app
/// can install, or one that is being installed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum ServerUpdateRow {
    #[default]
    None,
    /// The release the app can install now; `None` for a server too old to name one.
    Available(Option<String>),
    Updating,
}

/// The window's server update: how the server is managed, and the work under way.
pub(crate) struct ServerHost {
    backend: Option<Arc<dyn ServerUpdates>>,
    /// The release the app may install now (see [`ServerUpdateRow::Available`]).
    offer: Option<Option<String>>,
    probing: Option<Task<()>>,
    running: Option<Task<()>>,
}

impl ServerHost {
    pub(crate) fn new(backend: Option<Arc<dyn ServerUpdates>>) -> Self {
        Self {
            backend,
            offer: None,
            probing: None,
            running: None,
        }
    }

    /// What Preferences shows now.
    pub(crate) fn row(&self) -> ServerUpdateRow {
        if self.running.is_some() {
            return ServerUpdateRow::Updating;
        }
        match &self.offer {
            Some(version) => ServerUpdateRow::Available(version.clone()),
            None => ServerUpdateRow::None,
        }
    }
}

/// The backend a window gets: the real one, or none under `cfg(test)` (a test never
/// touches a service or the network).
pub(crate) fn default_backend() -> Option<Arc<dyn ServerUpdates>> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        let dir = kimun_notes::settings::config_dir().ok()?;
        Some(Arc::new(
            kimun_notes::server_manage::SystemServerUpdates::new(dir),
        ))
    }
}

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
        // A new vault's sync reports its own news; a respawn over the same
        // vault (Preferences saved, onboarding finished) keeps the memory,
        // or the same release would be toasted again.
        let root = vault.workspace_path().as_path().to_path_buf();
        if self.rag_sync_root.as_ref() != Some(&root) {
            self.server_update = None;
            self.rag_sync_root = Some(root);
        }
        if rag::rag_configured(settings) {
            self.rag_sync
                .replace(rag::spawn_rag_sync(vault, settings, tx));
        }
    }

    /// The server reported a newer release of itself (or none, any more):
    /// toast it once. The sync sends only changes, but a respawn starts from
    /// scratch, hence the memory. When the app can update this server, the
    /// toast says so and offers it (after a look at the machine, off the UI
    /// thread). Needs the tokio runtime entered.
    pub(crate) fn on_server_update(
        &mut self,
        update: Option<ServerUpdate>,
        cx: &mut Context<Self>,
    ) {
        if update == self.server_update {
            return;
        }
        self.server_update = update.clone();
        let Some(update) = update else {
            self.server_host.probing = None;
            self.set_server_offer(None, cx);
            return;
        };
        let (what, version) = match update {
            ServerUpdate::Newer(version) => (
                format!("Kimün server {version} is available"),
                Some(version),
            ),
            ServerUpdate::Legacy => ("A newer Kimün server is available".to_string(), None),
        };
        let (Some(backend), Some(url)) = (self.server_host.backend.clone(), self.server_url())
        else {
            self.flash(format!("{what}. {THE_OLD_WAY}"), cx);
            return;
        };
        let job = blocking(move || backend.available(&url));
        self.server_host.probing = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |w, cx| w.server_probe_landed(what, version, result, cx))
                .ok();
        }));
    }

    fn server_probe_landed(
        &mut self,
        what: String,
        version: Option<String>,
        result: Result<Result<ServerInstall, Unavailable>, String>,
        cx: &mut Context<Self>,
    ) {
        self.server_host.probing = None;
        match result {
            Ok(Ok(_)) => {
                self.set_server_offer(Some(version), cx);
                let action =
                    ToastAction::new(UPDATE_SERVER_NOW_LABEL, |w, _, cx| w.update_server(cx));
                self.flash_with_action(format!("{what}."), action, cx);
            }
            other => {
                match other {
                    Ok(Err(why)) => tracing::debug!("the app does not update this server: {why}"),
                    Err(e) => tracing::debug!("the server update check failed: {e}"),
                    Ok(Ok(_)) => {}
                }
                self.set_server_offer(None, cx);
                self.flash(format!("{what}. {THE_OLD_WAY}"), cx);
            }
        }
    }

    /// `Update now` (the toast, Preferences): update the server, off the UI thread. The open
    /// note is not involved: the app keeps running. A second call while one runs does nothing.
    pub(crate) fn update_server(&mut self, cx: &mut Context<Self>) {
        if self.server_host.running.is_some() {
            return;
        }
        let (Some(backend), Some(url)) = (self.server_host.backend.clone(), self.server_url())
        else {
            return;
        };
        self.flash(UPDATING_SERVER_TEXT, cx);
        let job = blocking(move || backend.update(&url, &|_| {}).map_err(|e| e.to_string()));
        self.server_host.running = Some(cx.spawn(async move |this, cx| {
            let result = job.await.and_then(|r| r);
            this.update(cx, |w, cx| w.server_update_landed(result, cx))
                .ok();
        }));
        self.push_server_row(cx);
    }

    fn server_update_landed(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        self.server_host.running = None;
        match result {
            Ok(version) => {
                self.server_host.offer = None;
                self.flash(format!("Kimün server updated to {version}."), cx);
            }
            Err(e) => {
                tracing::warn!("the server update failed: {e}");
                let retry =
                    ToastAction::new(UPDATE_SERVER_RETRY_LABEL, |w, _, cx| w.update_server(cx));
                self.flash_error_with_action(
                    format!("Could not update the Kimün server: {e}."),
                    retry,
                    cx,
                );
            }
        }
        self.push_server_row(cx);
    }

    /// The release the app can install now changed.
    fn set_server_offer(&mut self, offer: Option<Option<String>>, cx: &mut Context<Self>) {
        self.server_host.offer = offer;
        self.push_server_row(cx);
    }

    /// Tell an open Preferences what to show about the server's update.
    pub(crate) fn push_server_row(&mut self, cx: &mut Context<Self>) {
        let row = self.server_host.row();
        if let Some(view) = self.preferences.clone() {
            view.update(cx, |view, cx| view.set_server_update(row, cx));
        }
    }

    /// The configured server URL, or `None` when no server is set.
    fn server_url(&self) -> Option<String> {
        let settings = self.settings.as_ref()?.read().ok()?;
        kimun_notes::logic::preferences::configured_server_url(&settings)
    }

    /// Preferences' `Open server settings`: the server's own page, in the browser. `url` is the
    /// address in the draft, which may differ from the saved one: it lets you look before you save.
    pub(crate) fn open_server_settings(&mut self, url: &str, cx: &mut Context<Self>) {
        if let Err(e) = (self.opener)(OsStr::new(url)) {
            tracing::warn!("could not open {url}: {e}");
            self.flash_error(format!("Could not open {url}: {e}"), cx);
        }
    }

    /// Replace the server update backend (tests).
    #[cfg(test)]
    pub(crate) fn set_server_backend(&mut self, backend: Arc<dyn ServerUpdates>) {
        self.server_host.backend = Some(backend);
    }

    /// What the server can do changed (`AppMessage::RagStatus`): the Footer
    /// words it; a capability that flipped rebuilds the rail (a view placed
    /// in a slot stays put, only its cell comes or goes; the rail cursor
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
