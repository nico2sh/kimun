//! The column of pieces: which pieces are drawn and the spacers between.
//!
//! The pieces near the viewport are drawn, and so are the piece with the
//! keyboard and the piece in a drag. Spacers stand in for the rest, as
//! tall as the height table says ([`mod@super::window`]). After layout a
//! check compares each drawn piece with the table.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement, Pixels, Rems, ScrollHandle, Window, px,
};
use kimun_notes::settings::themes::Theme;

use super::FacetView;
use super::reveal::stop_row;
use super::rows;
use super::window::{Fit, Heights, Metrics, Row, Shape, Spacer, drawn_range, pinned};
use crate::editor_font;
use crate::editor_view::{EditorView, SLACK, line_height};
use crate::workspace::facet_ui::ring_room;

/// The gaps that draw with piece `i`: the one above it and, for the last
/// piece of a note, the one after it.
#[derive(Clone, Copy, Default)]
pub(super) struct PieceGaps {
    pub(super) lead: Option<usize>,
    /// The lead gap has a block above it. Then its down button, its count
    /// and `Show all` take a row above the crumbs. Else the count and
    /// `Show all` join the up row ([`count_in_up_row`]).
    pub(super) lead_above: bool,
    pub(super) tail: Option<usize>,
}

/// Whether a gap's count and `Show all` sit in its up row, from whether the
/// gap has a block `above` it. A gap with a block above draws a row above
/// the crumbs (or after the piece, for a tail gap), and they sit there.
/// Else that row is not drawn, and they join the up row.
pub(super) fn count_in_up_row(above: Option<usize>) -> bool {
    above.is_none()
}

/// A drawn piece: its number, its editor, and its first and last item in
/// the column.
type DrawnPiece = (usize, Entity<EditorView>, usize, usize);

impl FacetView {
    /// The column's items: a spacer, the drawn pieces in order with the
    /// pinned ones at their place between split spacers, a spacer, then the
    /// empty message while no chunk is drawn. Each piece draws its
    /// dateline, its gap row, its crumbs, its up row and its text.
    pub(super) fn render_body(
        &mut self,
        title_size: Pixels,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, Check) {
        #[cfg(test)]
        self.checked.set(0);
        let n = self.members.len();
        self.fit_images(window, cx);
        let gaps = self.piece_gaps(n);
        let fit = self.fit(title_size, window, cx);
        let shapes = gaps.iter().map(|g| self.shape(g)).collect();
        let range = {
            let mut heights = self.heights.borrow_mut();
            heights.set_metrics(fit.font, fit.rem, fit.face, fit.metrics);
            heights.set_shapes(shapes);
            self.reveal_column(&gaps, &heights, fit.rem);
            self.near(fit.rem, window, &heights)
        };
        let near = range.clone();
        let pins = self.pins(&gaps, window, cx);
        // A stop of the last frame can name a piece the plan dropped.
        let pins = pins.into_iter().filter(|&i| i < n);
        let mut order: Vec<usize> = range.clone().chain(pins).collect();
        order.sort_unstable();
        order.dedup();

        let mut body: Vec<AnyElement> = Vec::new();
        let mut drawn = Vec::with_capacity(order.len());
        let mut next = 0;
        for i in order {
            if i > next {
                body.push(self.spacer(next..i, fit));
            }
            let first = body.len();
            self.render_piece(i, gaps[i], title_size, t, cx, &mut body);
            drawn.push((i, self.chunks[i].clone(), first, body.len() - 1));
            next = i + 1;
        }
        if next < n {
            body.push(self.spacer(next..n, fit));
        }
        #[cfg(test)]
        {
            self.drawn = range;
        }
        if n == 0 {
            body.push(self.render_empty(t, cx));
        }
        let check = Check {
            drawn,
            near: (!self.draws_all()).then_some((near, fit.rem)),
            heights: self.heights.clone(),
            scroll: self.scroll.clone(),
            #[cfg(not(test))]
            warned: self.warned.clone(),
            #[cfg(test)]
            checked: self.checked.clone(),
        };
        (body, check)
    }

