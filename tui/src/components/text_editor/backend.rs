use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::ChildStdin;
use tokio_util::compat::Compat;

use nvim_rs::{Handler, Neovim, UiAttachOptions, create::tokio::new_child_cmd, error::LoopError};
use ratatui_textarea::TextArea;

use super::nvim_decode::{DecodedState, decode};
use super::nvim_rpc::key_event_to_nvim_string;
use super::snapshot::{EditorMode, NvimSnapshot};
use super::vim::VimEngine;
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
// InputInterpreter + TextareaBackend
// ---------------------------------------------------------------------------

/// How key events are translated into edits on a `TextArea` (adr/0012).
/// The engine is boxed so the `Direct` arm doesn't pay the engine's size
/// (registers, dot-repeat state, replace stack — ~230 bytes).
#[derive(Debug, Default)]
pub enum InputInterpreter {
    /// Today's behavior: keys go straight to the textarea.
    #[default]
    Direct,
    /// Built-in vim emulation.
    Vim(Box<VimEngine>),
}

/// The in-process textarea storage plus its input interpreter.
#[derive(Debug)]
pub struct TextareaBackend {
    pub ta: TextArea<'static>,
    pub input: InputInterpreter,
}

impl TextareaBackend {
    pub fn direct(ta: TextArea<'static>) -> Self {
        Self {
            ta,
            input: InputInterpreter::Direct,
        }
    }
    pub fn vim(ta: TextArea<'static>) -> Self {
        Self {
            ta,
            input: InputInterpreter::Vim(Box::default()),
        }
    }
}

// ---------------------------------------------------------------------------
// BackendState
// ---------------------------------------------------------------------------

#[allow(clippy::large_enum_variant)]
pub enum BackendState {
    Textarea(TextareaBackend),
    Nvim(NvimBackend),
}

impl BackendState {
    /// Whether the textarea backend is active — the named form of the
    /// structural guard, for sites that only need the yes/no.
    pub fn is_textarea(&self) -> bool {
        matches!(self, BackendState::Textarea(_))
    }

    /// True when the active backend is the built-in vim interpreter (any mode).
    pub fn is_vim(&self) -> bool {
        matches!(
            self,
            BackendState::Textarea(TextareaBackend {
                input: InputInterpreter::Vim(_),
                ..
            })
        )
    }

