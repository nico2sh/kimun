//! The **Logic layer**: feature state, rules and intents with no UI crate in
//! sight. `.github/scripts/check-logic-layer.sh` fails CI if anything under this
//! tree names `ratatui`, `crossterm`, `egui`, `eframe` or `gpui`. See ADR-0049.

pub mod actions;
pub mod clipboard;
pub mod input;
pub mod layout;

pub mod doc_meta;
pub mod editor;

pub mod message;
pub mod nav;
pub mod properties;
pub mod session;
pub mod sort;
pub mod startup;
pub mod text_action;
pub mod text_match;