    /// Piece `i`'s items: its note's dateline when it is the note's first
    /// piece, the row of the gap above it (above the crumbs), the piece
    /// (with the gap's up row under its crumbs), and the gap after a note's
    /// last piece.
    fn render_piece(
        &self,
        i: usize,
        gaps: PieceGaps,
        title_size: Pixels,
        t: &Theme,
        cx: &mut Context<Self>,
        body: &mut Vec<AnyElement>,
    ) {
        if let Some(g) = gaps.lead {
            let gap = self.members.gaps[g];
            body.extend(self.render_dateline(gap, i > 0, title_size, t));
            body.extend(
                self.render_gap(g, i > 0, t, cx)
                    .map(IntoElement::into_any_element),
            );
        }
        body.push(self.render_chunk(i, gaps.lead, t, cx));
        if let Some(g) = gaps.tail {
            body.extend(
                self.render_gap(g, true, t, cx)
                    .map(IntoElement::into_any_element),
            );
        }
    }

    /// A spacer for the pieces `range`.
    fn spacer(&self, range: Range<usize>, fit: Fit) -> AnyElement {
        Spacer {
            pieces: self.chunks[range.clone()].to_vec(),
            range,
            heights: self.heights.clone(),
            fit,
        }
        .into_any_element()
    }

    /// The gaps of each of the `n` pieces, from the plan.
    pub(super) fn piece_gaps(&self, n: usize) -> Vec<PieceGaps> {
        let mut out = vec![PieceGaps::default(); n];
        for (g, gap) in self.members.gaps.iter().enumerate() {
            match (gap.above, gap.below) {
                (above, Some(below)) if below < n => {
                    out[below].lead = Some(g);
                    out[below].lead_above = above.is_some();
                }
                (Some(above), None) if above < n => out[above].tail = Some(g),
                _ => {}
            }
        }
        out
    }

    /// The rows piece `gaps` draws besides its crumbs and its text.
    fn shape(&self, gaps: &PieceGaps) -> Shape {
        let lead = gaps
            .lead
            .and_then(|g| self.members.gaps.get(g).map(|gap| (g, gap)));
        let dateline = self.collection.is_some()
            && lead.is_some_and(|(_, gap)| {
                gap.above.is_none() && self.members.list.get(gap.member).is_some()
            });
        let lead_hidden = lead.is_some_and(|(g, _)| self.members.gap_hidden(g) > 0);
        Shape {
            dateline,
            lead_row: lead_hidden && gaps.lead_above,
            up_row: lead_hidden,
            tail_hidden: gaps.tail.is_some_and(|g| self.members.gap_hidden(g) > 0),
        }
    }

    /// The fixed heights of the rows around a piece, from the font, the rem
    /// and the styles the rows are drawn with.
    fn fit(&self, title_size: Pixels, window: &Window, cx: &App) -> Fit {
        let rem = window.rem_size();
        let scale = window.scale_factor();
        // A line of text at `size`, snapped to device pixels as layout does.
        let text_line = |size: Pixels| {
            let line = window.text_style().line_height.to_pixels(size.into(), rem);
            px((f32::from(line) * scale).round() / scale)
        };
        // The same numbers the rows are drawn with ([`rows`]).
        let column_gap = to_px(rows::column_gap(), rem);
        let button = to_px(rows::button(), rem);
        let small = text_line(to_px(rows::small_text(), rem));
        let row = button.max(small);
        let font = editor_font::font_size(cx);
        #[cfg(test)]
        let forged = self.forged;
        #[cfg(not(test))]
        let forged = px(0.);
        let metrics = Metrics {
            line: line_height(font),
            dateline: text_line(title_size).max(small) + column_gap,
            dateline_margin: to_px(rows::dateline_margin(), rem),
            gap_row: row + column_gap,
            gap_pull: to_px(rows::gap_pull(), rem),
            crumbs: button + ring_room() * 2. + forged,
            // Drawn as the crumbs row is: the room for a ring at both edges.
            up_row: row + ring_room() * 2.,
            // The editor pane's padding and the measure's slack.
            inset: to_px(rows::pane_y(), rem) + px(SLACK),
            column_gap,
        };
        Fit {
            font,
            rem,
            face: face_key(cx),
            metrics,
            pane: to_px(rows::pane_x(), rem),
        }
    }