    /// The textarea, when it is the active backend. Textarea-only features
    /// (autocomplete, smart edits, mouse selection) guard on this.
    pub fn as_textarea(&self) -> Option<&TextArea<'static>> {
        match self {
            BackendState::Textarea(tb) => Some(&tb.ta),
            BackendState::Nvim(_) => None,
        }
    }

    pub fn as_textarea_mut(&mut self) -> Option<&mut TextArea<'static>> {
        match self {
            BackendState::Textarea(tb) => Some(&mut tb.ta),
            BackendState::Nvim(_) => None,
        }
    }

    /// The nvim backend, when it is the active one.
    pub fn as_nvim(&self) -> Option<&NvimBackend> {
        match self {
            BackendState::Textarea(_) => None,
            BackendState::Nvim(nvim) => Some(nvim),
        }
    }

    /// The whole buffer as one string, whichever backend holds it.
    pub fn text(&self) -> String {
        match self {
            BackendState::Textarea(tb) => tb.ta.lines().join("\n"),
            BackendState::Nvim(nvim) => nvim.snapshot().lines.join("\n"),
        }
    }

    /// The cursor's (row, col), cheap on both backends — no line cloning.
    /// The nvim row is clamped to the mirrored line count (the mirror can
    /// lag the real cursor for a frame), matching the snapshot path.
    pub fn cursor(&self) -> (usize, usize) {
        match self {
            BackendState::Textarea(tb) => super::cursor_tuple(&tb.ta),
            BackendState::Nvim(nvim) => {
                let snap = nvim.snapshot();
                let max_row = snap.lines.len().saturating_sub(1);
                (snap.cursor.0.min(max_row), snap.cursor.1)
            }
        }
    }

    /// If the nvim backend's process has died, replace it with a textarea
    /// holding the last mirrored buffer, and report that it happened so the
    /// host can re-arm textarea-only features.
    pub fn recover_from_dead_nvim(&mut self) -> bool {
        let fallback_text = match self.as_nvim() {
            Some(nvim) if nvim.is_dead() => nvim.snapshot().lines.join("\n"),
            _ => return false,
        };
        tracing::warn!("nvim process died; falling back to textarea backend");
        *self = BackendState::Textarea(TextareaBackend::direct(TextArea::from(
            fallback_text.lines(),
        )));
        true
    }

    /// Reconcile the active input interpreter with a host-driven mouse
    /// selection change. The vim interpreter tracks it modally (a new
    /// selection enters Visual, a cleared one returns to Normal); the other
    /// backends have nothing to reconcile.
    pub fn sync_mouse_selection(&mut self, has_selection: bool) {
        if let BackendState::Textarea(TextareaBackend {
            input: InputInterpreter::Vim(e),
            ..
        }) = self
        {
            e.sync_mouse_selection(has_selection);
        }
    }

    /// True when a bare Space should start the leader sequence. Only the vim
    /// interpreter ever says yes (Normal mode, empty pending state); for every
    /// other backend Space is just typing.
    pub fn space_leads(&self) -> bool {
        matches!(self,
            BackendState::Textarea(TextareaBackend { input: InputInterpreter::Vim(e), .. })
            if e.space_leads())
    }

    /// True when the current selection visually includes the char under the
    /// cursor, so the highlight path extends the end col by one. Only the vim
    /// interpreter's charwise Visual mode (not VisualLine) selects this way.
    pub fn selection_includes_cursor(&self) -> bool {
        matches!(self,
            BackendState::Textarea(TextareaBackend { input: InputInterpreter::Vim(e), .. })
            if *e.mode() == EditorMode::Visual)
    }

    /// Reset any transient input-interpreter state for a freshly loaded note
    /// (the vim interpreter returns to Normal; the other backends carry no
    /// such state).
    pub fn reset_input_state(&mut self) {
        if let BackendState::Textarea(TextareaBackend {
            input: InputInterpreter::Vim(engine),
            ..
        }) = self
        {
            engine.reset_to_normal();
        }
    }

    /// If the active backend is the vim interpreter, run it for this key and
    /// return the outcome. Returns `None` for Direct / Nvim backends.
    pub fn vim_handle_key(
        &mut self,
        key: &ratatui::crossterm::event::KeyEvent,
    ) -> Option<super::vim::VimKeyOutcome> {
        match self {
            BackendState::Textarea(TextareaBackend {
                ta,
                input: InputInterpreter::Vim(engine),
            }) => Some(engine.handle_key(key, ta)),
            _ => None,
        }
    }

    /// The in-progress input-command hint for the footer (the vim
    /// interpreter's pending count/operator/find/g sequence). `None` when the
    /// active backend has no pending sequence.
    pub fn pending_input_hint(&self) -> Option<String> {
        match self {
            BackendState::Textarea(TextareaBackend {
                input: InputInterpreter::Vim(e),
                ..
            }) => e.pending_hint(),
            _ => None,
        }
    }

    /// The footer modal-mode label, when the backend has one (nvim, or the
    /// vim interpreter). `None` for the plain Direct textarea.
    pub fn mode_label(&self) -> Option<String> {
        match self {
            BackendState::Textarea(TextareaBackend {
                input: InputInterpreter::Vim(engine),
                ..
            }) => Some(engine.mode_label()),
            BackendState::Textarea(_) => None,
            BackendState::Nvim(nvim) => Some(nvim.snapshot().footer_label()),
        }
    }

    /// Alloc-free cursor-shape classifier for the render path.
    /// `None` = non-modal backend (Direct textarea — leave terminal cursor as-is).
    /// `Some(true)` = Insert mode (bar cursor).
    /// `Some(false)` = other modal mode (block cursor).
    pub fn modal_is_insert(&self) -> Option<bool> {
        match self {
            BackendState::Textarea(TextareaBackend {
                input: InputInterpreter::Vim(e),
                ..
            }) => Some(*e.mode() == EditorMode::Insert),
            BackendState::Textarea(_) => None,
            BackendState::Nvim(nvim) => Some(nvim.snapshot().mode == EditorMode::Insert),
        }
    }

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
        let tb = match editor_backend {
            EditorBackendSetting::Vim => TextareaBackend::vim(TextArea::default()),
            // Nvim is handled by the early return above; Textarea and any
            // future non-modal setting use the direct interpreter.
            EditorBackendSetting::Textarea | EditorBackendSetting::Nvim => {
                TextareaBackend::direct(TextArea::default())
            }
        };
        BackendState::Textarea(tb)
    }
}

// ---------------------------------------------------------------------------
// NvimBackend
// ---------------------------------------------------------------------------

