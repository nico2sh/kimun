//! Tests for the **Expand chevron** ([`super::chevron`]) on a bare drawer
//! list: which rows draw its zone, where the zone sits and how it looks,
//! where a row's score goes now that the zone covers the row's right end,
//! and what a click on the zone does (and does not do).
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, Bounds, Context, Div, Entity, Hsla, InputEvent as _, InteractiveElement as _,
    IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement as _, Pixels,
    Render, Rgba, SharedString, Stateful, StyleRefinement, Styled as _, TestAppContext,
    VisualTestContext, Window, div, px, size,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::ask::AskSource;
use kimun_notes::logic::list::ListRow;
use kimun_notes::logic::preview::PreviewModel;
use kimun_notes::settings::themes::Theme;

use super::ask_sources::{SourceRow, date_id};
use super::chevron::{
    ChevronLook, Reach, ZONE_REMS, ZoneOn, chevron, chevron_hover, preview_header,
};
use super::drawer_list::{DrawerList, DrawerListEvent, DrawerRow, EXPAND, OPEN_NOTE, chevron_id};
use super::files::{ClickPolicy, row_frame};
use super::result_row::{
    ROW_REMS, context_id, filename_id, first_line_id, result_row, score_id, title_id,
};
use crate::theme;

#[derive(Debug, Clone)]
struct Row(usize);

impl ListRow for Row {}

impl DrawerRow for Row {
    fn element_id(&self) -> SharedString {
        format!("chevron-test-row:{}", self.0).into()
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        let label = SharedString::from(format!("row {}", self.0));
        // The content takes all the width the row gives it, so its bounds
        // say how wide the row's content is.
        row_frame(self.element_id(), label.clone(), selected, 2.0, t).child(
            div()
                .id(text_of(self.0))
                .flex_1()
                .min_w_0()
                .child(label)
                .test_support(),
        )
    }
}

/// The list under a spacer, so the pointer's resting place is on no row.
struct Host {
    list: Entity<DrawerList<Row>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h(px(80.)).flex_none())
            .child(self.list.clone())
    }
}

/// What the list emitted, as `activate:N`, `verb:N:VERB` or the variant.
type Seen = Rc<RefCell<Vec<String>>>;

/// A three-row list under `policy`, row 1 selected, the chevron on.
fn three_rows(
    cx: &mut TestAppContext,
    policy: ClickPolicy,
) -> (Entity<DrawerList<Row>>, Seen, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let (host, cx) = cx.add_window_view(|_, cx| {
        let list = cx.new(|cx| {
            DrawerList::new("chevron-test", "Rows", Arc::new(|| {}), cx).with_click_policy(policy)
        });
        Host { list }
    });
    let list = host.read_with(cx, |host, _| host.list.clone());
    let seen: Seen = Rc::default();
    let log = seen.clone();
    cx.update(|_, cx| {
        cx.subscribe(&list, move |_, event: &DrawerListEvent<Row>, _| {
            log.borrow_mut().push(match event {
                DrawerListEvent::Activate(row) => format!("activate:{}", row.0),
                DrawerListEvent::Verb(row, verb) => format!("verb:{}:{verb}", row.0),
                DrawerListEvent::FileOp(..) => "file-op".into(),
                DrawerListEvent::Dismiss => "dismiss".into(),
            });
        })
        .detach();
    });
    list.update(cx, |list, cx| {
        list.set_rows((0..3).map(Row).collect(), "none", cx);
        list.select_at(1, cx);
        list.set_chevron(Some(Reach::Grow), cx);
    });
    cx.run_until_parked();
    (list, seen, cx)
}

fn text_of(row: usize) -> SharedString {
    format!("chevron-test-row:{row}:text").into()
}

/// Where `id` is laid out, drawn or not; `None` when it is not there.
fn bounds(id: SharedString, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.try_find(id).map(|e| e.bounds())
    })
}

fn chevron_of(row: usize) -> SharedString {
    chevron_id(&Row(row).element_id())
}

/// The chevron's accessible name on row `row`, when it is drawn.
fn shown(row: usize, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .try_find(chevron_of(row))
            .filter(|e| e.visible())
            .and_then(|e| e.label().map(str::to_string))
    })
}