    /// Hand the chunks the column's height when it changed, before they lay
    /// out: an image takes at most 60% of it.
    fn fit_images(&mut self, window: &Window, cx: &mut Context<Self>) {
        let height = f32::from(column_height(&self.scroll, window)) * window.scale_factor();
        if height != self.image_viewport_px {
            self.image_viewport_px = height;
            self.sync_image_sources(cx);
        }
    }

    /// The pieces near the viewport, from the column's scroll offset; every
    /// piece in tests that draw them all.
    fn near(&self, rem: Pixels, window: &Window, heights: &Heights) -> Range<usize> {
        if self.draws_all() {
            return 0..self.members.len();
        }
        near_range(&self.scroll, rem, window, heights)
    }

    #[cfg(test)]
    fn draws_all(&self) -> bool {
        self.draw_all
    }

    #[cfg(not(test))]
    fn draws_all(&self) -> bool {
        false
    }

    /// The pieces drawn wherever they are: the one with the keyboard (a
    /// stop in it, or its editor) and the one in a drag.
    fn pins(&self, gaps: &[PieceGaps], window: &Window, cx: &App) -> Vec<usize> {
        let stop = self.stops.borrow().focused(window);
        let of_stop = stop.and_then(|stop| match stop_row(&stop, gaps)? {
            Row::Lead(i) | Row::Crumbs(i) | Row::Up(i) | Row::Tail(i) => Some(i),
            Row::End => None,
        });
        let of_editor = || self.focused_piece(window, cx);
        let dragging = self.chunks.iter().position(|c| c.read(cx).dragging());
        pinned(of_stop.or_else(of_editor), dragging)
    }
}

/// A row value of [`rows`] in pixels at the rem size `rem`: the one place
/// the table side turns one into pixels.
pub(super) fn to_px(value: Rems, rem: Pixels) -> Pixels {
    value.to_pixels(rem)
}

/// The pieces near the viewport of the column that `scroll` tracks.
fn near_range(
    scroll: &ScrollHandle,
    rem: Pixels,
    window: &Window,
    heights: &Heights,
) -> Range<usize> {
    let viewport = column_height(scroll, window);
    // The offset is negative down the column; the table starts under
    // the column's padding.
    let offset = -scroll.offset().y - to_px(rows::column_pad(), rem);
    drawn_range(offset, viewport, heights)
}

/// The height of the column that `scroll` tracks. Before the first layout
/// the column has no height: the window's.
fn column_height(scroll: &ScrollHandle, window: &Window) -> Pixels {
    let height = scroll.bounds().size.height;
    if height > px(0.) {
        height
    } else {
        window.viewport_size().height
    }
}

/// A hash of what the editor face is chosen from. The family changes the
/// cell width, so the wrap.
fn face_key(cx: &App) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let inputs = editor_font::face_inputs(cx);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    inputs.kit_family.hash(&mut hasher);
    inputs.preferred.hash(&mut hasher);
    hasher.finish()
}

