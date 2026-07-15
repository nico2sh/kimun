//! Server configuration reading and [`RagClient`] construction — the single
//! place the TUI turns settings into a live client handle.

use kimun_core::NoteVault;
use kimun_server_client::RagClient;

use crate::settings::SharedSettings;

/// The configured server endpoint: `(url, token)`, or `None` when no server
/// URL is set. All RAG surfaces derive their client from this one read.
pub(super) fn server_config(settings: &SharedSettings) -> Option<(String, Option<String>)> {
    let settings = settings.read().ok()?;
    let global = &settings.workspace_config.as_ref()?.global;
    Some((
        global.kimun_server_url.clone()?,
        global.kimun_server_token.clone(),
    ))
}

/// Builds a [`RagClient`] for the current vault from config, or `None` when no
/// server URL is configured. Shared by every RAG query surface.
pub async fn rag_client(settings: &SharedSettings, vault: &NoteVault) -> Option<RagClient> {
    let (url, token) = server_config(settings)?;
    let vault_id = vault.vault_id().await.ok()?;
    Some(RagClient::new(url, token, vault_id.to_string()))
}

/// Whether a RAG server is configured (drives showing the semantic surface at
/// all). Reachability is a separate, runtime concern.
pub fn rag_configured(settings: &SharedSettings) -> bool {
    server_config(settings).is_some()
}
