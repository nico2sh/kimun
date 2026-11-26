//! The state of the Kimün server as the agent connector reports it, and the hint that names the
//! next step (ADR 0066).
//!
//! This module only reads. It probes `/health`, reads the install marker and `server.toml`, and
//! builds text. It never runs a command and never starts or changes anything on the machine.
//! Every hint tells the agent to pass the step to the user and not to run it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::server_client::sync::ServerCapability;
use crate::server_client::{RagClient, RagError};
use crate::server_manage::marker::{self, BINARY};
use crate::server_manage::{LAUNCHD_LABEL, ServiceKind, UNIT, is_local_url, port_of, service_port};
use crate::settings::SharedSettings;

/// The line that installs the server with a service, on Linux and macOS.
pub const INSTALL_LINE: &str =
    "curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service";

/// The address of a server installed on this machine with the default port.
pub const DEFAULT_ADDRESS: &str = "http://localhost:7573";

/// The server docs, with the Docker and WSL2 instructions for systems with no install script.
pub const SERVER_DOCS_URL: &str = "https://kimun.2co.dev/using-kimun/server/";

/// The closing words of a hint that names commands.
const TELL_USER_RUN: &str = "Tell the user to run this. Do not run it yourself.";

/// The closing words of a hint that names no command.
const TELL_USER: &str = "Tell the user. Do not run any command yourself.";

/// What the agent can expect from `semantic_search`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    /// No server address is configured.
    None,
    /// The install script put the server on this machine, the address points here, and the
    /// server does not answer.
    Stopped,
    /// An address is configured and nothing answers, with no sign of an install here.
    Unreachable,
    /// The server answers but has no embedder.
    Unconfigured,
    /// The server has an embedder and no LLM.
    SemanticOnly,
    /// The server has an embedder and an LLM.
    Full,
}

impl fmt::Display for ServerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ServerState::None => "none",
            ServerState::Stopped => "stopped",
            ServerState::Unreachable => "unreachable",
            ServerState::Unconfigured => "unconfigured",
            ServerState::SemanticOnly => "semantic-only",
            ServerState::Full => "full",
        })
    }
}

/// The system the hint is for. It decides whether the install script exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Linux,
    Macos,
    Windows,
    Other,
}

impl HostOs {
    /// The system this binary runs on.
    pub fn current() -> Self {
        if cfg!(target_os = "linux") {
            HostOs::Linux
        } else if cfg!(target_os = "macos") {
            HostOs::Macos
        } else if cfg!(windows) {
            HostOs::Windows
        } else {
            HostOs::Other
        }
    }
}

/// What the install marker shows about a server on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallEvidence {
    /// No usable marker: no install here, or one the app cannot see (Docker, Cargo, remote).
    None,
    /// The script installed the server with a service.
    Service {
        kind: ServiceKind,
        install_dir: PathBuf,
    },
    /// The script installed the server and set up no service.
    NoService { install_dir: PathBuf },
}

/// Reads the install marker in `config_dir`. A missing or unusable marker is
/// [`InstallEvidence::None`].
pub fn install_evidence(config_dir: &Path) -> InstallEvidence {
    match marker::load_recorded(config_dir) {
        Ok(marker::RecordedInstall {
            install_dir,
            service: Some(kind),
        }) => InstallEvidence::Service { kind, install_dir },
        Ok(marker::RecordedInstall {
            install_dir,
            service: None,
        }) => InstallEvidence::NoService { install_dir },
        Err(_) => InstallEvidence::None,
    }
}

/// What a hint needs to know besides the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintContext {
    pub os: HostOs,
    pub evidence: InstallEvidence,
    /// Whether a server address is configured.
    pub address_set: bool,
    /// The port the installed service listens on, when it is known.
    pub service_port: Option<u16>,
}

/// A hint in two parts: the step itself, and the closing words that address the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    /// The step, written for whoever reads it.
    pub body: String,
    /// The words that tell the agent to pass the step to the user and not to run it.
    pub tail: &'static str,
}

impl Hint {
    /// The hint for a person who reads it in the terminal: the step without the words for the
    /// agent.
    pub fn for_person(&self) -> &str {
        &self.body
    }
}

impl fmt::Display for Hint {
    /// The full hint for an agent: the step and the closing words.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.body, self.tail)
    }
}

/// The next step for `state` as the agent reads it, or `None` when semantic search is ready.
/// Pure: it only builds text.
pub fn hint(state: ServerState, ctx: &HintContext) -> Option<String> {
    hint_parts(state, ctx).map(|h| h.to_string())
}

