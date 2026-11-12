//! The **Edit surface**'s painter: a custom GPUI [`Element`] that fills its
//! pane, sizes the editor's cell grid to it, and paints the **Editor frame**
//! cell-exactly in a monospace face (ADR-0049 §5).
//!
//! Under vim the pane's last cell row is the **mode line** (mode label on the
//! leading edge, the pending command on the trailing edge) and the text gets
//! the rows above it; the caret is a bar where typing inserts and an inverted
//! block elsewhere ([`CaretShape`]). An open **find bar** docks in the rows
//! just above the mode line ([`crate::find_bar`]), under a hairline, and
//! takes the caret while it is open.
//!
//! What a row paints is decided in [`RowPaint`] (glyph pieces, fills, the
//! strokes that replace the rule and quote-bar glyphs, follow-target cells);
//! this file shapes it, caches the result per row content
//! ([`crate::row_cache`]), and draws it. The face comes from
//! [`EditorFonts`], which makes bold real (or synthetic) on every machine.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Bounds, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, ScrollEvent, ShapedLine,
    SharedString, StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window,
    WindowTextSystem, fill, point, px, relative, size,
};
use kimun_notes::settings::themes::Theme;

use crate::autocomplete::{PopupStyle, place, popup_cells, popup_element, popup_size};
use crate::editor_font;
use crate::editor_view::{CaretShape, EditorView, Painted};
use crate::find_bar;
use crate::fonts::EditorFonts;
use crate::grid::CellMetrics;
use crate::row_cache::RowCache;
use crate::row_paint::{RowPaint, cells_of, cluster_at};
use crate::theme::{self, CellStyle};

/// Row height as a multiple of the font size; the result is rounded to whole
/// pixels so rows tile without seams.
pub(crate) const LINE_HEIGHT: f32 = 1.5;

/// Physical strokes, not layout, so fixed pixel sizes (the design guides'
/// hairline exception): the caret bar, the blockquote bar, the horizontal
/// rule (and the find bar's divider) and text decorations.
const CARET_WIDTH: f32 = 2.0;
const QUOTE_BAR_WIDTH: f32 = 2.0;
const RULE_THICKNESS: f32 = 1.0;
const DECORATION_THICKNESS: f32 = 1.0;

/// Synthetic bold's second copy is offset by this fraction of the font size
/// (0.78px at 13px), so the added weight follows the text size.
const FAUX_BOLD_EM: f32 = 0.06;

/// How far synthetic bold's second copy sits right of the first.
pub fn faux_bold_offset(font_size: Pixels) -> Pixels {
    font_size * FAUX_BOLD_EM
}

/// The x, from the row's left edge, where a shaped cluster of `width` sits in
/// its `cells` cells from `col`: one-cell clusters are forced to the cell
/// advance and start at their cell; a wide cluster (shaped alone, in a
/// fallback face rarely exactly two cells wide) is centred in its cells.
pub fn glyph_x(cells: &CellMetrics, col: u16, n: u16, width: f32) -> f32 {
    let x = cells.x_of(col);
    if n > 1 {
        x + (cells.width() * f32::from(n) - width).max(0.0) / 2.0
    } else {
        x
    }
}

/// Where the mode line's two texts start in a row of `cols` cells: the label
/// one cell in from the leading edge, the hint ending one cell before the
/// trailing edge. The hint is dropped (`None`) when it would not keep a
/// two-cell gap from the label.
pub fn mode_line_cols(cols: u16, label_cells: u16, hint_cells: Option<u16>) -> (u16, Option<u16>) {
    const PAD: u16 = 1;
    const GAP: u16 = 2;
    let label_end = PAD.saturating_add(label_cells);
    let hint = hint_cells.and_then(|n| {
        let start = cols.checked_sub(PAD)?.checked_sub(n)?;
        (start >= label_end.saturating_add(GAP)).then_some(start)
    });
    (PAD, hint)
}

/// One glyph piece, shaped: its x from the row's left edge.
struct ShapedPiece {
    x: f32,
    line: ShapedLine,
    /// Bold in a family without a bold face: painted twice.
    faux_bold: bool,
}

/// A row as painted: its description and its shaped glyphs. Cached per row
/// content across frames.
pub struct ShapedRow {
    paint: Rc<RowPaint>,
    pieces: Vec<ShapedPiece>,
}

