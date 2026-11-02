use crate::logic::message::{AppMessage, AppTx};

/// The process-wide `arboard::Clipboard`.
///
/// **It must outlive the write.** On X11 `set_text` transfers *ownership* of the
/// CLIPBOARD selection to this process, and the contents are then served on
/// demand by arboard's background thread. Dropping the handle right after
/// writing can therefore lose what was just copied — arboard warns about exactly
/// this ("Clipboard was dropped very quickly after writing (0ms); clipboard
/// managers may not have seen the contents"). `set_text` still returns `Ok`, so
/// a per-call handle reports success while the paste silently fails.
///
/// One handle for the whole TUI, rather than the two policies that preceded it:
/// a per-call handle (loses ownership) and a per-component cached handle (kept
/// ownership, but a single failed `Clipboard::new()` at construction disabled
/// that component's clipboard for the entire session, silently). The `Option`
/// below is what avoids the latter — a failure is dropped, not cached, so the
/// next attempt opens a fresh connection.
static CLIPBOARD: std::sync::OnceLock<std::sync::Mutex<Option<arboard::Clipboard>>> =
    std::sync::OnceLock::new();

/// Run `f` against the shared clipboard, opening it if needed. A failing
/// operation drops the handle so the next call reconnects.
pub(crate) fn with_clipboard<T>(
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, arboard::Error> {
    let cell = CLIPBOARD.get_or_init(|| std::sync::Mutex::new(None));
    // A poisoned lock means a previous holder panicked mid-operation; the
    // handle is suspect, so take the guard and rebuild from scratch.
    let mut guard = cell.lock().unwrap_or_else(|e| {
        let mut g = e.into_inner();
        *g = None;
        g
    });
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new()?);
    }
    let result = f(guard.as_mut().expect("just opened"));
    // Drop the handle only on a failure that suggests the *connection* is bad.
    // `ContentNotAvailable` is a statement about the clipboard's contents, not
    // about the handle — and it is the ordinary answer when `take_clipboard_image`
    // probes a text clipboard ahead of every Ctrl+V. Dropping on it would
    // release the X11 CLIPBOARD-selection ownership we hold from our own last
    // copy, losing the copied text: the exact failure this shared handle exists
    // to prevent.
    if matches!(&result, Err(e) if !matches!(e, arboard::Error::ContentNotAvailable)) {
        *guard = None;
    }
    result
}

/// Put `text` on the OS clipboard and flash the outcome: `done_msg` on
/// success, `"clipboard: {e}"` on failure. **The** seam for every OS-clipboard
/// write in the TUI — list-row yanks, ask answers and sources, and the editor's
/// own Ctrl+C/Ctrl+X.
pub fn yank(text: String, done_msg: impl Into<String>, tx: &AppTx) {
    let msg = match with_clipboard(|c| c.set_text(text)) {
        Ok(()) => done_msg.into(),
        Err(e) => format!("clipboard: {e}"),
    };
    tx.send(AppMessage::FlashMessage(msg)).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yank_flashes_outcome_on_tx() {
        // Headless test runs may have no OS clipboard, so this asserts a
        // FlashMessage arrives either way — success or the "clipboard: {e}"
        // error form — not which one.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        yank("hello".to_string(), "hello copied", &tx);
        let ev = rx.try_recv().expect("yank sends exactly one event");
        match ev {
            AppMessage::FlashMessage(msg) => {
                assert!(
                    msg == "hello copied" || msg.starts_with("clipboard: "),
                    "unexpected flash message: {msg}"
                );
            }
            other => panic!("expected FlashMessage, got {other:?}"),
        }
    }
}
