//! Background sync loop (P4). When a server URL is configured, a spawned task
//! keeps the current vault in sync and reports connection status to the UI.
//!
//! The per-tick decisions — probe→capability→auth gating, the sticky
//! auth-failure flag, and the reconcile-vs-drain cadence — live in the pure
//! [`Cadence`] step functions so the whole status policy is testable without a
//! live server. The spawned task is only the shell: timer, client calls, and
//! channel sends.

use std::sync::Arc;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_server_client::{
    RagClient,
    sync::{RagSync, ServerCapability, ServerProbe},
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

/// What a tick decided to do, given the probe. Statuses inside are for the
/// shell to emit verbatim.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Emit the status and skip this tick's sync entirely.
    Skip(RagStatus),
    /// Optionally flash `Syncing`, then wait: the local index is rebuilding
    /// and syncing from an empty snapshot would wipe the server collection.
    Wait { flash: Option<RagStatus> },
    /// Optionally flash `Syncing`, then sync — a full reconcile tick when
    /// `reconcile`, the drain fast path otherwise.
    Run {
        flash: Option<RagStatus>,
        reconcile: bool,
    },
}

/// The sync call's result, stripped to what the status policy needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The pass ran to completion.
    Synced,
    /// Pass skipped: the index flipped to rebuilding between the gate and the
    /// call. Nothing was synced.
    SkippedRebuild,
    /// The server rejected our token (401/403): a credentials problem, not an
    /// unreachable server.
    AuthRejected,
    /// Any other sync failure — treated as unreachable.
    Failed,
}

/// The sync loop's per-tick state machine, kept pure so the policy is
/// table-testable. One `plan` before the sync call, one `settle` after.
struct Cadence {
    /// Force a reconcile on the first successful tick and after any offline
    /// gap; drain-only in between.
    ticks_since_reconcile: u32,
    /// Sticky across ticks: the last sync call was rejected with 401/403.
    /// Suppresses the per-tick "syncing" flash while the token stays wrong.
    auth_failed: bool,
    /// The probe's Ask capability, remembered by `plan` so `settle` reports a
    /// status consistent with the flashes emitted the same tick — the one
    /// derivation lives here, not in the shell.
    llm_available: bool,
}

impl Cadence {
    fn new() -> Self {
        Self {
            ticks_since_reconcile: RECONCILE_EVERY_N_TICKS,
            auth_failed: false,
            llm_available: false,
        }
    }

    /// Re-establish full consistency on the next successful tick.
    fn force_reconcile(&mut self) {
        self.ticks_since_reconcile = RECONCILE_EVERY_N_TICKS;
    }

    /// Decide this tick's action from the probe (adr/0024): offline,
    /// unconfigured (skip sync — the server rejects everything), unauthorized
    /// up front, or semantic-only/full (llm_available gates Ask).
    fn plan(&mut self, probe: Option<&ServerProbe>, has_token: bool, index_ready: bool) -> Plan {
        let probe = match probe {
            Some(p) => p,
            None => {
                self.force_reconcile();
                self.auth_failed = false;
                return Plan::Skip(RagStatus::Offline);
            }
        };
        if probe.capability == ServerCapability::Unconfigured {
            // When an embedder appears, start with a full reconcile.
            self.force_reconcile();
            return Plan::Skip(RagStatus::NotConfigured);
        }
        // The server gates its API behind a token and none is configured:
        // every sync call would 401 (`/health` itself is un-gated, which
        // is why the probe still succeeded). Say so up front instead of
        // rediscovering it as a failure burst every tick.
        if probe.auth_required && !has_token {
            self.force_reconcile();
            return Plan::Skip(RagStatus::Unauthorized);
        }
        let llm_available = probe.capability.llm_available();
        self.llm_available = llm_available;

        // While a wrong token keeps failing, skip the transient "syncing"
        // flash so the footer doesn't flicker syncing ↔ unauthorized.
        let flash = (!self.auth_failed).then_some(RagStatus::Syncing { llm_available });

        // The local index is empty while it (re)builds — a healed schema
        // on first launch, an upgrade, or a manual reindex. Syncing from
        // that snapshot is destructive (a reconcile reads "no notes" and
        // would wipe the server collection), so wait, and run a full
        // reconcile first thing once the index is filled.
        if !index_ready {
            self.force_reconcile();
            return Plan::Wait { flash };
        }

        let reconcile = self.ticks_since_reconcile >= RECONCILE_EVERY_N_TICKS;
        if reconcile {
            self.ticks_since_reconcile = 0;
        } else {
            self.ticks_since_reconcile += 1;
        }
        Plan::Run { flash, reconcile }
    }