/// Everything a cached row depends on besides its runs.
#[derive(Debug, Clone, PartialEq)]
pub struct RowKey {
    fonts: EditorFonts,
    font_size: Pixels,
    cell_width: u32,
    theme: std::sync::Arc<Theme>,
}

/// The view's cache of shaped rows.
pub type ShapedRows = RowCache<RowKey, ShapedRow>;

/// The ink and the decorations (the TUI's UNDERLINED / CROSSED_OUT) a style
/// paints with.
fn decoration(
    style: &CellStyle,
) -> (
    gpui_kit::Hsla,
    Option<UnderlineStyle>,
    Option<StrikethroughStyle>,
) {
    let color = style.fg.into();
    let underline = style.underline.then_some(UnderlineStyle {
        thickness: px(DECORATION_THICKNESS),
        color: Some(color),
        wavy: false,
    });
    let strike = style.strike.then_some(StrikethroughStyle {
        thickness: px(DECORATION_THICKNESS),
        color: Some(color),
    });
    (color, underline, strike)
}

/// Shape one row's pieces. Wide clusters (shaped alone) are centred in their
/// cells: a fallback face's CJK or emoji glyph is rarely exactly two cells.
fn shape_row(
    paint: RowPaint,
    fonts: &EditorFonts,
    text_system: &WindowTextSystem,
    font_size: Pixels,
    cells: &CellMetrics,
) -> ShapedRow {
    let pieces = paint
        .pieces
        .iter()
        .map(|p| {
            let (color, underline, strikethrough) = decoration(&p.style);
            let run = TextRun {
                len: p.text.len(),
                font: fonts.face(p.style.bold, p.style.italic),
                color,
                background_color: None,
                underline,
                strikethrough,
            };
            let force = p.grid.then(|| px(cells.width()));
            let line = text_system.shape_line(
                SharedString::from(p.text.clone()),
                font_size,
                &[run],
                force,
            );
            let x = if p.grid {
                cells.x_of(p.col)
            } else {
                glyph_x(cells, p.col, p.cells, f32::from(line.width()))
            };
            ShapedPiece {
                x,
                line,
                faux_bold: fonts.synthesizes(p.style.bold),
            }
        })
        .collect();
    ShapedRow {
        paint: Rc::new(paint),
        pieces,
    }
}

/// Snap a rectangle's edges to device pixels, so abutting fills and strokes
/// meet without an anti-aliased seam.
fn snap(b: Bounds<Pixels>, scale: f32) -> Bounds<Pixels> {
    let s = |v: Pixels| px((f32::from(v) * scale).round() / scale);
    let (x0, y0) = (s(b.origin.x), s(b.origin.y));
    let (x1, y1) = (s(b.origin.x + b.size.width), s(b.origin.y + b.size.height));
    Bounds::new(point(x0, y0), size(x1 - x0, (y1 - y0).max(px(1.0 / scale))))
}

/// Paints one [`EditorView`]. Built fresh each frame by the view's `render`.
pub struct EditorElement {
    view: Entity<EditorView>,
}

impl EditorElement {
    pub fn new(view: Entity<EditorView>) -> Self {
        Self { view }
    }

    /// The popup element, when one is open: built and laid out with the pane,
    /// since a layer handed to the window in prepaint must already have asked
    /// for its layout. Its size is whole cells of the editor's grid, known
    /// without the caret, which only prepaint finds.
    fn build_popup(&self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.view.read(cx).autocomplete_state()?;
        let inputs = editor_font::face_inputs(cx);
        let font_size = editor_font::font_size(cx);
        let line_height = px((f32::from(font_size) * LINE_HEIGHT).round());
        let text_system = window.text_system().clone();
        let (cells, font_family) = self.view.update(cx, |view, _| {
            let fonts = view.editor_fonts(&inputs, &text_system);
            let cells = CellMetrics::from_text_system(
                &text_system,
                &fonts.face(false, false),
                font_size,
                line_height,
            );
            // The popup's text paints in the face its cells are measured
            // with: the family the face rule resolved, whatever it chose.
            (cells, fonts.family)
        });
        let theme = theme::current(cx);
        let style = PopupStyle {
            cells: &cells,
            font_family,
            font_size,
            theme: &theme,
        };
        // Borrowed, not cloned: the state can hold hundreds of suggestions and
        // this runs every frame the popup is open.
        let state = self.view.read(cx).autocomplete_state()?;
        Some(popup_element(&self.view, state, &style, window))
    }