fn selected(list: &Entity<DrawerList<Row>>, cx: &mut VisualTestContext) -> Option<usize> {
    list.read_with(cx, |list, _| list.selected())
}

#[test]
fn reach_is_grow_until_full() {
    let mut model = PreviewModel::new();
    assert_eq!(Reach::of(&model), Reach::Grow, "collapsed");
    model.toggle(Some(VaultPath::new("a.md")));
    assert!(model.is_context());
    assert_eq!(Reach::of(&model), Reach::Grow, "context");
    model.toggle(Some(VaultPath::new("a.md")));
    assert!(model.is_full());
    assert_eq!(Reach::of(&model), Reach::Shrink, "full");

    assert_eq!(Reach::Grow.glyph(), "›");
    assert_eq!(Reach::Shrink.glyph(), "‹");
    assert_eq!(Reach::Grow.tooltip(), "Expand the preview (→)");
    assert_eq!(Reach::Shrink.tooltip(), "Collapse the preview (←)");
}

/// At rest only the selected row draws the chevron; the hovered row draws
/// it too; a list with no chevron draws none, and Shrink names itself.
#[gpui_kit::test]
fn zone_shows_on_selected_and_hovered_rows_only(cx: &mut TestAppContext) {
    let (list, _, cx) = three_rows(cx, ClickPolicy::Select);
    let grow = Some(Reach::Grow.tooltip().to_string());
    assert_eq!(shown(0, cx), None);
    assert_eq!(shown(1, cx), grow);
    assert_eq!(shown(2, cx), None);

    cx.update(|window, cx| {
        window.render_frame(cx);
        window.hover(Row(2).element_id(), cx);
    });
    assert_eq!(shown(0, cx), None);
    assert_eq!(shown(1, cx), grow, "the selected row keeps it");
    assert_eq!(shown(2, cx), grow, "the hovered row");

    list.update(cx, |list, cx| list.set_chevron(Some(Reach::Shrink), cx));
    assert_eq!(shown(1, cx), Some(Reach::Shrink.tooltip().to_string()));

    list.update(cx, |list, cx| list.set_chevron(None, cx));
    for row in 0..3 {
        assert!(
            cx.update(|window, cx| {
                window.render_frame(cx);
                window.try_find(chevron_of(row)).is_none()
            }),
            "no chevron on row {row}"
        );
    }
}

/// A click on another row's chevron selects that row, then sends it with
/// [`EXPAND`]: nothing else.
#[gpui_kit::test]
fn zone_click_selects_row_and_sends_expand_verb(cx: &mut TestAppContext) {
    let (list, seen, cx) = three_rows(cx, ClickPolicy::Select);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.hover(Row(2).element_id(), cx);
        window.render_frame(cx);
        window.click(chevron_of(2), cx);
    });
    cx.run_until_parked();
    assert_eq!(selected(&list, cx), Some(2));
    assert_eq!(*seen.borrow(), [format!("verb:2:{EXPAND}")]);

    // A right click on it does nothing.
    seen.borrow_mut().clear();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.right_click(chevron_of(2), cx);
    });
    cx.run_until_parked();
    assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());
}

/// Under `Open` a click on a row activates it, but a click on its chevron
/// never does, and a double-click on it sends one verb and opens nothing.
#[gpui_kit::test]
fn zone_click_does_not_open_the_note(cx: &mut TestAppContext) {
    let (list, seen, cx) = three_rows(cx, ClickPolicy::Open);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.hover(Row(0).element_id(), cx);
        window.render_frame(cx);
        window.click(chevron_of(0), cx);
    });
    cx.run_until_parked();
    assert_eq!(selected(&list, cx), Some(0));
    assert_eq!(*seen.borrow(), [format!("verb:0:{EXPAND}")]);

    seen.borrow_mut().clear();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.double_click(chevron_of(0), cx);
    });
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [format!("verb:0:{EXPAND}")]);

    // The row's own click still activates it.
    seen.borrow_mut().clear();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.click(Row(2).element_id(), cx);
    });
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), ["activate:2".to_string()]);
}

