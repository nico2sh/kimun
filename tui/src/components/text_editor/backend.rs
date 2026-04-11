use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::ChildStdin;
use tokio_util::compat::Compat;

use nvim_rs::{Handler, Neovim, UiAttachOptions, create::tokio::new_child_cmd, error::LoopError};
use ratatui_textarea::TextArea;

use super::nvim_rpc::key_event_to_nvim_string;
use super::snapshot::{NvimMode, NvimSnapshot};
use crate::components::events::{AppEvent, AppTx};
use crate::settings::EditorBackendSetting;

type NvimWriter = Compat<ChildStdin>;
type NvimClient = Neovim<NvimWriter>;

// ---------------------------------------------------------------------------
// Lua snippet: fetch all editor state in one round-trip.
//
// Command mode  → [mode, cmdtype, cmdline]
// Other modes   → [mode, lines, cursor, vpos]
// ---------------------------------------------------------------------------
const STATE_QUERY_LUA: &str = r#"
local m = vim.api.nvim_get_mode().mode
if m == 'c' then
  return {m, vim.fn.getcmdtype(), vim.fn.getcmdline()}
else
  local lines  = vim.api.nvim_buf_get_lines(0, 0, -1, false)
  local cursor = vim.api.nvim_win_get_cursor(0)
  local vpos   = vim.fn.getpos('v')
  return {m, lines, cursor, vpos}
end
"#;

// ---------------------------------------------------------------------------
// Handler — increments flush_tx counter on every "flush" redraw event.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct NvimHandler {
    flush_tx: tokio::sync::watch::Sender<u64>,
}

#[async_trait::async_trait]
impl Handler for NvimHandler {
    type Writer = NvimWriter;

