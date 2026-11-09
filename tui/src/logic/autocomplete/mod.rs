//! The **autocomplete seam**: trigger detection, the popup state machine, the
//! accept edit, the debounced/generation-guarded query engine and the
//! suggestion source. Toolkit-free; the TUI draws the popup and maps keys
//! onto [`PopupCommand`], and any other front end does the same.

pub mod apply;
pub mod command;
pub mod engine;
pub mod host;
pub mod source;
pub mod state;
pub mod trigger;

pub use apply::apply_accept;
pub use command::{PopupAction, PopupCommand, PopupOutcome, apply_command, key_command};
pub use engine::{
    AcceptAction, AutocompleteEngine, AutocompleteMode, HandleKeyOutcome, RedrawCallback,
};
pub use host::AutocompleteHost;
pub use source::{SuggestionItem, SuggestionSource, VaultSuggestions};
pub use state::{AutocompleteState, DEFAULT_MAX_VISIBLE_ROWS, Suggestion};
pub use trigger::{
    TriggerContext, TriggerKind, TriggerOptions, detect_trigger, detect_trigger_with,
};
