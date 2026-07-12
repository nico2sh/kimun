//! Wiring for the optional RAG server (P4). When a server URL is configured,
//! a background task keeps the current vault in sync and reports connection
//! status to the UI. Everything talks to the server through `kimun_rag_client`;
//! the TUI only spawns the loop and renders status.

use std::sync::Arc;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_rag_client::{RagClient, sync::RagSync};
use tokio::task::JoinHandle;

use crate::components::events::{AppEvent, AppTx};
use crate::settings::SharedSettings;

/// How often the background task drains + reconciles and refreshes status.
const SYNC_INTERVAL: Duration = Duration::from_secs(10);

/// RAG connection status surfaced in the footer. `Disabled` (no server
/// configured) is never sent — the loop simply doesn't start — so the footer
/// shows nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagStatus {
    Disabled,
    Offline,
    Syncing,
    Online,
}

impl RagStatus {
    /// Short footer label, or `None` when nothing should show.
    pub fn label(self) -> Option<&'static str> {
        match self {
            RagStatus::Disabled => None,
            RagStatus::Offline => Some("rag: offline"),
            RagStatus::Syncing => Some("rag: syncing"),
            RagStatus::Online => Some("rag: online"),
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
    let (url, token) = {
        let settings = settings.read().ok()?;
        let global = &settings.workspace_config.as_ref()?.global;
        (
            global.rag_server_url.clone()?,
            global.rag_server_token.clone(),
        )
    };

    Some(tokio::spawn(async move {
        let vault_id = match vault.vault_id().await {
            Ok(id) => id.to_string(),
            Err(e) => {
                log::warn!("RAG: cannot read vault id: {e}");
                let _ = tx.send(AppEvent::RagStatus(RagStatus::Offline));
                return;
            }
        };
        let client = RagClient::new(url, token, vault_id);
        let sync = RagSync::new(vault, client);

        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        loop {
            interval.tick().await;
            if sync.online().await {
                let _ = tx.send(AppEvent::RagStatus(RagStatus::Syncing));
                let status = match sync.tick().await {
                    Ok(()) => RagStatus::Online,
                    Err(e) => {
                        log::debug!("RAG sync tick failed: {e}");
                        RagStatus::Offline
                    }
                };
                let _ = tx.send(AppEvent::RagStatus(status));
            } else {
                let _ = tx.send(AppEvent::RagStatus(RagStatus::Offline));
            }
        }
    }))
}
