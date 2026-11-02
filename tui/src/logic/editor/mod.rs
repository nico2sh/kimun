//! The editing engines' Logic layer: key tables, vim emulation, and (as they
//! move in) the buffer and its policy. See ADR-0049.

pub mod display;
pub mod find_replace;
pub mod frame;
pub mod markdown;
pub mod markdown_edits;
pub mod overlay;
pub mod parse_incremental;
pub mod plain_keys;
pub mod request;
pub(crate) mod revisions;
pub mod rope_buffer;
pub mod snapshot;
pub mod text_coords;
pub mod typing_run;
pub(crate) mod vim;
pub(crate) mod vim_objects;
pub mod widener_metrics;