/// A row of a list with the chevron keeps the content width of a row of a
/// list without one: selected, hovered or idle, the chevron reserves
/// nothing and sits over the row instead.
#[gpui_kit::test]
fn idle_rows_keep_full_width(cx: &mut TestAppContext) {
    let (list, _, cx) = three_rows(cx, ClickPolicy::Select);
    let widths = |cx: &mut VisualTestContext| -> Vec<Pixels> {
        (0..3)
            .map(|row| bounds(text_of(row), cx).expect("content").size.width)
            .collect()
    };
    let with_chevron = widths(cx);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.hover(Row(2).element_id(), cx);
    });
    let hovered = widths(cx);
    list.update(cx, |list, cx| list.set_chevron(None, cx));
    let without = widths(cx);
    for row in 0..3 {
        assert!(without[row] > px(0.), "row {row}");
        assert_eq!(with_chevron[row], without[row], "row {row} at rest");
        assert_eq!(hovered[row], without[row], "row {row}, row 2 hovered");
    }
}

/// A search result row (FIND, SEM), drawn by the real row.
#[derive(Debug, Clone)]
struct Hit(usize);

impl ListRow for Hit {}

impl DrawerRow for Hit {
    fn element_id(&self) -> SharedString {
        format!("chevron-test-hit:{}", self.0).into()
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        result_row(
            self.element_id(),
            selected,
            "A title long enough to reach the right end of its row, and then some more",
            "a-title.md",
            "Some context that runs on and on until it is cut short at the end",
            Some("87%".into()),
            &[],
            t,
        )
    }
}

/// A Sources view row, drawn by the real row: a heading long enough to be
/// cut short.
fn source(rank: usize) -> SourceRow {
    SourceRow::new(
        rank,
        AskSource {
            path: VaultPath::new("a-source.md"),
            heading: "A heading long enough to reach the right end of its row, and then more"
                .into(),
            date: None,
            score: 0.87,
            text: "the chunk".into(),
            ordinal: rank,
        },
    )
}

/// [`source`], a journal entry: its date beside the heading.
fn dated_source(rank: usize) -> SourceRow {
    let mut row = source(rank);
    row.source.date = Some("2026-04-08".into());
    row
}

/// A list of `R` filling the window.
struct ListHost<R: DrawerRow> {
    list: Entity<DrawerList<R>>,
}

impl<R: DrawerRow> Render for ListHost<R> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col().child(self.list.clone())
    }
}

/// A list of `rows` filling the window, the first selected, the chevron on.
fn list_of<R: DrawerRow>(
    cx: &mut TestAppContext,
    rows: Vec<R>,
) -> (Entity<DrawerList<R>>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let (host, cx) = cx.add_window_view(|_, cx| {
        let list = cx.new(|cx| DrawerList::new("chevron-test-list", "Rows", Arc::new(|| {}), cx));
        ListHost { list }
    });
    let list = host.read_with(cx, |host, _| host.list.clone());
    list.update(cx, |list, cx| {
        list.set_rows(rows, "none", cx);
        list.select_at(0, cx);
        list.set_chevron(Some(Reach::Grow), cx);
    });
    cx.run_until_parked();
    (list, cx)
}

/// Move the pointer onto the element `id`.
fn hover(id: SharedString, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.hover(id, cx);
    });
}

/// Whether the last frame painted a quad over `zone` (layout pixels,
/// within 1 px) with a left border and no other: the zone's divider, drawn
/// where the layout put it.
fn painted_divider_at(zone: Bounds<Pixels>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let scale = window.scale_factor();
        window.painted_quads().iter().any(|quad| {
            let b = quad.bounds;
            let near = |a: f32, b: Pixels| (a / scale - f32::from(b)).abs() <= 1.;
            let w = quad.border_widths;
            near(b.origin.x.0, zone.left())
                && near(b.origin.y.0, zone.top())
                && near(b.size.width.0, zone.size.width)
                && near(b.size.height.0, zone.size.height)
                && w.left.0 > 0.
                && w.top.0 == 0.
                && w.right.0 == 0.
                && w.bottom.0 == 0.
        })
    })
}

