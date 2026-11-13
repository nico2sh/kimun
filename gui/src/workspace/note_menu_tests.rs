//! Tests for the **note menu** ([`super::note_menu`]): which items the
//! context shows, and what the view's keys pick.
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Focusable as _, TestAppContext, WeakEntity};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout_tree::LayoutTree;
use kimun_notes::logic::nav::DrawerView;

use super::file_dialogs::FileOpKind;
use super::note_menu::{
    MenuContext, MenuItem, NOTE_MENU_ID, NoteMenu, NoteMenuEvent, menu_item_element_id, menu_items,
};

const NOTE_ITEMS: [MenuItem; 4] = [
    MenuItem::Note(FileOpKind::Properties),
    MenuItem::Note(FileOpKind::Rename),
    MenuItem::Note(FileOpKind::Move),
    MenuItem::Note(FileOpKind::Delete),
];

#[test]
fn an_empty_group_shows_only_layout_actions() {
    let ctx = MenuContext {
        has_note: false,
        groups_in_tab: 1,
        tabs: 1,
    };
    assert_eq!(
        menu_items(&ctx),
        vec![MenuItem::SplitRight, MenuItem::SplitDown, MenuItem::NewTab]
    );
}

#[test]
fn close_group_needs_two_groups() {
    let one = MenuContext {
        has_note: true,
        groups_in_tab: 1,
        tabs: 1,
    };
    assert!(!menu_items(&one).contains(&MenuItem::CloseGroup));
    let two = MenuContext {
        groups_in_tab: 2,
        ..one
    };
    let items = menu_items(&two);
    assert!(items.contains(&MenuItem::CloseGroup));
    assert!(!items.contains(&MenuItem::CloseTab));
}

#[test]
fn close_tab_needs_two_tabs() {
    let one = MenuContext {
        has_note: true,
        groups_in_tab: 1,
        tabs: 1,
    };
    assert!(!menu_items(&one).contains(&MenuItem::CloseTab));
    let two = MenuContext { tabs: 2, ..one };
    let items = menu_items(&two);
    assert!(items.contains(&MenuItem::CloseTab));
    assert!(!items.contains(&MenuItem::CloseGroup));
}

#[test]
fn a_single_note_in_one_tab_shows_note_actions_split_right_split_down_new_tab() {
    let ctx = MenuContext {
        has_note: true,
        groups_in_tab: 1,
        tabs: 1,
    };
    let mut want = NOTE_ITEMS.to_vec();
    want.extend([MenuItem::SplitRight, MenuItem::SplitDown, MenuItem::NewTab]);
    assert_eq!(menu_items(&ctx), want);
    let all = MenuContext {
        has_note: true,
        groups_in_tab: 2,
        tabs: 2,
    };
    let items = menu_items(&all);
    assert_eq!(
        &items[items.len() - 2..],
        [MenuItem::CloseGroup, MenuItem::CloseTab]
    );
}

fn press(cx: &mut gpui_kit::VisualTestContext, key: &str) {
    cx.update(|window, cx| window.press(key, cx));
    cx.run_until_parked();
}

fn visible(cx: &mut gpui_kit::VisualTestContext, id: &'static str) -> bool {
    cx.update(|window, _| window.find(id).visible())
}

/// A menu in a bare window, with the events it emits collected.
fn open(
    ctx: MenuContext,
    cx: &mut TestAppContext,
) -> (
    gpui_kit::Entity<NoteMenu>,
    Rc<RefCell<Vec<NoteMenuEvent>>>,
    &mut gpui_kit::VisualTestContext,
) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        crate::keymap::init(
            cx,
            crate::keymap::Keymap::defaults(crate::keymap::Platform::LinuxWindows),
        )
    });
    let group = LayoutTree::default_layout(DrawerView::Files).active_group();
    let (menu, cx) = cx.add_window_view(|_, cx| {
        NoteMenu::new(
            group,
            Some(VaultPath::note_path_from("a")),
            ctx,
            WeakEntity::new_invalid(),
            cx,
        )
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|window, cx| {
        cx.subscribe(&menu, move |_, event: &NoteMenuEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
        let focus = menu.focus_handle(cx);
        window.focus(&focus, cx);
    });
    (menu, events, cx)
}

#[gpui_kit::test]
fn the_note_actions_keep_their_letters(cx: &mut TestAppContext) {
    let ctx = MenuContext {
        has_note: true,
        groups_in_tab: 1,
        tabs: 1,
    };
    let (_, events, cx) = open(ctx, cx);
    assert!(visible(cx, NOTE_MENU_ID));
    for (key, item) in ['p', 'r', 'm', 'd'].into_iter().zip(NOTE_ITEMS) {
        press(cx, &key.to_string());
        assert_eq!(events.borrow().last(), Some(&NoteMenuEvent::Picked(item)));
    }
    assert_eq!(events.borrow().len(), 4);
}

#[gpui_kit::test]
fn a_hidden_letter_picks_nothing_and_arrows_enter_escape_work(cx: &mut TestAppContext) {
    let ctx = MenuContext {
        has_note: false,
        groups_in_tab: 1,
        tabs: 1,
    };
    let (_, events, cx) = open(ctx, cx);
    press(cx, "d");
    assert!(events.borrow().is_empty());
    press(cx, "down");
    press(cx, "enter");
    assert_eq!(
        events.borrow().last(),
        Some(&NoteMenuEvent::Picked(MenuItem::SplitDown))
    );
    press(cx, "escape");
    assert_eq!(events.borrow().last(), Some(&NoteMenuEvent::Dismissed));
    assert!(visible(cx, menu_item_element_id(MenuItem::NewTab)));
}
