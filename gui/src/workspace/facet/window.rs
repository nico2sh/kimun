//! The height table of the facet view and the range it draws.
//!
//! The table keeps the rows of each piece, keyed by width, font, rem and
//! edit stamp. It gives the offsets and the column height.
//! [`Spacer`]s stand in for the pieces far from the viewport.

use std::cell::{Cell, Ref, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::{
    App, Element, ElementId, Entity, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Style, Window, px, size,
};

use crate::editor_view::{EditStamp, EditorView, measure_width};

/// Rows a piece is taken to have before it is measured.
const GUESS_ROWS: usize = 5;

/// The fixed heights the table adds to a piece's rows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Metrics {
    /// One row of a piece.
    pub line: Pixels,
    /// A dateline above the first piece of a note, with the column gap
    /// under it.
    pub dateline: Pixels,
    /// The room above a dateline that is not the column's first row.
    pub dateline_margin: Pixels,
    /// The row of a gap with hidden blocks, with one column gap.
    pub gap_row: Pixels,
    /// The crumbs row above a piece.
    pub crumbs: Pixels,
    /// What the piece's editor adds below its rows.
    pub inset: Pixels,
    /// The space between two pieces in the column.
    pub column_gap: Pixels,
}

/// Which rows a piece has besides its crumbs and its text.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Shape {
    pub dateline: bool,
    /// The gap above the piece hides blocks, so it draws a row.
    pub gap_hidden: bool,
    /// The gap after the last piece of a note hides blocks: a row under it.
    pub tail_hidden: bool,
}

/// What a measured row count is good for besides the width.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Keys {
    font: Pixels,
    rem: Pixels,
    /// A hash of the editor face's family inputs.
    face: u64,
}

/// A piece's identity: the id of its editor entity.
pub(crate) type PieceId = u64;

/// A measured row count, the piece and the stamp it holds for.
type Slot = Option<(usize, PieceId, EditStamp)>;

/// The height table: rows per piece and the fixed rows around them.
/// Offsets are prefix sums, rebuilt once after a change.
#[derive(Default)]
pub(crate) struct Heights {
    keys: Option<Keys>,
    /// The editor width the rows were measured at; `None` until known.
    width: Option<Pixels>,
    metrics: Metrics,
    shapes: Vec<Shape>,
    rows: Vec<Slot>,
    /// Where each piece starts, and the column height last.
    tops: RefCell<Vec<Pixels>>,
    stale: Cell<bool>,
    /// Something changed since the last [`Self::take_changed`].
    changed: Cell<bool>,
}

impl Heights {
    /// Use this font, rem, face and these metrics. A new font, rem or face
    /// clears every row count; the result says whether it did.
    pub(crate) fn set_metrics(&mut self, font: Pixels, rem: Pixels, face: u64, m: Metrics) -> bool {
        if self.metrics != m {
            self.touch();
        }
        self.metrics = m;
        let keys = Some(Keys { font, rem, face });
        let old = std::mem::replace(&mut self.keys, keys);
        old.is_some() && old != keys && self.clear()
    }

    /// The editor width the rows are measured at. A new width clears every
    /// row count; the result says whether it did. A width of 0 is no
    /// measure and changes nothing, and so does the first width known: the
    /// rows recorded before it were measured at it.
    pub(crate) fn set_width(&mut self, width: Pixels) -> bool {
        if width <= px(0.) {
            return false;
        }
        // Two sums of the same width can differ in their last bit.
        let same = |old: Pixels| (old - width).abs() < px(0.01);
        match self.width.replace(width) {
            None => false,
            Some(old) if same(old) => {
                self.width = Some(old);
                false
            }
            Some(_) => self.clear(),
        }
    }

    /// Both steps at once (tests).
    #[cfg(test)]
    pub(crate) fn fit(
        &mut self,
        width: Pixels,
        font: Pixels,
        rem: Pixels,
        face: u64,
        m: Metrics,
    ) -> bool {
        let metrics = self.set_metrics(font, rem, face, m);
        self.set_width(width) || metrics
    }

    /// Forget every row count. True.
    fn clear(&mut self) -> bool {
        self.rows.iter_mut().for_each(|row| *row = None);
        self.touch();
        true
    }

    /// The shape of each piece, one entry per piece. Rows already measured
    /// stay.
    pub(crate) fn set_shapes(&mut self, shapes: Vec<Shape>) {
        if self.shapes == shapes {
            return;
        }
        self.rows.resize(shapes.len(), None);
        self.shapes = shapes;
        self.touch();
    }

