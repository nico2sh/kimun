//! The TUI half of autocomplete: the popup's drawing and the key/mouse
//! mapping. The rules live in [`crate::logic::autocomplete`]; its public
//! names are re-exported here so hosts keep one import path.

pub mod controller;
pub mod popup;

pub use crate::logic::autocomplete::{
    AcceptAction, AutocompleteHost, AutocompleteMode, AutocompleteState, DEFAULT_MAX_VISIBLE_ROWS,
    HandleKeyOutcome, PopupAction, PopupOutcome, RedrawCallback, Suggestion, TriggerContext,
    TriggerKind, TriggerOptions, detect_trigger, detect_trigger_with,
};
pub use controller::AutocompleteController;
pub use popup::render;
