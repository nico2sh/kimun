//! The link chooser's rows: a fixed list — every note a followed link resolved
//! to when it named several — narrowed by a case-insensitive substring filter.

use async_trait::async_trait;

use crate::logic::files::FileListEntry;
use crate::logic::list::{Emit, RowSource};
use crate::logic::pickers::NoteRecord;

/// One row per result, in the given order: filename, display title
/// (`<no title>` for a blank one), never a journal date.
pub fn link_rows(results: Vec<NoteRecord>) -> Vec<FileListEntry> {
    results
        .into_iter()
        .map(|(entry, content)| FileListEntry::Note {
            filename: entry.path.get_parent_path().1,
            title: FileListEntry::display_title(content.title),
            path: entry.path,
            journal_date: None,
            is_open: false,
        })
        .collect()
}

/// The note rows of `rows` whose title, filename or path contains `query`
/// (ignoring case), in order; every row for a blank query.
pub fn filter_link_rows(rows: &[FileListEntry], query: &str) -> Vec<FileListEntry> {
    if query.is_empty() {
        return rows.to_vec();
    }
    let q = query.to_lowercase();
    rows.iter()
        .filter(|e| match e {
            FileListEntry::Note {
                title,
                filename,
                path,
                ..
            } => {
                title.to_lowercase().contains(&q)
                    || filename.to_lowercase().contains(&q)
                    || path.to_string().to_lowercase().contains(&q)
            }
            _ => false,
        })
        .cloned()
        .collect()
}

/// The chooser as a [`RowSource`] over [`link_rows`], filtered per query with
/// [`filter_link_rows`]. No leading row.
pub struct LinkResultsProvider {
    entries: Vec<FileListEntry>,
}

impl LinkResultsProvider {
    /// A chooser listing every one of `results`.
    pub fn from_results(results: Vec<NoteRecord>) -> Self {
        Self {
            entries: link_rows(results),
        }
    }
}

#[async_trait]
impl RowSource<FileListEntry> for LinkResultsProvider {
    async fn load(&self, query: &str, emit: Emit<FileListEntry>) {
        emit.replace(filter_link_rows(&self.entries, query));
    }
}