/// The zone of the row `id` sits flush with the row's right end and runs
/// its full height, [`ZONE_REMS`] wide, laid out and painted there: never
/// at the row's left side.
fn assert_flush_right(id: &SharedString, cx: &mut VisualTestContext) {
    let rem = cx.update(|window, _| window.rem_size());
    let row = bounds(id.clone(), cx).expect("row");
    let zone = bounds(chevron_id(id), cx).expect("zone");
    assert!(
        (zone.right() - row.right()).abs() <= px(1.),
        "{id}: zone {zone:?} flush right in {row:?}"
    );
    assert!(
        (zone.top() - row.top()).abs() <= px(1.)
            && (zone.size.height - row.size.height).abs() <= px(1.),
        "{id}: zone {zone:?} as tall as {row:?}"
    );
    assert!(
        (zone.size.width - rem * ZONE_REMS).abs() <= px(1.),
        "{id}: zone {zone:?} {ZONE_REMS} rem wide"
    );
    assert!(
        zone.left() > row.center().x,
        "{id}: zone {zone:?} in the row's right half, {row:?}"
    );
    assert!(
        painted_divider_at(zone, cx),
        "{id}: the divider painted at {zone:?}"
    );
}

/// The zone of the selected row and of the hovered row, on a one-line row
/// and on a three-line one (FIND, SEM), at the window's width and at a
/// narrow one, and on the open preview's header: flush with the right end,
/// as tall as the row (the header's line), at the right end once painted.
#[gpui_kit::test]
fn zone_is_flush_right_and_full_height(cx: &mut TestAppContext) {
    let (_, _, cx) = three_rows(cx, ClickPolicy::Select);
    assert_flush_right(&Row(1).element_id(), cx);
    hover(Row(2).element_id(), cx);
    assert_flush_right(&Row(2).element_id(), cx);
    assert_flush_right(&Row(1).element_id(), cx);
    cx.simulate_resize(size(px(320.), px(600.)));
    cx.run_until_parked();
    assert_flush_right(&Row(1).element_id(), cx);
}

#[gpui_kit::test]
fn zone_is_flush_right_and_full_height_on_a_result_row(cx: &mut TestAppContext) {
    let (_, cx) = list_of(cx, (0..3).map(Hit).collect());
    let rem = cx.update(|window, _| window.rem_size());
    let first = Hit(0).element_id();
    assert_eq!(
        bounds(first.clone(), cx).expect("row").size.height,
        rem * ROW_REMS,
        "a three-line row"
    );
    assert_flush_right(&first, cx);
    hover(Hit(2).element_id(), cx);
    assert_flush_right(&Hit(2).element_id(), cx);
}

/// The open preview's header, laid out as the views lay it out: a column
/// above the preview.
struct HeaderHost;

const HEADER_ID: &str = "chevron-test-header";

impl Render for HeaderHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                preview_header(HEADER_ID.into(), "A title".into(), Reach::Shrink, &t)
                    .test_support(),
            )
            .child(div().flex_1())
    }
}

#[gpui_kit::test]
fn zone_is_flush_right_and_full_height_on_the_header(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (_, cx) = cx.add_window_view(|_, _| HeaderHost);
    let header = SharedString::from(HEADER_ID);
    assert_flush_right(&header, cx);
    let title_room = bounds(header.clone(), cx).expect("header").size.width;
    assert!(title_room > px(100.), "{title_room:?}");
}