/// The next step for `state` in two parts, or `None` when semantic search is ready. Pure: it only
/// builds text.
pub fn hint_parts(state: ServerState, ctx: &HintContext) -> Option<Hint> {
    let server_use = format!("`kimun server use {DEFAULT_ADDRESS}`");
    let local_address = match ctx.service_port {
        Some(port) => format!("http://localhost:{port}"),
        None => DEFAULT_ADDRESS.to_string(),
    };
    let unreachable = (
        "The Kimun server does not answer. Start it, or check kimun_server_url in the kimun configuration.".to_string(),
        TELL_USER,
    );
    let (body, tail) = match (state, &ctx.evidence) {
        (ServerState::None, InstallEvidence::None) => {
            let install = match ctx.os {
                HostOs::Linux | HostOs::Macos => format!("To install it, run `{INSTALL_LINE}`."),
                HostOs::Windows | HostOs::Other => format!(
                    "This system has no install script. To run it with Docker or WSL2, follow {SERVER_DOCS_URL}."
                ),
            };
            (
                format!(
                    "No Kimun server is set up, so semantic_search is not available. {install} Then run {server_use} and restart your agent app."
                ),
                TELL_USER_RUN,
            )
        }
        (ServerState::None, _) => {
            let set_address = if ctx.address_set {
                String::new()
            } else {
                " but no server address is set".to_string()
            };
            (
                format!(
                    "A Kimun server is installed on this machine{set_address}. Run `kimun server use {local_address}` and restart your agent app."
                ),
                TELL_USER_RUN,
            )
        }
        (ServerState::Stopped, InstallEvidence::Service { kind, .. }) => {
            let start = match kind {
                ServiceKind::Systemd => format!("Start it with `systemctl --user start {UNIT}`."),
                ServiceKind::Launchd => format!(
                    "Start it with `launchctl kickstart gui/$(id -u)/{LAUNCHD_LABEL}`. If that command says it cannot find the service, run `launchctl load ~/Library/LaunchAgents/{LAUNCHD_LABEL}.plist` instead."
                ),
            };
            (
                format!(
                    "The Kimun server is installed on this machine but does not answer. {start}"
                ),
                TELL_USER_RUN,
            )
        }
        (ServerState::Stopped, InstallEvidence::NoService { install_dir }) => (
            format!(
                "The Kimun server is installed on this machine without a service, and it does not answer. Run `{}` in a terminal and keep the terminal open, or run the installer again with a service: `{INSTALL_LINE}`.",
                install_dir.join(BINARY).display()
            ),
            TELL_USER_RUN,
        ),
        // Stopped needs evidence, so this case only comes from a caller that skips `classify`.
        (ServerState::Stopped, InstallEvidence::None) | (ServerState::Unreachable, _) => {
            unreachable
        }
        (ServerState::Unconfigured, _) => (
            "The Kimun server has no embedder. Configure an embedder on the server to enable semantic_search.".to_string(),
            TELL_USER,
        ),
        (ServerState::SemanticOnly | ServerState::Full, _) => return None,
    };
    Some(Hint { body, tail })
}

/// The result of one `/health` probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Nothing listens: the connection was refused or could not be made.
    Refused,
    /// Something listens but gave no usable answer: a time-out, an HTTP error status, or a
    /// reply that does not parse. A server that runs but hangs lands here.
    Failed,
    /// The server answered.
    Capability(ServerCapability),
}

/// The state for a probe result. Pure. Stopped needs all four facts: the connection was refused,
/// the address is local, the marker shows an install, and the address uses the service's port. A
/// container on another port, a remote address, or a server that hangs or answers with an error
/// is unreachable, because a start command does not help a server that already runs.
pub fn classify(
    address: Option<&str>,
    probe: Probe,
    evidence: &InstallEvidence,
    service_port: u16,
) -> ServerState {
    let Some(address) = address else {
        return ServerState::None;
    };
    match probe {
        Probe::Capability(ServerCapability::Unconfigured) => ServerState::Unconfigured,
        Probe::Capability(ServerCapability::SemanticOnly) => ServerState::SemanticOnly,
        Probe::Capability(ServerCapability::Full) => ServerState::Full,
        Probe::Failed => ServerState::Unreachable,
        Probe::Refused => {
            let installed_here = *evidence != InstallEvidence::None;
            if installed_here && is_local_url(address) && port_of(address) == Some(service_port) {
                ServerState::Stopped
            } else {
                ServerState::Unreachable
            }
        }
    }
}