    /// Hand the popup to the window as a layer above everything else, placed
    /// against the caret cell and the window, and record where it went.
    fn place_popup(
        &self,
        element: AnyElement,
        request: &PopupRequest,
        cells: &CellMetrics,
        pane: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (cols, rows) = request.cells;
        let popup = popup_size(cells, cols, rows);
        let at = place(
            request.caret,
            popup,
            Bounds::new(Point::default(), window.viewport_size()),
        );
        // The popup's layout sits at the pane's origin, and the window adds
        // that to the offset it is given: hand it the difference, so it lands
        // exactly on `at`.
        let offset = point(at.x - pane.x, at.y - pane.y);
        window.defer_draw(element, offset, 1, None);
        self.view.update(cx, |view, _| {
            if let Some(painted) = view.painted.as_mut() {
                painted.popup = Some(Bounds::new(at, popup));
            }
        });
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A shaped line to paint at a window position, twice when bold is
/// synthetic.
struct PlacedLine {
    at: Point<Pixels>,
    line: ShapedLine,
    faux_bold: bool,
}

/// Everything `paint` draws, resolved in `prepaint`.
pub struct EditorPrepaint {
    hitbox: Hitbox,
    /// The text rows only (the pane minus the mode line): where the pointer
    /// is an I-beam. The mode line keeps the default arrow.
    text_hitbox: Option<Hitbox>,
    /// Cells drawing a follow target: a pointing hand there.
    target_hitboxes: Vec<Hitbox>,
    text_area: Bounds<Pixels>,
    target_areas: Vec<Bounds<Pixels>>,
    cells: CellMetrics,
    line_height: Pixels,
    /// Synthetic bold's offset at this font size.
    faux_offset: Pixels,
    background: Rgba,
    /// Cell-aligned backgrounds and strokes (rules, quote bars), snapped.
    fills: Vec<(Bounds<Pixels>, Rgba)>,
    strokes: Vec<(Bounds<Pixels>, Rgba)>,
    lines: Vec<PlacedLine>,
    /// The IME preedit: the cells it covers, their background (the caret
    /// cell's, so it reads as part of the line it is typed into), and its
    /// underline-styled text.
    preedit: Option<(Bounds<Pixels>, Rgba, ShapedLine)>,
    caret: Option<(Bounds<Pixels>, Rgba)>,
    /// A block caret's glyph, repainted over the block in the cell's
    /// background colour so it stays readable.
    caret_glyph: Option<PlacedLine>,
    /// The mode line's texts (label, then the hint), when shown.
    mode_line: Vec<PlacedLine>,
}

/// An open autocomplete popup waiting to be handed to the window: its size in
/// cells and the caret cell it hangs from.
struct PopupRequest {
    cells: (u16, u16),
    caret: Bounds<Pixels>,
}

impl Element for EditorElement {
    /// The autocomplete popup, built and laid out with the pane.
    type RequestLayoutState = Option<AnyElement>;
    type PrepaintState = EditorPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Fill the pane; the grid is whatever fits.
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        // The popup's layout hangs from the pane's so the window measures it
        // (its text) with everything else; where it is drawn is decided in
        // prepaint, by the caret.
        let popup = self
            .build_popup(window, cx)
            .map(|mut element| (element.request_layout(window, cx), element));
        let layout_id = window.request_layout(style, popup.iter().map(|(id, _)| *id), cx);
        (layout_id, popup.map(|(_, element)| element))
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // The user's font setting, else the kit theme's monospace face and
        // size ([`editor_font::face_inputs`]); every colour comes from the
        // kimün theme (ADR-0049 §6).
        let inputs = editor_font::face_inputs(cx);
        let font_size = editor_font::font_size(cx);
        let line_height = px((f32::from(font_size) * LINE_HEIGHT).round());
        let scale = window.scale_factor();
        let text_system = window.text_system().clone();
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let focused = self.view.read(cx).focus_handle.is_focused(window);
        let origin = bounds.origin;

        let (mut prepaint, popup) = self.view.update(cx, |view, view_cx| {
            let fonts = view.editor_fonts(&inputs, &text_system);
            let base_font = fonts.face(false, false);
            let cells =
                CellMetrics::from_text_system(&text_system, &base_font, font_size, line_height);
            let (cols, pane_rows) = cells.fit(bounds.size.width.into(), bounds.size.height.into());
            let cell_bounds = |col: u16, row: u16, n: u16| {
                Bounds::new(
                    point(
                        origin.x + px(cells.x_of(col)),
                        origin.y + px(cells.y_of(row)),
                    ),
                    size(px(cells.width() * f32::from(n)), line_height),
                )
            };

            let shape = view.caret_shape();
            // The mode line takes the last row, when there is a row to spare.
            let mode_line = view.mode_line().filter(|_| pane_rows > 1);
            let chrome = u16::from(mode_line.is_some());
            // The find bar docks above it. Clamped rather than dropped (the
            // TUI's rule): an open bar takes every key, so it must show even
            // when the text is left no rows.
            let bar_rows = view
                .editor
                .find_bar()
                .map_or(0, find_bar::rows)
                .min(pane_rows - chrome);
            let rows = pane_rows - chrome - bar_rows;
            view.editor.prepare_frame(cols, rows);
            // The bar has the keyboard while open: no caret in the text.
            let bar_open = view.editor.find_open();
            let frame = view
                .editor
                .session()
                .display
                .frame(cols, focused && !bar_open);
            let theme_in_effect = theme::current(view_cx);
            let theme = &*theme_in_effect;

            view.row_cache.begin_frame(RowKey {
                fonts: fonts.clone(),
                font_size,
                cell_width: cells.width().to_bits(),
                theme: theme_in_effect.clone(),
            });
            let mut fills = Vec::new();
            let mut strokes = Vec::new();
            let mut lines = Vec::new();
            let mut row_paint = Vec::new();
            let mut target_areas = Vec::new();
            // Background under the caret cell, for the preedit's backing and
            // the block caret's glyph.
            let mut caret_bg = None;
            // The cluster under a block caret, and its style.
            let mut under_caret = None;
            for (r, row) in frame.rows.iter().enumerate() {
                let Ok(r) = u16::try_from(r) else { break };
                let shaped = view.row_cache.get_or_build(&row.runs, || {
                    shape_row(
                        RowPaint::new(&row.runs, theme),
                        &fonts,
                        &text_system,
                        font_size,
                        &cells,
                    )
                });
                let paint = &shaped.paint;
                if let Some((cc, cr)) = frame.cursor
                    && cr == r
                {
                    caret_bg = paint.fill_at(cc);
                    if shape == CaretShape::Block {
                        under_caret = cluster_at(&row.runs, cc).map(|(col, n, text, run)| {
                            (col, n, text, theme::run_style(&row.runs[run], theme))
                        });
                    }
                }
                for f in &paint.fills {
                    fills.push((snap(cell_bounds(f.col, r, f.cells), scale), f.color));
                }
                let row_top = origin.y + px(cells.y_of(r));
                for rule in &paint.rules {
                    let at = cell_bounds(rule.col, r, rule.cells);
                    let mid = row_top + line_height / 2.0 - px(RULE_THICKNESS / 2.0);
                    let b = Bounds::new(
                        point(at.origin.x, mid),
                        size(at.size.width, px(RULE_THICKNESS)),
                    );
                    strokes.push((snap(b, scale), rule.color));
                }
                for bar in &paint.bars {
                    let x = origin.x
                        + px(cells.x_of(bar.col) + cells.width() / 2.0 - QUOTE_BAR_WIDTH / 2.0);
                    let b = Bounds::new(point(x, row_top), size(px(QUOTE_BAR_WIDTH), line_height));
                    strokes.push((snap(b, scale), bar.color));
                }
                for &(col, n) in &paint.targets {
                    target_areas.push(cell_bounds(col, r, n));
                }
                for piece in &shaped.pieces {
                    lines.push(PlacedLine {
                        at: point(origin.x + px(piece.x), row_top),
                        line: piece.line.clone(),
                        faux_bold: piece.faux_bold,
                    });
                }
                row_paint.push(shaped.paint.clone());
            }
            view.row_cache.end_frame();

            let caret_at = frame.cursor.filter(|&(_, r)| r < rows);
            let caret_cell = caret_at.map(|(c, r)| cell_bounds(c, r, 1));
            let preedit = view.preedit.as_deref().zip(caret_cell).map(|(text, at)| {
                let n = cells_of(text);
                let fg = theme::foreground(theme).into();
                let run = TextRun {
                    len: text.len(),
                    font: base_font.clone(),
                    color: fg,
                    background_color: None,
                    underline: Some(UnderlineStyle {
                        thickness: px(DECORATION_THICKNESS),
                        color: Some(fg),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                let shaped = text_system.shape_line(
                    SharedString::from(text.to_string()),
                    font_size,
                    &[run],
                    None,
                );
                let mut covered = at;
                covered.size.width = px(cells.width() * f32::from(n));
                let backing = caret_bg.unwrap_or_else(|| theme::background(theme));
                (covered, backing, shaped)
            });
            let caret_color = theme::caret(theme);
            let mut caret_glyph = None;
            let caret = caret_at
                .zip(caret_cell)
                .map(|((_, caret_row), mut at)| match shape {
                    CaretShape::Bar => {
                        // The caret sits after the preedit while composing.
                        if let Some((covered, _, _)) = &preedit {
                            at.origin.x += covered.size.width;
                        }
                        at.size.width = px(CARET_WIDTH);
                        (at, caret_color)
                    }
                    CaretShape::Block => {
                        // Inverted: the block in the caret colour, the glyph in the
                        // colour behind it. Past the row's end the block is one
                        // empty cell; on a wide char it covers both cells.
                        if let Some((col, n, text, style)) = &under_caret {
                            at = cell_bounds(*col, caret_row, *n);
                            let ink = caret_bg.unwrap_or_else(|| theme::background(theme));
                            let run = TextRun {
                                len: text.len(),
                                font: fonts.face(style.bold, style.italic),
                                color: ink.into(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let force = (*n == 1).then(|| px(cells.width()));
                            let shaped = text_system.shape_line(
                                SharedString::from(text.clone()),
                                font_size,
                                &[run],
                                force,
                            );
                            // Placed like the row's own glyph: a wide one centred.
                            let x = glyph_x(&cells, *col, *n, f32::from(shaped.width()));
                            caret_glyph = Some(PlacedLine {
                                at: point(origin.x + px(x), at.origin.y),
                                line: shaped,
                                faux_bold: fonts.synthesizes(style.bold),
                            });
                        }
                        (at, caret_color)
                    }
                });

            // The find bar: a hairline over its first row, its rows shaped like
            // text rows (not cached: two rows, rebuilt only while it is open),
            // and a bar caret in its focused field.
            let bar_paint = view
                .editor
                .find_bar()
                .filter(|_| bar_rows > 0)
                .map(|bar| find_bar::paint(bar, view.preedit.as_deref(), theme, cols, focused));
            let mut bar_caret = None;
            if let Some(bar) = &bar_paint {
                let top = origin.y + px(cells.y_of(rows));
                let b = Bounds::new(
                    point(origin.x, top),
                    size(px(cells.width() * f32::from(cols)), px(RULE_THICKNESS)),
                );
                strokes.push((snap(b, scale), theme::divider(theme)));
                for (i, row) in bar.rows.iter().take(usize::from(bar_rows)).enumerate() {
                    let r = rows + i as u16;
                    let shaped = shape_row(row.clone(), &fonts, &text_system, font_size, &cells);
                    let row_top = origin.y + px(cells.y_of(r));
                    for piece in &shaped.pieces {
                        lines.push(PlacedLine {
                            at: point(origin.x + px(piece.x), row_top),
                            line: piece.line.clone(),
                            faux_bold: piece.faux_bold,
                        });
                    }
                }
                bar_caret = bar
                    .caret
                    .filter(|&(_, r)| r < bar_rows)
                    .map(|(c, r)| cell_bounds(c, rows + r, 1));
            }
            let (caret, caret_cell) = match bar_caret {
                Some(mut at) => {
                    let cell = at;
                    at.size.width = px(CARET_WIDTH);
                    (Some((at, caret_color)), Some(cell))
                }
                None if bar_open => (None, None),
                None => (caret, caret_cell),
            };
            let shape = if bar_open { CaretShape::Bar } else { shape };

            let mut mode_texts = Vec::new();
            if let Some(line) = &mode_line {
                let label_cells = cells_of(&line.label);
                let hint_cells = line.hint.as_deref().map(cells_of);
                let (label_col, hint_col) = mode_line_cols(cols, label_cells, hint_cells);
                let mut place = |text: &str, col: u16, color: Rgba, bold: bool| {
                    let run = TextRun {
                        len: text.len(),
                        font: fonts.face(bold, false),
                        color: color.into(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped = text_system.shape_line(
                        SharedString::from(text.to_string()),
                        font_size,
                        &[run],
                        Some(px(cells.width())),
                    );
                    mode_texts.push(PlacedLine {
                        at: cell_bounds(col, rows + bar_rows, 0).origin,
                        line: shaped,
                        faux_bold: fonts.synthesizes(bold),
                    });
                };
                place(&line.label, label_col, theme::mode_label(theme), true);
                if let (Some(hint), Some(col)) = (line.hint.as_deref(), hint_col) {
                    place(hint, col, theme::secondary(theme), false);
                }
            }
            view.painted = Some(Painted {
                origin,
                cells,
                rows,
                caret: caret_cell,
                caret_shape: caret.as_ref().map(|_| shape),
                mode_line: mode_line.clone(),
                row_paint,
                fonts: fonts.clone(),
                caret_glyph: caret_glyph.as_ref().map(|g| g.at),
                find_bar: bar_paint,
                popup: None,
            });
            // The popup hangs from the caret's cell: nowhere to hang from
            // (scrolled off, the find bar has the keyboard), nothing drawn.
            let popup = view
                .autocomplete_state()
                .map(popup_cells)
                .zip(caret_cell)
                .map(|(cells, caret)| PopupRequest { cells, caret });
            let prepaint = EditorPrepaint {
                hitbox,
                cells,
                line_height,
                faux_offset: faux_bold_offset(font_size),
                background: theme::background(theme),
                fills,
                strokes,
                lines,
                preedit,
                caret,
                caret_glyph,
                mode_line: mode_texts,
                text_hitbox: None,
                target_hitboxes: Vec::new(),
                target_areas,
                text_area: if mode_line.is_some() || bar_rows > 0 {
                    Bounds::new(origin, size(bounds.size.width, px(cells.y_of(rows))))
                } else {
                    bounds
                },
            };
            (prepaint, popup)
        });
        if let (Some(request), Some(element)) = (popup, request_layout.take()) {
            self.place_popup(
                element,
                &request,
                &prepaint.cells,
                bounds.origin,
                window,
                cx,
            );
        }
        prepaint.text_hitbox =
            Some(window.insert_hitbox(prepaint.text_area, HitboxBehavior::Normal));
        prepaint.target_hitboxes = prepaint
            .target_areas
            .iter()
            .map(|b| window.insert_hitbox(*b, HitboxBehavior::Normal))
            .collect();
        prepaint
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.view.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        if let Some(text) = &prepaint.text_hitbox {
            window.set_cursor_style(CursorStyle::IBeam, text);
        }
        // Later requests win where hitboxes overlap: the hand over a target.
        for target in &prepaint.target_hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, target);
        }

        let line_height = prepaint.line_height;
        let faux_offset = prepaint.faux_offset;
        let paint_line = |placed: &PlacedLine, window: &mut Window, cx: &mut App| {
            // A glyph the face cannot raster is skipped, not fatal.
            placed
                .line
                .paint(placed.at, line_height, TextAlign::Left, None, window, cx)
                .ok();
            if placed.faux_bold {
                let at = point(placed.at.x + faux_offset, placed.at.y);
                placed
                    .line
                    .paint(at, line_height, TextAlign::Left, None, window, cx)
                    .ok();
            }
        };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, prepaint.background));
            for (at, color) in &prepaint.fills {
                window.paint_quad(fill(*at, *color));
            }
            for (at, color) in &prepaint.strokes {
                window.paint_quad(fill(*at, *color));
            }
            for placed in &prepaint.lines {
                paint_line(placed, window, cx);
            }
            if let Some((covered, backing, line)) = &prepaint.preedit {
                window.paint_quad(fill(*covered, *backing));
                line.paint(
                    covered.origin,
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
            }
            if let Some((at, color)) = prepaint.caret {
                window.paint_quad(fill(at, color));
            }
            if let Some(glyph) = &prepaint.caret_glyph {
                paint_line(glyph, window, cx);
            }
            for text in &prepaint.mode_line {
                paint_line(text, window, cx);
            }
        });

        let origin = bounds.origin;
        let cells = prepaint.cells;
        window.on_mouse_event({
            let view = self.view.clone();
            let hitbox = prepaint.hitbox.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase.bubble() && hitbox.is_hovered(window) {
                    view.update(cx, |v, cx| {
                        v.pointer_down(event, origin, &cells, window, cx)
                    });
                    cx.stop_propagation();
                }
            }
        });
        // Move and release are taken anywhere: a drag that leaves the pane
        // keeps selecting, and its release must end it.
        window.on_mouse_event({
            let view = self.view.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() {
                    view.update(cx, |v, cx| {
                        v.pointer_move(event, origin, &cells, window, cx)
                    });
                }
            }
        });
        window.on_mouse_event({
            let view = self.view.clone();
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() {
                    view.update(cx, |v, cx| v.pointer_up(event, origin, &cells, window, cx));
                }
            }
        });
        window.on_mouse_event({
            let view = self.view.clone();
            let hitbox = prepaint.hitbox.clone();
            move |event: &ScrollEvent, phase, window, cx| {
                if phase.bubble() && hitbox.should_handle_scroll(window) {
                    view.update(cx, |v, cx| {
                        v.scroll_wheel(event, origin, &cells, window, cx)
                    });
                    cx.stop_propagation();
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_line_puts_the_hint_on_the_trailing_edge_while_it_fits() {
        // "NORMAL" (6) at col 1; "2d" ends one cell before col 40.
        assert_eq!(mode_line_cols(40, 6, Some(2)), (1, Some(37)));
        assert_eq!(mode_line_cols(40, 6, None), (1, None));
        // Needs a two-cell gap after the label (ends at 7): 9 + 2 + 1 = 12.
        assert_eq!(mode_line_cols(12, 6, Some(2)), (1, Some(9)));
        assert_eq!(mode_line_cols(11, 6, Some(2)), (1, None));
        assert_eq!(mode_line_cols(1, 6, Some(2)), (1, None));
    }

    #[test]
    fn wide_glyphs_are_centred_in_their_cells_and_narrow_ones_start_on_theirs() {
        let cells = CellMetrics::new(8.0, 20.0);
        assert_eq!(glyph_x(&cells, 3, 1, 8.0), 24.0);
        // A 10px CJK glyph in two 8px cells: 3px each side.
        assert_eq!(glyph_x(&cells, 3, 2, 10.0), 27.0);
        // Wider than its cells: starts on its cell, never left of it.
        assert_eq!(glyph_x(&cells, 3, 2, 20.0), 24.0);
    }

    #[test]
    fn synthetic_bold_offset_follows_the_font_size() {
        assert!(faux_bold_offset(px(13.)) > px(0.7) && faux_bold_offset(px(13.)) < px(0.8));
        assert_eq!(faux_bold_offset(px(26.)), faux_bold_offset(px(13.)) * 2.0);
    }

    #[test]
    fn snapping_lands_edges_on_device_pixels_and_never_collapses() {
        let b = Bounds::new(point(px(10.3), px(4.6)), size(px(7.8), px(0.2)));
        let s = snap(b, 2.0);
        assert_eq!(s.origin, point(px(10.5), px(4.5)));
        // 10.3 + 7.8 = 18.1 -> 18.0 at 2x.
        assert_eq!(s.size.width, px(7.5));
        // A sub-pixel stroke keeps one device pixel.
        assert_eq!(s.size.height, px(0.5));
        // Abutting cells share an edge after snapping.
        let a = snap(
            Bounds::new(point(px(0.), px(0.)), size(px(7.8), px(18.))),
            1.0,
        );
        let c = snap(
            Bounds::new(point(px(7.8), px(0.)), size(px(7.8), px(18.))),
            1.0,
        );
        assert_eq!(a.origin.x + a.size.width, c.origin.x);
    }
}
