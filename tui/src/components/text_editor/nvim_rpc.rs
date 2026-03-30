use std::collections::HashMap;
use std::io::Write;
use std::process::{ChildStdin, ChildStdout};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Convert a crossterm `KeyEvent` to a Neovim key string suitable for `nvim_feedkeys`.
///
/// Returns `None` for events that have no Neovim equivalent (e.g., modifier-only events).
/// Space maps to `<Space>` and `<` maps to `<lt>` to avoid ambiguity in Neovim's key parser.
pub fn key_event_to_nvim_string(key: &KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    let base = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                return Some(format!("<C-{}>", c.to_lowercase()));
            }
            if alt {
                return Some(format!("<A-{c}>"));
            }
            match c {
                ' ' => return Some("<Space>".into()),
                '<' => return Some("<lt>".into()),
                _ => return Some(c.to_string()),
            }
        }
        KeyCode::Enter => "<CR>",
        KeyCode::Backspace => "<BS>",
        KeyCode::Delete => "<Del>",
        KeyCode::Esc => "<Esc>",
        KeyCode::Tab => "<Tab>",
        KeyCode::BackTab => "<S-Tab>",
        KeyCode::Up => "<Up>",
        KeyCode::Down => "<Down>",
        KeyCode::Left => "<Left>",
        KeyCode::Right => "<Right>",
        KeyCode::Home => "<Home>",
        KeyCode::End => "<End>",
        KeyCode::PageUp => "<PageUp>",
        KeyCode::PageDown => "<PageDown>",
        KeyCode::Insert => "<Insert>",
        KeyCode::F(n) => return Some(format!("<F{n}>")),
        _ => return None,
    };

    Some(base.to_string())
}

/// Minimal msgpack-RPC client for `nvim --embed`.
///
/// Spawns a background reader thread that routes responses to callers waiting on
/// `std::sync::mpsc` channels. Fire-and-forget requests use `send()`; requests
/// that need a response use `call_blocking()`, which blocks the calling thread.
#[derive(Clone)]
pub struct NvimRpc {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<u32, mpsc::Sender<Result<rmpv::Value, String>>>>>,
    msg_id: Arc<AtomicU32>,
}

