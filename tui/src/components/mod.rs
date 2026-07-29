pub mod activity_rail;
pub mod ask_sources;
pub mod ask_thread;
pub mod attachment_view;
pub mod autocomplete;
pub mod autosave_timer;
pub mod command_palette;
pub mod config_panel;
pub mod dialogs;
pub mod dir_browser;
pub mod drawer;
pub mod drawer_views;
pub mod event_state;
pub mod events;
pub mod file_list;
pub mod footer_bar;
pub mod hints;
pub mod indexing;
pub mod markdown_lines;
pub mod note_browser;
pub mod overlay;
pub mod panel;
pub mod preferences;
pub mod preview_highlight;
pub mod preview_pane;
pub mod query_highlight;
pub mod query_list_panel;
pub mod query_panel;
pub mod query_vars;
pub mod rich_row;
pub mod saved_search_breadcrumb;
pub mod saved_searches_modal;
pub mod search_list;
pub mod semantic_search;
pub mod sidebar;
pub mod single_line_input;
pub mod text_editor;
pub mod which_key;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::settings::themes::Theme;

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
/// next attempt opens a fresh connection (adr/0031).
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
    // to prevent (adr/0031).
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
    tx.send(AppEvent::FlashMessage(msg)).ok();
}

/// Perform a [`crate::components::search_list::KeyReaction::Yank`]: copy the
/// row's target and name what was copied ("path copied", "tag copied"), or say
/// there was nothing to copy.
///
/// The `None` branch is the point. Silence there is indistinguishable from a
/// clipboard failure or from an unbound key, which is precisely how the missing
/// note-browser yank stayed invisible (adr/0032).
pub fn yank_row(target: Option<search_list::YankTarget>, tx: &AppTx) {
    match target {
        Some(t) => yank(t.text, format!("{} copied", t.noun), tx),
        None => {
            tx.send(AppEvent::FlashMessage("nothing to copy".into()))
                .ok();
        }
    }
}

/// Centre a popup occupying `percent_x`% × `percent_y`% of `area`.
/// A centered rect of fixed cell size, clamped to `r` — the counterpart to
/// the percentage-based [`centered_rect`] for dialogs with intrinsic sizes.
pub fn fixed_centered_rect(width: u16, height: u16, r: Rect) -> Rect {
    let width = width.min(r.width);
    let height = height.min(r.height);
    Rect {
        x: r.x + (r.width - width) / 2,
        y: r.y + (r.height - height) / 2,
        width,
        height,
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_height = (area.height as u32 * percent_y as u32 / 100) as u16;
    let popup_width = (area.width as u32 * percent_x as u32 / 100) as u16;
    Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    }
}

pub trait Component {
    /// Handle an event. Send `AppEvent`s through `tx` for app-level effects.
    /// Returns whether this component consumed the event.
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let _ = (event, tx);
        EventState::NotConsumed
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool);

    /// Context-sensitive shortcut hints shown in the hints bar when this
    /// component is focused.  Each entry is `(key_display, label)`.
    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![]
    }
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
            AppEvent::FlashMessage(msg) => {
                assert!(
                    msg == "hello copied" || msg.starts_with("clipboard: "),
                    "unexpected flash message: {msg}"
                );
            }
            other => panic!("expected FlashMessage, got {other:?}"),
        }
    }

    #[test]
    fn centered_rect_is_centered() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 40,
        };
        let r = centered_rect(80, 75, area);
        assert_eq!(r.width, 80);
        assert_eq!(r.height, 30);
        assert_eq!(r.x, 10); // (100 - 80) / 2
        assert_eq!(r.y, 5); // (40 - 30) / 2
    }

    #[test]
    fn centered_rect_does_not_underflow() {
        // Very small area — must not panic.
        let area = Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        };
        let _ = centered_rect(80, 75, area);
    }
}