pub struct NvimBackend {
    nvim: NvimClient,
    snapshot: Arc<Mutex<NvimSnapshot>>,
    is_dead: Arc<AtomicBool>,
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
    last_ui_size: Mutex<(u16, u16)>,
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
    /// Locked view of the mirrored nvim state (cursor, lines, mode, dirty…).
    /// Poison-recovering: a panicked refresh task never wedges the UI.
    pub fn snapshot(&self) -> std::sync::MutexGuard<'_, NvimSnapshot> {
        self.snapshot.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Whether the nvim process / IO loop has died (the host falls back to
    /// the textarea backend when it has).
    pub fn is_dead(&self) -> bool {
        self.is_dead.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Clear the mirrored dirty flag — the buffer was just persisted.
    pub fn mark_clean(&self) {
        self.snapshot().dirty = false;
    }

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
        // Pin nvim's tabstop to the renderer's TAB_STOP so tab-column math and
        // cursor placement can never desync.
        let _ = nvim
            .command(&format!("set tabstop={}", super::markdown::TAB_STOP))
            .await;

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
    ///
    /// Contract: the synchronous snapshot pre-populate (lines + cursor +
    /// dirty=false + content_gen bump) happens BEFORE `in_flight` is set
    /// and the buf_set_lines RPC is spawned. A keystroke arriving between
    /// the synchronous return of `set_text` and the spawned task actually
    /// reaching nvim ends up routed via `handle_key`, and the refresh task
    /// will skip snapshot updates while `in_flight=true` (see
    /// `apply_lua_state`). Once the spawned RPC completes and `in_flight`
    /// flips back to false, the refresh task will observe whatever buffer
    /// state nvim has — including both the loaded content AND any keys the
    /// user pressed in the interim. `snap.lines != new_lines` will then
    /// re-set `dirty=true`. The window where `dirty=false` after a
    /// concurrent keystroke is bounded by one refresh cycle (~30 ms).
    /// Do NOT move the `in_flight.store(true)` earlier or clear it
    /// before the RPC actually completes — both invariants are load-bearing.
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

    /// Insert `text` at nvim's current cursor position via `nvim_paste`.
    /// Honours nvim's current mode (insert/normal/visual) — visual replaces the
    /// selection, normal/insert insert at cursor — so it works as a drop-in
    /// for the textarea backend's insert/replace flow.
    pub fn paste(&self, text: &str, tx: AppTx) {
        self.ensure_refresh_task(&tx);
        let nvim = self.nvim.clone();
        let is_dead = self.is_dead.clone();
        let key_tx = self.key_tx.clone();
        let payload = text.to_string();
        tokio::spawn(async move {
            // phase = -1 → single-chunk paste (not part of a streamed sequence).
            match nvim.paste(&payload, false, -1).await {
                Ok(_) => {
                    key_tx.send_modify(|v| *v = v.wrapping_add(1));
                }
                Err(e) => {
                    if e.is_channel_closed() {
                        is_dead.store(true, Ordering::SeqCst);
                        tx.send(AppEvent::Redraw).ok();
                    }
                    tracing::debug!("nvim_paste error: {e}");
                }
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

/// Decode the Lua state bundle (pure, in [`super::nvim_decode`]) and merge it
/// into the live snapshot. Decoding owns the wire-format facts; this function
/// owns the stateful bookkeeping that decoding cannot: the `in_flight` gate and
/// the `content_gen`/`dirty` revision counters.
fn apply_lua_state(
    snapshot: &Arc<Mutex<NvimSnapshot>>,
    in_flight: &Arc<AtomicBool>,
    value: nvim_rs::Value,
) {
    let Some(decoded) = decode(&value) else {
        return;
    };

    let mut snap = snapshot.lock().unwrap_or_else(|p| p.into_inner());

    match decoded {
        DecodedState::Command { cmdline } => {
            snap.mode = EditorMode::Command;
            snap.cmdline = Some(cmdline);
        }
        DecodedState::Content {
            mode,
            lines,
            cursor,
            visual_selection,
        } => {
            if lines != snap.lines && !in_flight.load(Ordering::SeqCst) {
                snap.dirty = true;
                snap.lines = lines;
                snap.content_gen = snap.content_gen.wrapping_add(1);
            }
            snap.cursor = cursor;
            snap.mode = mode;
            snap.cmdline = None;
            snap.visual_selection = visual_selection;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_textarea::TextArea;

    #[test]
    fn direct_backend_has_no_mode_label() {
        let b = BackendState::Textarea(TextareaBackend::direct(TextArea::default()));
        assert_eq!(b.mode_label(), None);
    }

    #[test]
    fn vim_backend_reports_normal_label() {
        let b = BackendState::Textarea(TextareaBackend::vim(TextArea::default()));
        assert_eq!(b.mode_label().as_deref(), Some("NORMAL"));
    }

    #[test]
    fn space_leads_only_for_vim_backend() {
        assert!(
            !BackendState::Textarea(TextareaBackend::direct(TextArea::default())).space_leads()
        );
        assert!(BackendState::Textarea(TextareaBackend::vim(TextArea::default())).space_leads());
    }

    #[test]
    fn modal_is_insert_classifies_backends() {
        // Direct textarea → None (non-modal, leave terminal cursor alone).
        assert_eq!(
            BackendState::Textarea(TextareaBackend::direct(TextArea::default())).modal_is_insert(),
            None
        );
        // Vim backend starts in Normal mode → Some(false) (block cursor).
        assert_eq!(
            BackendState::Textarea(TextareaBackend::vim(TextArea::default())).modal_is_insert(),
            Some(false)
        );
    }
}