/// The element [`chevron`] returns is the zone: a left border alone, in
/// the divider colour, the glyph in the dim text colour; on a row the
/// row's own fill, opaque, and the row's rounded right corners; on the
/// header no fill and square corners. Under the pointer ([`chevron_hover`])
/// the faint fill and the text colour.
#[test]
fn zone_has_a_divider_on_its_left_edge() {
    for t in Theme::builtins() {
        for on in [ZoneOn::SelectedRow, ZoneOn::HoveredRow, ZoneOn::Header] {
            let look = ChevronLook::of(&t, on);
            let name = format!("{} {on:?}", t.name);
            assert_eq!(look.glyph, theme::secondary(&t), "{name}: glyph token");
            assert_eq!(look.hover_glyph, theme::foreground(&t), "{name}");
            let mut zone = chevron("chevron-style".into(), Reach::Grow, on, &t);
            let style = zone.style();
            let widths = &style.border_widths;
            let width = |side: Option<gpui_kit::AbsoluteLength>| {
                side.map_or(px(0.), |width| width.to_pixels(px(16.)))
            };
            assert!(width(widths.left) > px(0.), "{name}: left border");
            for (side, w) in [
                ("top", widths.top),
                ("right", widths.right),
                ("bottom", widths.bottom),
            ] {
                assert_eq!(width(w), px(0.), "{name}: {side} border");
            }
            assert_eq!(style.border_color, Some(Hsla::from(look.divider)), "{name}");
            let fill = style
                .background
                .as_ref()
                .and_then(|fill| fill.color())
                .and_then(|fill| fill.as_solid());
            assert_eq!(fill, on.fill(&t).map(Hsla::from), "{name}: rest fill");
            let radii = &style.corner_radii;
            let rounded = |r: Option<gpui_kit::AbsoluteLength>| {
                r.is_some_and(|r| r.to_pixels(px(16.)) > px(0.))
            };
            let row = on != ZoneOn::Header;
            assert_eq!(rounded(radii.top_right), row, "{name}: top right corner");
            assert_eq!(rounded(radii.bottom_right), row, "{name}: bottom right");
            assert!(
                !rounded(radii.top_left) && !rounded(radii.bottom_left),
                "{name}"
            );
            assert_eq!(
                style.text.color,
                Some(Hsla::from(look.glyph)),
                "{name}: glyph"
            );

            let hover = chevron_hover(look, StyleRefinement::default());
            let fill = hover.background.as_ref().and_then(|fill| fill.color());
            assert_eq!(
                fill.and_then(|fill| fill.as_solid()),
                Some(Hsla::from(look.hover_fill)),
                "{name}: hover fill"
            );
            assert_eq!(
                hover.text.color,
                Some(Hsla::from(look.hover_glyph)),
                "{name}: hover glyph"
            );
        }
    }
}

/// The least distance two colours must keep to read as different here:
/// the straight-line distance between them in sRGB, each channel 0 to 1
/// (black to white is about 1.73). 0.05 is a little more than a step of
/// 12 of 255 on every channel: clearly visible, while the theme's own
/// hover tint lies about 0.05 to 0.1 off the panel.
const MIN_DISTANCE: f32 = 0.05;

fn distance(a: Rgba, b: Rgba) -> f32 {
    ((a.r - b.r).powi(2) + (a.g - b.g).powi(2) + (a.b - b.b).powi(2)).sqrt()
}

/// `color` as drawn over the opaque `under`.
fn over(color: Rgba, under: Rgba) -> Rgba {
    let mix = |c: f32, u: f32| u + (c - u) * color.a;
    Rgba {
        r: mix(color.r, under.r),
        g: mix(color.g, under.g),
        b: mix(color.b, under.b),
        a: 1.,
    }
}

/// The zone reads on a selected row and on a hovered row in every built-in
/// theme. Its rest fill is the row's fill, opaque (it masks the text
/// beneath); its hover fill is opaque too; that hover fill and the divider
/// drawn over the rest fill stand off the row's fill by [`MIN_DISTANCE`]
/// at least. (Each is measured against the fill of the row it is drawn on:
/// the zone is never drawn over both rows at once.) On the header, which
/// has no fill, the see-through hover tint and divider stand off the panel
/// by as much.
#[test]
fn zone_reads_on_every_row() {
    for t in Theme::builtins() {
        for on in [ZoneOn::HoveredRow, ZoneOn::SelectedRow, ZoneOn::Header] {
            let look = ChevronLook::of(&t, on);
            let name = format!("{} {on:?}", t.name);
            let under = match on.fill(&t) {
                Some(row) => {
                    assert_eq!(look.fill, Some(row), "{name}: the row's fill");
                    assert_eq!(row.a, 1., "{name}: an opaque rest fill");
                    assert_eq!(look.hover_fill.a, 1., "{name}: an opaque hover fill");
                    row
                }
                None => {
                    assert_eq!(look.fill, None, "{name}");
                    theme::background(&t)
                }
            };
            for (part, color) in [("hover fill", look.hover_fill), ("divider", look.divider)] {
                let gap = distance(over(color, under), under);
                assert!(
                    gap >= MIN_DISTANCE,
                    "{name}: the {part} is {gap:.3} off what it is drawn on"
                );
            }
        }
    }
}