    /// Fold the sync call's outcome into the status to report, using the
    /// capability `plan` recorded this tick.
    fn settle(&mut self, outcome: Outcome) -> RagStatus {
        let llm_available = self.llm_available;
        match outcome {
            Outcome::Synced => {
                self.auth_failed = false;
                RagStatus::Online { llm_available }
            }
            // Keep reporting Syncing (not Online) and force a full reconcile
            // once the index is ready again.
            Outcome::SkippedRebuild => {
                self.force_reconcile();
                self.auth_failed = false;
                RagStatus::Syncing { llm_available }
            }
            Outcome::AuthRejected => {
                self.auth_failed = true;
                RagStatus::Unauthorized
            }
            Outcome::Failed => {
                self.auth_failed = false;
                RagStatus::Offline
            }
        }
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
        let mut cadence = Cadence::new();

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

            // One probe drives reachability, capability, and auth (adr/0024).
            let probe = sync.probe().await;

            let reconcile = match cadence.plan(probe.as_ref(), token.is_some(), sync.index_ready())
            {
                Plan::Skip(status) => {
                    let _ = tx.send(AppEvent::RagStatus(status));
                    continue;
                }
                Plan::Wait { flash } => {
                    if let Some(status) = flash {
                        let _ = tx.send(AppEvent::RagStatus(status));
                    }
                    continue;
                }
                Plan::Run { flash, reconcile } => {
                    if let Some(status) = flash {
                        let _ = tx.send(AppEvent::RagStatus(status));
                    }
                    reconcile
                }
            };

            let result = if reconcile {
                sync.tick().await // drain + reconcile
            } else {
                sync.drain().await // fast path
            };
            let outcome = match &result {
                Ok(true) => Outcome::Synced,
                Ok(false) => Outcome::SkippedRebuild,
                Err(e) if e.is_auth() => {
                    log::warn!("RAG server rejected the configured token: {e}");
                    Outcome::AuthRejected
                }
                Err(e) => {
                    log::debug!("RAG sync failed: {e}");
                    Outcome::Failed
                }
            };
            let status = cadence.settle(outcome);
            let _ = tx.send(AppEvent::RagStatus(status));
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(capability: ServerCapability, auth_required: bool) -> ServerProbe {
        ServerProbe {
            capability,
            auth_required,
        }
    }

    #[test]
    fn offline_probe_reports_offline_and_forces_reconcile() {
        let mut c = Cadence::new();
        // Get past the initial forced reconcile so the reset is observable.
        c.plan(Some(&probe(ServerCapability::Full, false)), true, true);
        assert_eq!(c.plan(None, true, true), Plan::Skip(RagStatus::Offline));
        // The reconnect tick reconciles immediately.
        assert_eq!(
            c.plan(Some(&probe(ServerCapability::Full, false)), true, true),
            Plan::Run {
                flash: Some(RagStatus::Syncing {
                    llm_available: true
                }),
                reconcile: true,
            }
        );
    }

    #[test]
    fn unconfigured_server_skips_sync() {
        let mut c = Cadence::new();
        assert_eq!(
            c.plan(
                Some(&probe(ServerCapability::Unconfigured, false)),
                true,
                true
            ),
            Plan::Skip(RagStatus::NotConfigured)
        );
    }

    #[test]
    fn auth_required_without_token_reports_unauthorized_up_front() {
        let mut c = Cadence::new();
        assert_eq!(
            c.plan(Some(&probe(ServerCapability::Full, true)), false, true),
            Plan::Skip(RagStatus::Unauthorized)
        );
    }

    #[test]
    fn auth_required_with_token_syncs() {
        let mut c = Cadence::new();
        assert!(matches!(
            c.plan(Some(&probe(ServerCapability::Full, true)), true, true),
            Plan::Run { .. }
        ));
    }

    #[test]
    fn index_not_ready_waits_and_reconciles_once_filled() {
        let mut c = Cadence::new();
        // Drain a few ticks first so the pending reconcile is the wait's doing.
        c.plan(Some(&probe(ServerCapability::Full, false)), true, true);
        c.plan(Some(&probe(ServerCapability::Full, false)), true, true);
        assert_eq!(
            c.plan(Some(&probe(ServerCapability::Full, false)), true, false),
            Plan::Wait {
                flash: Some(RagStatus::Syncing {
                    llm_available: true
                })
            }
        );
        assert_eq!(
            c.plan(Some(&probe(ServerCapability::Full, false)), true, true),
            Plan::Run {
                flash: Some(RagStatus::Syncing {
                    llm_available: true
                }),
                reconcile: true,
            }
        );
    }

    #[test]
    fn reconcile_cadence_first_tick_then_drains_then_reconciles_again() {
        let mut c = Cadence::new();
        let p = probe(ServerCapability::Full, false);
        // First successful tick reconciles.
        assert!(matches!(
            c.plan(Some(&p), true, true),
            Plan::Run {
                reconcile: true,
                ..
            }
        ));
        // The next N-1 ticks drain.
        for _ in 0..RECONCILE_EVERY_N_TICKS {
            assert!(matches!(
                c.plan(Some(&p), true, true),
                Plan::Run {
                    reconcile: false,
                    ..
                }
            ));
        }
        // The Nth tick reconciles again.
        assert!(matches!(
            c.plan(Some(&p), true, true),
            Plan::Run {
                reconcile: true,
                ..
            }
        ));
    }

    #[test]
    fn auth_rejection_is_sticky_and_suppresses_the_syncing_flash() {
        let mut c = Cadence::new();
        let p = probe(ServerCapability::Full, true);
        assert!(matches!(c.plan(Some(&p), true, true), Plan::Run { .. }));
        assert_eq!(c.settle(Outcome::AuthRejected), RagStatus::Unauthorized);
        // While the token stays wrong: no syncing flash (no footer flicker).
        assert_eq!(
            c.plan(Some(&p), true, true),
            Plan::Run {
                flash: None,
                reconcile: false,
            }
        );
        // A successful pass clears the stickiness.
        assert_eq!(
            c.settle(Outcome::Synced),
            RagStatus::Online {
                llm_available: true
            }
        );
        assert!(matches!(
            c.plan(Some(&p), true, true),
            Plan::Run { flash: Some(_), .. }
        ));
    }

    #[test]
    fn skipped_pass_reports_syncing_and_forces_reconcile() {
        let mut c = Cadence::new();
        let p = probe(ServerCapability::SemanticOnly, false);
        c.plan(Some(&p), true, true);
        assert_eq!(
            c.settle(Outcome::SkippedRebuild),
            RagStatus::Syncing {
                llm_available: false
            }
        );
        // The next runnable tick is a full reconcile.
        assert!(matches!(
            c.plan(Some(&p), true, true),
            Plan::Run {
                reconcile: true,
                ..
            }
        ));
    }

    #[test]
    fn sync_failure_reports_offline() {
        let mut c = Cadence::new();
        let p = probe(ServerCapability::Full, false);
        c.plan(Some(&p), true, true);
        assert_eq!(c.settle(Outcome::Failed), RagStatus::Offline);
    }
}
