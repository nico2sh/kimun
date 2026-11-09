//! Note-preview text helpers — moved to [`crate::logic::preview`]; re-exported
//! here so the TUI's existing paths keep compiling.

pub use crate::logic::preview::{style_ranges, wrap_line};
