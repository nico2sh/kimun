//! The `vault_info` tool: three facts an agent needs before it searches. Today's
//! date, the date keys the search tools accept, and the state of semantic search.

use std::path::PathBuf;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_core::error::VaultError;

use crate::rag::server_config;
use crate::rag::state::{HostOs, ServerReport, report, report_for};
use crate::settings::SharedSettings;

/// How long a health probe may take. A server that does not answer in time
/// counts as unreachable, below the client's own request timeout.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The config folder and the system a server report reads. Production code leaves it out and
/// gets the real config folder and the running system. Tests give a temp folder and a chosen
/// system, so they never touch the real home folder.
#[derive(Debug, Clone)]
pub(super) struct ServerEnv {
    pub(super) config_dir: PathBuf,
    pub(super) os: HostOs,
}

impl ServerEnv {
    /// The real config folder and the running system. A missing config folder gives an empty
    /// path, which holds no marker.
    fn production() -> Self {
        Self {
            config_dir: crate::settings::config_dir().unwrap_or_default(),
            os: HostOs::current(),
        }
    }
}

/// The state of the server and its hint, with one probe bounded by `timeout`. With no `env` it
/// reads the real config folder.
pub(super) async fn server_report(
    settings: Option<&SharedSettings>,
    env: Option<&ServerEnv>,
    timeout: Duration,
) -> ServerReport {
    if let (Some(settings), None) = (settings, env) {
        return report(settings, timeout).await;
    }
    let (address, token) = match settings.and_then(server_config) {
        Some((address, token)) => (Some(address), token),
        None => (None, None),
    };
    let production;
    let env = match env {
        Some(env) => env,
        None => {
            production = ServerEnv::production();
            &production
        }
    };
    report_for(
        address.as_deref(),
        token.as_deref(),
        &env.config_dir,
        env.os,
        timeout,
    )
    .await
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
/// not ready. The hint stays on one line.
pub(super) async fn render(
    vault: &NoteVault,
    settings: Option<&SharedSettings>,
    env: Option<&ServerEnv>,
) -> Result<String, VaultError> {
    let today = chrono::Local::now().format("%Y-%m-%d");
    let keys = searchable_date_keys(vault).await?;
    let report = server_report(settings, env, PROBE_TIMEOUT).await;
    let keys = if keys.is_empty() {
        "(none)".to_string()
    } else {
        keys.join(", ")
    };
    let mut out = format!(
        "today: {today}\ndate keys: {keys}\nsemantic search: {}",
        report.state
    );
    if let Some(hint) = report.hint {
        out.push_str("\nhint: ");
        out.push_str(&hint.to_string().replace(['\r', '\n'], " "));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::mcp::KimunHandler;
    use crate::rag::state::{INSTALL_LINE, ServerState};
    use crate::server_manage::marker::MARKER_FILE;
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

    /// An env with no marker and no `server.toml`: the folder does not exist.
    fn no_install_env() -> ServerEnv {
        ServerEnv {
            config_dir: std::env::temp_dir().join("kimun-mcp-test-no-config-dir"),
            os: HostOs::Linux,
        }
    }

    /// A config folder with a systemd marker and `server.toml` naming `port` as the service port.
    fn installed_env(port: u16) -> (TempDir, ServerEnv) {
        let config = TempDir::new().unwrap();
        std::fs::write(
            config.path().join(MARKER_FILE),
            format!(
                "installer = \"install-server.sh\"\ninstall_dir = '{}'\nservice = \"systemd\"\n",
                std::env::temp_dir().join("kimun-bin").display()
            ),
        )
        .unwrap();
        std::fs::write(
            config.path().join("server.toml"),
            format!("[server]\nport = {port}\n"),
        )
        .unwrap();
        let env = ServerEnv {
            config_dir: config.path().to_path_buf(),
            os: HostOs::Linux,
        };
        (config, env)
    }

    /// A port nothing listens on.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    async fn state_for(settings: Option<SharedSettings>) -> ServerState {
        server_report(settings.as_ref(), Some(&no_install_env()), PROBE_TIMEOUT)
            .await
            .state
    }

    #[tokio::test]
    async fn state_is_none_without_settings_or_url() {
        assert_eq!(state_for(None).await, ServerState::None);
        assert_eq!(state_for(Some(settings_for(None))).await, ServerState::None);
    }

    #[tokio::test]
    async fn state_is_unreachable_when_the_probe_fails() {
        let s = settings_for(Some(&down_url()));
        assert_eq!(state_for(Some(s)).await, ServerState::Unreachable);
    }

    #[tokio::test]
    async fn a_hung_server_is_unreachable_in_vault_info() {
        // Bound and never accepted: the connection opens, no reply ever comes.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let report = server_report(
            Some(&settings_for(Some(&url))),
            Some(&no_install_env()),
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(report.state, ServerState::Unreachable);
        drop(listener);
    }

    #[tokio::test]
    async fn state_is_unconfigured_without_an_embedder() {
        let url = health_server(r#"{"status":"ok","embedder":null,"llm_provider":null}"#);
        let state = state_for(Some(settings_for(Some(&url)))).await;
        assert_eq!(state, ServerState::Unconfigured);
    }

    #[tokio::test]
    async fn state_is_semantic_only_without_an_llm() {
        let url = health_server(r#"{"status":"ok","embedder":"fake","llm_provider":null}"#);
        let state = state_for(Some(settings_for(Some(&url)))).await;
        assert_eq!(state, ServerState::SemanticOnly);
    }

    #[tokio::test]
    async fn state_is_full_with_embedder_and_llm() {
        let url = health_server(r#"{"status":"ok","embedder":"fake","llm_provider":"x"}"#);
        let state = state_for(Some(settings_for(Some(&url)))).await;
        assert_eq!(state, ServerState::Full);
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
        let handler = KimunHandler::new(open_vault(&dir).await).with_server_env(no_install_env());
        let text = info_text(&handler).await;
        let lines: Vec<&str> = text.lines().collect();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(lines[0], format!("today: {today}"));
        assert_eq!(lines[1], "date keys: (none)");
        assert_eq!(lines[2], "semantic search: none");
        assert!(lines[3].starts_with("hint: "), "{text}");
        assert_eq!(lines.len(), 4, "{text}");
    }

    #[tokio::test]
    async fn vault_info_none_names_the_install_line_without_a_marker() {
        let dir = TempDir::new().unwrap();
        let handler = KimunHandler::new(open_vault(&dir).await)
            .with_settings(settings_for(None))
            .with_server_env(no_install_env());
        let text = info_text(&handler).await;
        assert!(
            text.lines().nth(3).unwrap().contains(INSTALL_LINE),
            "{text}"
        );
    }

    #[tokio::test]
    async fn vault_info_none_names_server_use_with_a_marker() {
        let dir = TempDir::new().unwrap();
        let (_config, env) = installed_env(7573);
        let handler = KimunHandler::new(open_vault(&dir).await)
            .with_settings(settings_for(None))
            .with_server_env(env);
        let text = info_text(&handler).await;
        let hint = text.lines().nth(3).unwrap();
        assert!(
            hint.contains("kimun server use http://localhost:7573"),
            "{text}"
        );
        assert!(!hint.contains(INSTALL_LINE), "{text}");
    }

    #[tokio::test]
    async fn vault_info_reports_stopped_with_the_start_command() {
        let dir = TempDir::new().unwrap();
        let port = closed_port();
        let (_config, env) = installed_env(port);
        let url = format!("http://127.0.0.1:{port}");
        let handler = KimunHandler::new(open_vault(&dir).await)
            .with_settings(settings_for(Some(&url)))
            .with_server_env(env);
        let text = info_text(&handler).await;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[2], "semantic search: stopped");
        assert!(
            lines[3].contains("systemctl --user start kimun-server.service"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn vault_info_hints_say_not_to_run_the_command() {
        let dir = TempDir::new().unwrap();
        let port = closed_port();
        let (_config, installed) = installed_env(port);
        let url = format!("http://127.0.0.1:{port}");
        let cases = [
            (None, no_install_env()),
            (Some(url.clone()), installed),
            (Some(down_url()), no_install_env()),
        ];
        for (url, env) in cases {
            let handler = KimunHandler::new(open_vault(&dir).await)
                .with_settings(settings_for(url.as_deref()))
                .with_server_env(env);
            let text = info_text(&handler).await;
            let hint = text.lines().nth(3).unwrap_or_default();
            assert!(hint.contains("Tell the user"), "{text}");
            assert!(
                hint.contains("not run") || hint.contains("Do not run"),
                "{text}"
            );
            assert_eq!(text.lines().count(), 4, "the hint is one line: {text}");
        }
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
        let handler = KimunHandler::new(vault)
            .with_settings(settings_for(Some(&url)))
            .with_server_env(no_install_env());
        let text = info_text(&handler).await;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[1], "date keys: journal, due, property:journal");
        assert_eq!(lines[2], "semantic search: full");
        assert_eq!(lines.len(), 3, "no hint when ready: {text}");
    }
}