    /// The offsets must be built again.
    fn touch(&self) {
        self.stale.set(true);
        self.changed.set(true);
    }

    /// Whether a height changed since the last call.
    pub(crate) fn take_changed(&self) -> bool {
        self.changed.replace(false)
    }

    /// The rows of piece `i`, if `id` measured them at this `stamp`.
    pub(crate) fn rows(&self, i: usize, id: PieceId, stamp: EditStamp) -> Option<usize> {
        match self.rows.get(i)? {
            Some((rows, at, s)) if *at == id && *s == stamp => Some(*rows),
            _ => None,
        }
    }

    /// Record that piece `i`, which is `id`, has `rows` rows at `stamp`.
    pub(crate) fn set_rows(&mut self, i: usize, rows: usize, id: PieceId, stamp: EditStamp) {
        let fresh = Some((rows, id, stamp));
        if let Some(slot) = self.rows.get_mut(i)
            && *slot != fresh
        {
            *slot = fresh;
            self.touch();
        }
    }

    /// The height of piece `i`: its rows and the rows around them.
    pub(crate) fn height_of(&self, i: usize) -> Pixels {
        let m = &self.metrics;
        let shape = self.shapes.get(i).copied().unwrap_or_default();
        let rows = self
            .rows
            .get(i)
            .copied()
            .flatten()
            .map_or(GUESS_ROWS, |(r, ..)| r);
        let mut height = m.crumbs + m.inset + m.line * rows.max(1) as f32;
        if shape.dateline {
            height += m.dateline;
        }
        if shape.gap_hidden {
            height += m.gap_row;
        }
        if shape.tail_hidden {
            height += m.gap_row;
        }
        height
    }

    /// The room above piece `i` besides the column gap: a dateline's
    /// margin, except on the column's first row.
    fn margin_of(&self, i: usize) -> Pixels {
        match self.shapes.get(i) {
            Some(shape) if i > 0 && shape.dateline => self.metrics.dateline_margin,
            _ => px(0.),
        }
    }

