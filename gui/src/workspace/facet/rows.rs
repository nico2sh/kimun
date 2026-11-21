//! The numbers of the facet column's rows that the drawing and the height
//! table both read. A change here moves the drawn row and the table's
//! height of it together.
//! Every value is a `Rems`. The height table turns it into pixels.

use gpui_kit::{Rems, rems};

pub(crate) use crate::editor_view::{PANE_PAD_X_REMS, PANE_PAD_Y_REMS};

/// The column's padding above and below its items (was `py_3`).
const COLUMN_PAD_REMS: f32 = 0.75;
/// The space between two items of the column (was `gap_3`).
const COLUMN_GAP_REMS: f32 = 0.75;
/// The room above a dateline that is not the column's first row (was
/// `mt_3`).
const DATELINE_MARGIN_REMS: f32 = 0.75;
/// The font size of a gap row's count and a dateline's meta (was
/// `text_xs`, which sets the font size only: the line height stays the
/// text style's).
const SMALL_TEXT_REMS: f32 = 0.75;
/// The height of a button and a pill, and a button's least width (was
/// `h_6`, `min_w_6`, and `size_6` for an icon chip).
const BUTTON_REMS: f32 = 1.5;

/// The values a test can move with `set_override`, by their place.
pub(crate) const BUTTON: usize = 0;
pub(crate) const COLUMN_GAP: usize = 1;
pub(crate) const COLUMN_PAD: usize = 2;

#[cfg(test)]
thread_local! {
    /// A test's stand-ins for [`BUTTON`], [`COLUMN_GAP`] and [`COLUMN_PAD`].
    static OVERRIDES: std::cell::Cell<[Option<f32>; 3]> = const { std::cell::Cell::new([None; 3]) };
}

/// Use `rems` for the value `which` from now on, `None` for the default
/// (tests).
#[cfg(test)]
pub(crate) fn set_override(which: usize, rems: Option<f32>) {
    OVERRIDES.with(|o| {
        let mut all = o.get();
        all[which] = rems;
        o.set(all);
    });
}

/// `default` rems, or what a test set for `_which`.
fn value(_which: usize, default: f32) -> Rems {
    #[cfg(test)]
    if let Some(set) = OVERRIDES.with(|o| o.get())[_which] {
        return rems(set);
    }
    rems(default)
}

/// The column's padding above and below its items.
pub(crate) fn column_pad() -> Rems {
    value(COLUMN_PAD, COLUMN_PAD_REMS)
}

/// The space between two items of the column.
pub(crate) fn column_gap() -> Rems {
    value(COLUMN_GAP, COLUMN_GAP_REMS)
}

/// The room above a dateline that is not the column's first row.
pub(crate) fn dateline_margin() -> Rems {
    rems(DATELINE_MARGIN_REMS)
}

/// The font size of a gap row's count and a dateline's meta.
pub(crate) fn small_text() -> Rems {
    rems(SMALL_TEXT_REMS)
}

/// The height of a button and a pill, and a button's least width.
pub(crate) fn button() -> Rems {
    value(BUTTON, BUTTON_REMS)
}

/// How much narrower a piece's editor is than the piece: its pane's
/// padding at both sides.
pub(crate) fn pane_x() -> Rems {
    rems(PANE_PAD_X_REMS * 2.)
}

/// What a piece's editor pane adds above and below its rows.
pub(crate) fn pane_y() -> Rems {
    rems(PANE_PAD_Y_REMS * 2.)
}
