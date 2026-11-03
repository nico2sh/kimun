//! The toolkit-free list engine behind every query-over-an-async-loaded list:
//! the row seams ([`ListRow`], [`RowSource`], [`Emit`]), the generation-stamped
//! loader, and [`ListCore`] (rows, display order, filter, leading row,
//! selection). The TUI's `SearchList` composes a `ListCore` with its input,
//! autocomplete and hit-testing. See CONTEXT.md (SearchList).

mod core;
mod load;
mod seams;

pub use self::core::{ListCore, ListHolder, ListSetup, Polled};
pub use self::seams::{
    Emit, Filter, ListRow, Loaded, OrderFn, RankFn, RowSource, StaticRowSource, YankTarget,
};
