//! The element ids of the facet view.
//!
//! It owns the element ids that drawing and tests share, so both name an
//! element the same way.
//! It does not own the elements, their state or their drawing. The render
//! code and the view do that.

use gpui_kit::SharedString;
use kimun_core::note::{Facet, FacetKind};

use super::ListKind;

/// Element ids, for tests.
pub(crate) const FACET_VIEW_ID: &str = "facet-view";
pub(crate) const SHOW_MORE_ID: &str = "facet-show-more";
pub(crate) const MORE_QUERY_ID: &str = "facet-more-query";
/// The open list's frame (the popover under its chip).
pub(crate) const MORE_LIST_ID: &str = "facet-more-list";
pub(crate) const FACET_BAR_ID: &str = "facet-bar";
/// The frame that clips the bar while its height moves. It is not drawn at
/// rest. A debug selector, not an element id: the frame must not change the
/// element path of the bar inside it.
pub(crate) const BAR_FRAME_ID: &str = "facet-bar-frame";
pub(crate) const SUMMARY_ID: &str = "facet-summary";
/// The message that stands where the pieces would be: the note is empty, or
/// the picks select nothing.
pub(crate) const EMPTY_ID: &str = "facet-empty";
/// The `Remove last pick` button of the empty message.
pub(crate) const REMOVE_LAST_ID: &str = "facet-remove-last";
/// The one-line hint under the bar, until the first pick.
pub(crate) const HINT_ID: &str = "facet-hint";
/// The frame that clips the hint while it collapses after the first pick.
/// A debug selector, not an element id, like [`BAR_FRAME_ID`].
pub(crate) const HINT_FRAME_ID: &str = "facet-hint-frame";
pub(crate) const CLEAR_ID: &str = "facet-clear";
/// The summary row's `Undo` button, after a clear.
pub(crate) const UNDO_ID: &str = "facet-undo";
/// The summary row's `Picks cleared` text, after a clear.
pub(crate) const CLEARED_ID: &str = "facet-cleared";
/// Row 1 of the bar: the kind menu chips and the search chip.
pub(crate) const LINE_ID: &str = "facet-line";
/// The muted last line of a list that draws only [`super::line::LIST_CAP`] matches.
pub(crate) const LIST_REST_ID: &str = "facet-list-rest";
/// The muted line of a list whose filter matches nothing.
pub(crate) const LIST_EMPTY_ID: &str = "facet-list-empty";
/// The scrolling column of pieces.
pub(crate) const COLUMN_ID: &str = "facet-column";
/// A collection's `Refresh` text button, in its empty state.
pub(crate) const REFRESH_ID: &str = "facet-refresh";
/// A collection's muted line under the bar ([`mod@super::collection`]).
pub(crate) const COLLECTION_LINE_ID: &str = "facet-collection-line";
/// The muted `100 of 340 pieces shown` beside `Show N more`.
pub(crate) const SHOW_MORE_COUNT_ID: &str = "facet-show-more-count";

fn kind_slug(kind: FacetKind) -> &'static str {
    match kind {
        FacetKind::Heading => "heading",
        FacetKind::Tag => "tag",
        FacetKind::NoteLink => "link",
    }
}

/// The picked pill of `facet` in the picks row.
pub(crate) fn pill_id(facet: &Facet) -> SharedString {
    format!("facet-pill:{}:{}", kind_slug(facet.kind), facet.key).into()
}

/// The count on the picked pill of `facet`.
#[cfg(test)]
pub(crate) fn pill_count_id(facet: &Facet) -> SharedString {
    crate::workspace::facet_ui::pill_count(&pill_id(facet))
}

/// The remove mark on the picked pill of `facet`.
#[cfg(test)]
pub(crate) fn remove_mark_id(facet: &Facet) -> SharedString {
    crate::workspace::facet_ui::remove_mark(&pill_id(facet))
}

/// The outline icon in front of the picked heading pill of `facet`.
#[cfg(test)]
pub(crate) fn outline_mark_id(facet: &Facet) -> SharedString {
    crate::workspace::facet_ui::outline_mark(&pill_id(facet))
}

