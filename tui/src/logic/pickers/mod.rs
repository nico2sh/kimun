//! Note pickers, toolkit-free: the rows behind the file finder
//! ([`FileFinderProvider`]) and the link chooser ([`LinkResultsProvider`]),
//! and what choosing a row does ([`picker_open`]). Rows are FILES'
//! [`FileListEntry`](crate::logic::files::FileListEntry) and the sources are
//! [`RowSource`](crate::logic::list::RowSource)s, so a UI drives them with a
//! [`ListCore`](crate::logic::list::ListCore) — the TUI's note browser through
//! its `SearchList`. The Ctrl+K search browser's query-syntax source is
//! [`resolving_search_source`]. See adr/0049.

mod finder;
mod link_results;
mod open;
mod search;

pub use self::finder::{
    FileFinderProvider, NoteRecord, create_row, finder_order, finder_rows, fuzzy_match, note_row,
    recency_order,
};
pub use self::link_results::{LinkResultsProvider, filter_link_rows, link_rows};
pub use self::open::{PickerOpen, picker_open};
pub use self::search::{resolving_search_source, resolving_search_source_with_create};
pub use crate::logic::files::format_journal_date;

#[cfg(test)]
mod tests;
