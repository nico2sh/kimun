//! The note-scoped drawer views' data, toolkit-free: the rows and sources
//! behind **TAGS** ([`TagEntry`], [`TagSource`]), **LINKS** ([`LinkEntry`],
//! [`LinksSource`]), **OUTLINE** ([`OutlineEntry`], [`OutlineModel`]) and
//! **SEM** ([`SemanticSource`], rows are FILES' `FileListEntry`).
//! Rows are [`ListRow`](crate::logic::list::ListRow)s and the sources are
//! [`RowSource`](crate::logic::list::RowSource)s, so a UI drives them with a
//! [`ListCore`](crate::logic::list::ListCore) — the TUI's drawer views through
//! their `QueryListPanel` body. Drawing (the TUI's `SearchRow`) stays in the
//! UI layer. See adr/0049.

mod links;
mod outline;
mod semantic;
mod tags;

pub use self::links::{LinkEntry, LinksSource, link_entries, outgoing_entries};
pub use self::outline::{Changed, OutlineEntry, OutlineModel};
pub use self::semantic::{SemanticSource, chunks_to_entries};
pub use self::tags::{TagEntry, TagSource, tag_entries};

#[cfg(test)]
mod tests;
