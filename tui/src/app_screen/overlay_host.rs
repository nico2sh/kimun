//! `OverlayHost` — owner of the active editor overlay (note browser, Saved
//! Searches modal, or dialog). Owns focus save/restore and routes input /
//! app-messages / render to the active overlay. One overlay may be *parked*
//! under the active one (the sort dialog over a sortable note browser):
//! it is drawn but gets no input, and closing the active overlay restores it.

use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent, OverlayData};
use crate::components::overlay::{Overlay, OverlayKind};
use crate::components::sortable::SortableList;
use crate::settings::themes::Theme;

pub struct OverlayHost<F> {
    active: Option<Box<dyn Overlay>>,
    /// The overlay [`Self::open_over`] parked under `active` — one level only.
    parked: Option<Box<dyn Overlay>>,
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
            parked: None,
            saved_focus: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    pub fn active_kind(&self) -> Option<OverlayKind> {
        self.active.as_ref().map(|o| o.kind())
    }

    /// The active overlay's query string, if it is query-backed (note browser).
    pub fn active_query(&self) -> Option<&str> {
        self.active.as_ref().and_then(|o| o.query())
    }

    /// The active overlay's saved-search provenance (its breadcrumb name), if
    /// any. Pre-fills the save-search dialog's name field.
    pub fn active_saved_search_provenance(&self) -> Option<&str> {
        self.active
            .as_ref()
            .and_then(|o| o.saved_search_provenance())
    }

    /// Open `overlay`. Saves `panel_token` only if no overlay is currently
    /// active, so a chained open preserves the original opener focus.
    /// Replacing an already-open overlay is allowed; the previous overlay is
    /// dropped and the saved opener focus is preserved. A parked overlay is
    /// dropped too: the replaced stack must not come back on close.
    pub fn open(&mut self, overlay: Box<dyn Overlay>, panel_token: F) {
        if self.saved_focus.is_none() {
            self.saved_focus = Some(panel_token);
        }
        self.parked = None;
        self.active = Some(overlay);
    }

    /// Open `overlay` over the active one, parking it (drawn, but no input)
    /// until `overlay` closes. The saved opener focus is untouched. Any
    /// overlay already parked is dropped — stacking is one level deep.
    pub fn open_over(&mut self, overlay: Box<dyn Overlay>) {
        self.parked = self.active.take();
        self.active = Some(overlay);
    }

    /// The parked overlay's kind, `None` when nothing is parked.
    pub fn parked_kind(&self) -> Option<OverlayKind> {
        self.parked.as_ref().map(|o| o.kind())
    }

    /// The active overlay as a sortable list, if it is one.
    pub fn active_sortable(&self) -> Option<&dyn SortableList> {
        self.active.as_ref().and_then(|o| o.as_sortable())
    }

    /// The parked overlay as a sortable list, if it is one — where a sort
    /// chosen in the dialog stacked over it lands.
    pub fn parked_sortable_mut(&mut self) -> Option<&mut dyn SortableList> {
        self.parked.as_mut().and_then(|o| o.as_sortable_mut())
    }

    /// Close the active overlay. With an overlay parked under it, that one
    /// becomes active again and `None` is returned — its opener is the parked
    /// overlay, so panel focus must not move. Otherwise return the saved
    /// opener focus to restore.
    pub fn close(&mut self) -> Option<F> {
        if let Some(parked) = self.parked.take() {
            self.active = Some(parked);
            return None;
        }
        self.active = None;
        self.saved_focus.take()
    }

