//! The column reveal: a column stop that takes the keyboard scrolls into view.
//!
//! The stop's row comes from the height table, so the reveal is right in
//! the frame the stop arrives, before the drawn range is picked. The column
//! moves by the least amount that shows the row with its focus ring whole.
//! Tab into the column enters at piece 0, Shift+Tab at the last piece.

use gpui_kit::{Context, Pixels, Window, point, px};
use kimun_core::note::GAP_SHOW_ALL_MAX;

use super::FacetView;
use super::body::{PieceGaps, count_in_up_row, to_px};
use super::keys::Stop;
use super::rows::column_pad;
use super::window::{Heights, Row};
use crate::workspace::facet_ui::ring_room;

/// The row of the column that holds `stop`, from each piece's gaps. `None`
/// for a stop of the bar or the empty message.
pub(super) fn stop_row(stop: &Stop, gaps: &[PieceGaps]) -> Option<Row> {
    match *stop {
        // A stop of the last frame can name a piece a new plan dropped.
        Stop::Crumb(i, _) | Stop::Open(i) => (i < gaps.len()).then_some(Row::Crumbs(i)),
        // A lead gap's down button sits above the crumbs, its up button
        // under them. `Show all` sits with the count ([`count_in_up_row`]).
        Stop::GapDown(g) | Stop::GapUp(g) | Stop::ShowAll(g) => {
            gaps.iter().enumerate().find_map(|(i, p)| {
                let up = match stop {
                    Stop::GapUp(_) => true,
                    Stop::ShowAll(_) => !p.lead_above,
                    _ => false,
                };
                let lead = if up { Row::Up(i) } else { Row::Lead(i) };
                (p.lead == Some(g))
                    .then_some(lead)
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
pub(crate) fn reveal_offset(
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

/// The scroll offset that brings the next (`down`) or previous piece to
/// the top, from the pieces' tops in the column (`tops`, in order), the
/// column's offset now (`current`, ≥ 0) and how far it scrolls (`max`).
/// `None` when no piece lies that way. Above the first piece (the column's
/// inset) the first piece is the one shown: down goes to the second.
pub(super) fn piece_offset(tops: &[f32], current: f32, max: f32, down: bool) -> Option<f32> {
    let target = if down {
        let current = current.max(tops.first().copied().unwrap_or(0.0));
        tops.iter().copied().find(|&y| y > current + 0.5)
    } else {
        tops.iter().copied().rev().find(|&y| y < current - 0.5)
    }?;
    Some(target.clamp(0.0, max.max(0.0)))
}

impl FacetView {
    /// When a column stop took the keyboard since the last frame (by Tab
    /// most often), scroll the column so its row shows with its ring.
    /// Called before the drawn range is picked, so the range follows.
    /// A stop that arrives while the column has no height yet waits for
    /// the next frame, as long as it keeps the keyboard.
    pub(super) fn reveal_column(&self, gaps: &[PieceGaps], heights: &Heights, rem: Pixels) {
        let stop = {
            let stops = self.stops.borrow();
            let held = self.held_reveal.take().filter(|stop| stops.holds(stop));
            stops.arrived().cloned().or(held)
        };
        let Some(stop) = stop else {
            return;
        };
        if let Some(row) = stop_row(&stop, gaps)
            && !self.reveal_row(row, heights, rem)
        {
            self.held_reveal.replace(Some(stop));
        }
    }

    /// Scroll the column by the least amount that shows `row` and its ring.
    /// False when the column has no height to show it in.
    fn reveal_row(&self, row: Row, heights: &Heights, rem: Pixels) -> bool {
        let viewport = self.scroll.bounds().size.height;
        if viewport <= px(0.) {
            return false;
        }
        // The table starts under the column's `py_3`.
        let pad = to_px(column_pad(), rem);
        let (top, bottom) = heights.row_span(row);
        let now = self.scroll.offset();
        let span = (pad + top, pad + bottom);
        if let Some(y) = reveal_offset(span, -now.y, viewport, ring_room()) {
            self.scroll.set_offset(point(now.x, -y));
        }
        true
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

    /// Piece `i`'s stops in the order they are painted: the row of the gap
    /// above it, its crumbs, `Open note`, the gap's up row under them, and
    /// the row of the gap after it.
    fn piece_stops(&self, i: usize, gaps: PieceGaps) -> Vec<Stop> {
        let mut out = self.gap_stops(gaps.lead, false);
        let crumbs = self.members.chunk(i).map_or(0, |c| c.crumbs.len());
        out.extend((0..crumbs).map(|k| Stop::Crumb(i, k)));
        out.push(Stop::Open(i));
        out.extend(self.gap_stops(gaps.lead, true));
        out.extend(self.gap_stops(gaps.tail, false));
        out
    }

    /// The stops of gap `g`'s up row (`up`), as `render_up_row` paints
    /// them, or of its other row, as `render_gap` paints it.
    fn gap_stops(&self, g: Option<usize>, up: bool) -> Vec<Stop> {
        let Some((g, gap)) = g.and_then(|g| Some((g, *self.members.gaps.get(g)?))) else {
            return Vec::new();
        };
        let hidden = self.members.gap_hidden(g);
        let drawn = if up { gap.below } else { gap.above };
        if hidden == 0 || drawn.is_none() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(2);
        out.push(if up { Stop::GapUp(g) } else { Stop::GapDown(g) });
        if hidden <= GAP_SHOW_ALL_MAX && count_in_up_row(gap.above) == up {
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

#[cfg(test)]
mod piece_offset_tests {
    use super::piece_offset;

    const TOPS: [f32; 3] = [12.0, 200.0, 400.0];

    #[test]
    fn down_goes_to_the_first_piece_below_the_top() {
        assert_eq!(piece_offset(&TOPS, 0.0, 1000.0, true), Some(200.0));
        assert_eq!(piece_offset(&TOPS, 12.0, 1000.0, true), Some(200.0));
        assert_eq!(piece_offset(&TOPS, 400.0, 1000.0, true), None);
    }

    #[test]
    fn up_goes_to_the_last_piece_above_the_top() {
        assert_eq!(piece_offset(&TOPS, 200.0, 1000.0, false), Some(12.0));
        assert_eq!(piece_offset(&TOPS, 150.0, 1000.0, false), Some(12.0));
        assert_eq!(piece_offset(&TOPS, 12.0, 1000.0, false), None);
    }

    #[test]
    fn the_offset_stops_where_the_column_ends() {
        assert_eq!(piece_offset(&TOPS, 0.0, 150.0, true), Some(150.0));
        assert_eq!(piece_offset(&TOPS, 200.0, 300.0, true), Some(300.0));
    }
}