    /// Rebuild the prefix sums if something changed.
    fn tops(&self) -> Ref<'_, Vec<Pixels>> {
        let fresh = self.tops.borrow().len() == self.shapes.len() + 1;
        if self.stale.replace(false) || !fresh {
            let mut tops = self.tops.borrow_mut();
            tops.clear();
            let mut y = px(0.);
            for i in 0..self.shapes.len() {
                y += self.margin_of(i);
                tops.push(y);
                y += self.height_of(i) + self.metrics.column_gap;
            }
            // The column ends after its last piece, not after a gap.
            tops.push((y - self.metrics.column_gap).max(px(0.)));
        }
        self.tops.borrow()
    }

    /// The height of the pieces in `range` and the gaps between them, with
    /// the margin above the first: what a spacer for them stands in for.
    pub(crate) fn span(&self, range: Range<usize>) -> Pixels {
        if range.is_empty() {
            return px(0.);
        }
        let tops = self.tops();
        let end = range.end - 1;
        tops[end] + self.height_of(end) - tops[range.start] + self.margin_of(range.start)
    }

    /// Where piece `i` starts in the column of pieces.
    pub(crate) fn top_of(&self, piece: usize) -> Pixels {
        self.tops()[piece.min(self.shapes.len())]
    }

    /// Where `j` and `k` bring piece `i`: to its dateline, else to its
    /// crumbs, under the row of the gap above it.
    pub(crate) fn step_top(&self, i: usize) -> Pixels {
        let top = self.top_of(i);
        match self.shapes.get(i) {
            Some(shape) if !shape.dateline && shape.gap_hidden => top + self.metrics.gap_row,
            _ => top,
        }
    }

    /// Where `row` lies in the column of pieces, top and bottom.
    pub(crate) fn row_span(&self, row: Row) -> (Pixels, Pixels) {
        let m = &self.metrics;
        // A gap row or `Show more`, without the column gap under it.
        let line = m.gap_row - m.column_gap;
        let (i, top) = match row {
            Row::End => {
                let top = self.total() + m.column_gap;
                return (top, top + line);
            }
            Row::Lead(i) | Row::Crumbs(i) | Row::Tail(i) => (i, self.top_of(i)),
        };
        let shape = self.shapes.get(i).copied().unwrap_or_default();
        let top = top + if shape.dateline { m.dateline } else { px(0.) };
        match row {
            Row::Lead(_) => (top, top + line),
            Row::Tail(_) => {
                let bottom = self.top_of(i) + self.height_of(i);
                (bottom - line, bottom)
            }
            _ => {
                let top = top + if shape.gap_hidden { m.gap_row } else { px(0.) };
                (top, top + m.crumbs)
            }
        }
    }

    /// The height of the column of pieces.
    pub(crate) fn total(&self) -> Pixels {
        self.top_of(self.shapes.len())
    }

    /// Whether the offsets are rebuilt at the next read (tests).
    #[cfg(test)]
    pub(crate) fn is_stale(&self) -> bool {
        self.stale.get()
    }

    /// The editor width the rows were measured at; 0 until known (tests).
    #[cfg(test)]
    pub(crate) fn width(&self) -> Pixels {
        self.width.unwrap_or(px(0.))
    }

    /// The first piece that ends below `y`.
    fn first_ending_below(&self, y: Pixels) -> usize {
        let tops = self.tops();
        let (mut lo, mut hi) = (0, self.shapes.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if tops[mid] + self.height_of(mid) <= y {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// The first piece that starts at or below `y`.
    fn first_starting_at(&self, y: Pixels) -> usize {
        let tops = self.tops();
        tops[..self.shapes.len()].partition_point(|&top| top < y)
    }
}

/// A row of the column that holds tab stops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Row {
    /// The gap row above piece `.0`.
    Lead(usize),
    /// The crumbs row of piece `.0`.
    Crumbs(usize),
    /// The gap row under piece `.0`, the last piece of its note.
    Tail(usize),
    /// The `Show more` row after the last piece.
    End,
}

/// The room above and below the column's pieces (`py_3`).
pub(crate) fn column_pad(rem: Pixels) -> Pixels {
    rem * 0.75
}

/// The space between two items of the column (`gap_3`).
pub(crate) fn column_gap(rem: Pixels) -> Pixels {
    rem * 0.75
}

/// The pieces that touch the viewport or lie within one viewport of it.
pub(crate) fn drawn_range(offset: Pixels, viewport: Pixels, heights: &Heights) -> Range<usize> {
    let viewport = viewport.max(px(0.));
    let last = (heights.total() - viewport).max(px(0.));
    let offset = offset.clamp(px(0.), last);
    let start = heights.first_ending_below(offset - viewport);
    let end = heights.first_starting_at(offset + viewport + viewport);
    start..end.max(start)
}

/// The pieces that must be drawn whatever the range: the focused one
/// takes the keys and the dragged one takes the mouse. At most two.
pub(crate) fn pinned(focused: Option<usize>, dragging: Option<usize>) -> Vec<usize> {
    let mut pins: Vec<usize> = focused.into_iter().collect();
    if let Some(d) = dragging.filter(|d| Some(*d) != focused) {
        pins.push(d);
    }
    pins
}

/// What a spacer measures its pieces with besides the width it is given.
#[derive(Clone, Copy)]
pub(crate) struct Fit {
    pub font: Pixels,
    pub rem: Pixels,
    /// A hash of the editor face's family inputs: a new family wraps anew.
    pub face: u64,
    pub metrics: Metrics,
    /// How much narrower a piece's editor is than the piece.
    pub pane: Pixels,
}

/// An element that stands in for the pieces `range` and paints nothing.
/// It is as tall as they are at the width layout gives it: a piece the
/// table has no rows for at that width and stamp is laid out there once.
pub(crate) struct Spacer {
    pub range: Range<usize>,
    /// The editors of the pieces in `range`, in order.
    pub pieces: Vec<Entity<EditorView>>,
    pub heights: Rc<RefCell<Heights>>,
    pub fit: Fit,
}

impl IntoElement for Spacer {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Spacer {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            flex_shrink: 0.0,
            ..Style::default()
        };
        let (range, fit) = (self.range.clone(), self.fit);
        let (pieces, heights) = (std::mem::take(&mut self.pieces), self.heights.clone());
        let id = window.request_measured_layout(style, move |known, available, window, cx| {
            let Some(width) = measure_width(known, available) else {
                return size(px(0.), heights.borrow().span(range.clone()));
            };
            let mut table = heights.borrow_mut();
            let inner = width - fit.pane;
            // An editor with no width lays out nothing and stores no rows:
            // one column gives far too many.
            if inner > px(0.) {
                table.set_width(inner);
                for (piece, i) in pieces.iter().zip(range.clone()) {
                    let (id, stamp) = (piece.entity_id().as_u64(), piece.read(cx).edit_stamp());
                    if table.rows(i, id, stamp).is_none() {
                        let rows = piece.update(cx, |p, cx| p.rows_at(inner, window, cx));
                        table.set_rows(i, rows, id, stamp);
                    }
                }
            }
            size(width, table.span(range.clone()))
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui_kit::Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: gpui_kit::Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
}