/// A state with its hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerReport {
    pub state: ServerState,
    pub hint: Option<Hint>,
}

/// Probes the server at `address` once, bounded by `timeout`, and reads the marker and
/// `server.toml` in `config_dir`. Needs no vault: the health call does not use the vault id.
pub async fn report_for(
    address: Option<&str>,
    token: Option<&str>,
    config_dir: &Path,
    os: HostOs,
    timeout: Duration,
) -> ServerReport {
    report_with(
        address,
        token,
        install_evidence(config_dir),
        Some(service_port(config_dir)),
        os,
        timeout,
    )
    .await
}

/// [`report_for`] with the configured address and token and the real config folder.
pub async fn report(settings: &SharedSettings, timeout: Duration) -> ServerReport {
    let (address, token) = match super::client::server_config(settings) {
        Some((address, token)) => (Some(address), token),
        None => (None, None),
    };
    let os = HostOs::current();
    match crate::settings::config_dir() {
        Ok(dir) => report_for(address.as_deref(), token.as_deref(), &dir, os, timeout).await,
        // No config folder means no marker and no known service port.
        Err(_) => {
            report_with(
                address.as_deref(),
                token.as_deref(),
                InstallEvidence::None,
                None,
                os,
                timeout,
            )
            .await
        }
    }
}

async fn report_with(
    address: Option<&str>,
    token: Option<&str>,
    evidence: InstallEvidence,
    service_port: Option<u16>,
    os: HostOs,
    timeout: Duration,
) -> ServerReport {
    let probe = match address {
        None => Probe::Failed,
        Some(address) => probe(address, token, timeout).await,
    };
    let state = match (service_port, probe) {
        (Some(port), _) => classify(address, probe, &evidence, port),
        // With no known port nothing shows that the address is the service, so a refused
        // connection counts as a plain failure.
        (None, Probe::Refused) => classify(address, Probe::Failed, &evidence, 0),
        (None, _) => classify(address, probe, &evidence, 0),
    };
    let ctx = HintContext {
        os,
        evidence,
        address_set: address.is_some(),
        service_port,
    };
    ServerReport {
        state,
        hint: hint_parts(state, &ctx),
    }
}

/// One `/health` call. A refused connection is [`Probe::Refused`]. A time-out (a connect time-out
/// too), an HTTP error status or a reply that does not parse is [`Probe::Failed`].
async fn probe(address: &str, token: Option<&str>, timeout: Duration) -> Probe {
    let client = RagClient::new(address, token.map(str::to_string), String::new());
    match tokio::time::timeout(timeout, client.health()).await {
        Ok(Ok(health)) => Probe::Capability(ServerCapability::from_health(&health)),
        Ok(Err(RagError::Http(e))) if connection_refused(e.is_connect(), e.is_timeout()) => {
            Probe::Refused
        }
        Ok(Err(_)) | Err(_) => Probe::Failed,
    }
}

