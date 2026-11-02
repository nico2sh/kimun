//! Cell-grid metrics: the bridge between GPUI's pixel space and the logic
//! layer's cell space (`EditorFrame` columns/rows).
//!
//! The arithmetic is pure over `f32` pixels so it is unit-testable without a
//! window; only [`CellMetrics::from_text_system`] touches GPUI's text system.

use gpui_kit::{Font, Pixels, TextSystem};

/// Size of one monospace cell, in logical pixels. Always strictly positive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    width: f32,
    height: f32,
}

impl CellMetrics {
    /// Smallest cell edge accepted; guards every division below.
    const MIN_EDGE: f32 = 1.0;

    /// Build from explicit pixel sizes. Non-finite or sub-pixel values are
    /// clamped to 1px so conversions can never divide by zero.
    pub fn new(width: f32, height: f32) -> Self {
        let sane = |v: f32| {
            if v.is_finite() {
                v.max(Self::MIN_EDGE)
            } else {
                Self::MIN_EDGE
            }
        };
        Self {
            width: sane(width),
            height: sane(height),
        }
    }

    /// Derive metrics for `font` at `font_size`: the cell width is the advance
    /// of `'m'` (all glyphs share it in a monospace face) and the height is the
    /// caller's `line_height`, which also drives row placement when painting.
    pub fn from_text_system(
        text_system: &TextSystem,
        font: &Font,
        font_size: Pixels,
        line_height: Pixels,
    ) -> Self {
        let font_id = text_system.resolve_font(font);
        let advance = text_system
            .advance(font_id, font_size, 'm')
            .map(|s| f32::from(s.width))
            // A face with no 'm' glyph: fall back to the usual 0.6em.
            .unwrap_or_else(|_| f32::from(font_size) * 0.6);
        Self::new(advance, f32::from(line_height))
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    /// Column containing pixel offset `x` from the widget origin. Negative
    /// offsets read as column 0; huge ones saturate at `u16::MAX`.
    pub fn col_at(&self, x: f32) -> u16 {
        to_cell(x / self.width)
    }

    /// Row containing pixel offset `y` from the widget origin.
    pub fn row_at(&self, y: f32) -> u16 {
        to_cell(y / self.height)
    }

    /// Left edge of column `col`, in pixels from the widget origin.
    pub fn x_of(&self, col: u16) -> f32 {
        f32::from(col) * self.width
    }

    /// Top edge of row `row`, in pixels from the widget origin.
    pub fn y_of(&self, row: u16) -> f32 {
        f32::from(row) * self.height
    }

    /// Whole columns and rows that fit a pane of `width` x `height` pixels
    /// (floor, at least 1 each so the editor never gets a zero-sized frame).
    pub fn fit(&self, width: f32, height: f32) -> (u16, u16) {
        (self.col_at(width).max(1), self.row_at(height).max(1))
    }
}

/// Floor a fractional cell index into `u16`, clamping below at 0. NaN reads as 0.
fn to_cell(v: f32) -> u16 {
    if v.is_nan() || v <= 0.0 {
        0
    } else if v >= f32::from(u16::MAX) {
        u16::MAX
    } else {
        v.floor() as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: fn() -> CellMetrics = || CellMetrics::new(8.0, 16.0);

    #[test]
    fn pixel_to_cell_floors() {
        let m = M();
        assert_eq!(m.col_at(0.0), 0);
        assert_eq!(m.col_at(7.99), 0);
        assert_eq!(m.col_at(8.0), 1);
        assert_eq!(m.col_at(79.0), 9);
        assert_eq!(m.row_at(15.9), 0);
        assert_eq!(m.row_at(16.0), 1);
    }

    #[test]
    fn negative_and_nan_clamp_to_zero_huge_saturates() {
        let m = M();
        assert_eq!(m.col_at(-50.0), 0);
        assert_eq!(m.row_at(f32::NAN), 0);
        assert_eq!(m.col_at(1.0e12), u16::MAX);
    }

    #[test]
    fn cell_to_pixel_is_the_cell_origin() {
        let m = M();
        assert_eq!(m.x_of(3), 24.0);
        assert_eq!(m.y_of(2), 32.0);
        assert_eq!(m.col_at(m.x_of(37)), 37);
        assert_eq!(m.row_at(m.y_of(12)), 12);
    }

    #[test]
    fn fit_floors_and_has_a_minimum_of_one() {
        let m = M();
        assert_eq!(m.fit(805.0, 485.0), (100, 30));
        assert_eq!(m.fit(7.0, 15.0), (1, 1));
        assert_eq!(m.fit(0.0, -4.0), (1, 1));
    }

    #[test]
    fn degenerate_metrics_never_divide_by_zero() {
        let m = CellMetrics::new(0.0, f32::NAN);
        assert_eq!((m.width(), m.height()), (1.0, 1.0));
        assert_eq!(m.fit(10.0, 5.0), (10, 5));
    }

    #[test]
    fn fractional_cell_width_keeps_columns_exact() {
        // 7.2px cells (typical at 12px font): column boundaries are not integers.
        let m = CellMetrics::new(7.2, 14.0);
        assert_eq!(m.col_at(7.1), 0);
        assert_eq!(m.col_at(7.3), 1);
        assert_eq!(m.fit(720.0, 140.0), (100, 10));
    }
}
