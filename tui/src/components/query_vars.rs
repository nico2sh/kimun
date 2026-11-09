//! Query variables — moved to [`crate::logic::query`]; re-exported here so the
//! TUI's existing paths keep compiling.

pub use crate::logic::query::{
    QueryContext, VAR_NOTE, query_has_variables, query_is_unresolvable, resolve_query,
};
