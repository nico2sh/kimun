//! The toolkit-free list engine behind every query-over-an-async-loaded list:
//! the row seams ([`ListRow`], [`RowSource`], [`Emit`]), the generation-stamped
//! loader, and [`ListCore`] (rows, display order, filter, leading row,
//! selection), and [`ResolvingRowSource`], which resolves query variables
//! before an inner source sees the query. The TUI's `SearchList` composes a
//! `ListCore` with its input, autocomplete and hit-testing. See CONTEXT.md (SearchList).

mod core;
mod load;
mod resolving;
mod seams;

pub use self::core::{ListCore, ListHolder, ListSetup, Polled, fuzzy_rank};
pub use self::resolving::{ResolvingRowSource, Unresolvable};
pub use self::seams::{
    Emit, Filter, ListRow, Loaded, OrderFn, RankFn, RowSource, StaticRowSource, YankTarget,
};