/// Whether a request error means that nothing listens. A connect error that is a time-out is not
/// a refusal: something may be there and slow, so it counts as a failure.
fn connection_refused(is_connect: bool, is_timeout: bool) -> bool {
    is_connect && !is_timeout
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_client::sync::ServerCapability;
    use crate::server_manage::ServiceKind;
    use crate::server_manage::marker::MARKER_FILE;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// The time-out for a probe that expects a refused connection. On Windows a closed local
    /// port can take one to two seconds to refuse. On Linux and macOS the refusal comes at once.
    const REFUSED_TIMEOUT: Duration = Duration::from_secs(10);

    fn install_dir() -> PathBuf {
        std::env::temp_dir().join("kimun-bin")
    }

    fn ctx(os: HostOs, evidence: InstallEvidence, address_set: bool) -> HintContext {
        HintContext {
            os,
            evidence,
            address_set,
            service_port: None,
        }
    }

    fn evidence_kinds() -> Vec<InstallEvidence> {
        vec![
            InstallEvidence::None,
            InstallEvidence::Service {
                kind: ServiceKind::Systemd,
                install_dir: install_dir(),
            },
            InstallEvidence::Service {
                kind: ServiceKind::Launchd,
                install_dir: install_dir(),
            },
            InstallEvidence::NoService {
                install_dir: install_dir(),
            },
        ]
    }

    fn assert_tells_the_user(text: &str) {
        assert!(text.contains("Tell the user"), "{text}");
        assert!(text.contains("not run"), "{text}");
    }

    /// The words every hint for a state must hold, given the context.
    fn expected_parts(state: ServerState, ctx: &HintContext) -> Vec<String> {
        let server_use = format!("kimun server use {DEFAULT_ADDRESS}");
        let restart = "restart your agent app".to_string();
        match (state, &ctx.evidence) {
            (ServerState::None, InstallEvidence::None) => match ctx.os {
                HostOs::Linux | HostOs::Macos => vec![INSTALL_LINE.into(), server_use, restart],
                HostOs::Windows | HostOs::Other => {
                    vec![
                        SERVER_DOCS_URL.into(),
                        "Docker".into(),
                        "WSL2".into(),
                        server_use,
                        restart,
                    ]
                }
            },
            (ServerState::None, _) => {
                let address = match ctx.service_port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => DEFAULT_ADDRESS.to_string(),
                };
                vec![format!("kimun server use {address}"), restart]
            }
            (ServerState::Stopped, InstallEvidence::Service { kind, .. }) => match kind {
                ServiceKind::Systemd => vec!["systemctl --user start kimun-server.service".into()],
                ServiceKind::Launchd => vec![
                    "launchctl kickstart gui/$(id -u)/dev.2co.kimun-server".into(),
                    "If that command says it cannot find the service, run `launchctl load ~/Library/LaunchAgents/dev.2co.kimun-server.plist` instead.".into(),
                ],
            },
            (ServerState::Stopped, InstallEvidence::NoService { install_dir }) => vec![
                install_dir.join("kimun-server").display().to_string(),
                "terminal".into(),
                INSTALL_LINE.into(),
            ],
            (ServerState::Stopped, InstallEvidence::None) | (ServerState::Unreachable, _) => {
                vec![
                    "The Kimun server does not answer. Start it, or check kimun_server_url in the kimun configuration.".into(),
                ]
            }
            (ServerState::Unconfigured, _) => vec![
                "The Kimun server has no embedder. Configure an embedder on the server to enable semantic_search.".into(),
            ],
            (ServerState::SemanticOnly | ServerState::Full, _) => vec![],
        }
    }

    #[test]
    fn hint_table_over_every_state_evidence_and_system() {
        let states = [
            ServerState::None,
            ServerState::Stopped,
            ServerState::Unreachable,
            ServerState::Unconfigured,
            ServerState::SemanticOnly,
            ServerState::Full,
        ];
        for os in [HostOs::Linux, HostOs::Macos, HostOs::Windows, HostOs::Other] {
            for evidence in evidence_kinds() {
                for (state, port) in states
                    .iter()
                    .flat_map(|s| [(*s, None), (*s, Some(7573)), (*s, Some(9123))])
                {
                    let address_set = state != ServerState::None;
                    let mut c = ctx(os, evidence.clone(), address_set);
                    c.service_port = port;
                    let hint = hint(state, &c);
                    let parts = expected_parts(state, &c);
                    if parts.is_empty() {
                        assert_eq!(hint, None, "{state} {os:?} {evidence:?}");
                        continue;
                    }
                    let text =
                        hint.unwrap_or_else(|| panic!("no hint for {state} {os:?} {evidence:?}"));
                    for part in &parts {
                        assert!(
                            text.contains(part.as_str()),
                            "{state} {os:?} {evidence:?}: missing {part:?} in {text}"
                        );
                    }
                    assert_tells_the_user(&text);
                    // The part for a person holds the same step without the words for the agent.
                    let parts_of = hint_parts(state, &c).unwrap();
                    let person = parts_of.for_person();
                    for part in &parts {
                        assert!(person.contains(part.as_str()), "{part:?} in {person}");
                    }
                    assert!(!person.contains("Tell the user"), "{person}");
                    assert!(!person.contains("not run"), "{person}");
                }
            }
        }
    }

    #[test]
    fn only_a_connect_error_that_is_not_a_time_out_counts_as_refused() {
        assert!(connection_refused(true, false));
        assert!(!connection_refused(true, true));
        assert!(!connection_refused(false, true));
        assert!(!connection_refused(false, false));
    }

    #[test]
    fn the_install_line_is_not_given_on_windows() {
        let text = hint(
            ServerState::None,
            &ctx(HostOs::Windows, InstallEvidence::None, false),
        )
        .unwrap();
        assert!(!text.contains("install-server.sh"), "{text}");
    }

    #[test]
    fn state_labels_keep_the_old_wording() {
        let labels: Vec<String> = [
            ServerState::None,
            ServerState::Stopped,
            ServerState::Unreachable,
            ServerState::Unconfigured,
            ServerState::SemanticOnly,
            ServerState::Full,
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            labels,
            [
                "none",
                "stopped",
                "unreachable",
                "unconfigured",
                "semantic-only",
                "full"
            ]
        );
    }

    fn systemd() -> InstallEvidence {
        InstallEvidence::Service {
            kind: ServiceKind::Systemd,
            install_dir: install_dir(),
        }
    }

    #[test]
    fn classify_stopped_needs_local_address_matching_port_and_evidence() {
        let no_service = InstallEvidence::NoService {
            install_dir: install_dir(),
        };
        for address in [
            "http://localhost:7573",
            "http://127.0.0.1:7573",
            "http://[::1]:7573/",
        ] {
            assert_eq!(
                classify(Some(address), Probe::Refused, &systemd(), 7573),
                ServerState::Stopped,
                "{address}"
            );
            assert_eq!(
                classify(Some(address), Probe::Refused, &no_service, 7573),
                ServerState::Stopped,
                "{address}"
            );
        }
        // A server that is there but hangs, or answers with an error, is not stopped.
        assert_eq!(
            classify(
                Some("http://localhost:7573"),
                Probe::Failed,
                &systemd(),
                7573
            ),
            ServerState::Unreachable
        );
        // Each condition alone is not enough.
        assert_eq!(
            classify(
                Some("http://localhost:7573"),
                Probe::Refused,
                &InstallEvidence::None,
                7573
            ),
            ServerState::Unreachable
        );
        assert_eq!(
            classify(
                Some("http://localhost:7573"),
                Probe::Refused,
                &systemd(),
                8000
            ),
            ServerState::Unreachable
        );
        // A server that answers is never stopped.
        assert_eq!(
            classify(
                Some("http://localhost:7573"),
                Probe::Capability(ServerCapability::Full),
                &systemd(),
                7573
            ),
            ServerState::Full
        );
    }

    #[test]
    fn classify_remote_address_is_unreachable() {
        for address in ["http://example.com:7573", "http://192.168.1.4:7573"] {
            assert_eq!(
                classify(Some(address), Probe::Refused, &systemd(), 7573),
                ServerState::Unreachable,
                "{address}"
            );
        }
    }

    #[test]
    fn classify_docker_on_another_port_is_unreachable() {
        assert_eq!(
            classify(
                Some("http://localhost:8080"),
                Probe::Refused,
                &systemd(),
                7573
            ),
            ServerState::Unreachable
        );
    }

    #[test]
    fn classify_no_address_is_none() {
        assert_eq!(
            classify(None, Probe::Refused, &systemd(), 7573),
            ServerState::None
        );
        assert_eq!(
            classify(
                None,
                Probe::Capability(ServerCapability::Full),
                &InstallEvidence::None,
                7573
            ),
            ServerState::None
        );
    }

    #[test]
    fn classify_maps_each_capability() {
        let addr = Some("http://example.com:7573");
        let none = &InstallEvidence::None;
        assert_eq!(
            classify(
                addr,
                Probe::Capability(ServerCapability::Unconfigured),
                none,
                7573
            ),
            ServerState::Unconfigured
        );
        assert_eq!(
            classify(
                addr,
                Probe::Capability(ServerCapability::SemanticOnly),
                none,
                7573
            ),
            ServerState::SemanticOnly
        );
        assert_eq!(
            classify(addr, Probe::Capability(ServerCapability::Full), none, 7573),
            ServerState::Full
        );
    }

    fn write_marker(dir: &Path, service: &str) {
        // A TOML literal string (single quotes) takes a Windows path as it is.
        std::fs::write(
            dir.join(MARKER_FILE),
            format!(
                "installer = \"install-server.sh\"\ninstall_dir = '{}'\nservice = \"{service}\"\n",
                install_dir().display()
            ),
        )
        .unwrap();
    }

    #[test]
    fn install_evidence_reads_a_marker_with_and_without_a_service() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(install_evidence(dir.path()), InstallEvidence::None);

        write_marker(dir.path(), "systemd");
        assert_eq!(install_evidence(dir.path()), systemd());

        write_marker(dir.path(), "launchd");
        assert_eq!(
            install_evidence(dir.path()),
            InstallEvidence::Service {
                kind: ServiceKind::Launchd,
                install_dir: install_dir(),
            }
        );

        write_marker(dir.path(), "none");
        assert_eq!(
            install_evidence(dir.path()),
            InstallEvidence::NoService {
                install_dir: install_dir(),
            }
        );

        std::fs::write(dir.path().join(MARKER_FILE), "garbage = [").unwrap();
        assert_eq!(install_evidence(dir.path()), InstallEvidence::None);
    }

    /// A config folder with a systemd marker and `server.toml` naming `port` as the service port.
    fn config_with_marker_on(port: u16) -> tempfile::TempDir {
        let config = tempfile::tempdir().unwrap();
        write_marker(config.path(), "systemd");
        std::fs::write(
            config.path().join("server.toml"),
            format!("[server]\nport = {port}\n"),
        )
        .unwrap();
        config
    }

    #[tokio::test]
    async fn a_hung_local_server_with_a_marker_is_unreachable() {
        // Bound and never accepted: the connection opens, no reply ever comes.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = config_with_marker_on(port);
        let url = format!("http://127.0.0.1:{port}");
        let report = report_for(
            Some(&url),
            None,
            config.path(),
            HostOs::Linux,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(report.state, ServerState::Unreachable);
        drop(listener);
    }

    #[tokio::test]
    async fn a_local_server_answering_500_with_a_marker_is_unreachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let config = config_with_marker_on(port);
        let url = format!("http://127.0.0.1:{port}");
        let report = report_for(
            Some(&url),
            None,
            config.path(),
            HostOs::Linux,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!(report.state, ServerState::Unreachable);
    }

    #[tokio::test]
    async fn a_closed_local_port_with_a_marker_is_stopped() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let config = config_with_marker_on(port);
        let url = format!("http://127.0.0.1:{port}");
        let report = report_for(
            Some(&url),
            None,
            config.path(),
            HostOs::Linux,
            REFUSED_TIMEOUT,
        )
        .await;
        assert_eq!(report.state, ServerState::Stopped);
    }

    #[test]
    fn the_server_use_hint_names_the_service_port() {
        let mut c = ctx(HostOs::Linux, systemd(), false);
        c.service_port = Some(9123);
        let text = hint(ServerState::None, &c).unwrap();
        assert!(
            text.contains("kimun server use http://localhost:9123"),
            "{text}"
        );
        c.service_port = None;
        let text = hint(ServerState::None, &c).unwrap();
        assert!(text.contains(&format!("kimun server use {DEFAULT_ADDRESS}")));
    }

    #[tokio::test]
    async fn report_for_without_an_address_names_the_port_from_server_toml() {
        let config = config_with_marker_on(9123);
        let report = report_for(
            None,
            None,
            config.path(),
            HostOs::Linux,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(report.state, ServerState::None);
        assert!(
            report
                .hint
                .unwrap()
                .to_string()
                .contains("kimun server use http://localhost:9123")
        );
    }

    #[tokio::test]
    async fn report_for_times_out_as_unreachable() {
        // Bound and never accepted: the connection opens, no reply ever comes.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let config = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let report = report_for(
            Some(&url),
            None,
            config.path(),
            HostOs::Linux,
            Duration::from_millis(50),
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(report.state, ServerState::Unreachable);
        assert_tells_the_user(&report.hint.unwrap().to_string());
        drop(listener);
    }

    #[tokio::test]
    async fn report_for_reads_the_service_port_from_server_toml() {
        // A closed local port, named as the service port in server.toml.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let config = tempfile::tempdir().unwrap();
        write_marker(config.path(), "systemd");
        let url = format!("http://127.0.0.1:{port}");
        let timeout = REFUSED_TIMEOUT;

        // The default service port does not match, so nothing shows this is our server.
        let report = report_for(Some(&url), None, config.path(), HostOs::Linux, timeout).await;
        assert_eq!(report.state, ServerState::Unreachable);

        std::fs::write(
            config.path().join("server.toml"),
            format!("[server]\nport = {port}\n"),
        )
        .unwrap();
        let report = report_for(Some(&url), None, config.path(), HostOs::Linux, timeout).await;
        assert_eq!(report.state, ServerState::Stopped);
        assert!(
            report
                .hint
                .unwrap()
                .to_string()
                .contains("systemctl --user start kimun-server.service")
        );
    }

    #[tokio::test]
    async fn report_for_without_an_address_is_none_with_the_install_line() {
        let config = tempfile::tempdir().unwrap();
        let report = report_for(
            None,
            None,
            config.path(),
            HostOs::Macos,
            Duration::from_millis(50),
        )
        .await;
        assert_eq!(report.state, ServerState::None);
        assert!(report.hint.unwrap().to_string().contains(INSTALL_LINE));
    }
}