/// The row of `facet` in an open list.
pub(crate) fn more_item_id(facet: &Facet) -> SharedString {
    format!("facet-more-item:{}:{}", kind_slug(facet.kind), facet.key).into()
}

/// The tick before the label of `facet`'s row, while it is picked.
#[cfg(test)]
pub(crate) fn row_tick_id(facet: &Facet) -> SharedString {
    crate::workspace::facet_ui::row_tick(&more_item_id(facet))
}

/// The in-view count at the right end of `facet`'s row.
#[cfg(test)]
pub(crate) fn row_count_id(facet: &Facet) -> SharedString {
    crate::workspace::facet_ui::row_count(&more_item_id(facet))
}

/// The label over `kind`'s group in the list of every kind (`Tags`,
/// `Links`, `Sections`).
pub(crate) fn list_group_id(kind: FacetKind) -> SharedString {
    format!("facet-list-group:{}", kind_slug(kind)).into()
}

/// The menu chip that opens `kind`'s list (the search chip for
/// [`ListKind::All`]).
pub(crate) fn menu_id(kind: ListKind) -> SharedString {
    match kind {
        ListKind::Tags => "facet-menu-tags".into(),
        ListKind::Links => "facet-menu-links".into(),
        ListKind::Sections => "facet-menu-sections".into(),
        ListKind::All => "facet-menu-all".into(),
    }
}

/// Chunk `i`.
pub(crate) fn chunk_id(i: usize) -> SharedString {
    format!("facet-chunk:{i}").into()
}

/// Chunk `i`'s `Open note` button.
pub(crate) fn open_id(i: usize) -> SharedString {
    format!("facet-open:{i}").into()
}

/// Gap `g`'s button that reveals the block below the chunk above it. In a
/// single-note view gap `g` sits before chunk `g`, and the last gap follows
/// the last chunk ([`mod@super::members`] numbers the gaps of several notes).
pub(crate) fn gap_down_id(g: usize) -> SharedString {
    format!("facet-gap-down:{g}").into()
}

/// Gap `g`'s button that reveals the block above the chunk below it.
pub(crate) fn gap_up_id(g: usize) -> SharedString {
    format!("facet-gap-up:{g}").into()
}

/// Gap `g`'s `Show all` button, while it hides
/// [`kimun_core::note::GAP_SHOW_ALL_MAX`] blocks or fewer.
pub(crate) fn show_all_id(g: usize) -> SharedString {
    format!("facet-show-all:{g}").into()
}

/// Gap `g`'s count of hidden blocks (`3 blocks hidden`).
pub(crate) fn gap_count_id(g: usize) -> SharedString {
    format!("facet-gap-count:{g}").into()
}

/// Crumb `index` (outermost first) of chunk `chunk`'s breadcrumb.
pub(crate) fn crumb_id(chunk: usize, index: usize) -> SharedString {
    format!("facet-crumb:{chunk}:{index}").into()
}

/// The dateline of member `member` (its index in the member list).
pub(crate) fn dateline_id(member: usize) -> SharedString {
    format!("facet-dateline:{member}").into()
}

/// The title on member `member`'s dateline.
pub(crate) fn dateline_title_id(member: usize) -> SharedString {
    format!("facet-dateline-title:{member}").into()
}

/// The hairline on member `member`'s dateline.
pub(crate) fn dateline_rule_id(member: usize) -> SharedString {
    format!("facet-dateline-rule:{member}").into()
}

/// The folder on member `member`'s dateline, when it has one.
pub(crate) fn dateline_folder_id(member: usize) -> SharedString {
    format!("facet-dateline-folder:{member}").into()
}

/// The piece count on member `member`'s dateline, at its right end; its
/// name is the whole meta text ([`dateline_meta`](super::collection::dateline_meta)).
pub(crate) fn dateline_meta_id(member: usize) -> SharedString {
    format!("facet-dateline-meta:{member}").into()
}
