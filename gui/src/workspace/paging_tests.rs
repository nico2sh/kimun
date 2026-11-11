//! Tests for **Page Up and Page Down** on a bare drawer list: the page size
//! ([`page_rows`]) and what the keys do to the selection.
#![cfg(test)]

use std::sync::Arc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext as _, Context, Div, Entity, Focusable as _, IntoElement, ParentElement as _, Render,
    SharedString, Stateful, Styled as _, TestAppContext, VisualTestContext, Window, div, px,
};
use kimun_notes::logic::list::ListRow;
use kimun_notes::settings::themes::Theme;

use super::drawer_list::{DrawerList, DrawerRow, page_rows};
use super::files::row_frame;

#[derive(Debug, Clone)]
struct Row(usize);

impl ListRow for Row {}

impl DrawerRow for Row {
    fn element_id(&self) -> SharedString {
        format!("paging-test-row:{}", self.0).into()
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        let label = SharedString::from(format!("row {}", self.0));
        row_frame(self.element_id(), label.clone(), selected, 2.0, t).child(label)
    }
}

/// The list's height: a whole number of rows fits only by chance.
const LIST_HEIGHT: f32 = 170.;

/// The list in a box of [`LIST_HEIGHT`].
struct Host {
    list: Entity<DrawerList<Row>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(LIST_HEIGHT))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .child(self.list.clone()),
            )
            .child(div().flex_1())
    }
}

/// A list of `count` rows with the keyboard, row `selected` selected and
/// drawn once (the scroll handle knows its bounds).
fn list_of(
    count: usize,
    selected: usize,
    cx: &mut TestAppContext,
) -> (Entity<DrawerList<Row>>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let (host, cx) = cx.add_window_view(|_, cx| {
        let list = cx.new(|cx| DrawerList::new("paging-test", "Rows", Arc::new(|| {}), cx));
        Host { list }
    });
    let list = host.read_with(cx, |host, _| host.list.clone());
    list.update(cx, |list, cx| {
        list.set_rows((0..count).map(Row).collect(), "none", cx);
        list.select_at(selected, cx);
    });
    cx.update(|window, cx| {
        let handle = list.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        window.render_frame(cx);
    });
    cx.run_until_parked();
    (list, cx)
}

fn selected(list: &Entity<DrawerList<Row>>, cx: &mut VisualTestContext) -> Option<usize> {
    list.read_with(cx, |list, _| list.selected())
}

fn press(key: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.press(key, cx);
        window.render_frame(cx);
    });
}

/// The rows a page holds in the drawn list: the box over the height of
/// row `pos` (one that is drawn).
fn page(pos: usize, cx: &mut VisualTestContext) -> usize {
    let row = cx.update(|window, cx| {
        window.render_frame(cx);
        window.find(Row(pos).element_id()).bounds().size.height
    });
    page_rows(px(LIST_HEIGHT), row)
}

/// Whether row `pos` is drawn inside the list's box.
fn in_view(pos: usize, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .try_find(Row(pos).element_id())
            .is_some_and(|row| row.visible())
    })
}

#[test]
fn page_rows_is_at_least_one() {
    assert_eq!(page_rows(px(10.), px(32.)), 1, "less than a row");
    assert_eq!(page_rows(px(0.), px(32.)), 1, "no room at all");
    assert_eq!(page_rows(px(100.), px(0.)), 1, "rows not measured");
}

#[test]
fn page_rows_floors() {
    assert_eq!(page_rows(px(170.), px(32.)), 5);
    assert_eq!(page_rows(px(160.), px(32.)), 5);
    assert_eq!(page_rows(px(191.9), px(32.)), 5);
    assert_eq!(page_rows(px(192.), px(32.)), 6);
}

/// Page Down moves the selection one page down and keeps it in view; at
/// the end it stops on the last row. With a modifier it does nothing.
#[gpui_kit::test]
fn page_down_moves_one_page_and_clamps(cx: &mut TestAppContext) {
    let (list, cx) = list_of(12, 0, cx);
    let page = page(0, cx);
    assert!(page > 1, "the box holds several rows ({page})");
    press("pagedown", cx);
    assert_eq!(selected(&list, cx), Some(page));
    assert!(in_view(page, cx), "the selection is scrolled into view");
    press("shift-pagedown", cx);
    assert_eq!(selected(&list, cx), Some(page), "a modifier: nothing");
    press("pagedown", cx);
    press("pagedown", cx);
    press("pagedown", cx);
    assert_eq!(selected(&list, cx), Some(11), "stops on the last row");
    assert!(in_view(11, cx));
}

/// Page Up moves the selection one page up and keeps it in view; at the
/// top it stops on the first row.
#[gpui_kit::test]
fn page_up_moves_one_page_and_clamps(cx: &mut TestAppContext) {
    let (list, cx) = list_of(12, 11, cx);
    let page = page(11, cx);
    assert!(page > 1, "the box holds several rows ({page})");
    press("pageup", cx);
    assert_eq!(selected(&list, cx), Some(11 - page));
    assert!(
        in_view(11 - page, cx),
        "the selection is scrolled into view"
    );
    press("pageup", cx);
    press("pageup", cx);
    press("pageup", cx);
    assert_eq!(selected(&list, cx), Some(0), "stops on the first row");
    assert!(in_view(0, cx));
}
