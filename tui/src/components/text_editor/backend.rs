use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ratatui_textarea::TextArea;

use super::nvim_rpc::{key_event_to_nvim_string, NvimRpc};
use super::snapshot::{NvimMode, NvimSnapshot};
use crate::settings::EditorBackendSetting;

/// Which editor engine `TextEditorComponent` is currently using.
pub enum BackendState {
    Textarea(TextArea<'static>),
    Nvim(NvimBackend),
}

impl BackendState {
    /// Construct the appropriate backend from settings.
    ///
    /// Falls back to `Textarea` if `editor_backend` is `Textarea`, or if the
    /// nvim binary cannot be found or spawned (logs a warning in that case).
    pub fn from_settings(
        editor_backend: &EditorBackendSetting,
        nvim_path: Option<&PathBuf>,
    ) -> Self {
        if matches!(editor_backend, EditorBackendSetting::Nvim) {
            match NvimBackend::new(nvim_path) {
                Ok(backend) => return BackendState::Nvim(backend),
                Err(e) => {
                    log::warn!("nvim backend unavailable, falling back to textarea: {e}")
                }
            }
        }
        BackendState::Textarea(TextArea::default())
    }
}

pub struct NvimBackend {
    pub rpc: Arc<NvimRpc>,
    pub snapshot: Arc<Mutex<NvimSnapshot>>,
    /// Monotonically increasing counter; incremented before each refresh task.
    /// The refresh task checks this before writing to the snapshot — if the value
    /// has moved on, the result is discarded (superseded by a later keystroke).
    pub refresh_gen: Arc<AtomicU64>,
    /// Set to `true` by the reader thread on EOF (nvim process died).
    pub is_dead: Arc<AtomicBool>,
    child: Option<std::process::Child>,
}

impl Drop for NvimBackend {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

impl NvimBackend {
    /// Spawn `nvim --embed` and create the backend.
    ///
    /// `nvim_path` overrides the binary; if `None`, `nvim` is resolved from `PATH`.
    pub fn new(nvim_path: Option<&PathBuf>) -> Result<Self, String> {
        let binary = nvim_path
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "nvim".to_string());

        let mut child = std::process::Command::new(&binary)
            .arg("--embed")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn {binary}: {e}"))?;

        let stdin = child.stdin.take().ok_or("nvim stdin unavailable")?;
        let stdout = child.stdout.take().ok_or_else(|| {
            let _ = child.kill();
            "nvim stdout unavailable"
        })?;

        let is_dead = Arc::new(AtomicBool::new(false));
        let rpc = Arc::new(NvimRpc::new_with_dead_signal(stdin, stdout, is_dead.clone()));

        // Attach as a minimal UI. Without this, nvim runs as a passive RPC server
        // and never fires its input-processing event loop — keystrokes queued via
        // nvim_input are silently ignored until something calls vgetc().
        rpc.send(
            "nvim_ui_attach",
            vec![
                rmpv::Value::Integer(80.into()),
                rmpv::Value::Integer(24.into()),
                rmpv::Value::Map(vec![]), // empty options dict
            ],
        );

        // Fire-and-forget init commands (no response needed).
        rpc.send("nvim_command", vec![rmpv::Value::String("set noswapfile".into())]);
        rpc.send("nvim_command", vec![rmpv::Value::String("set buftype=nofile".into())]);
        rpc.send("nvim_command", vec![rmpv::Value::String("set nomodeline".into())]);

        Ok(Self {
            rpc,
            snapshot: Arc::new(Mutex::new(NvimSnapshot::default())),
            refresh_gen: Arc::new(AtomicU64::new(0)),
            is_dead,
            child: Some(child),
        })
    }

    /// Load content into the nvim buffer and update the snapshot directly.
    ///
    /// Fire-and-forget: sends `nvim_buf_set_lines` without waiting for the response.
    /// The snapshot is pre-populated from `text` so `get_text()` works immediately.
    pub fn set_text(&self, text: &str) {
        let lines: Vec<rmpv::Value> = text
            .lines()
            .map(|l| rmpv::Value::String(l.into()))
            .collect();
        self.rpc.send(
            "nvim_buf_set_lines",
            vec![
                rmpv::Value::Integer(0.into()),
                rmpv::Value::Integer(0.into()),
                rmpv::Value::Integer((-1i64).into()),
                rmpv::Value::Boolean(false),
                rmpv::Value::Array(lines),
            ],
        );
        let mut snap = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());
        snap.lines = text.lines().map(|l| l.to_string()).collect();
        if snap.lines.is_empty() {
            snap.lines.push(String::new());
        }
        snap.cursor = (0, 0);
        snap.dirty = false;
    }