/// What the column checks after layout: the rows of each drawn piece go
/// into the table, and each drawn piece must sit where the table says.
pub(super) struct Check {
    drawn: Vec<DrawnPiece>,
    /// The near range this frame drew and the rem it used; `None` when
    /// every piece is drawn (tests).
    near: Option<(Range<usize>, Pixels)>,
    heights: Rc<RefCell<Heights>>,
    scroll: ScrollHandle,
    /// A debug run of the app warned of the mismatch that goes on now.
    #[cfg(not(test))]
    warned: Rc<Cell<bool>>,
    #[cfg(test)]
    checked: Rc<Cell<usize>>,
}

/// A drawn piece the verify step compares: its number and its first and
/// last item in the column.
type Known = (usize, usize, usize);

impl Check {
    /// Run after the column's children are laid out: record, then verify
    /// in a debug build.
    pub(super) fn run(&self, window: &Window, cx: &App) {
        let known = self.record(window, cx);
        if cfg!(debug_assertions) {
            self.verify(&known);
        }
    }

    /// The step a release build runs too. Each drawn piece's rows go into
    /// the table. When a jump in width, font or text moved the pieces and
    /// the range at the offset now is not the drawn one, it asks for one
    /// more frame, or the viewport can show only a spacer. It returns the
    /// pieces to verify, none in a release build.
    fn record(&self, window: &Window, cx: &App) -> Vec<Known> {
        let verify = cfg!(debug_assertions);
        let mut known = Vec::with_capacity(if verify { self.drawn.len() } else { 0 });
        let mut heights = self.heights.borrow_mut();
        for (i, piece, first, last) in &self.drawn {
            let editor = piece.read(cx);
            let stamp = editor.edit_stamp();
            // The editor element clears `measured_width` each frame before
            // its measure, so `Some` means measured in this frame.
            if let (Some(rows), Some(width), Some(at)) = (
                editor.measured_rows,
                editor.measured_width,
                editor.measured_stamp,
            ) && at == stamp
                // No rows are kept for an editor with no width: one column
                // gives far too many.
                && width > px(0.)
            {
                heights.set_width(width);
                heights.set_rows(*i, rows, piece.entity_id().as_u64(), stamp);
                if verify {
                    known.push((*i, *first, *last));
                }
            }
        }
        if heights.take_changed()
            && let Some((near, rem)) = &self.near
            && near_range(&self.scroll, *rem, window, &heights) != *near
        {
            window.request_animation_frame();
        }
        known
    }

    /// The debug step: each piece in `known` must sit where the table
    /// says, within a pixel. Tests panic on a mismatch. A debug run of the
    /// app warns once per mismatch, until the table matches again.
    fn verify(&self, known: &[Known]) {
        // No item laid out: nothing to compare.
        let Some(origin) = self.scroll.bounds_for_item(0).map(|b| b.top()) else {
            return;
        };
        let heights = self.heights.borrow();
        #[cfg(test)]
        let mut checked = 0;
        let mut wrong = None;
        for &(i, first, last) in known {
            let (Some(a), Some(b)) = (
                self.scroll.bounds_for_item(first),
                self.scroll.bounds_for_item(last),
            ) else {
                continue;
            };
            let (top, bottom) = (a.top() - origin, b.bottom() - origin);
            let want = heights.top_of(i);
            let want_bottom = want + heights.height_of(i);
            let fits = (top - want).abs() <= px(1.) && (bottom - want_bottom).abs() <= px(1.);
            if !fits && wrong.is_none() {
                wrong = Some(format!(
                    "the height table is wrong for piece {i}: drawn {top:?}..{bottom:?}, \
                     the table says {want:?}..{want_bottom:?}"
                ));
            }
            #[cfg(test)]
            {
                checked += 1;
            }
        }
        #[cfg(test)]
        if let Some(wrong) = wrong {
            panic!("{wrong}");
        }
        #[cfg(not(test))]
        match wrong {
            Some(wrong) if !self.warned.replace(true) => tracing::warn!("{wrong}"),
            Some(_) => {}
            None => self.warned.set(false),
        }
        #[cfg(test)]
        self.checked.set(checked);
    }
}
