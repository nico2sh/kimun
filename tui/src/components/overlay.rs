//! The `Overlay` trait and its supporting types — the contract every editor
//! overlay (note browser, Saved Searches modal, or dialog) implements so the
//! `OverlayHost` can route input / app-messages / render to it uniformly.

use std::sync::Arc;

use kimun_core::NoteVault;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::settings::themes::Theme;

/// Identifies which overlay is active — used for toggle, focus label, and hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    NoteBrowser,
    SavedSearches,
    CommandPalette,
    Dialog,
}

impl OverlayKind {
    /// Footer label for this overlay kind.
    pub fn label(&self) -> &'static str {
        match self {
            OverlayKind::NoteBrowser => "NOTE BROWSER",
            OverlayKind::SavedSearches => "SAVED SEARCHES",
            OverlayKind::CommandPalette => "COMMANDS",
            OverlayKind::Dialog => "DIALOG",
        }
    }
}

/// Outcome of routing an `AppEvent` to the active overlay. Overlays never
/// request their own dismissal here: dialogs close by emitting the
/// `AppEvent::CloseOverlay` event, which the editor handles separately.
#[derive(Debug)]
pub enum OverlayMsg {
    /// The overlay did not recognise the message.
    NotConsumed,
    /// The overlay handled the message and stays open.
    Consumed,
}

// No `Send` bound: `EditorScreen` (which hosts overlays) is itself non-`Send`
// because of its `ratatui-textarea` buffer (see `AppScreen` in `app_screen/mod.rs`),
// and it is only ever driven on the main `block_on` future, never spawned.
pub trait Overlay {
    fn kind(&self) -> OverlayKind;
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState;
    fn handle_app_message(
        &mut self,
        _msg: &AppEvent,
        _vault: &Arc<NoteVault>,
        _tx: &AppTx,
    ) -> OverlayMsg {
        OverlayMsg::NotConsumed
    }
    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme);
    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![]
    }
    /// The query string this overlay holds, if it is query-backed (the note
    /// browser). Used by the editor's save-current-query action to source the
    /// query from the active overlay. Defaults to `None` for non-query overlays.
    fn query(&self) -> Option<&str> {
        None
    }
    /// The saved-search name this overlay's query came from (its breadcrumb
    /// provenance), if any. Used to pre-fill the save-search dialog's name.
    /// Defaults to `None` for overlays without a breadcrumb.
    fn saved_search_provenance(&self) -> Option<&str> {
        None
    }
}
