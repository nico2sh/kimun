//! Background sync loop (P4). When a server URL is configured, a spawned task
//! keeps the current vault in sync and reports connection status to the UI.

use std::sync::Arc;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_server_client::{
    RagClient,
    sync::{RagSync, ServerCapability},
};
use tokio::task::JoinHandle;

use super::RagStatus;
use super::client::server_config;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::SharedSettings;

/// How often the background task flushes pending changes and refreshes status.
const SYNC_INTERVAL: Duration = Duration::from_secs(10);

/// Run a full reconcile (index-wide read + full-collection hash fetch) only
/// every Nth interval — the drain fast path handles the common case, and a
/// reconnect forces a reconcile immediately. At 10s × 30 that's ~5 min.
const RECONCILE_EVERY_N_TICKS: u32 = 30;

/// Spawns the background sync loop for `vault` if a RAG server is configured.
/// Returns the task handle (abort it when the vault is rebuilt), or `None` when
/// the feature is off. Status is delivered to the UI via [`AppEvent::RagStatus`].
pub fn spawn_rag_sync(
    vault: Arc<NoteVault>,
    settings: &SharedSettings,
    tx: AppTx,
) -> Option<JoinHandle<()>> {
    let (url, token) = server_config(settings)?;

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
        // Sticky across ticks: the last sync call was rejected with 401/403.
        // Suppresses the per-tick "syncing" flash while the token stays wrong.
        let mut auth_failed = false;

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

            // One probe drives reachability, capability, and auth (adr/0024):
            // offline, unconfigured (skip sync — the server rejects
            // everything), or semantic-only/full (llm_available gates Ask).
            let probe = match sync.probe().await {
                Some(p) => p,
                None => {
                    let _ = tx.send(AppEvent::RagStatus(RagStatus::Offline));
                    // Re-establish full consistency on the next successful tick.
                    ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;
                    auth_failed = false;
                    continue;
                }
            };
            if probe.capability == ServerCapability::Unconfigured {
                let _ = tx.send(AppEvent::RagStatus(RagStatus::NotConfigured));
                // When an embedder appears, start with a full reconcile.
                ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;
                continue;
            }
            // The server gates its API behind a token and none is configured:
            // every sync call would 401 (`/health` itself is un-gated, which
            // is why the probe still succeeded). Say so up front instead of
            // rediscovering it as a failure burst every tick.
            if probe.auth_required && token.is_none() {
                let _ = tx.send(AppEvent::RagStatus(RagStatus::Unauthorized));
                ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;
                continue;
            }
            let llm_available = probe.capability.llm_available();

            // The local index is empty while it (re)builds — a healed schema
            // on first launch, an upgrade, or a manual reindex. Syncing from
            // that snapshot is destructive (a reconcile reads "no notes" and
            // would wipe the server collection), so wait, and run a full
            // reconcile first thing once the index is filled.
            // While a wrong token keeps failing, skip the transient "syncing"
            // flash so the footer doesn't flicker syncing ↔ unauthorized.
            if !auth_failed {
                let _ = tx.send(AppEvent::RagStatus(RagStatus::Syncing { llm_available }));
            }
            if !sync.index_ready() {
                ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;
                continue;
            }

            let result = if ticks_since_reconcile >= RECONCILE_EVERY_N_TICKS {
                ticks_since_reconcile = 0;
                match sync.tick().await {
                    // Reconcile skipped: the index flipped to rebuilding
                    // between the gate above and the pass. Retry next tick.
                    Ok(false) => {
                        ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;
                        Ok(())
                    }
                    Ok(true) => Ok(()),
                    Err(e) => Err(e),
                }
            } else {
                ticks_since_reconcile += 1;
                sync.drain().await // fast path
            };
            let status = match result {
                Ok(()) => {
                    auth_failed = false;
                    RagStatus::Online { llm_available }
                }
                // The server rejected our token (401/403): a credentials
                // problem, not an unreachable server.
                Err(e) if e.is_auth() => {
                    log::warn!("RAG server rejected the configured token: {e}");
                    auth_failed = true;
                    RagStatus::Unauthorized
                }
                Err(e) => {
                    log::debug!("RAG sync failed: {e}");
                    auth_failed = false;
                    RagStatus::Offline
                }
            };
            let _ = tx.send(AppEvent::RagStatus(status));
        }
    }))
}
