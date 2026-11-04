//! The **Preview pane**, toolkit-free: the expand state machine (Collapsed →
//! Context → Full) and the content scroll anchoring ([`PreviewModel`]), the
//! hot-line seam ([`Highlight`]), and the text helpers every preview surface
//! shares ([`style_ranges`], [`wrap_line`]). A UI owns the drawing, its
//! regions (the TUI's clickable full-view header) and the line layout, and
//! hands the model the laid-out line count each frame
//! ([`PreviewModel::place`]). See CONTEXT.md (Preview pane).

mod highlight;
mod pane;

pub use self::highlight::{style_ranges, wrap_line};
pub use self::pane::{ContentScroll, ExpandState, Highlight, PreviewModel};
