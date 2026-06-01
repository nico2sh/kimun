//! `SearchList`: the one module behind every query-input-over-an-async-loaded
//! list surface in the TUI. See CONTEXT.md.

mod seams;

pub use seams::{Emit, Loaded, RowSource, SearchRow};
