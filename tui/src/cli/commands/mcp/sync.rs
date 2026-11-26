// tui/src/cli/commands/mcp/sync.rs
//
// Keeps the Kimun server in step with notes written through the MCP tools.

//! The sync loop for `kimun mcp`. The loop itself is the one the apps run
//! ([`rag::spawn_rag_sync_with`]). This module only chooses what to do with its
//! status, which is to write it to the log, and drains once on exit.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kimun_core::NoteVault;
use tokio::task::JoinHandle;

use crate::rag::{self, RagStatus, SyncEvent, SyncReport};
use crate::settings::SharedSettings;

/// How long the flush on exit may take before the process gives up on it.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// Starts the sync loop for `vault` when a server URL is configured, on the
/// same interval as the apps. Returns `None` and starts nothing otherwise.
pub fn spawn(vault: Arc<NoteVault>, settings: &SharedSettings) -> Option<JoinHandle<()>> {
    spawn_with(vault, settings, rag::SYNC_INTERVAL)
}

fn spawn_with(
    vault: Arc<NoteVault>,
    settings: &SharedSettings,
    interval: Duration,
) -> Option<JoinHandle<()>> {
    rag::spawn_rag_sync_with(vault, settings, log_report(), interval)
}

/// Writes each status change to the log. The loop reports a status on every
/// tick, so only a change is logged. Server-update notices are for the apps'
/// footer and are ignored here.
fn log_report() -> SyncReport {
    let last: Mutex<Option<RagStatus>> = Mutex::new(None);
    Arc::new(move |event| {
        let SyncEvent::Status(status) = event else {
            return;
        };
        let Ok(mut last) = last.lock() else { return };
        if *last != Some(status) {
            log::info!("Kimun server sync: {status:?}");
            *last = Some(status);
        }
    })
}

/// A best-effort sync pass on exit, so a write made just before the agent
/// app closes the connection still reaches the server. It gives up after
/// [`FLUSH_TIMEOUT`] and only logs a failure.
pub async fn flush_once(vault: &Arc<NoteVault>, settings: &SharedSettings) {
    flush_once_with(vault, settings, FLUSH_TIMEOUT).await;
}