impl NvimRpc {
    /// Create an `NvimRpc` from stdin/stdout of a spawned `nvim --embed` process,
    /// with a shared `is_dead` flag set to `true` when the reader thread exits (nvim died).
    ///
    /// Starts the background reader thread immediately.
    pub fn new_with_dead_signal(
        stdin: ChildStdin,
        stdout: ChildStdout,
        is_dead: Arc<AtomicBool>,
    ) -> Self {
        let stdin = Arc::new(Mutex::new(stdin));
        let pending: Arc<Mutex<HashMap<u32, mpsc::Sender<_>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_reader = pending.clone();

        std::thread::Builder::new()
            .name("nvim-rpc-reader".into())
            .spawn(move || {
                let mut reader = std::io::BufReader::new(stdout);
                loop {
                    match rmpv::decode::read_value(&mut reader) {
                        Ok(value) => {
                            let Some(arr) = value.as_array() else { continue };
                            let msg_type =
                                arr.first().and_then(|v| v.as_i64()).unwrap_or(-1);
                            // Response: [1, msg_id, error, result]
                            if msg_type == 1 && arr.len() >= 4 {
                                let id = arr[1].as_u64().unwrap_or(0) as u32;
                                let err = &arr[2];
                                let result = arr[3].clone();
                                let outcome = if err.is_nil() {
                                    Ok(result)
                                } else {
                                    Err(format!("{err}"))
                                };
                                let mut p = pending_reader.lock().unwrap();
                                if let Some(tx) = p.remove(&id) {
                                    let _ = tx.send(outcome);
                                }
                            }
                            // Notifications (type 2) are ignored.
                        }
                        Err(e) => {
                            log::debug!("nvim RPC reader exiting: {e}");
                            is_dead.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            })
            .expect("failed to spawn nvim-rpc-reader thread");

        Self {
            stdin,
            pending,
            msg_id: Arc::new(AtomicU32::new(0)),
        }
    }

    fn write_msg(&self, msg: &rmpv::Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().unwrap();
        rmpv::encode::write_value(&mut *stdin, msg)
            .map_err(|e| format!("msgpack encode: {e}"))?;
        stdin.flush().map_err(|e| format!("stdin flush: {e}"))?;
        Ok(())
    }

    /// Send a request without waiting for a response (fire and forget).
    pub fn send(&self, method: &str, params: Vec<rmpv::Value>) {
        let id = self.msg_id.fetch_add(1, Ordering::Relaxed);
        let msg = rmpv::Value::Array(vec![
            rmpv::Value::Integer(0.into()),
            rmpv::Value::Integer(id.into()),
            rmpv::Value::String(method.into()),
            rmpv::Value::Array(params),
        ]);
        if let Err(e) = self.write_msg(&msg) {
            log::debug!("nvim send error ({method}): {e}");
        }
    }

    /// Send a request and block until the response arrives (up to 5 seconds).
    pub fn call_blocking(
        &self,
        method: &str,
        params: Vec<rmpv::Value>,
    ) -> Result<rmpv::Value, String> {
        let id = self.msg_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            let mut p = self.pending.lock().unwrap();
            p.insert(id, tx);
        }
        let msg = rmpv::Value::Array(vec![
            rmpv::Value::Integer(0.into()),
            rmpv::Value::Integer(id.into()),
            rmpv::Value::String(method.into()),
            rmpv::Value::Array(params),
        ]);
        self.write_msg(&msg)?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| format!("timeout waiting for {method} response"))?
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn letter_j_in_normal() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('j'), KeyModifiers::NONE)), Some("j".into()));
    }

    #[test]
    fn uppercase_J_with_shift() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('J'), KeyModifiers::SHIFT)), Some("J".into()));
    }

    #[test]
    fn enter_maps_to_cr() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Enter, KeyModifiers::NONE)), Some("<CR>".into()));
    }

    #[test]
    fn backspace_maps_to_bs() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Backspace, KeyModifiers::NONE)), Some("<BS>".into()));
    }

    #[test]
    fn delete_maps_to_del() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Delete, KeyModifiers::NONE)), Some("<Del>".into()));
    }

    #[test]
    fn escape_maps_to_esc() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Esc, KeyModifiers::NONE)), Some("<Esc>".into()));
    }

    #[test]
    fn tab_maps_to_tab() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Tab, KeyModifiers::NONE)), Some("<Tab>".into()));
    }

    #[test]
    fn ctrl_w_maps() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('w'), KeyModifiers::CONTROL)), Some("<C-w>".into()));
    }

    #[test]
    fn ctrl_r_maps() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('r'), KeyModifiers::CONTROL)), Some("<C-r>".into()));
    }

    #[test]
    fn arrow_up() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Up, KeyModifiers::NONE)), Some("<Up>".into()));
    }

    #[test]
    fn arrow_down() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Down, KeyModifiers::NONE)), Some("<Down>".into()));
    }

    #[test]
    fn arrow_left() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Left, KeyModifiers::NONE)), Some("<Left>".into()));
    }

    #[test]
    fn arrow_right() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Right, KeyModifiers::NONE)), Some("<Right>".into()));
    }

    #[test]
    fn home_key() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Home, KeyModifiers::NONE)), Some("<Home>".into()));
    }

    #[test]
    fn end_key() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::End, KeyModifiers::NONE)), Some("<End>".into()));
    }

    #[test]
    fn page_up() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::PageUp, KeyModifiers::NONE)), Some("<PageUp>".into()));
    }

    #[test]
    fn page_down() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::PageDown, KeyModifiers::NONE)), Some("<PageDown>".into()));
    }

    #[test]
    fn f1_key() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::F(1), KeyModifiers::NONE)), Some("<F1>".into()));
    }

    #[test]
    fn f12_key() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::F(12), KeyModifiers::NONE)), Some("<F12>".into()));
    }

    #[test]
    fn alt_j() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('j'), KeyModifiers::ALT)), Some("<A-j>".into()));
    }

    #[test]
    fn space() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char(' '), KeyModifiers::NONE)), Some("<Space>".into()));
    }

    #[test]
    fn less_than_char() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('<'), KeyModifiers::NONE)), Some("<lt>".into()));
    }

    #[test]
    fn backslash_char() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('\\'), KeyModifiers::NONE)), Some("\\".into()));
    }
}
