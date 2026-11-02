//! The **Logic layer**: feature state, rules and intents with no UI crate in
//! sight. `.github/scripts/check-logic-layer.sh` fails CI if anything under this
//! tree names `ratatui`, `crossterm`, `egui` or `eframe`. See ADR-0049.

pub mod input;

pub mod editor;

pub mod message;
pub mod nav;
pub mod properties;
pub mod sort;
pub mod text_action;
