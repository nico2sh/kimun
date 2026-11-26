// tui/src/cli/commands/server.rs
//
// Server CLI commands: `use` saves the Kimün server address, `status` reports its state.
// Neither one changes the machine beyond the settings file (ADR 0066).

use std::path::Path;
use std::time::Duration;

use clap::Subcommand;
use color_eyre::eyre::{Result, eyre};

use crate::rag::state::{HostOs, ServerReport, report_for};
use crate::settings::AppSettings;

use super::mcp::PROBE_TIMEOUT;

/// The last line of `server use`.
const RESTART_LINE: &str = "Restart your agent app so the MCP server picks up the change.";

#[derive(Subcommand, Debug)]
pub enum ServerSubcommand {
    /// Save the address of the Kimün server, or remove it with --off
    Use {
        /// Address of the server, for example http://localhost:7573
        #[arg(conflicts_with = "off")]
        url: Option<String>,
        /// Token for the server. Without it, a saved token is kept only for the same address
        #[arg(long, conflicts_with = "off")]
        token: Option<String>,
        /// Remove the saved address and token
        #[arg(long, conflicts_with_all = ["url", "token"])]
        off: bool,
    },
    /// Show the state of the Kimün server and the next step
    Status,
}

/// Accepts only `http://` or `https://` addresses. Trims whitespace and trailing slashes.
pub fn normalize_url(input: &str) -> Result<String, String> {
    const FORM: &str = "Use an address like http://localhost:7573 or https://kimun.example.com.";
    let trimmed = input.trim().trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"));
    match rest {
        Some(host) if !host.is_empty() && !host.starts_with(['/', ':', '?', '#']) => {
            Ok(trimmed.to_string())
        }
        Some(_) => Err(format!("The address has no host. {FORM}")),
        None if trimmed.is_empty() => Err(format!("The address is empty. {FORM}")),
        None => Err(format!(
            "The address must start with http:// or https://. {FORM}"
        )),
    }
}

pub async fn run(sub: ServerSubcommand, settings: &mut AppSettings) -> Result<()> {
    let config_dir = crate::settings::config_dir().map_err(|e| eyre!("{e}"))?;
    let out = run_in(sub, settings, &config_dir, HostOs::current(), PROBE_TIMEOUT).await?;
    println!("{out}");
    Ok(())
}

/// The state and the hint for a person at the terminal. The hint leaves out the words that tell
/// an agent not to run the step, because here the reader is the one who runs it.
fn describe(report: &ServerReport) -> String {
    match &report.hint {
        Some(hint) => format!("Server state: {}\n{}", report.state, hint.for_person()),
        None => format!("Server state: {}", report.state),
    }
}

fn saved(settings: &AppSettings) -> (Option<String>, Option<String>) {
    match settings.workspace_config.as_ref() {
        Some(wc) => (
            wc.global.kimun_server_url.clone(),
            wc.global.kimun_server_token.clone(),
        ),
        None => (None, None),
    }
}

