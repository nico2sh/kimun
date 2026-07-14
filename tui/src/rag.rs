//! Wiring for the optional RAG server (P4). When a server URL is configured,
//! a background task keeps the current vault in sync and reports connection
//! status to the UI. Everything talks to the server through `kimun_server_client`;
//! the TUI only spawns the loop and renders status.

use std::sync::Arc;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_server_client::{
    RagClient,
    sync::{RagSync, ServerCapability},
};
use tokio::task::JoinHandle;

use crate::components::events::{AppEvent, AppTx};
use crate::settings::SharedSettings;

/// How often the background task flushes pending changes and refreshes status.
const SYNC_INTERVAL: Duration = Duration::from_secs(10);

/// Run a full reconcile (index-wide read + full-collection hash fetch) only
/// every Nth interval — the drain fast path handles the common case, and a
/// reconnect forces a reconcile immediately. At 10s × 30 that's ~5 min.
const RECONCILE_EVERY_N_TICKS: u32 = 30;

/// RAG connection status surfaced in the footer. `Disabled` (no server
/// configured) is never sent — the loop simply doesn't start — so the footer
/// shows nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagStatus {
    Disabled,
    Offline,
    /// Reachable but the server has no embedder configured (adr/0024): nothing
    /// works server-side, so the loop skips pushing and reconciling entirely —
    /// every call would 503 — and just reports the state.
    NotConfigured,
    /// Reachable, a sync pass in flight. `llm_available` carries whether the
    /// server has an LLM configured (question-answering possible), so Ask stays
    /// gated consistently while syncing.
    Syncing {
        llm_available: bool,
    },
    /// Reachable and idle. `llm_available` = the server has an LLM (Q&A on);
    /// `false` = semantic-only (search only).
    Online {
        llm_available: bool,
    },
}

/// A completed RAG answer delivered back to the answer overlay via
/// [`AppEvent::OverlayData(OverlayData::RagAnswerReady)`](crate::components::events::AppEvent).
#[derive(Debug, Clone)]
pub struct RagAnswer {
    pub answer: String,
    pub sources: Vec<RagSource>,
}

/// A cited source chunk — enough to render a row and open the note.
#[derive(Debug, Clone)]
pub struct RagSource {
    pub path: kimun_core::nfs::VaultPath,
    pub title: String,
}

impl RagStatus {
    /// Short footer label, or `None` when nothing should show.
    pub fn label(self) -> Option<&'static str> {
        match self {
            RagStatus::Disabled => None,
            RagStatus::Offline => Some("rag: offline"),
            RagStatus::NotConfigured => Some("rag: not configured"),
            RagStatus::Syncing { .. } => Some("rag: syncing"),
            RagStatus::Online { .. } => Some("rag: online"),
        }
    }

    /// Whether question-answering (Ask) is available right now: the server is
    /// reachable AND has an LLM configured. `false` when offline, disabled, or
    /// connected to a semantic-only server — the Ask overlay is hidden in those
    /// cases (adr/0022).
    pub fn llm_available(self) -> bool {
        matches!(
            self,
            RagStatus::Online {
                llm_available: true
            } | RagStatus::Syncing {
                llm_available: true
            }
        )
    }
}

/// Spawns the background sync loop for `vault` if a RAG server is configured.
/// Returns the task handle (abort it when the vault is rebuilt), or `None` when
/// the feature is off. Status is delivered to the UI via [`AppEvent::RagStatus`].
pub fn spawn_rag_sync(
    vault: Arc<NoteVault>,
    settings: &SharedSettings,
    tx: AppTx,
) -> Option<JoinHandle<()>> {
    let (url, token) = {
        let settings = settings.read().ok()?;
        let global = &settings.workspace_config.as_ref()?.global;
        (
            global.kimun_server_url.clone()?,
            global.kimun_server_token.clone(),
        )
    };

    Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        // Don't stack missed ticks into a back-to-back burst if a slow tick
        // overruns the interval (large vault / slow server).
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Resolve the vault id (which registers the observer) lazily so a
        // transient failure just retries next tick instead of killing sync for
        // the whole session.
        let mut sync: Option<RagSync> = None;
        // Force a reconcile on the first successful tick and after any offline
        // gap; drain-only in between.
        let mut ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;

        loop {
            interval.tick().await;

            if sync.is_none() {
                match vault.vault_id().await {
                    Ok(id) => {
                        let client = RagClient::new(url.clone(), token.clone(), id.to_string());
                        sync = Some(RagSync::new(vault.clone(), client));
                    }
                    Err(e) => {
                        log::warn!("RAG: cannot read vault id (will retry): {e}");
                        let _ = tx.send(AppEvent::RagStatus(RagStatus::Offline));
                        continue;
                    }
                }
            }
            let sync = sync.as_ref().expect("sync established above");

            // One probe drives reachability and capability (adr/0024): offline,
            // unconfigured (skip sync — the server rejects everything), or
            // semantic-only/full (llm_available gates Ask).
            let capability = match sync.probe().await {
                Some(c) => c,
                None => {
                    let _ = tx.send(AppEvent::RagStatus(RagStatus::Offline));
                    // Re-establish full consistency on the next successful tick.
                    ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;
                    continue;
                }
            };
            if capability == ServerCapability::Unconfigured {
                let _ = tx.send(AppEvent::RagStatus(RagStatus::NotConfigured));
                // When an embedder appears, start with a full reconcile.
                ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;
                continue;
            }
            let llm_available = capability.llm_available();

            let _ = tx.send(AppEvent::RagStatus(RagStatus::Syncing { llm_available }));
            let result = if ticks_since_reconcile >= RECONCILE_EVERY_N_TICKS {
                ticks_since_reconcile = 0;
                sync.tick().await // drain + reconcile
            } else {
                ticks_since_reconcile += 1;
                sync.drain().await // fast path
            };
            let status = match result {
                Ok(()) => RagStatus::Online { llm_available },
                Err(e) => {
                    log::debug!("RAG sync failed: {e}");
                    RagStatus::Offline
                }
            };
            let _ = tx.send(AppEvent::RagStatus(status));
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_configured_status_labels_and_gates() {
        assert_eq!(
            RagStatus::NotConfigured.label(),
            Some("rag: not configured")
        );
        assert!(!RagStatus::NotConfigured.llm_available());
    }
}
