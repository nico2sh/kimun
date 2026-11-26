//! The `vault_info` tool: three facts an agent needs before it searches. Today's
//! date, the date keys the search tools accept, and the state of semantic search.

use std::fmt;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_core::error::VaultError;

use crate::server_client::sync::ServerCapability;
use crate::settings::SharedSettings;

/// What the agent can expect from `semantic_search`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SemanticState {
    /// No settings or no server URL configured.
    None,
    /// A server is configured but the health probe failed.
    Unreachable,
    /// The server answers but has no embedder.
    Unconfigured,
    /// The server has an embedder and no LLM.
    SemanticOnly,
    /// The server has an embedder and an LLM.
    Full,
}

impl fmt::Display for SemanticState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SemanticState::None => "none",
            SemanticState::Unreachable => "unreachable",
            SemanticState::Unconfigured => "unconfigured",
            SemanticState::SemanticOnly => "semantic-only",
            SemanticState::Full => "full",
        })
    }
}

impl SemanticState {
    /// One line naming the fix, for a state where semantic search is not ready.
    pub(super) fn hint(&self) -> Option<&'static str> {
        match self {
            SemanticState::None => {
                Some("Set kimun_server_url in the kimun configuration to enable semantic_search.")
            }
            SemanticState::Unreachable => Some(
                "The Kimun server does not answer. Start it, or check kimun_server_url in the kimun configuration.",
            ),
            SemanticState::Unconfigured => Some(
                "The Kimun server has no embedder. Configure an embedder on the server to enable semantic_search.",
            ),
            SemanticState::SemanticOnly | SemanticState::Full => None,
        }
    }
}

/// How long a health probe may take. A server that does not answer in time
/// counts as unreachable, below the client's own request timeout.
pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// [`semantic_state`], bounded by `timeout`. On timeout the state is
/// [`SemanticState::Unreachable`].
pub(super) async fn probe_state(
    settings: &SharedSettings,
    vault: &NoteVault,
    timeout: Duration,
) -> SemanticState {
    tokio::time::timeout(timeout, semantic_state(settings, vault))
        .await
        .unwrap_or(SemanticState::Unreachable)
}

/// Probes the configured server once. Uses the same health mapping as the sync
/// code, [`ServerCapability::from_health`].
pub(super) async fn semantic_state(settings: &SharedSettings, vault: &NoteVault) -> SemanticState {
    let Some(client) = crate::rag::rag_client(settings, vault).await else {
        return SemanticState::None;
    };
    match client.health().await {
        Err(_) => SemanticState::Unreachable,
        Ok(health) => match ServerCapability::from_health(&health) {
            ServerCapability::Unconfigured => SemanticState::Unconfigured,
            ServerCapability::SemanticOnly => SemanticState::SemanticOnly,
            ServerCapability::Full => SemanticState::Full,
        },
    }
}