/// The quad the last frame painted at `at` (layout pixels, within 1 px)
/// filled in `fill`.
fn quad_at(at: Bounds<Pixels>, fill: Hsla, cx: &mut VisualTestContext) -> Option<gpui_kit::Quad> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        let scale = window.scale_factor();
        window.painted_quads().into_iter().find(|quad| {
            let b = quad.bounds;
            let near = |a: f32, b: Pixels| (a / scale - f32::from(b)).abs() <= 1.;
            near(b.origin.x.0, at.left())
                && near(b.origin.y.0, at.top())
                && near(b.size.width.0, at.size.width)
                && near(b.size.height.0, at.size.height)
                && quad.background.as_solid() == Some(fill)
        })
    })
}

/// On a selected three-line row (FIND, SEM) the zone is opaque: painted
/// after the row's fill, in that very fill, so anything of the row that
/// still reached under it would be hidden. (The text lines stop short of it
/// now, [`every_line_of_a_result_row_ends_before_the_zone`]; the scene does
/// not say where text lies in the paint order, but the zone is the row's
/// last child, so GPUI paints it after the row's text.) So is the zone of a
/// hovered row, in the hover fill.
#[gpui_kit::test]
fn zone_masks_the_text_beneath(cx: &mut TestAppContext) {
    let (_, cx) = list_of(cx, (0..3).map(Hit).collect());
    hover(Hit(2).element_id(), cx);
    let t = cx.update(|_, cx| theme::current(cx));
    for (row, on) in [(0, ZoneOn::SelectedRow), (2, ZoneOn::HoveredRow)] {
        let id = Hit(row).element_id();
        let row_bounds = bounds(id.clone(), cx).expect("row");
        let zone = bounds(chevron_id(&id), cx).expect("zone");
        let fill = Hsla::from(on.fill(&t).expect("a row fill"));
        assert_eq!(fill.a, 1., "{id}: an opaque fill");
        let painted = quad_at(zone, fill, cx).expect("the zone painted in the row's fill");
        let under = quad_at(row_bounds, fill, cx).expect("the row's fill painted");
        assert!(
            painted.order > under.order,
            "{id}: the zone {} over the row {}",
            painted.order,
            under.order
        );
    }
}

/// Where the row `id`'s title, score and zone lie.
fn title_score_zone(
    id: &SharedString,
    cx: &mut VisualTestContext,
) -> (Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
    (
        bounds(title_id(id), cx).expect("title"),
        bounds(score_id(id), cx).expect("score"),
        bounds(chevron_id(id), cx).expect("zone"),
    )
}

/// The rows `first` (selected) and `second` (hovered) of `list`: wide, the
/// score sits just after the whole title; narrow, the title is cut short
/// (its box narrower than its text), the score keeps its whole width, and
/// it ends left of the zone in both states.
fn assert_score_beside_title(
    first: SharedString,
    second: SharedString,
    cx: &mut VisualTestContext,
) {
    let rem = cx.update(|window, _| window.rem_size());
    hover(second.clone(), cx);
    let mut wide = Vec::new();
    for id in [&first, &second] {
        let (title, score, _) = title_score_zone(id, cx);
        assert!(
            score.left() >= title.right() && score.left() - title.right() <= rem,
            "{id}: the score {score:?} just after the title {title:?}"
        );
        wide.push((title, score));
    }
    cx.simulate_resize(size(px(360.), px(600.)));
    cx.run_until_parked();
    hover(second.clone(), cx);
    for (id, (wide_title, wide_score)) in [&first, &second].into_iter().zip(wide) {
        let (title, score, zone) = title_score_zone(id, cx);
        assert!(
            title.size.width < wide_title.size.width,
            "{id}: the title {title:?} cut short (its text is {wide_title:?})"
        );
        assert!(
            score.left() >= title.right(),
            "{id}: {score:?} after {title:?}"
        );
        assert_eq!(
            score.size.width, wide_score.size.width,
            "{id}: the score keeps its width"
        );
        assert!(
            score.right() <= zone.left() && !score.intersects(&zone),
            "{id}: the score {score:?} left of the zone {zone:?}"
        );
    }
}

/// SEM's row and the Sources view's row put the score just after the
/// title, never under the zone: a long title gives way (an ellipsis), the
/// score does not.
#[gpui_kit::test]
fn score_sits_next_to_the_title_on_a_result_row(cx: &mut TestAppContext) {
    let (_, cx) = list_of(cx, (0..2).map(Hit).collect());
    assert_score_beside_title(Hit(0).element_id(), Hit(1).element_id(), cx);
}

