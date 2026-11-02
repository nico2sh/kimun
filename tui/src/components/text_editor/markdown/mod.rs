//! Terminal rendering of markdown. The pure parse/measure half lives in
//! `crate::logic::editor::markdown`; only the spanner (still ratatui-typed) is here.

mod spanner;
pub use spanner::MarkdownSpanner;
