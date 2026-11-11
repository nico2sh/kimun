//! Tests for the **Expand chevron** ([`super::chevron`]) on a bare drawer
//! list: which rows draw it, and what a click on it does (and does not do).
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, Bounds, Context, Div, Entity, InputEvent as _, InteractiveElement as _,
    IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement as _, Pixels,
    Render, SharedString, Stateful, Styled as _, TestAppContext, VisualTestContext, Window, div,
    px,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::list::ListRow;
use kimun_notes::logic::preview::PreviewModel;
use kimun_notes::settings::themes::Theme;

use super::chevron::{CHEVRON_REMS, ChevronLook, Reach};
use super::drawer_list::{DrawerList, DrawerListEvent, DrawerRow, EXPAND, OPEN_NOTE, chevron_id};
use super::files::{ClickPolicy, row_frame};
use super::result_row::{ROW_REMS, result_row, score_id};
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
fn chevron_shows_on_selected_row_only_at_rest(cx: &mut TestAppContext) {
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
fn chevron_click_selects_row_and_sends_expand_verb(cx: &mut TestAppContext) {
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
fn chevron_click_does_not_activate_row(cx: &mut TestAppContext) {
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

struct HitHost {
    list: Entity<DrawerList<Hit>>,
}

impl Render for HitHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col().child(self.list.clone())
    }
}

/// On a three-line row (FIND, SEM: 3.75 rem) the chevron is a compact
/// button centred on the row: it leaves the first line's right end (SEM's
/// score) uncovered.
#[gpui_kit::test]
fn chevron_does_not_cover_the_first_line_of_a_three_line_row(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (host, cx) = cx.add_window_view(|_, cx| {
        let list = cx.new(|cx| DrawerList::new("chevron-test-hits", "Hits", Arc::new(|| {}), cx));
        HitHost { list }
    });
    let list = host.read_with(cx, |host, _| host.list.clone());
    list.update(cx, |list, cx| {
        list.set_rows((0..2).map(Hit).collect(), "none", cx);
        list.select_at(0, cx);
        list.set_chevron(Some(Reach::Grow), cx);
    });
    cx.run_until_parked();
    let rem = cx.update(|window, _| window.rem_size());
    let id = Hit(0).element_id();
    let row = bounds(id.clone(), cx).expect("row");
    let score = bounds(score_id(&id), cx).expect("score");
    let chevron = bounds(chevron_id(&id), cx).expect("chevron");
    assert_eq!(row.size.height, rem * ROW_REMS, "a three-line row");
    assert!(
        !chevron.intersects(&score),
        "the chevron {chevron:?} covers the score {score:?}"
    );
    assert!(score.bottom() <= chevron.top(), "{score:?} {chevron:?}");
    // Compact, centred, at the right end, inside the row.
    assert_eq!(chevron.size.width, rem * CHEVRON_REMS);
    assert_eq!(chevron.size.height, rem * CHEVRON_REMS);
    assert!(
        (chevron.center().y - row.center().y).abs() <= px(1.),
        "{chevron:?} centred on {row:?}"
    );
    assert!(chevron.right() <= row.right(), "{chevron:?} in {row:?}");
    assert!(
        chevron.left() > score.left(),
        "{chevron:?} at the right end, {score:?}"
    );
}

/// The clickable area shows: the button has a fill and a hairline border
/// from the theme at rest, a stronger fill and the text colour under the
/// pointer. Read off [`ChevronLook`], the colours [`chevron`] draws with
/// (the test window cannot read styles back), for every built-in theme.
#[test]
fn chevron_button_has_a_visible_surface() {
    for t in Theme::builtins() {
        let look = ChevronLook::of(&t);
        let name = &t.name;
        assert_eq!(look.fill, theme::hover_background(&t), "{name}");
        assert_eq!(look.border, theme::divider(&t), "{name}");
        assert_eq!(look.glyph, theme::secondary(&t), "{name}");
        assert_eq!(look.hover_fill, theme::selected_background(&t), "{name}");
        assert_eq!(look.hover_glyph, theme::foreground(&t), "{name}");
        assert!(look.fill.a > 0. && look.border.a > 0., "{name}: {look:?}");
        assert_ne!(look.hover_fill, look.fill, "{name}: the hover is stronger");
    }
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