    async fn handle_notify(&self, name: String, args: Vec<nvim_rs::Value>, _neovim: NvimClient) {
        if name != "redraw" {
            return;
        }
        for arg in &args {
            if let Some(events) = arg.as_array() {
                for event in events {
                    if let Some(ea) = event.as_array()
                        && ea.first().and_then(|v| v.as_str()) == Some("flush")
                    {
                        self.flush_tx.send_modify(|v| *v = v.wrapping_add(1));
                        return;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BackendState
// ---------------------------------------------------------------------------

#[allow(clippy::large_enum_variant)]
pub enum BackendState {
    Textarea(TextArea<'static>),
    Nvim(NvimBackend),
}

impl BackendState {
    pub fn from_settings(
        editor_backend: &EditorBackendSetting,
        nvim_path: Option<&PathBuf>,
    ) -> Self {
        if matches!(editor_backend, EditorBackendSetting::Nvim) {
            match NvimBackend::new(nvim_path) {
                Ok(backend) => return BackendState::Nvim(backend),
                Err(e) => {
                    tracing::warn!("nvim backend unavailable, falling back to textarea: {e}")
                }
            }
        }
        BackendState::Textarea(TextArea::default())
    }
}

// ---------------------------------------------------------------------------
// NvimBackend
// ---------------------------------------------------------------------------

pub struct NvimBackend {
    pub nvim: NvimClient,
    pub snapshot: Arc<Mutex<NvimSnapshot>>,
    pub is_dead: Arc<AtomicBool>,
    /// Set while a `buf_set_lines` call spawned by `set_text` is in flight.
    /// The refresh task skips line/dirty updates while this is `true` to avoid
    /// overwriting the pre-populated snapshot with stale nvim state.
    set_text_in_flight: Arc<AtomicBool>,
    /// Incremented by the handler on every flush event.
    flush_rx: tokio::sync::watch::Receiver<u64>,
    /// Incremented by handle_key after each successful nvim_input call.
    /// Gives the refresh task a wakeup path even when nvim doesn't send flush.
    key_tx: tokio::sync::watch::Sender<u64>,
    /// Stored until the refresh task is started on the first handle_key call.
    pending_key_rx: Mutex<Option<tokio::sync::watch::Receiver<u64>>>,
    /// Tracks the last size passed to `ui_attach`/`ui_try_resize` so we only
    /// send a resize RPC when the terminal rect actually changes.
    pub last_ui_size: Mutex<(u16, u16)>,
    io_handle: tokio::task::JoinHandle<Result<(), Box<LoopError>>>,
    child: Option<tokio::process::Child>,
}

impl Drop for NvimBackend {
    fn drop(&mut self) {
        // Abort the IO loop first so it stops sending on flush_tx,
        // which lets the refresh task's flush_rx.changed() return Err and exit.
        self.io_handle.abort();
        if let Some(ref mut child) = self.child {
            let _ = child.start_kill();
        }
    }
}

impl NvimBackend {
    pub fn new(nvim_path: Option<&PathBuf>) -> Result<Self, String> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(Self::new_async(nvim_path))
        })
    }

    async fn new_async(nvim_path: Option<&PathBuf>) -> Result<Self, String> {
        let binary = nvim_path
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "nvim".to_string());

        let (flush_tx, flush_rx) = tokio::sync::watch::channel(0u64);
        let (key_tx, key_rx) = tokio::sync::watch::channel(0u64);
        let handler = NvimHandler { flush_tx };

        let mut cmd = tokio::process::Command::new(&binary);
        cmd.arg("--embed").stderr(std::process::Stdio::null());

        let (nvim, io_handle, child) = new_child_cmd(&mut cmd, handler)
            .await
            .map_err(|e| format!("failed to spawn {binary}: {e}"))?;

        let mut ui_opts = UiAttachOptions::new();
        ui_opts.set_rgb(false);
        nvim.ui_attach(80, 24, &ui_opts)
            .await
            .map_err(|e| format!("nvim_ui_attach failed: {e}"))?;

        let _ = nvim.command("set noswapfile").await;
        let _ = nvim.command("set buftype=nofile").await;
        let _ = nvim.command("set nomodeline").await;
        let _ = nvim.command("set expandtab").await;
        let _ = nvim.command("set tabstop=4").await;

        Ok(Self {
            nvim,
            snapshot: Arc::new(Mutex::new(NvimSnapshot::default())),
            is_dead: Arc::new(AtomicBool::new(false)),
            set_text_in_flight: Arc::new(AtomicBool::new(false)),
            flush_rx,
            key_tx,
            pending_key_rx: Mutex::new(Some(key_rx)),
            last_ui_size: Mutex::new((80, 24)),
            io_handle,
            child: Some(child),
        })
    }

    /// Start the long-running refresh task on the first call; no-op afterwards.
    fn ensure_refresh_task(&self, tx: &AppTx) {
        let mut guard = self
            .pending_key_rx
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(key_rx) = guard.take() else { return };

        let nvim = self.nvim.clone();
        let snapshot = self.snapshot.clone();
        let is_dead = self.is_dead.clone();
        let in_flight = self.set_text_in_flight.clone();
        let flush_rx = self.flush_rx.clone();
        let tx = tx.clone();

        tokio::spawn(async move {
            let mut key_rx = key_rx;
            let mut flush_rx = flush_rx;

            loop {
                // Wake on either:
                //  • flush event (nvim finished processing input — best path)
                //  • key signal  (nvim_input returned; give nvim 30 ms to flush first)
                tokio::select! {
                    res = flush_rx.changed() => {
                        if res.is_err() {
                            // Sender dropped — nvim IO loop ended.
                            is_dead.store(true, Ordering::SeqCst);
                            tx.send(AppEvent::Redraw).ok();
                            break;
                        }
                        // Flush arrived — state is fresh, query immediately.
                    }
                    res = key_rx.changed() => {
                        if res.is_err() { break; }
                        // nvim_input returned. Wait up to 30 ms for flush before
                        // querying; proceed regardless so nothing is ever stuck.
                        tokio::time::timeout(
                            Duration::from_millis(30),
                            flush_rx.changed(),
                        ).await.ok();
                    }
                }

                match nvim.exec_lua(STATE_QUERY_LUA, vec![]).await {
                    Ok(value) => {
                        apply_lua_state(&snapshot, &in_flight, value);
                        tx.send(AppEvent::Redraw).ok();
                    }
                    Err(e) => {
                        if e.is_channel_closed() {
                            is_dead.store(true, Ordering::SeqCst);
                            tx.send(AppEvent::Redraw).ok();
                            break;
                        }
                        // Non-fatal (e.g. transient Lua error): log and continue.
                        tracing::debug!("exec_lua error: {e}");
                    }
                }
            }
        });
    }

    /// Load content into the nvim buffer and pre-populate the snapshot.
    pub fn set_text(&self, text: &str) {
        let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

        {
            let mut snap = self.snapshot.lock().unwrap_or_else(|p| p.into_inner());
            snap.lines = if lines.is_empty() {
                vec![String::new()]
            } else {
                lines.clone()
            };
            snap.cursor = (0, 0);
            snap.dirty = false;
            snap.content_gen = snap.content_gen.wrapping_add(1);
        }

        let nvim = self.nvim.clone();
        let is_dead = self.is_dead.clone();
        let in_flight = self.set_text_in_flight.clone();
        in_flight.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            let buf = match nvim.get_current_buf().await {
                Ok(b) => b,
                Err(e) => {
                    in_flight.store(false, Ordering::SeqCst);
                    if e.is_channel_closed() {
                        is_dead.store(true, Ordering::SeqCst);
                    }
                    tracing::warn!("set_text get_current_buf: {e}");
                    return;
                }
            };
            if let Err(e) = buf.set_lines(0, -1, false, lines).await {
                tracing::warn!("set_text buf_set_lines: {e}");
            }
            in_flight.store(false, Ordering::SeqCst);
        });
    }

    /// Notify nvim of a terminal resize, but only when the dimensions actually change.
    pub fn maybe_resize(&self, width: u16, height: u16) {
        let mut guard = self.last_ui_size.lock().unwrap_or_else(|p| p.into_inner());
        if *guard == (width, height) {
            return;
        }
        *guard = (width, height);
        drop(guard);

        let nvim = self.nvim.clone();
        let is_dead = self.is_dead.clone();
        tokio::spawn(async move {
            if let Err(e) = nvim.ui_try_resize(width as i64, height as i64).await {
                if e.is_channel_closed() {
                    is_dead.store(true, Ordering::SeqCst);
                }
                tracing::debug!("ui_try_resize error: {e}");
            }
        });
    }

    /// Forward a keystroke to nvim.
    pub fn handle_key(&self, key: &ratatui::crossterm::event::KeyEvent, tx: AppTx) {
        self.ensure_refresh_task(&tx);

        let Some(nvim_key) = key_event_to_nvim_string(key) else {
            tracing::debug!("unmappable key: {key:?}");
            return;
        };

        let nvim = self.nvim.clone();
        let is_dead = self.is_dead.clone();
        let key_tx = self.key_tx.clone();

        tokio::spawn(async move {
            match nvim.input(&nvim_key).await {
                Ok(_) => {
                    // Signal the refresh task: a key was just sent.
                    key_tx.send_modify(|v| *v = v.wrapping_add(1));
                }
                Err(e) => {
                    if e.is_channel_closed() {
                        is_dead.store(true, Ordering::SeqCst);
                        tx.send(AppEvent::Redraw).ok();
                    }
                    tracing::debug!("nvim_input error: {e}");
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Parse the Lua state bundle and apply it to the snapshot.
// ---------------------------------------------------------------------------

/// Convert a UTF-8 byte offset to a Unicode scalar (char) index.
///
/// `nvim_win_get_cursor` and `getpos()` return byte offsets. This converts them
/// to char indices so the rest of the rendering pipeline can use char-indexed
/// operations consistently. If the offset falls in the middle of a multi-byte
/// sequence it is snapped to the nearest valid char boundary.
fn byte_offset_to_char_idx(line: &str, byte_offset: usize) -> usize {
    // Walk backward from the offset to the nearest valid char boundary, then
    // count chars up to that point. Handles mid-codepoint offsets safely.
    let safe = (0..=byte_offset.min(line.len()))
        .rev()
        .find(|&i| line.is_char_boundary(i))
        .unwrap_or(0);
    line[..safe].chars().count()
}

fn apply_lua_state(
    snapshot: &Arc<Mutex<NvimSnapshot>>,
    in_flight: &Arc<AtomicBool>,
    value: nvim_rs::Value,
) {
    let Some(arr) = value.as_array() else { return };
    let mode_str = match arr.first().and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return,
    };
    let mode = NvimMode::from_nvim_str(mode_str);

    let mut snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());

    if mode == NvimMode::Command {
        let cmdtype = arr
            .get(1)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cmdline = arr
            .get(2)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        snap.mode = mode;
        snap.cmdline = Some(format!("{cmdtype}{cmdline}"));
        return;
    }

    // Lines.
    let new_lines: Vec<String> = arr
        .get(1)
        .and_then(|v| v.as_array())
        .map(|ls| {
            ls.iter()
                .filter_map(|l| l.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let new_lines = if new_lines.is_empty() {
        vec![String::new()]
    } else {
        new_lines
    };

    // Cursor: nvim_win_get_cursor → [row(1-indexed), col(0-indexed byte offset)].
    // Convert the byte offset to a char index so all downstream code works in
    // char-index space uniformly (independent of multi-byte character widths).
    let cursor = arr
        .get(2)
        .and_then(|v| v.as_array())
        .and_then(|c| {
            let row = c.first()?.as_u64()? as usize;
            let byte_col = c.get(1)?.as_u64()? as usize;
            let row0 = row.saturating_sub(1);
            let char_col = new_lines
                .get(row0)
                .map(|line| byte_offset_to_char_idx(line, byte_col))
                .unwrap_or(byte_col);
            Some((row0, char_col))
        })
        .unwrap_or((0, 0));

    // Visual selection: getpos("v") → [bufnum, lnum(1-indexed), col(1-indexed byte offset), off].
    // Convert the 1-indexed byte col to a 0-indexed char index.
    let visual_selection = if matches!(mode, NvimMode::Visual | NvimMode::VisualLine) {
        arr.get(3)
            .and_then(|v| v.as_array())
            .and_then(|p| {
                let lnum = p.get(1)?.as_u64()? as usize;
                let vcol_byte = p.get(2)?.as_u64()? as usize;
                if lnum == 0 {
                    return None;
                }
                let row0 = lnum.saturating_sub(1);
                let char_col = new_lines
                    .get(row0)
                    .map(|line| byte_offset_to_char_idx(line, vcol_byte.saturating_sub(1)))
                    .unwrap_or(vcol_byte.saturating_sub(1));
                Some((row0, char_col))
            })
            .map(|anchor| {
                let (mut start, mut end) = if anchor <= cursor {
                    (anchor, cursor)
                } else {
                    (cursor, anchor)
                };
                if mode == NvimMode::VisualLine {
                    start.1 = 0;
                    end.1 = usize::MAX;
                }
                (start, end)
            })
    } else {
        None
    };

    if new_lines != snap.lines && !in_flight.load(Ordering::SeqCst) {
        snap.dirty = true;
        snap.lines = new_lines;
        snap.content_gen = snap.content_gen.wrapping_add(1);
    }
    snap.cursor = cursor;
    snap.mode = mode;
    snap.cmdline = None;
    snap.visual_selection = visual_selection;
}
