//! How a text-sized editor measures its own height. A read-only piece is
//! as tall as its wrapped text. Its rows are only known after a layout at
//! a width. So the editor element asks GPUI for a measured layout, and the
//! measure lays the text out here at the width it is given.
//!
//! The measure stores the columns it laid out at. Prepaint uses them, so
//! the drawn wrap always matches the measured height, even when edge
//! snapping moves the final width by one device pixel.

use std::num::NonZeroU64;

use gpui_kit::{AvailableSpace, Context, Pixels, Size, Window, px, size};

use super::EditorView;
use crate::editor_element::LINE_HEIGHT;
use crate::editor_font;
use crate::fonts::FaceStyle;
use crate::grid::CellMetrics;

/// The height a measure adds below the last row, so rounding never cuts
/// the last row off.
pub(crate) const SLACK: f32 = 1.0;

/// What a layout depends on besides the width: the text revision and the
/// caret. The caret decides which link shows written out and whether the
/// front matter is folded. An edit or a caret move changes the stamp.
pub(crate) type EditStamp = (NonZeroU64, (usize, usize));

/// The row height of an editor at `font_size`, in whole pixels.
pub(crate) fn line_height(font_size: Pixels) -> Pixels {
    px((f32::from(font_size) * LINE_HEIGHT).round())
}

/// The width a measure lays out at: a known width, else a definite
/// available width. A min-content or max-content probe gets none.
pub(crate) fn measure_width(
    known: Size<Option<Pixels>>,
    available: Size<AvailableSpace>,
) -> Option<Pixels> {
    known.width.or(match available.width {
        AvailableSpace::Definite(width) => Some(width),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
    })
}

impl EditorView {
    /// Lay the text out at `width` pixels and return its visual rows. It
    /// stores the columns it used, for prepaint.
    pub(crate) fn rows_at(
        &mut self,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let inputs = editor_font::face_inputs(cx);
        let font_size = editor_font::font_size(cx);
        let text_system = window.text_system().clone();
        let fonts = self.editor_fonts(&inputs, &text_system, cx);
        let cells = CellMetrics::from_text_system(
            &text_system,
            &fonts.face(FaceStyle::default()),
            font_size,
            line_height(font_size),
        );
        let cols = cells.col_at(width.into()).max(1);
        #[cfg(test)]
        probe::width(cx.entity_id(), width);
        // A chunk's images take lines: they are laid out before the rows
        // are counted.
        let line_px = f32::from(line_height(font_size));
        if let Some(pane) = self.read_only_pane(cols, cells.width(), line_px, window.scale_factor())
        {
            self.prepare_images(pane, cx);
        }
        // The height only sets the viewport. It does not change the wrap.
        self.prepare_counted(cols, u16::MAX, cx.entity_id());
        self.measured_cols = Some(cols);
        self.measured_stamp = Some(self.edit_stamp());
        self.editor
            .session()
            .display
            .layout
            .visual_line_count()
            .max(1)
    }

    /// The measured size of a text-sized editor: its full width, and its
    /// rows times the line height plus [`SLACK`]. A probe without a
    /// definite width lays nothing out and reports the last rows.
    pub(crate) fn measure(
        &mut self,
        known: Size<Option<Pixels>>,
        available: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Size<Pixels> {
        let line_height = line_height(editor_font::font_size(cx));
        #[cfg(test)]
        probe::asked(cx.entity_id(), format!("{known:?} {available:?}"));
        let Some(width) = measure_width(known, available) else {
            #[cfg(test)]
            probe::skipped(cx.entity_id());
            let rows = self.measured_rows.unwrap_or(1);
            return size(px(0.), line_height * rows as f32 + px(SLACK));
        };
        let rows = match self.measured_rows {
            Some(rows) if self.measured_width == Some(width) => rows,
            _ => self.rows_at(width, window, cx),
        };
        self.measured_rows = Some(rows);
        self.measured_width = Some(width);
        size(width, line_height * rows as f32 + px(SLACK))
    }

    /// The text revision and the caret now.
    pub(crate) fn edit_stamp(&self) -> EditStamp {
        (self.editor.content_revision(), self.editor.cursor())
    }

    /// Whether the measure of this frame prepared the frame that prepaint
    /// would prepare at `cols`. Then prepaint skips its own preparation.
    ///
    /// The skip assumes that nothing edits the editor and nothing moves
    /// its caret between layout and prepaint of the same frame. An edit in
    /// that window would draw the old wrap. A debug build panics on such
    /// a change. In a release build this returns false. Then prepaint
    /// prepares the frame again and draws the current text at the stale
    /// measured height until the next redraw, so the text can be clipped.
    pub(crate) fn measured_this_frame(&self, cols: u16) -> bool {
        if self.measured_width.is_none() || self.measured_cols != Some(cols) {
            return false;
        }
        let unchanged = self.measured_stamp == Some(self.edit_stamp());
        debug_assert!(
            unchanged,
            "a text-sized editor changed between its measure and its prepaint"
        );
        unchanged
    }

    /// Prepare the editor's frame at `cols` by `rows`. Tests count the
    /// calls that laid the text out again.
    pub(crate) fn prepare_counted(&mut self, cols: u16, rows: u16, _id: gpui_kit::EntityId) {
        #[cfg(test)]
        let before = self.layout_identity();
        self.editor.prepare_frame(cols, rows);
        #[cfg(test)]
        if self.layout_identity() != before {
            probe::relayout(_id);
        }
    }

    /// What changes when the layout is built again: the address of its
    /// lines and its width. A new layout is built while the old one lives,
    /// so a rebuild always moves the address.
    #[cfg(test)]
    fn layout_identity(&self) -> (usize, usize) {
        let layout = &self.editor.session().display.layout;
        (layout.visual_lines().as_ptr() as usize, layout.width())
    }
}

/// A test log of the measures. It records, per editor, the distinct widths
/// it was measured at, the probes it skipped and the layouts it built.
#[cfg(test)]
pub(crate) mod probe {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use gpui_kit::{EntityId, Pixels};

    /// What one editor saw since the last [`take`].
    #[derive(Debug, Default, Clone, PartialEq)]
    pub(crate) struct Measures {
        /// The distinct definite widths, in the order seen.
        pub(crate) widths: Vec<Pixels>,
        /// The measure calls with a width.
        pub(crate) calls: usize,
        /// The probes without a definite width.
        pub(crate) skipped: usize,
        /// The frame preparations that built the layout again.
        pub(crate) relayouts: usize,
        /// The known and available sizes of each call, as written.
        pub(crate) asked: Vec<String>,
    }

    thread_local! {
        static LOG: RefCell<HashMap<EntityId, Measures>> = RefCell::default();
    }

    fn with(id: EntityId, f: impl FnOnce(&mut Measures)) {
        LOG.with(|log| f(log.borrow_mut().entry(id).or_default()));
    }

    pub(super) fn width(id: EntityId, width: Pixels) {
        with(id, |m| {
            m.calls += 1;
            if !m.widths.contains(&width) {
                m.widths.push(width);
            }
        });
    }

    pub(super) fn asked(id: EntityId, sizes: String) {
        with(id, |m| m.asked.push(sizes));
    }

    pub(super) fn skipped(id: EntityId) {
        with(id, |m| m.skipped += 1);
    }

    pub(super) fn relayout(id: EntityId) {
        with(id, |m| m.relayouts += 1);
    }

    /// The log so far, and a fresh one from now on.
    pub(crate) fn take() -> HashMap<EntityId, Measures> {
        LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
    }
}