#[gpui_kit::test]
fn score_sits_next_to_the_title_on_a_source_row(cx: &mut TestAppContext) {
    let (_, cx) = list_of(cx, vec![source(1), source(2)]);
    assert_score_beside_title(source(1).element_id(), source(2).element_id(), cx);
}

/// A press on the row right after a chevron click (the OS counts it as
/// the second of a double-click) is a click of its own: it selects, and
/// does not open the row's note.
#[gpui_kit::test]
fn a_row_press_after_the_chevron_is_no_double_click(cx: &mut TestAppContext) {
    let (list, seen, cx) = three_rows(cx, ClickPolicy::Select);
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.hover(Row(2).element_id(), cx);
        window.render_frame(cx);
        window.click(chevron_of(2), cx);
    });
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), [format!("verb:2:{EXPAND}")]);
    seen.borrow_mut().clear();
    let at = bounds(text_of(2), cx).expect("content").center();
    cx.update(|window, cx| {
        let modifiers = Modifiers::default();
        let down = MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers,
            click_count: 2,
            first_mouse: false,
        };
        let up = MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers,
            click_count: 2,
        };
        window.dispatch_event(down.to_platform_input(), cx);
        window.dispatch_event(up.to_platform_input(), cx);
    });
    cx.run_until_parked();
    assert_eq!(selected(&list, cx), Some(2));
    assert!(
        !seen.borrow().contains(&format!("verb:2:{OPEN_NOTE}")),
        "{:?}",
        seen.borrow()
    );

    // Without the chevron before it, the same double press opens the note.
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.double_click(text_of(0), cx);
    });
    cx.run_until_parked();
    assert!(
        seen.borrow().contains(&format!("verb:0:{OPEN_NOTE}")),
        "{:?}",
        seen.borrow()
    );
}

/// In a very narrow drawer a journal source's heading line (title, date,
/// score) still ends short of the zone: the title gives way, then the date;
/// nothing of the line reaches under the zone, selected or hovered.
#[gpui_kit::test]
fn a_dated_source_row_stays_clear_of_the_zone_when_narrow(cx: &mut TestAppContext) {
    let (_, cx) = list_of(cx, vec![dated_source(1), dated_source(2)]);
    let wide_score = bounds(score_id(&dated_source(1).element_id()), cx)
        .expect("score")
        .size
        .width;
    for width in [px(360.), px(240.), px(180.), px(150.)] {
        cx.simulate_resize(size(width, px(600.)));
        cx.run_until_parked();
        hover(dated_source(2).element_id(), cx);
        for rank in [1, 2] {
            let id = dated_source(rank).element_id();
            let zone = bounds(chevron_id(&id), cx).expect("zone");
            let score = bounds(score_id(&id), cx).expect("score");
            assert_eq!(
                score.size.width, wide_score,
                "{id} at {width:?}: whole score"
            );
            for (part, at) in [
                ("title", bounds(title_id(&id), cx).expect("title")),
                ("date", bounds(date_id(&id), cx).expect("date")),
                ("score", score),
            ] {
                assert!(
                    at.right() <= zone.left() && !at.intersects(&zone),
                    "{id} at {width:?}: the {part} {at:?} reaches the zone {zone:?}"
                );
            }
        }
    }
}

/// In a very narrow drawer a result row's first line (title, score) still
/// ends short of the zone: the title gives way, the score keeps its width
/// and ends left of the zone, selected or hovered.
#[gpui_kit::test]
fn a_result_row_score_stays_clear_of_the_zone_when_narrow(cx: &mut TestAppContext) {
    let (_, cx) = list_of(cx, (0..2).map(Hit).collect());
    let wide_score = bounds(score_id(&Hit(0).element_id()), cx)
        .expect("score")
        .size
        .width;
    for width in [px(360.), px(240.), px(180.), px(150.), px(120.), px(100.)] {
        cx.simulate_resize(size(width, px(600.)));
        cx.run_until_parked();
        hover(Hit(1).element_id(), cx);
        for row in [0, 1] {
            let id = Hit(row).element_id();
            let zone = bounds(chevron_id(&id), cx).expect("zone");
            let score = bounds(score_id(&id), cx).expect("score");
            let line = bounds(first_line_id(&id), cx).expect("first line");
            assert_eq!(score.size.width, wide_score, "{id} at {width:?}");
            assert!(
                score.right() <= zone.left() && !score.intersects(&zone),
                "{id} at {width:?}: the score {score:?} reaches the zone {zone:?}"
            );
            assert!(
                line.right() <= zone.left(),
                "{id} at {width:?}: the line {line:?} runs under the zone {zone:?}"
            );
        }
    }
}

