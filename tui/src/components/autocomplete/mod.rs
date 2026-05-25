pub mod controller;
pub mod host;
pub mod popup;
pub mod state;
pub mod trigger;

pub use controller::{
    AcceptAction, AutocompleteController, AutocompleteMode, HandleKeyOutcome, RedrawCallback,
};
pub use host::AutocompleteHost;
pub use popup::{PopupAction, PopupOutcome, handle_key, render};
pub use state::{AutocompleteState, DEFAULT_MAX_VISIBLE_ROWS, Suggestion};
pub use trigger::{
    TriggerContext, TriggerKind, TriggerOptions, detect_trigger, detect_trigger_with,
};