    /// Send a keystroke to nvim and spawn a blocking task to refresh the snapshot.
    pub fn handle_key(&self, key: &ratatui::crossterm::event::KeyEvent) {
        let Some(nvim_key) = key_event_to_nvim_string(key) else {
            log::debug!("unmappable key: {key:?}");
            return;
        };

        let current_gen = self.refresh_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let rpc = self.rpc.clone();
        let snapshot = self.snapshot.clone();
        let refresh_gen = self.refresh_gen.clone();

        // `call_blocking` uses std::sync::mpsc::recv_timeout which blocks the OS thread.
        // spawn_blocking gives us a dedicated thread so we don't stall the async executor.
        tokio::task::spawn_blocking(move || {
            // Send the key through the normal input queue (same path as real keyboard
            // input). nvim_input is the correct API for embedded UIs — unlike
            // nvim_feedkeys it goes through libuv's input buffer which the event loop
            // drains synchronously before handling the next RPC message.
            if let Err(e) = rpc.call_blocking(
                "nvim_input",
                vec![rmpv::Value::String(nvim_key.into())],
            ) {
                log::debug!("nvim_input error: {e}");
                return;
            }

            // Mark dirty now that nvim has confirmed the key was processed.
            snapshot.lock().unwrap_or_else(|p| p.into_inner()).dirty = true;

            if refresh_gen.load(Ordering::SeqCst) != current_gen {
                return; // superseded by a later keystroke
            }

            // Get the current mode.
            let mode_str = rpc
                .call_blocking("nvim_get_mode", vec![])
                .ok()
                .and_then(|v| {
                    v.as_map()?.iter()
                        .find(|(k, _)| k.as_str() == Some("mode"))
                        .and_then(|(_, v)| v.as_str().map(|s| s.to_string()))
                })
                .unwrap_or_else(|| "n".to_string());

            let mode = NvimMode::from_nvim_str(&mode_str);

            if mode == NvimMode::Command {
                let cmdtype = rpc
                    .call_blocking(
                        "nvim_call_function",
                        vec![
                            rmpv::Value::String("getcmdtype".into()),
                            rmpv::Value::Array(vec![]),
                        ],
                    )
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();

                let cmdline_text = rpc
                    .call_blocking(
                        "nvim_call_function",
                        vec![
                            rmpv::Value::String("getcmdline".into()),
                            rmpv::Value::Array(vec![]),
                        ],
                    )
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default();

                if refresh_gen.load(Ordering::SeqCst) == current_gen {
                    let mut snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());
                    snap.mode = mode;
                    snap.cmdline = Some(format!("{cmdtype}{cmdline_text}"));
                }
            } else {
                let lines_val = rpc.call_blocking(
                    "nvim_buf_get_lines",
                    vec![
                        rmpv::Value::Integer(0.into()),
                        rmpv::Value::Integer(0.into()),
                        rmpv::Value::Integer((-1i64).into()),
                        rmpv::Value::Boolean(false),
                    ],
                );

                // Check generation before fetching cursor to avoid mismatched line/cursor state.
                if refresh_gen.load(Ordering::SeqCst) != current_gen {
                    return;
                }

                let cursor_val = rpc.call_blocking(
                    "nvim_win_get_cursor",
                    vec![rmpv::Value::Integer(0.into())],
                );

                if refresh_gen.load(Ordering::SeqCst) == current_gen {
                    let lines = lines_val
                        .ok()
                        .and_then(|v| {
                            v.as_array().map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect::<Vec<_>>()
                            })
                        })
                        .unwrap_or_else(|| vec![String::new()]);

                    // nvim_win_get_cursor returns [row, col]:
                    // row is 1-indexed, col is 0-indexed byte offset.
                    let cursor = cursor_val
                        .ok()
                        .and_then(|v| {
                            v.as_array().map(|arr| {
                                let row = arr
                                    .first()
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(1) as usize;
                                let col = arr
                                    .get(1)
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0) as usize;
                                (row.saturating_sub(1), col)
                            })
                        })
                        .unwrap_or((0, 0));

                    let mut snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());
                    snap.lines = lines;
                    snap.cursor = cursor;
                    snap.mode = mode;
                    snap.cmdline = None;
                }
            }
        });
    }
}
