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

/// Put `text` on the OS clipboard and flash the outcome: `done_msg` on
/// success, `"clipboard: {e}"` on failure. The shared seam for one-shot
/// yanks (query results, ask answers/sources, editor wikilinks/paths) that
/// open a fresh `arboard::Clipboard` per call and report through
/// `AppEvent::FlashMessage`.
///
/// `TextEditorComponent` does *not* use this: it caches its own
/// `arboard::Clipboard` handle (see `text_editor/mod.rs`) because it copies
/// on every selection change and can't afford to reopen the clipboard each
/// time — that hot path stays as-is.
pub fn yank(text: String, done_msg: impl Into<String>, tx: &AppTx) {
    let msg = match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
        Ok(()) => done_msg.into(),
        Err(e) => format!("clipboard: {e}"),
    };
    tx.send(AppEvent::FlashMessage(msg)).ok();
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
