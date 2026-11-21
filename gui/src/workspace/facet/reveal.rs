//! The column reveal: a column stop that takes the keyboard scrolls into view.
//!
//! The stop's row comes from the height table, so the reveal is right in
//! the frame the stop arrives, before the drawn range is picked. The column
//! moves by the least amount that shows the row with its focus ring whole.
//! Tab into the column enters at piece 0, Shift+Tab at the last piece.

use gpui_kit::{Context, Pixels, Window, point, px};
use kimun_core::note::GAP_SHOW_ALL_MAX;

use super::FacetView;
use super::body::PieceGaps;
use super::keys::Stop;
use super::window::{Heights, Row, column_pad};
use crate::workspace::facet_ui::ring_room;

/// The row of the column that holds `stop`, from each piece's gaps. `None`
/// for a stop of the bar or the empty message.
pub(super) fn stop_row(stop: &Stop, gaps: &[PieceGaps]) -> Option<Row> {
    match *stop {
        Stop::Crumb(i, _) | Stop::Open(i) => Some(Row::Crumbs(i)),
        Stop::GapDown(g) | Stop::GapUp(g) | Stop::ShowAll(g) => {
            gaps.iter().enumerate().find_map(|(i, p)| {
                (p.lead == Some(g))
                    .then_some(Row::Lead(i))
                    .or((p.tail == Some(g)).then_some(Row::Tail(i)))
            })
        }
        Stop::ShowMore => Some(Row::End),
        _ => None,
    }
}

/// The offset (≥ 0, down the column) that shows `top..bottom` with `room`
/// around it in a viewport `viewport` tall, moving the least from
/// `offset`. A row taller than the viewport shows its top. `None` when the
/// row shows already.
pub(super) fn reveal_offset(
    (top, bottom): (Pixels, Pixels),
    offset: Pixels,
    viewport: Pixels,
    room: Pixels,
) -> Option<Pixels> {
    let (top, bottom) = (top - room, bottom + room);
    if top < offset {
        Some(top.max(px(0.)))
    } else if bottom > offset + viewport {
        Some((bottom - viewport).min(top))
    } else {
        None
    }
}

impl FacetView {
    /// When a column stop took the keyboard since the last frame (by Tab
    /// most often), scroll the column so its row shows with its ring.
    /// Called before the drawn range is picked, so the range follows.
    pub(super) fn reveal_column(&self, gaps: &[PieceGaps], heights: &Heights, rem: Pixels) {
        let row = self
            .stops
            .borrow()
            .arrived()
            .and_then(|stop| stop_row(stop, gaps));
        if let Some(row) = row {
            self.reveal_row(row, heights, rem);
        }
    }

    /// Scroll the column by the least amount that shows `row` and its ring.
    fn reveal_row(&self, row: Row, heights: &Heights, rem: Pixels) {
        let viewport = self.scroll.bounds().size.height;
        if viewport <= px(0.) {
            return;
        }
        // The table starts under the column's `py_3`.
        let pad = column_pad(rem);
        let (top, bottom) = heights.row_span(row);
        let now = self.scroll.offset();
        let span = (pad + top, pad + bottom);
        if let Some(y) = reveal_offset(span, -now.y, viewport, ring_room()) {
            self.scroll.set_offset(point(now.x, -y));
        }
    }

    /// Tab from the stop just before the column, or Shift+Tab (`back`) from
    /// the stop just after it: the keyboard goes to the first stop of piece
    /// 0 (the last stop of the last piece), as when every piece is drawn,
    /// and the column scrolls to it. The stop takes the keyboard in the next
    /// frame (`focus_after`). True when taken.
    pub(super) fn enter_column(
        &mut self,
        back: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let n = self.members.len();
        let gaps = self.piece_gaps(n);
        let in_column = |s: &Stop| stop_row(s, &gaps).is_some_and(|r| r != Row::End);
        let entering = {
            let stops = self.stops.borrow();
            let drawn = stops.drawn();
            let at = stops.focused(window).filter(|s| !in_column(s));
            let at = at.and_then(|f| drawn.iter().position(|s| *s == f));
            let next = at.and_then(|at| match back {
                true => at.checked_sub(1).and_then(|i| drawn.get(i)),
                false => drawn.get(at + 1),
            });
            next.is_some_and(in_column)
        };
        let piece = if back {
            n.checked_sub(1)
        } else {
            (n > 0).then_some(0)
        };
        let Some(piece) = piece.filter(|_| entering) else {
            return false;
        };
        let stops = self.piece_stops(piece, gaps[piece]);
        let target = if back { stops.last() } else { stops.first() };
        let Some(target) = target.cloned() else {
            return false;
        };
        if back {
            if let Some(row) = stop_row(&target, &gaps) {
                self.reveal_row(row, &self.heights.borrow(), window.rem_size());
            }
        } else {
            let x = self.scroll.offset().x;
            self.scroll.set_offset(point(x, px(0.)));
        }
        self.focus_after = Some(target);
        cx.notify();
        true
    }

    /// Piece `i`'s stops in the order they are painted: the gap row above
    /// it, its crumbs, `Open note`, and the gap row under it.
    fn piece_stops(&self, i: usize, gaps: PieceGaps) -> Vec<Stop> {
        let mut out = self.gap_stops(gaps.lead);
        let crumbs = self.members.chunk(i).map_or(0, |c| c.crumbs.len());
        out.extend((0..crumbs).map(|k| Stop::Crumb(i, k)));
        out.push(Stop::Open(i));
        out.extend(self.gap_stops(gaps.tail));
        out
    }

    /// The stops of gap `g`'s row, as `render_gap` paints them.
    fn gap_stops(&self, g: Option<usize>) -> Vec<Stop> {
        let Some((g, gap)) = g.and_then(|g| Some((g, *self.members.gaps.get(g)?))) else {
            return Vec::new();
        };
        let hidden = self.members.gap_hidden(g);
        if hidden == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(3);
        if gap.above.is_some() {
            out.push(Stop::GapDown(g));
        }
        if gap.below.is_some() {
            out.push(Stop::GapUp(g));
        }
        if hidden <= GAP_SHOW_ALL_MAX {
            out.push(Stop::ShowAll(g));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;

    use super::reveal_offset;

    #[test]
    fn a_row_in_view_does_not_move_the_column() {
        assert_eq!(
            reveal_offset((px(110.), px(140.)), px(100.), px(100.), px(3.)),
            None
        );
    }

    #[test]
    fn a_row_above_comes_to_the_top_with_room_for_the_ring() {
        let y = reveal_offset((px(50.), px(80.)), px(100.), px(100.), px(3.));
        assert_eq!(y, Some(px(47.)));
        let y = reveal_offset((px(1.), px(30.)), px(100.), px(100.), px(3.));
        assert_eq!(y, Some(px(0.)), "never above the column's top");
    }

    #[test]
    fn a_row_below_comes_to_the_bottom_and_a_tall_row_shows_its_top() {
        let y = reveal_offset((px(250.), px(280.)), px(100.), px(100.), px(3.));
        assert_eq!(y, Some(px(183.)));
        let y = reveal_offset((px(250.), px(500.)), px(100.), px(100.), px(3.));
        assert_eq!(y, Some(px(247.)));
    }
}
