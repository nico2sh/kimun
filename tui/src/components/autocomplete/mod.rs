pub mod popup;
pub mod state;
pub mod trigger;

pub use popup::{handle_key, render, PopupAction, PopupOutcome};
pub use state::{AutocompleteState, Suggestion, DEFAULT_MAX_VISIBLE_ROWS};
pub use trigger::{detect_trigger, TriggerContext, TriggerKind};
