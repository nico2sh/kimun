//! What the last frame drew, for the IME and for tests: the grid's place,
//! the caret and its shape, the mode line and the popup box.
//! The editor element fills it in. This module does not paint.

use std::rc::Rc;

use gpui_kit::{Bounds, Pixels, Point};

use super::EditorView;
use crate::find_bar::BarPaint;
use crate::fonts::EditorFonts;
use crate::grid::CellMetrics;
use crate::row_paint::RowPaint;

/// How the caret is drawn. Mirrors the TUI's terminal cursor shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretShape {
    /// A thin bar before the cell: where typing inserts (plain mode, vim
    /// Insert).
    Bar,
    /// The whole cell inverted, its glyph still readable: vim Normal, Visual
    /// and every other non-Insert mode, where the cursor sits *on* a char.
    Block,
}

/// What the mode line shows: vim's mode on the leading edge, the command
/// typed so far (a count, an operator: `2d`) on the trailing edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeLine {
    pub label: String,
    pub hint: Option<String>,
}

/// Where the last frame put the grid, for input that arrives outside paint
/// (the IME asking where the caret is) and for tests.
#[derive(Debug, Clone, PartialEq)]
pub struct Painted {
    /// Window position of cell (0, 0).
    pub origin: Point<Pixels>,
    pub cells: CellMetrics,
    /// Text rows of the grid; the mode line, when shown, is the row below.
    pub rows: u16,
    /// The caret cell in window coordinates, when one is drawn.
    pub caret: Option<Bounds<Pixels>>,
    /// The caret's shape, when one is drawn.
    pub caret_shape: Option<CaretShape>,
    /// The mode line, when one is drawn (vim only).
    pub mode_line: Option<ModeLine>,
    /// What each text row painted, top to bottom.
    pub row_paint: Vec<Rc<RowPaint>>,
    /// The face it was painted in.
    pub fonts: EditorFonts,
    /// Where a block caret's repainted glyph starts, when one is drawn.
    pub caret_glyph: Option<Point<Pixels>>,
    /// The find bar, when open: docked in the rows below the text (above the
    /// mode line). While it is open `caret` is the bar's.
    pub find_bar: Option<BarPaint>,
    /// The autocomplete popup, when one is drawn: its box in window
    /// coordinates.
    pub popup: Option<Bounds<Pixels>>,
    /// Where the pointer is the pointing hand: the cells of every follow
    /// target drawn, in window coordinates. The text cursor elsewhere.
    /// Kept for tests only: the element sets the cursor from its own copy.
    #[cfg(test)]
    pub hand_areas: Vec<Bounds<Pixels>>,
}

impl Painted {
    /// Whether the pointer at `at` shows the pointing hand (tests).
    #[cfg(test)]
    pub(crate) fn hand_at(&self, at: Point<Pixels>) -> bool {
        self.hand_areas.iter().any(|area| area.contains(&at))
    }
}

impl EditorView {
    /// The caret's shape for the editor's mode: the TUI's rule (bar in vim
    /// Insert, block in every other vim mode). The plain editor always types,
    /// so it gets the bar.
    pub(crate) fn caret_shape(&self) -> CaretShape {
        match self.editor.modal_is_insert() {
            Some(false) => CaretShape::Block,
            Some(true) | None => CaretShape::Bar,
        }
    }

    /// The mode line's content; `None` for the plain editor, which has no mode.
    pub(crate) fn mode_line(&self) -> Option<ModeLine> {
        self.editor.mode_label().map(|label| ModeLine {
            label,
            hint: self.editor.pending_input_hint(),
        })
    }
}
