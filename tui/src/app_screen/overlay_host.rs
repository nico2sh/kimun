//! `OverlayHost` — single-slot owner of the active editor overlay (note
//! browser, Saved Searches modal, or dialog). Owns focus save/restore and
//! routes input / app-messages / render to the active overlay.

use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::overlay::{Overlay, OverlayKind, OverlayMsg};
use crate::settings::themes::Theme;

pub struct OverlayHost<F> {
    active: Option<Box<dyn Overlay>>,
    /// Opener panel focus, saved when an overlay first opens and returned to
    /// the caller on close. Mirrors the old `DialogManager` chained-open
    /// guard: a second `open` while one is active does NOT overwrite the
    /// saved focus.
    saved_focus: Option<F>,
}

impl<F> OverlayHost<F> {
    pub fn new() -> Self {
        Self {
            active: None,
            saved_focus: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    pub fn active_kind(&self) -> Option<OverlayKind> {
        self.active.as_ref().map(|o| o.kind())
    }

    /// Open `overlay`. Saves `panel_token` only if no overlay is currently
    /// active, so a chained open preserves the original opener focus.
    /// Replacing an already-open overlay is allowed; the previous overlay is
    /// dropped and the saved opener focus is preserved.
    pub fn open(&mut self, overlay: Box<dyn Overlay>, panel_token: F) {
        if self.saved_focus.is_none() {
            self.saved_focus = Some(panel_token);
        }
        self.active = Some(overlay);
    }

    /// Close the active overlay; return the saved opener focus to restore.
    pub fn close(&mut self) -> Option<F> {
        self.active = None;
        self.saved_focus.take()
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        if let Some(o) = &mut self.active {
            o.handle_input(event, tx)
        } else {
            EventState::NotConsumed
        }
    }

    pub fn handle_app_message(
        &mut self,
        msg: &AppEvent,
        vault: &Arc<NoteVault>,
        tx: &AppTx,
    ) -> OverlayMsg {
        if let Some(o) = &mut self.active {
            o.handle_app_message(msg, vault, tx)
        } else {
            OverlayMsg::NotConsumed
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        if let Some(o) = &mut self.active {
            o.render(f, area, theme);
        }
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        self.active
            .as_ref()
            .map(|o| o.hint_shortcuts())
            .unwrap_or_default()
    }
}

impl<F> Default for OverlayHost<F> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    struct FakeOverlay(OverlayKind);
    impl Overlay for FakeOverlay {
        fn kind(&self) -> OverlayKind {
            self.0
        }
        fn handle_input(&mut self, _e: &InputEvent, _tx: &AppTx) -> EventState {
            EventState::Consumed
        }
        fn render(&mut self, _f: &mut Frame, _a: Rect, _t: &Theme) {}
    }

    #[test]
    fn new_is_closed() {
        let host: OverlayHost<u8> = OverlayHost::new();
        assert!(!host.is_open());
        assert_eq!(host.active_kind(), None);
    }

    #[test]
    fn open_saves_focus_and_close_restores_it() {
        let mut host: OverlayHost<u8> = OverlayHost::new();
        host.open(Box::new(FakeOverlay(OverlayKind::NoteBrowser)), 1);
        assert!(host.is_open());
        assert_eq!(host.active_kind(), Some(OverlayKind::NoteBrowser));
        assert_eq!(host.close(), Some(1));
        assert!(!host.is_open());
    }

    #[test]
    fn chained_open_preserves_first_focus_token() {
        let mut host: OverlayHost<u8> = OverlayHost::new();
        host.open(Box::new(FakeOverlay(OverlayKind::NoteBrowser)), 1);
        host.open(Box::new(FakeOverlay(OverlayKind::Dialog)), 99);
        assert_eq!(host.active_kind(), Some(OverlayKind::Dialog));
        assert_eq!(host.close(), Some(1));
    }

    #[test]
    fn close_when_empty_returns_none() {
        let mut host: OverlayHost<u8> = OverlayHost::new();
        assert_eq!(host.close(), None);
    }
}
