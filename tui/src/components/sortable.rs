//! One contract for every list the sort dialog can sort — the sidebar, the
//! Query panel, the Ctrl+K search browser and the Ctrl+O file finder. The dialog is built from a
//! list's [`SortState`] and [`SortableList::allows_property`], and every
//! selection it emits is applied through [`SortableList::apply_sort`], so the
//! targets cannot drift apart in how a sort lands.

use kimun_core::{OrderBy, OrderField, SearchTerms, with_order_directive};

use crate::logic::message::AppTx;
pub use crate::logic::sort::{PropertySort, is_blank_property, property_key};
use crate::logic::sort::{SortField, SortOrder, SortState};

/// The clickable sort chip a sortable panel draws right-aligned on its
/// search box border: ` Name ↑ `, ` Title ↓ `, ` due ↑ ` (a property sort
/// shows its key). Plain text, so it reads the same in every font. A click
/// on it opens the sort dialog — the mouse's `Ctrl+R`. A property key longer
/// than [`SORT_CHIP_MAX_NAME`] cells is cut with `…`, so it never crowds the
/// search box's own title.
pub fn sort_chip_label(field: &SortField, order: SortOrder) -> String {
    let name = match field {
        SortField::Name => "Name",
        SortField::Title => "Title",
        SortField::Property(key) => key.as_str(),
    };
    format!(
        " {} {} ",
        truncate_cells(name, SORT_CHIP_MAX_NAME),
        order.label()
    )
}

/// Widest field name the sort chip shows, in terminal cells (`…` included).
pub const SORT_CHIP_MAX_NAME: usize = 12;

/// `s` cut to at most `max` display cells, ending in `…` when cut.
fn truncate_cells(s: &str, max: usize) -> std::borrow::Cow<'_, str> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if s.width() <= max {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    std::borrow::Cow::Owned(out)
}

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

/// The sort a query string asks for, read from its first order directive.
/// `(Name, Ascending)` when the query has none. Shared by every query-backed
/// list so the dialog opens on what the query actually says.
pub fn order_of_query(query: &str) -> (SortField, SortOrder) {
    directive_of_query(query).unwrap_or((SortField::Name, SortOrder::Ascending))
}

/// The sort a query's first usable order directive asks for; `None` when it
/// has none (a bare `or:prop:` with no key is not usable).
pub fn directive_of_query(query: &str) -> Option<(SortField, SortOrder)> {
    let st = SearchTerms::from_query_string(query);
    let (field, asc) = match st.order_by.first()? {
        OrderBy::Title { asc } => (SortField::Title, *asc),
        OrderBy::FileName { asc } => (SortField::Name, *asc),
        OrderBy::Property { key, asc } => (SortField::Property(key.clone()), *asc),
    };
    let order = if asc {
        SortOrder::Ascending
    } else {
        SortOrder::Descending
    };
    Some((field, order))
}

/// `query` with its order directive replaced by `field` / `order` — the query
/// string is the single source of truth for a query-backed list's sort.
/// `None` for a property sort with no key yet (not a usable order).
pub fn query_with_sort(query: &str, field: &SortField, order: SortOrder) -> Option<String> {
    if is_blank_property(field) {
        return None;
    }
    let order_field = match field {
        SortField::Name => OrderField::FileName,
        SortField::Title => OrderField::Title,
        SortField::Property(key) => OrderField::Property(key.clone()),
    };
    let asc = matches!(order, SortOrder::Ascending);
    Some(with_order_directive(query, order_field, asc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_chip_names_the_field_and_cuts_long_keys() {
        assert_eq!(
            sort_chip_label(&SortField::Name, SortOrder::Ascending),
            " Name ↑ "
        );
        assert_eq!(
            sort_chip_label(&SortField::Property("due".into()), SortOrder::Descending),
            " due ↓ "
        );
        let long = sort_chip_label(
            &SortField::Property("a-very-long-property-name".into()),
            SortOrder::Ascending,
        );
        assert_eq!(long, " a-very-long… ↑ ");
        let name = long.trim().trim_end_matches(" ↑");
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(name),
            SORT_CHIP_MAX_NAME
        );
    }

    #[test]
    fn order_of_query_reads_the_directive() {
        assert_eq!(
            order_of_query("widget -or:title"),
            (SortField::Title, SortOrder::Descending)
        );
        assert_eq!(
            order_of_query("#work or:prop:due"),
            (SortField::Property("due".into()), SortOrder::Ascending)
        );
        assert_eq!(
            order_of_query("widget"),
            (SortField::Name, SortOrder::Ascending)
        );
    }

    #[test]
    fn query_with_sort_round_trips_and_skips_empty_keys() {
        let q = query_with_sort("x", &SortField::Name, SortOrder::Descending).unwrap();
        assert_eq!(order_of_query(&q), (SortField::Name, SortOrder::Descending));
        assert_eq!(
            query_with_sort("x", &SortField::Property(" ".into()), SortOrder::Ascending),
            None
        );
    }
}