/// The date keys exactly as the search tools accept them: `journal` for the
/// journal source, each property key as is, and `property:journal` for a date
/// property literally named journal.
pub(super) async fn searchable_date_keys(vault: &NoteVault) -> Result<Vec<String>, VaultError> {
    let mut keys = Vec::new();
    if vault.has_journal_dates().await? {
        keys.push("journal".to_string());
    }
    for key in vault.date_property_keys().await? {
        if key == "journal" {
            keys.push("property:journal".to_string());
        } else {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// The tool's text, one fact per line, with a hint line after a state that is
/// not ready.
pub(super) async fn render(
    vault: &NoteVault,
    settings: Option<&SharedSettings>,
) -> Result<String, VaultError> {
    let today = chrono::Local::now().format("%Y-%m-%d");
    let keys = searchable_date_keys(vault).await?;
    let state = match settings {
        Some(settings) => probe_state(settings, vault, PROBE_TIMEOUT).await,
        None => SemanticState::None,
    };
    let keys = if keys.is_empty() {
        "(none)".to_string()
    } else {
        keys.join(", ")
    };
    let mut out = format!("today: {today}\ndate keys: {keys}\nsemantic search: {state}");
    if let Some(hint) = state.hint() {
        out.push('\n');
        out.push_str(hint);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::mcp::KimunHandler;
    use crate::settings::AppSettings;
    use crate::settings::workspace_config::WorkspaceConfig;
    use kimun_core::VaultConfig;
    use kimun_core::nfs::VaultPath;
    use rmcp::model::RawContent;
    use std::io::{Read, Write};
    use std::sync::{Arc, RwLock};
    use tempfile::TempDir;

    /// Serves `health` for every request until the process ends.
    fn health_server(health: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    health.len(),
                    health
                );
            }
        });
        url
    }

    fn down_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    }

    fn settings_for(url: Option<&str>) -> SharedSettings {
        let mut config = WorkspaceConfig::new_empty();
        config.global.kimun_server_url = url.map(str::to_string);
        let mut settings = AppSettings::default();
        settings.workspace_config = Some(config);
        Arc::new(RwLock::new(settings))
    }

    async fn open_vault(dir: &TempDir) -> NoteVault {
        let vault = NoteVault::new(VaultConfig::new(crate::test_support::sys(dir.path())))
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        vault
    }

    async fn state_for(settings: Option<SharedSettings>) -> SemanticState {
        let dir = TempDir::new().unwrap();
        let vault = open_vault(&dir).await;
        match settings {
            Some(s) => semantic_state(&s, &vault).await,
            None => SemanticState::None,
        }
    }

    #[tokio::test]
    async fn state_is_none_without_settings_or_url() {
        assert_eq!(state_for(None).await, SemanticState::None);
        assert_eq!(
            state_for(Some(settings_for(None))).await,
            SemanticState::None
        );
    }

    #[tokio::test]
    async fn state_is_unreachable_when_the_probe_fails() {
        let s = settings_for(Some(&down_url()));
        assert_eq!(state_for(Some(s)).await, SemanticState::Unreachable);
    }

    #[tokio::test]
    async fn state_is_unconfigured_without_an_embedder() {
        let url = health_server(r#"{"status":"ok","embedder":null,"llm_provider":null}"#);
        let state = state_for(Some(settings_for(Some(&url)))).await;
        assert_eq!(state, SemanticState::Unconfigured);
    }

    #[tokio::test]
    async fn state_is_semantic_only_without_an_llm() {
        let url = health_server(r#"{"status":"ok","embedder":"fake","llm_provider":null}"#);
        let state = state_for(Some(settings_for(Some(&url)))).await;
        assert_eq!(state, SemanticState::SemanticOnly);
    }

    #[tokio::test]
    async fn state_is_full_with_embedder_and_llm() {
        let url = health_server(r#"{"status":"ok","embedder":"fake","llm_provider":"x"}"#);
        let state = state_for(Some(settings_for(Some(&url)))).await;
        assert_eq!(state, SemanticState::Full);
        assert_eq!(state.to_string(), "full");
    }

    async fn info_text(handler: &KimunHandler) -> String {
        let result = handler.vault_info().await.unwrap();
        match &result.content[0].raw {
            RawContent::Text(t) => t.text.clone(),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn vault_info_without_settings_reports_none_with_a_hint() {
        let dir = TempDir::new().unwrap();
        let handler = KimunHandler::new(open_vault(&dir).await);
        let text = info_text(&handler).await;
        let lines: Vec<&str> = text.lines().collect();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(lines[0], format!("today: {today}"));
        assert_eq!(lines[1], "date keys: (none)");
        assert_eq!(lines[2], "semantic search: none");
        assert!(lines[3].contains("kimun_server_url"), "{text}");
    }

    #[tokio::test]
    async fn vault_info_lists_date_keys_exactly_as_searchable() {
        let dir = TempDir::new().unwrap();
        let vault = open_vault(&dir).await;
        vault
            .create_note(&VaultPath::new("/journal/2026-11-20.md"), "# day\n")
            .await
            .unwrap();
        vault
            .create_note(
                &VaultPath::new("a.md"),
                "---\ndue: 2026-11-25\njournal: 2026-11-01\n---\nx\n",
            )
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        let url = health_server(r#"{"status":"ok","embedder":"fake","llm_provider":"x"}"#);
        let handler = KimunHandler::new(vault).with_settings(settings_for(Some(&url)));
        let text = info_text(&handler).await;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[1], "date keys: journal, due, property:journal");
        assert_eq!(lines[2], "semantic search: full");
        assert_eq!(lines.len(), 3, "no hint when ready: {text}");
    }
}