async fn flush_once_with(vault: &Arc<NoteVault>, settings: &SharedSettings, timeout: Duration) {
    match tokio::time::timeout(timeout, rag::sync_once(vault, settings)).await {
        // No server configured, or the vault id cannot be read.
        Ok(None) => {}
        Ok(Some(Ok(true))) => log::debug!("Kimun server sync: flushed on exit"),
        Ok(Some(Ok(false))) => {
            log::info!("Kimun server sync: the index is not ready, nothing flushed on exit")
        }
        Ok(Some(Err(e))) => log::warn!("Kimun server sync: flush on exit failed: {e}"),
        Err(_) => log::warn!(
            "Kimun server sync: flush on exit stopped after {} seconds",
            timeout.as_secs_f32()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::mcp::{CreateNoteParams, KimunHandler};
    use crate::settings::workspace_config::WorkspaceConfig;
    use crate::settings::{AppSettings, SharedSettings};
    use kimun_core::{NoteVault, VaultConfig};
    use rmcp::handler::server::wrapper::Parameters;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    /// What the fake server does with a request.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        /// Answer like a server with an embedder.
        Answer,
        /// Accept the connection, read the request and never answer.
        Hang,
    }

    /// A small HTTP server that stands in for the Kimun server. It keeps the
    /// pushed hashes by path, so a push of the same note leaves it unchanged,
    /// and it records each request in order: `health`, `hashes`, `label`,
    /// `push <path>` (one entry per pushed note) and `delete <path>`.
    struct FakeServer {
        url: String,
        log: Arc<Mutex<Vec<String>>>,
        hashes: Arc<Mutex<HashMap<String, String>>>,
        /// While false, the server closes each connection with no answer and
        /// logs `down`, like a server that is not running.
        up: Arc<AtomicBool>,
    }

    impl FakeServer {
        fn start(mode: Mode) -> Self {
            Self::start_with(mode, true)
        }

        /// A server that starts down. [`FakeServer::bring_up`] starts it.
        fn start_down() -> Self {
            Self::start_with(Mode::Answer, false)
        }

        fn bring_up(&self) {
            self.up.store(true, Ordering::SeqCst);
        }

        fn start_with(mode: Mode, up: bool) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let log = Arc::new(Mutex::new(Vec::new()));
            let hashes = Arc::new(Mutex::new(HashMap::new()));
            let up = Arc::new(AtomicBool::new(up));
            let (log_in, hashes_in, up_in) = (log.clone(), hashes.clone(), up.clone());
            std::thread::spawn(move || {
                // Held so a hanging connection stays open.
                let mut held = Vec::new();
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    if !up_in.load(Ordering::SeqCst) {
                        log_in.lock().unwrap().push("down".into());
                        drop(stream);
                        continue;
                    }
                    let (method, path, body) = read_request(&mut stream);
                    if mode == Mode::Hang {
                        held.push(stream);
                        continue;
                    }
                    let reply = answer(&method, &path, &body, &log_in, &hashes_in);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        reply.len(),
                        reply
                    );
                }
            });
            Self {
                url,
                log,
                hashes,
                up,
            }
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        fn hashes(&self) -> HashMap<String, String> {
            self.hashes.lock().unwrap().clone()
        }

        fn count(&self, entry: &str) -> usize {
            self.log().iter().filter(|e| *e == entry).count()
        }

        /// Waits until `done` holds for the request log, or panics after
        /// `limit`.
        async fn wait_for(&self, limit: Duration, done: impl Fn(&[String]) -> bool) {
            let start = Instant::now();
            while !done(&self.log()) {
                assert!(
                    start.elapsed() < limit,
                    "fake server never saw the request: {:?}",
                    self.log()
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> (String, String, String) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf).to_string();
            match text.split_once("\r\n\r\n") {
                Some((head, body)) => {
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= len || n == 0 {
                        break;
                    }
                }
                None if n == 0 => break,
                None => {}
            }
        }
        let text = String::from_utf8_lossy(&buf).to_string();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut first = head.lines().next().unwrap_or("").split(' ');
        let method = first.next().unwrap_or("").to_string();
        let path = first.next().unwrap_or("").to_string();
        (method, path, body.to_string())
    }

    fn answer(
        method: &str,
        path: &str,
        body: &str,
        log: &Mutex<Vec<String>>,
        hashes: &Mutex<HashMap<String, String>>,
    ) -> String {
        let json = || serde_json::from_str::<serde_json::Value>(body).unwrap();
        let mut log = log.lock().unwrap();
        match (method, path) {
            ("GET", "/health") => {
                log.push("health".into());
                r#"{"status":"ok","embedder":"fake","llm_provider":null,"version":"9.9.9"}"#.into()
            }
            ("GET", p) if p.ends_with("/hashes") => {
                log.push("hashes".into());
                serde_json::to_string(&*hashes.lock().unwrap()).unwrap()
            }
            ("PUT", p) if p.ends_with("/label") => {
                log.push("label".into());
                "{}".into()
            }
            ("POST", "/api/index/docs") => {
                for doc in json()["docs"].as_array().unwrap() {
                    let path = doc["path"].as_str().unwrap().to_string();
                    let hash = doc["hash"].as_str().unwrap().to_string();
                    log.push(format!("push {path}"));
                    hashes.lock().unwrap().insert(path, hash);
                }
                r#"{"job_id":"job-1"}"#.into()
            }
            ("POST", "/api/index/delete") => {
                for path in json()["paths"].as_array().unwrap() {
                    let path = path.as_str().unwrap().to_string();
                    log.push(format!("delete {path}"));
                    hashes.lock().unwrap().remove(&path);
                }
                "{}".into()
            }
            _ => {
                log.push(format!("other {method} {path}"));
                "{}".into()
            }
        }
    }

    fn settings_for(url: Option<&str>) -> SharedSettings {
        let mut config = WorkspaceConfig::new_empty();
        config.global.kimun_server_url = url.map(str::to_string);
        let mut settings = AppSettings::default();
        settings.workspace_config = Some(config);
        Arc::new(RwLock::new(settings))
    }

    async fn open_vault(dir: &std::path::Path) -> Arc<NoteVault> {
        let vault = NoteVault::new(VaultConfig::new(crate::test_support::sys(dir)))
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        Arc::new(vault)
    }

    /// A URL where nothing listens: the port was free a moment ago.
    fn down_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    }

    async fn create_through_tool(vault: &Arc<NoteVault>, path: &str) {
        let handler = KimunHandler::from_shared(vault.clone());
        let result = handler
            .create_note(Parameters(CreateNoteParams {
                path: path.to_string(),
                content: "# Written by an agent\n\nbody".to_string(),
            }))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
    }

    #[tokio::test]
    async fn write_reaches_server_within_one_interval() {
        let server = FakeServer::start(Mode::Answer);
        let dir = TempDir::new().unwrap();
        let vault = open_vault(dir.path()).await;
        let settings = settings_for(Some(&server.url));
        let task = spawn_with(vault.clone(), &settings, Duration::from_millis(100))
            .expect("a server is configured");

        // The sync reconciles at start.
        server
            .wait_for(Duration::from_secs(10), |log| {
                log.iter().any(|e| e == "hashes")
            })
            .await;

        let probes_before = server.count("health");
        create_through_tool(&vault, "from-agent").await;
        server
            .wait_for(Duration::from_secs(10), |log| {
                log.iter().any(|e| e == "push /from-agent.md")
            })
            .await;
        task.abort();

        // Each tick probes once. The push came on the tick in flight at the
        // write or on the next one, so within one interval, not on the
        // reconcile that runs only every 30 ticks. A tick may have probed
        // before `probes_before` was read and drained before the write was
        // marked dirty, so up to two probes may come before the push.
        let log = server.log();
        let at = log.iter().position(|e| e == "push /from-agent.md").unwrap();
        let probes = log[..at].iter().filter(|e| *e == "health").count();
        assert!(
            probes <= probes_before + 2,
            "the push waited {} ticks: {log:?}",
            probes - probes_before
        );
    }

    #[tokio::test]
    async fn write_succeeds_when_server_down() {
        let dir = TempDir::new().unwrap();
        let vault = open_vault(dir.path()).await;
        let settings = settings_for(Some(&down_url()));
        let task = spawn_with(vault.clone(), &settings, Duration::from_millis(50))
            .expect("a server is configured");
        // Let a few ticks fail against the closed port first.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let start = Instant::now();
        create_through_tool(&vault, "offline-note").await;
        let took = start.elapsed();
        task.abort();
        assert!(
            took < Duration::from_secs(2),
            "the write waited for the server: {took:?}"
        );
        assert!(
            vault
                .exists(&kimun_core::nfs::VaultPath::note_path_from("offline-note"))
                .await
        );
    }

    #[tokio::test]
    async fn sync_catches_up_after_the_server_comes_back() {
        let server = FakeServer::start_down();
        let dir = TempDir::new().unwrap();
        let vault = open_vault(dir.path()).await;
        let settings = settings_for(Some(&server.url));
        let task = spawn_with(vault.clone(), &settings, Duration::from_millis(50))
            .expect("a server is configured");

        // The write lands while the server is down, and a few ticks fail.
        create_through_tool(&vault, "while-down").await;
        server
            .wait_for(Duration::from_secs(10), |log| {
                log.iter().filter(|e| *e == "down").count() >= 3
            })
            .await;
        assert_eq!(server.count("push /while-down.md"), 0);

        server.bring_up();
        server
            .wait_for(Duration::from_secs(10), |log| {
                log.iter().any(|e| e == "push /while-down.md")
            })
            .await;
        task.abort();
    }

    #[tokio::test]
    async fn repeated_push_of_same_note_changes_nothing() {
        let server = FakeServer::start(Mode::Answer);
        let dir = TempDir::new().unwrap();
        let settings = settings_for(Some(&server.url));

        // The first process pushes the note.
        let first = open_vault(dir.path()).await;
        create_through_tool(&first, "shared").await;
        flush_once(&first, &settings).await;
        assert_eq!(server.count("push /shared.md"), 1, "{:?}", server.log());
        let after_first = server.hashes();

        // A second process on the same vault pushes again. The hashes match,
        // so nothing is sent and the server keeps the same state.
        let second = open_vault(dir.path()).await;
        flush_once(&second, &settings).await;
        assert_eq!(server.count("push /shared.md"), 1, "{:?}", server.log());
        assert_eq!(server.hashes(), after_first);
    }

    #[tokio::test]
    async fn flush_once_times_out() {
        let server = FakeServer::start(Mode::Hang);
        let dir = TempDir::new().unwrap();
        let vault = open_vault(dir.path()).await;
        create_through_tool(&vault, "pending").await;
        let settings = settings_for(Some(&server.url));

        let start = Instant::now();
        flush_once_with(&vault, &settings, Duration::from_millis(300)).await;
        let took = start.elapsed();
        assert!(
            took < Duration::from_secs(3),
            "flush did not stop at its timeout: {took:?}"
        );
    }

    #[tokio::test]
    async fn no_server_configured_starts_nothing() {
        let dir = TempDir::new().unwrap();
        let vault = open_vault(dir.path()).await;
        let settings = settings_for(None);
        assert!(spawn_with(vault.clone(), &settings, Duration::from_millis(50)).is_none());
        // Flushing without a server returns at once and does nothing.
        flush_once(&vault, &settings).await;
    }
}