    /// Close the whole stack — the active and any parked overlay — and
    /// return the saved opener focus (`None` when nothing was open).
    pub fn close_all(&mut self) -> Option<F> {
        self.parked = None;
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

    /// Route an **Overlay data** result to the active overlay. With no
    /// overlay open — or when the active overlay is not the kind the data
    /// was addressed to — the result is stale by definition and dies here;
    /// there is deliberately nothing to return, because nothing else may
    /// ever see overlay data (see CONTEXT.md **Overlay data**).
    pub fn handle_data(&mut self, data: &OverlayData, vault: &Arc<NoteVault>, tx: &AppTx) {
        if let Some(o) = &mut self.active {
            o.handle_data(data, vault, tx);
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        if let Some(o) = &mut self.parked {
            o.render(f, area, theme);
        }
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

    #[test]
    fn open_over_parks_the_active_overlay() {
        let mut host: OverlayHost<u8> = OverlayHost::new();
        host.open(Box::new(FakeOverlay(OverlayKind::NoteBrowser)), 1);
        host.open_over(Box::new(FakeOverlay(OverlayKind::Dialog)));
        assert_eq!(host.active_kind(), Some(OverlayKind::Dialog));
        assert_eq!(host.parked_kind(), Some(OverlayKind::NoteBrowser));
    }

    #[test]
    fn close_restores_the_parked_overlay_without_a_focus_token() {
        let mut host: OverlayHost<u8> = OverlayHost::new();
        host.open(Box::new(FakeOverlay(OverlayKind::NoteBrowser)), 1);
        host.open_over(Box::new(FakeOverlay(OverlayKind::Dialog)));
        assert_eq!(host.close(), None, "the opener is the parked overlay");
        assert_eq!(host.active_kind(), Some(OverlayKind::NoteBrowser));
        assert_eq!(host.parked_kind(), None);
        assert_eq!(host.close(), Some(1), "second close returns the opener");
        assert!(!host.is_open());
    }

    /// Opening a replacement overlay drops whatever was parked: closing the
    /// replacement must not resurrect a stale browser.
    #[test]
    fn open_drops_the_parked_overlay() {
        let mut host: OverlayHost<u8> = OverlayHost::new();
        host.open(Box::new(FakeOverlay(OverlayKind::NoteBrowser)), 1);
        host.open_over(Box::new(FakeOverlay(OverlayKind::Dialog)));
        host.open(Box::new(FakeOverlay(OverlayKind::SavedSearches)), 2);
        assert_eq!(host.parked_kind(), None);
        assert_eq!(host.close(), Some(1), "the original opener focus");
        assert!(!host.is_open());
    }

    /// `close_all` closes the active and the parked overlay in one go and
    /// returns the opener focus.
    #[test]
    fn close_all_closes_the_whole_stack() {
        let mut host: OverlayHost<u8> = OverlayHost::new();
        host.open(Box::new(FakeOverlay(OverlayKind::NoteBrowser)), 1);
        host.open_over(Box::new(FakeOverlay(OverlayKind::Dialog)));
        assert_eq!(host.close_all(), Some(1));
        assert!(!host.is_open());
        assert_eq!(host.parked_kind(), None);
        assert_eq!(host.close_all(), None, "nothing left to close");
    }

    #[test]
    fn render_draws_parked_and_active() {
        use ratatui::{Terminal, backend::TestBackend};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static DRAWN: AtomicUsize = AtomicUsize::new(0);
        struct Counting(OverlayKind);
        impl Overlay for Counting {
            fn kind(&self) -> OverlayKind {
                self.0
            }
            fn handle_input(&mut self, _e: &InputEvent, _tx: &AppTx) -> EventState {
                EventState::Consumed
            }
            fn render(&mut self, _f: &mut Frame, _a: Rect, _t: &Theme) {
                DRAWN.fetch_add(1, Ordering::SeqCst);
            }
        }
        let mut host: OverlayHost<u8> = OverlayHost::new();
        host.open(Box::new(Counting(OverlayKind::NoteBrowser)), 1);
        host.open_over(Box::new(Counting(OverlayKind::Dialog)));
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(40, 10)).unwrap();
        t.draw(|f| host.render(f, f.area(), &theme)).unwrap();
        assert_eq!(DRAWN.load(Ordering::SeqCst), 2);
    }
}
