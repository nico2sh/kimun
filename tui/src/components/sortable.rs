//! One contract for every list the sort dialog can sort — the sidebar, the
//! Query panel, the Ctrl+K search browser and the Ctrl+O file finder. The dialog is built from a
//! list's [`SortState`] and [`SortableList::allows_property`], and every
//! selection it emits is applied through [`SortableList::apply_sort`], so the
//! targets cannot drift apart in how a sort lands. The query order directive
//! helpers and the chip label live in [`crate::logic::sort`] (re-exported here).

use crate::logic::message::AppTx;
pub use crate::logic::sort::{
    PropertySort, SORT_CHIP_MAX_NAME, directive_of_query, is_blank_property, order_of_query,
    property_key, query_with_sort, sort_chip_label,
};
use crate::logic::sort::{SortField, SortOrder, SortState};

/// [`sort_chip_label`] as a border-title line in the theme's action style,
/// ready for a [`BorderChip`](crate::components::clickable::BorderChip).
pub fn sort_chip_line(
    field: &SortField,
    order: SortOrder,
    theme: &crate::settings::themes::Theme,
) -> ratatui::text::Line<'static> {
    ratatui::text::Line::from(ratatui::text::Span::styled(
        sort_chip_label(field, order),
        theme.action(),
    ))
}

/// A list the sort dialog can sort.
pub trait SortableList {
    fn sort_state(&self) -> SortState;
    /// Apply a selection from the sort dialog. Property sorts are only
    /// offered where [`Self::allows_property`] says so.
    fn apply_sort(&mut self, state: &SortState, tx: &AppTx);
    /// Whether the dialog offers Property. Query-backed lists put the key in
    /// the query; listings fetch values with [`PropertySort`].
    fn allows_property(&self) -> bool;
    /// The rows are in the list's natural order (recency, match rank), not
    /// any sort the dialog offers — it then shows "Unsorted" until a pick.
    /// [`Self::sort_state`] still names the field / order the first toggle
    /// starts from. Only the file finder and the Ctrl+K recents say so.
    fn is_unsorted(&self) -> bool {
        false
    }
}
