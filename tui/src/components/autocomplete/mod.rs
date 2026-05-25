pub mod controller;
pub mod host;
pub mod popup;
pub mod state;
pub mod trigger;

pub use controller::{AcceptAction, AutocompleteController, AutocompleteMode, HandleKeyOutcome};
pub use host::AutocompleteHost;
pub use popup::{handle_key, render, PopupAction, PopupOutcome};
pub use state::{AutocompleteState, Suggestion, DEFAULT_MAX_VISIBLE_ROWS};
pub use trigger::{detect_trigger, TriggerContext, TriggerKind};