/// The work of `run`, with the config folder, system and timeout given. Returns the text to print.
async fn run_in(
    sub: ServerSubcommand,
    settings: &mut AppSettings,
    config_dir: &Path,
    os: HostOs,
    timeout: Duration,
) -> Result<String> {
    match sub {
        ServerSubcommand::Status => {
            let (url, token) = saved(settings);
            let report =
                report_for(url.as_deref(), token.as_deref(), config_dir, os, timeout).await;
            Ok(describe(&report))
        }
        ServerSubcommand::Use { url, token, off } => {
            if !off && url.is_none() {
                return Err(eyre!(
                    "Give a server address or --off. Use: kimun server use <url> [--token <token>]"
                ));
            }
            let wc = settings.workspace_config.as_mut().ok_or_else(|| {
                eyre!("No workspaces configured. Run `kimun workspace init <path>` first.")
            })?;
            if off {
                wc.global.kimun_server_url = None;
                wc.global.kimun_server_token = None;
                settings.save_to_disk()?;
                return Ok("Server address and token removed.".to_string());
            }
            let address = normalize_url(&url.expect("checked above")).map_err(|e| eyre!(e))?;
            let same = wc
                .global
                .kimun_server_url
                .as_deref()
                .is_some_and(|old| normalize_url(old).as_deref() == Ok(address.as_str()));
            let new_token = match token {
                Some(t) => Some(t),
                None if same => wc.global.kimun_server_token.take(),
                None => None,
            };
            wc.global.kimun_server_url = Some(address.clone());
            wc.global.kimun_server_token = new_token.clone();
            settings.save_to_disk()?;
            let report = report_for(
                Some(&address),
                new_token.as_deref(),
                config_dir,
                os,
                timeout,
            )
            .await;
            Ok(format!(
                "Server address saved: {address}\n{}\n{RESTART_LINE}",
                describe(&report)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::workspace_config::WorkspaceConfig;
    use clap::Parser;

    const SHORT: Duration = Duration::from_millis(500);
    /// A local port nothing listens on.
    const DOWN: &str = "http://127.0.0.1:1";

    #[derive(Parser, Debug)]
    struct Wrap {
        #[command(subcommand)]
        sub: ServerSubcommand,
    }

    struct Env {
        _dir: tempfile::TempDir,
        config_dir: std::path::PathBuf,
        settings: AppSettings,
    }

    fn env(url: Option<&str>, token: Option<&str>) -> Env {
        let dir = tempfile::TempDir::new().unwrap();
        let config_dir = dir.path().canonicalize().unwrap();
        let mut wc = WorkspaceConfig::new_empty();
        wc.global.kimun_server_url = url.map(String::from);
        wc.global.kimun_server_token = token.map(String::from);
        let mut settings = AppSettings::default();
        settings.config_file = Some(config_dir.join("config.toml"));
        settings.workspace_config = Some(wc);
        Env {
            _dir: dir,
            config_dir,
            settings,
        }
    }

    async fn go(e: &mut Env, sub: ServerSubcommand) -> Result<String> {
        run_in(sub, &mut e.settings, &e.config_dir, HostOs::Linux, SHORT).await
    }

    fn use_cmd(url: Option<&str>, token: Option<&str>, off: bool) -> ServerSubcommand {
        ServerSubcommand::Use {
            url: url.map(String::from),
            token: token.map(String::from),
            off,
        }
    }

    fn current(e: &Env) -> (Option<String>, Option<String>) {
        saved(&e.settings)
    }

    #[test]
    fn normalize_url_accepts_http_and_https_and_trims_slashes() {
        assert_eq!(
            normalize_url("http://localhost:7573").unwrap(),
            "http://localhost:7573"
        );
        assert_eq!(
            normalize_url("  https://a.example.com//  ").unwrap(),
            "https://a.example.com"
        );
    }

    #[test]
    fn normalize_url_rejects_other_schemes_and_empty() {
        for bad in [
            "",
            "   ",
            "ftp://host",
            "localhost:7573",
            "http://",
            "https:///x",
            "file:///etc",
        ] {
            let err = normalize_url(bad).unwrap_err();
            assert!(err.contains("http://"), "{bad}: {err}");
        }
    }

    #[tokio::test]
    async fn use_saves_the_address_when_the_server_is_down() {
        let mut e = env(None, None);
        let out = go(&mut e, use_cmd(Some("  http://127.0.0.1:1/ "), None, false))
            .await
            .unwrap();
        assert_eq!(current(&e).0.as_deref(), Some(DOWN));
        assert!(e.config_dir.join("config.toml").exists());
        assert!(out.contains("Server state:"), "{out}");
        assert!(out.ends_with(RESTART_LINE), "{out}");
    }

    #[tokio::test]
    async fn use_with_a_new_address_and_no_token_clears_the_old_token() {
        let mut e = env(Some("http://127.0.0.1:2"), Some("secret"));
        go(&mut e, use_cmd(Some(DOWN), None, false)).await.unwrap();
        assert_eq!(current(&e), (Some(DOWN.to_string()), None));
    }

    #[tokio::test]
    async fn use_with_the_same_address_keeps_the_token() {
        let mut e = env(Some(DOWN), Some("secret"));
        go(&mut e, use_cmd(Some("http://127.0.0.1:1/"), None, false))
            .await
            .unwrap();
        assert_eq!(
            current(&e),
            (Some(DOWN.to_string()), Some("secret".to_string()))
        );
    }

    #[tokio::test]
    async fn use_with_a_token_sets_it() {
        let mut e = env(Some(DOWN), Some("old"));
        go(
            &mut e,
            use_cmd(Some("http://127.0.0.1:2"), Some("new"), false),
        )
        .await
        .unwrap();
        assert_eq!(current(&e).1.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn use_off_clears_address_and_token() {
        let mut e = env(Some(DOWN), Some("secret"));
        let out = go(&mut e, use_cmd(None, None, true)).await.unwrap();
        assert_eq!(current(&e), (None, None));
        assert!(out.contains("removed"), "{out}");
    }

    #[tokio::test]
    async fn use_with_nothing_is_a_usage_error() {
        let mut e = env(None, None);
        let err = go(&mut e, use_cmd(None, None, false)).await.unwrap_err();
        assert!(err.to_string().contains("--off"), "{err}");
    }

    #[tokio::test]
    async fn use_without_a_workspace_config_is_an_error() {
        let mut e = env(None, None);
        e.settings.workspace_config = None;
        let err = go(&mut e, use_cmd(Some(DOWN), None, false))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("No workspaces configured"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn use_with_a_bad_address_changes_nothing() {
        let mut e = env(Some(DOWN), Some("secret"));
        let _ = go(&mut e, use_cmd(Some("ftp://x"), None, false))
            .await
            .unwrap_err();
        assert_eq!(
            current(&e),
            (Some(DOWN.to_string()), Some("secret".to_string()))
        );
    }

    #[tokio::test]
    async fn status_changes_nothing() {
        let mut e = env(Some(DOWN), Some("secret"));
        let out = go(&mut e, ServerSubcommand::Status).await.unwrap();
        assert!(out.starts_with("Server state:"), "{out}");
        assert_eq!(
            current(&e),
            (Some(DOWN.to_string()), Some("secret".to_string()))
        );
        assert!(!e.config_dir.join("config.toml").exists());
    }

    #[tokio::test]
    async fn the_token_is_never_printed() {
        let mut e = env(Some(DOWN), Some("s3cr3t-token"));
        let a = go(&mut e, ServerSubcommand::Status).await.unwrap();
        let b = go(&mut e, use_cmd(Some(DOWN), Some("s3cr3t-token"), false))
            .await
            .unwrap();
        let c = go(
            &mut e,
            use_cmd(Some("http://127.0.0.1:2"), Some("s3cr3t-token"), false),
        )
        .await
        .unwrap();
        let d = go(&mut e, use_cmd(None, None, true)).await.unwrap();
        let err = go(
            &mut e,
            use_cmd(Some("s3cr3t-token"), Some("s3cr3t-token"), false),
        )
        .await
        .unwrap_err();
        for text in [a, b, c, d, err.to_string(), format!("{err:?}")] {
            assert!(!text.contains("s3cr3t-token"), "{text}");
        }
    }

    fn assert_speaks_to_the_person(out: &str) {
        assert!(!out.contains("Tell the user"), "{out}");
        assert!(!out.contains("Do not run"), "{out}");
    }

    #[tokio::test]
    async fn status_shows_the_command_without_the_words_for_the_agent() {
        let mut e = env(None, None);
        let out = go(&mut e, ServerSubcommand::Status).await.unwrap();
        assert!(out.contains(crate::rag::state::INSTALL_LINE), "{out}");
        assert!(out.contains("kimun server use"), "{out}");
        assert_speaks_to_the_person(&out);
    }

    #[tokio::test]
    async fn use_shows_the_command_without_the_words_for_the_agent() {
        let mut e = env(None, None);
        // An install with a systemd service on port 1, where nothing listens.
        std::fs::write(
            e.config_dir.join(crate::server_manage::marker::MARKER_FILE),
            // The folder must be absolute on every system, so it comes from the temp folder.
            format!(
                "installer = \"install-server.sh\"\ninstall_dir = '{}'\nservice = \"systemd\"\n",
                std::env::temp_dir().join("kimun-bin").display()
            ),
        )
        .unwrap();
        std::fs::write(e.config_dir.join("server.toml"), "[server]\nport = 1\n").unwrap();
        // A refused connection on Windows can take seconds, so the time-out is long.
        let out = run_in(
            use_cmd(Some(DOWN), None, false),
            &mut e.settings,
            &e.config_dir,
            HostOs::Linux,
            Duration::from_secs(10),
        )
        .await
        .unwrap();
        assert!(
            out.contains("systemctl --user start kimun-server.service"),
            "{out}"
        );
        assert_speaks_to_the_person(&out);
        assert!(out.ends_with(RESTART_LINE), "{out}");
    }

    #[test]
    fn clap_parses_the_server_commands() {
        match Wrap::try_parse_from(["x", "use", "http://h:1", "--token", "T"])
            .unwrap()
            .sub
        {
            ServerSubcommand::Use { url, token, off } => {
                assert_eq!(url.as_deref(), Some("http://h:1"));
                assert_eq!(token.as_deref(), Some("T"));
                assert!(!off);
            }
            other => panic!("{other:?}"),
        }
        match Wrap::try_parse_from(["x", "use", "--off"]).unwrap().sub {
            ServerSubcommand::Use { url, token, off } => {
                assert!(url.is_none() && token.is_none() && off);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            Wrap::try_parse_from(["x", "status"]).unwrap().sub,
            ServerSubcommand::Status
        ));
        assert!(Wrap::try_parse_from(["x", "use", "--off", "http://h:1"]).is_err());
        assert!(Wrap::try_parse_from(["x", "use", "--off", "--token", "T"]).is_err());
    }
}
