//! The editing engines' Logic layer: key tables, vim emulation, and (as they
//! move in) the buffer and its policy. See ADR-0049.

pub mod claim;
pub mod display;
// `logic::editor::editor` is the plan's name for the `Editor` type's home;
// callers use the `Editor` re-export below.
#[allow(clippy::module_inception)]
pub mod editor;
pub mod find_replace;
pub mod frame;
pub mod handled;
pub mod jobs;
pub mod markdown;
pub mod markdown_edits;
pub mod overlay;
pub mod parse_incremental;
pub mod plain_keys;
pub mod request;
pub(crate) mod revisions;
pub mod rope_buffer;
pub mod session;
pub mod snapshot;
pub mod text_coords;
pub mod textarea;
pub mod typing_run;
pub(crate) mod vim;
pub(crate) mod vim_objects;
pub mod widener_metrics;

pub use editor::Editor;