/// A FIND row: [`Hit`]'s, without a score.
#[derive(Debug, Clone)]
struct Found(usize);

impl ListRow for Found {}

impl DrawerRow for Found {
    fn element_id(&self) -> SharedString {
        format!("chevron-test-found:{}", self.0).into()
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        result_row(
            self.element_id(),
            selected,
            "A title long enough to reach the right end of its row, and then some more",
            "a-title-that-is-just-as-long-as-the-title-is-and-runs-past-the-end.md",
            "Some context that runs on and on until it is cut short at the end, and then it runs on again",
            None,
            &[],
            t,
        )
    }
}

/// Every text line of a row ends left of the zone while the zone is there
/// (the selected row, the hovered row), so a line cut short keeps its
/// ellipsis in sight; an idle row's lines keep the row's whole width.
fn assert_lines_clear_of_the_zone<R: DrawerRow>(
    rows: Vec<R>,
    lines: &[fn(&SharedString) -> SharedString],
    cx: &mut TestAppContext,
) {
    let ids: Vec<SharedString> = rows.iter().map(DrawerRow::element_id).collect();
    let (_, cx) = list_of(cx, rows);
    let rem = cx.update(|window, _| window.rem_size());
    hover(ids[1].clone(), cx);
    for (at, id) in ids.iter().enumerate() {
        let zone = bounds(chevron_id(id), cx);
        let row = bounds(id.clone(), cx).expect("row");
        for line in lines {
            let line_id = line(id);
            let line = bounds(line_id.clone(), cx).expect("line");
            match at {
                // 0 selected, 1 hovered: the zone is there.
                0 | 1 => {
                    let zone = zone.expect("zone");
                    assert!(
                        line.right() <= zone.left(),
                        "{line_id}: {line:?} runs under the zone {zone:?}"
                    );
                }
                // 2 idle: no zone; the line runs to the row's padding.
                _ => assert!(
                    line.right() >= row.right() - rem * 0.5 - px(1.),
                    "{line_id}: {line:?} narrowed on the idle row {row:?}"
                ),
            }
        }
    }
}

#[gpui_kit::test]
fn every_line_of_a_result_row_ends_before_the_zone(cx: &mut TestAppContext) {
    assert_lines_clear_of_the_zone(
        (0..3).map(Hit).collect(),
        &[first_line_id, filename_id, context_id],
        cx,
    );
}

#[gpui_kit::test]
fn every_line_of_a_find_row_ends_before_the_zone(cx: &mut TestAppContext) {
    assert_lines_clear_of_the_zone(
        (0..3).map(Found).collect(),
        &[first_line_id, filename_id, context_id],
        cx,
    );
}

#[gpui_kit::test]
fn every_line_of_a_source_row_ends_before_the_zone(cx: &mut TestAppContext) {
    assert_lines_clear_of_the_zone(
        vec![source(1), source(2), source(3)],
        &[first_line_id, filename_id],
        cx,
    );
}

/// In a real view's window `handle`: the zone of the row (or header) `id`
/// lies inside it, its right edge within 1 px of the row's.
pub(super) fn assert_zone_flush_right_in(
    handle: gpui_kit::AnyWindowHandle,
    id: SharedString,
    cx: &mut TestAppContext,
) {
    let (row, zone) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find(id.clone()).bounds(),
                window.find(chevron_id(&id)).bounds(),
            )
        })
        .unwrap();
    assert!(
        (zone.right() - row.right()).abs() <= px(1.),
        "{id}: zone {zone:?} flush right in {row:?}"
    );
    assert!(
        zone.left() >= row.left() && zone.top() >= row.top() && zone.bottom() <= row.bottom(),
        "{id}: zone {zone:?} inside {row:?}"
    );
}
