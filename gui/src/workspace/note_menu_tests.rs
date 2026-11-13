//! Tests for the **note menu** ([`super::note_menu`]): which items the
//! context shows, what the view's keys pick, and the menu in a real
//! window: where it opens, what a pick runs, and when it closes.
#![cfg(test)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout_tree::LayoutTree;
use kimun_notes::logic::nav::DrawerView;

use super::file_dialogs::FileOpKind;
use super::note_menu::{
    MenuContext, MenuItem, NOTE_MENU_ID, NoteMenu, NoteMenuEvent, divider_at, menu_item_element_id,
    menu_item_hint_id, menu_items,
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
        NoteMenu::new(group, Some(VaultPath::note_path_from("a")), ctx, cx)
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

// ── the view's keys, the divider, the hints ─────────────────────────────

/// A parent around the menu that records the keys reaching it.
struct KeyHost {
    menu: gpui_kit::Entity<NoteMenu>,
    keys: Rc<RefCell<Vec<String>>>,
}

impl gpui_kit::Render for KeyHost {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{InteractiveElement as _, ParentElement as _};
        let keys = self.keys.clone();
        gpui_kit::div()
            .on_key_down(move |event: &gpui_kit::KeyDownEvent, _, _| {
                let k = &event.keystroke;
                let ctrl = if k.modifiers.control { "ctrl-" } else { "" };
                keys.borrow_mut().push(format!("{ctrl}{}", k.key))
            })
            .child(self.menu.clone())
    }
}

#[gpui_kit::test]
fn keys_the_menu_does_not_use_reach_its_parent(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        crate::keymap::init(
            cx,
            crate::keymap::Keymap::defaults(crate::keymap::Platform::LinuxWindows),
        )
    });
    let group = LayoutTree::default_layout(DrawerView::Files).active_group();
    let keys = Rc::new(RefCell::new(Vec::new()));
    let seen = keys.clone();
    let (host, cx) = cx.add_window_view(|_, cx| {
        let menu = cx.new(|cx| {
            NoteMenu::new(
                group,
                Some(VaultPath::note_path_from("a")),
                MenuContext {
                    has_note: true,
                    groups_in_tab: 1,
                    tabs: 1,
                },
                cx,
            )
        });
        KeyHost { menu, keys: seen }
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|window, cx| {
        let menu = host.read(cx).menu.clone();
        cx.subscribe(&menu, move |_, event: &NoteMenuEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
        let focus = menu.focus_handle(cx);
        window.focus(&focus, cx);
    });
    press(cx, "x");
    press(cx, "ctrl-k");
    assert!(events.borrow().is_empty(), "neither picks anything");
    assert_eq!(*keys.borrow(), vec!["x".to_string(), "ctrl-k".to_string()]);
    // A letter the menu uses stops there.
    press(cx, "r");
    assert_eq!(keys.borrow().len(), 2);
}

#[gpui_kit::test]
fn shift_and_the_letter_still_picks(cx: &mut TestAppContext) {
    let ctx = MenuContext {
        has_note: true,
        groups_in_tab: 1,
        tabs: 1,
    };
    let (_, events, cx) = open(ctx, cx);
    press(cx, "shift-r");
    assert_eq!(
        events.borrow().last(),
        Some(&NoteMenuEvent::Picked(MenuItem::Note(FileOpKind::Rename)))
    );
}

#[test]
fn the_divider_parts_note_actions_from_layout_actions_only_when_both_show() {
    let with_note = menu_items(&MenuContext {
        has_note: true,
        groups_in_tab: 1,
        tabs: 1,
    });
    assert_eq!(divider_at(&with_note), Some(NOTE_ITEMS.len()));
    let empty = menu_items(&MenuContext {
        has_note: false,
        groups_in_tab: 2,
        tabs: 2,
    });
    assert_eq!(divider_at(&empty), None);
}

#[gpui_kit::test]
fn a_layout_row_shows_its_chord(cx: &mut TestAppContext) {
    let ctx = MenuContext {
        has_note: true,
        groups_in_tab: 2,
        tabs: 2,
    };
    let (_, _, cx) = open(ctx, cx);
    let hint = cx.update(|window, _| {
        window
            .find(menu_item_hint_id(MenuItem::SplitRight))
            .label()
            .map(str::to_string)
    });
    let want = cx.update(|_, cx| {
        crate::keymap::chord_label(
            cx,
            &kimun_notes::logic::actions::ActionShortcuts::SplitSlotRight,
        )
        .map(|c| c.to_string())
    });
    assert!(want.is_some());
    assert_eq!(hint, want);
}

// ── the menu in the workspace ───────────────────────────────────────────

mod in_workspace {
    use std::time::Duration;

    use gpui_kit::MouseButton;
    use gpui_kit::test::{ClickOptions, TestWindowExt as _};
    use gpui_kit::{AppContext as _, Bounds, Focusable as _, Pixels, TestAppContext, point, px};
    use kimun_core::nfs::VaultPath;
    use kimun_notes::logic::layout_tree::{Axis, GroupId};
    use tokio::runtime::Runtime;

    use super::super::drop_zones::GroupDrag;
    use super::super::file_dialogs::FileOpKind;
    use super::super::file_ops::{FileDialogView, FileOpRequest};
    use super::super::group_header::{
        CORNER_DIM_OPACITY, CORNER_MENU_ID, GROUP_ID, GROUP_MENU_ID, corner_opacity, group_element,
    };
    use super::super::modal::ModalKind;
    use super::super::note_menu::{
        MenuItem, NOTE_MENU_ID, NOTE_MENU_LAYER_ID, menu_item_element_id,
    };
    use super::super::route::NO_NOTE;
    use super::super::start_tests::{Started, drive, open_started, runtime, vault_start};
    use crate::startup::FirstNote;
    use crate::test_vault::{TestVault, test_vault};

    const NOTES: &[(&str, &str)] = &[("a.md", "alpha"), ("b.md", "beta")];
    const SPLIT_RIGHT: &str = "alt-shift-r";
    const CLOSE: &str = "alt-shift-w";

    fn path(p: &str) -> VaultPath {
        VaultPath::new(p)
    }

    fn start(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
        let v = rt.block_on(test_vault(NOTES, &[]));
        let started = open_started(
            cx,
            vault_start(&v, FirstNote::Note(path("a.md"))),
            v.settings.clone(),
        );
        drive(rt, cx, |cx| started.text(cx) == "alpha");
        (v, started)
    }

    fn idle(s: &Started, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let w = s.workspace.read(cx);
            !w.note_held()
                && w.layout.region().all_groups().into_iter().all(|g| {
                    w.state(g).and_then(|s| s.notes.as_ref()).is_some_and(|h| {
                        !h.is_switching() && !h.is_open_wanted() && !h.session.is_open_pending()
                    })
                })
        })
    }

    fn groups(s: &Started, cx: &TestAppContext) -> Vec<GroupId> {
        cx.read(|cx| {
            let region = s.workspace.read(cx).layout.region();
            region.groups(region.shown())
        })
    }

    fn active(s: &Started, cx: &TestAppContext) -> GroupId {
        cx.read(|cx| s.workspace.read(cx).layout.active_group())
    }

    fn note_in(s: &Started, group: GroupId, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .and_then(|h| h.note.clone())
        })
    }

    /// Two groups, `a.md` left and `b.md` right, the left one active.
    fn two_groups(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started, [GroupId; 2]) {
        let (v, s) = start(cx, rt);
        cx.update(|cx| {
            s.workspace.update(cx, |w, cx| {
                w.open_note_in_split(path("b.md"), Axis::Horizontal, cx)
            })
        });
        drive(rt, cx, |cx| {
            let g = groups(&s, cx);
            g.len() == 2 && note_in(&s, g[1], cx) == Some(path("b.md")) && idle(&s, cx)
        });
        let g = groups(&s, cx);
        cx.update(|cx| s.workspace.update(cx, |w, cx| w.focus_group(g[0], cx)));
        cx.run_until_parked();
        assert_eq!(active(&s, cx), g[0]);
        (v, s, [g[0], g[1]])
    }

    fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn click(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn settle(s: &Started, cx: &mut TestAppContext) {
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(s.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    /// The menu's bounds, while it is drawn.
    fn menu_bounds(s: &Started, cx: &mut TestAppContext) -> Option<Bounds<Pixels>> {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(NOTE_MENU_ID)
                .filter(|e| e.visible())
                .map(|e| e.bounds())
        })
        .unwrap()
    }

    fn menu_open(s: &Started, cx: &mut TestAppContext) -> bool {
        let open = cx.read(|cx| s.workspace.read(cx).note_menu.is_some());
        assert_eq!(open, menu_bounds(s, cx).is_some(), "drawn while open");
        open
    }

    /// The group the open menu acts on.
    fn menu_group(s: &Started, cx: &TestAppContext) -> Option<GroupId> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .note_menu
                .as_ref()
                .map(|m| m.view.read(cx).group())
        })
    }

    fn menu_focused(s: &Started, cx: &mut TestAppContext) -> bool {
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            let Some(menu) = workspace
                .read(cx)
                .note_menu
                .as_ref()
                .map(|m| m.view.clone())
            else {
                return false;
            };
            menu.focus_handle(cx).is_focused(window)
        })
        .unwrap()
    }

    fn editor_focused(s: &Started, group: GroupId, cx: &mut TestAppContext) -> bool {
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            let editor = workspace.read(cx).state(group).unwrap().editor.clone();
            editor.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap()
    }

    fn rename_target(s: &Started, cx: &TestAppContext) -> VaultPath {
        cx.read(|cx| match s.workspace.read(cx).file_dialog.clone() {
            Some(FileDialogView::Rename(view)) => view.upgrade().unwrap().read(cx).target().clone(),
            _ => panic!("no rename dialog"),
        })
    }

    fn shows(s: &Started, item: MenuItem, cx: &mut TestAppContext) -> bool {
        s.shows(menu_item_element_id(item), cx)
    }

    #[gpui_kit::test]
    fn right_click_in_a_group_opens_the_menu_for_that_group(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s, [left, right]) = two_groups(cx, &rt);
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .within(group_element(GROUP_ID, 1))
                .right_click("editor", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(menu_open(&s, cx));
        assert_eq!(menu_group(&s, cx), Some(right));
        assert_eq!(active(&s, cx), right, "the clicked group is active");
        assert!(menu_focused(&s, cx), "the menu has the keyboard");
        press(&s, "r", cx);
        settle(&s, cx);
        assert!(!menu_open(&s, cx));
        assert_eq!(s.shown_modal(cx), Some(ModalKind::Rename));
        assert_eq!(
            rename_target(&s, cx),
            path("b.md"),
            "the clicked group's note"
        );
        press(&s, "escape", cx);
        settle(&s, cx);
        assert!(editor_focused(&s, right, cx));
        assert_ne!(active(&s, cx), left);
    }

    #[gpui_kit::test]
    fn right_click_with_a_selection_copies_and_opens_no_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        press(&s, "shift-right", cx);
        press(&s, "shift-right", cx);
        cx.update_window(s.handle, |_, window, cx| window.right_click("editor", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(!menu_open(&s, cx), "a selection: no menu");
        // The editor's copy ran: it reports on the OS clipboard (no GPUI
        // clipboard in between), "copied" or why not.
        let flash = s.flash(cx).unwrap_or_default();
        assert!(
            flash == "copied" || flash.starts_with("clipboard:"),
            "{flash}"
        );
        // Without a selection it opens.
        press(&s, "right", cx);
        cx.update_window(s.handle, |_, window, cx| window.right_click("editor", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(menu_open(&s, cx));
        assert!(shows(&s, MenuItem::Note(FileOpKind::Properties), cx));
    }

    #[gpui_kit::test]
    fn f2_opens_the_menu_for_the_active_group_and_an_empty_group_gets_layout_actions_only(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        press(&s, "f2", cx);
        assert!(menu_open(&s, cx));
        assert_eq!(menu_group(&s, cx), Some(active(&s, cx)));
        assert!(menu_focused(&s, cx));
        assert!(shows(&s, MenuItem::Note(FileOpKind::Properties), cx));
        assert!(shows(&s, MenuItem::Note(FileOpKind::Rename), cx));
        press(&s, "escape", cx);
        assert!(!menu_open(&s, cx));

        // An empty group: layout actions only, and no "no note" flash.
        press(&s, SPLIT_RIGHT, cx);
        let empty = active(&s, cx);
        assert_eq!(note_in(&s, empty, cx), None);
        press(&s, "f2", cx);
        assert!(menu_open(&s, cx));
        assert_eq!(menu_group(&s, cx), Some(empty));
        assert!(!shows(&s, MenuItem::Note(FileOpKind::Rename), cx));
        assert!(shows(&s, MenuItem::SplitRight, cx));
        assert!(shows(&s, MenuItem::CloseGroup, cx));
        assert_ne!(s.flash(cx).as_deref(), Some(NO_NOTE));
        // Under the active group's top-right corner.
        let menu = menu_bounds(&s, cx).unwrap();
        let group = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window.find(group_element(GROUP_ID, 1)).bounds()
            })
            .unwrap();
        // The menu's frame (its padding and border) sits inside the corner.
        let inset = |a: Pixels, b: Pixels| a >= px(0.) && a < px(8.) && (a - b).abs() < px(1.);
        assert!(
            inset(group.right() - menu.right(), menu.top() - group.top()),
            "{menu:?} {group:?}"
        );
    }

    #[gpui_kit::test]
    fn picking_split_right_splits_the_menu_group(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s, [left, right]) = two_groups(cx, &rt);
        // The menu of the right group, opened while the left one is active.
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.open_note_menu(right, point(px(400.), px(200.)), window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(menu_group(&s, cx), Some(right));
        click(&s, menu_item_element_id(MenuItem::SplitRight), cx);
        assert!(!menu_open(&s, cx));
        let g = groups(&s, cx);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0], left);
        assert_eq!(g[1], right, "the new group comes after the menu's group");
        assert_eq!(active(&s, cx), g[2]);
    }

    #[gpui_kit::test]
    fn picking_new_tab_adds_a_tab(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        press(&s, "f2", cx);
        click(&s, menu_item_element_id(MenuItem::NewTab), cx);
        assert!(!menu_open(&s, cx));
        let tabs = cx.read(|cx| s.workspace.read(cx).layout.region().tabs().len());
        assert_eq!(tabs, 2);
        // Two tabs: the menu offers Close tab, which closes the shown one.
        press(&s, "f2", cx);
        assert!(shows(&s, MenuItem::CloseTab, cx));
        click(&s, menu_item_element_id(MenuItem::CloseTab), cx);
        drive(&rt, cx, |cx| {
            cx.read(|cx| s.workspace.read(cx).layout.region().tabs().len()) == 1
        });
    }

    #[gpui_kit::test]
    fn picking_close_group_goes_through_the_flush_path(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s, [left, right]) = two_groups(cx, &rt);
        // Unsaved text in the right group, whose menu closes it.
        let editor = cx.read(|cx| s.workspace.read(cx).state(right).unwrap().editor.clone());
        cx.update(|cx| {
            editor.update(cx, |view, cx| {
                view.editor.set_text(String::new());
                view.editor.insert_at_cursor("unsaved beta");
                cx.notify();
            })
        });
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.open_note_menu(right, point(px(400.), px(200.)), window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
        // Unsaved until the close writes it.
        assert!(cx.read(|cx| editor.read(cx).editor.is_dirty()));
        let before = rt.block_on(v.vault.get_note_text(&path("b.md"))).ok();
        assert_eq!(before.as_deref(), Some("beta"));
        click(&s, menu_item_element_id(MenuItem::CloseGroup), cx);
        drive(&rt, cx, |cx| groups(&s, cx).len() == 1 && idle(&s, cx));
        assert_eq!(groups(&s, cx), vec![left]);
        let on_disk = rt.block_on(v.vault.get_note_text(&path("b.md"))).ok();
        assert_eq!(on_disk.as_deref(), Some("unsaved beta"), "nothing lost");
    }

    #[gpui_kit::test]
    fn escape_and_a_click_outside_dismiss_and_return_the_keyboard(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let group = active(&s, cx);
        press(&s, "f2", cx);
        assert!(menu_focused(&s, cx));
        press(&s, "escape", cx);
        assert!(!menu_open(&s, cx));
        assert!(editor_focused(&s, group, cx));

        press(&s, "f2", cx);
        assert!(menu_open(&s, cx));
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click_at(NOTE_MENU_LAYER_ID, point(px(2.), px(2.)), cx)
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!menu_open(&s, cx));
        assert!(editor_focused(&s, group, cx));
        assert_eq!(s.text(cx), "alpha", "nothing typed or changed");
    }

    #[gpui_kit::test]
    fn the_menu_closes_when_its_note_is_renamed_or_its_group_closes(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s, [_, right]) = two_groups(cx, &rt);
        // Renamed from elsewhere (FILES' dialog) while the menu is open.
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.open_note_menu(right, point(px(400.), px(200.)), window, cx)
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert!(menu_open(&s, cx));
        let request = FileOpRequest::Rename {
            from: path("b.md"),
            to: path("c.md"),
        };
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.request_file_op(request, window, cx))
        })
        .unwrap();
        drive(&rt, cx, |cx| {
            cx.read(|cx| s.workspace.read(cx).file_task.is_none()) && idle(&s, cx)
        });
        assert_eq!(note_in(&s, right, cx), Some(path("c.md")));
        assert!(!menu_open(&s, cx), "a renamed note's menu closes");

        // The group closes by its chord while its menu is open.
        cx.update(|cx| s.workspace.update(cx, |w, cx| w.focus_group(right, cx)));
        cx.run_until_parked();
        press(&s, "f2", cx);
        assert_eq!(menu_group(&s, cx), Some(right));
        press(&s, CLOSE, cx);
        drive(&rt, cx, |cx| groups(&s, cx).len() == 1 && idle(&s, cx));
        assert!(!menu_open(&s, cx), "a closed group's menu closes");
    }

    #[gpui_kit::test]
    fn the_menu_stays_inside_the_window_at_the_bottom_right_corner(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let group = active(&s, cx);
        let workspace = s.workspace.clone();
        let size = cx
            .update_window(s.handle, |_, window, cx| {
                let size = window.viewport_size();
                let corner = point(size.width - px(1.), size.height - px(1.));
                workspace.update(cx, |w, cx| w.open_note_menu(group, corner, window, cx));
                size
            })
            .unwrap();
        cx.run_until_parked();
        let menu = menu_bounds(&s, cx).expect("drawn");
        assert!(menu.left() >= px(0.) && menu.top() >= px(0.), "{menu:?}");
        assert!(menu.right() <= size.width, "{menu:?} in {size:?}");
        assert!(menu.bottom() <= size.height, "{menu:?} in {size:?}");
    }

    fn tab_count(s: &Started, cx: &TestAppContext) -> usize {
        cx.read(|cx| s.workspace.read(cx).layout.region().tabs().len())
    }

    #[gpui_kit::test]
    fn the_right_click_menu_opens_at_the_pointer(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let offset = point(px(40.), px(30.));
        let pointer = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                let origin = window.find("editor").bounds().origin;
                window.click_with_options(
                    "editor",
                    ClickOptions::new()
                        .with_offset(offset)
                        .with_button(MouseButton::Right),
                    cx,
                );
                origin + offset
            })
            .unwrap();
        cx.run_until_parked();
        let menu = menu_bounds(&s, cx).expect("open");
        // The menu's frame (its padding and border) holds it in from the
        // point.
        let dx = menu.left() - pointer.x;
        let dy = menu.top() - pointer.y;
        assert!(dx >= px(0.) && dx < px(8.), "{menu:?} at {pointer:?}");
        assert!(dy >= px(0.) && dy < px(8.), "{menu:?} at {pointer:?}");
    }

    #[gpui_kit::test]
    fn a_tab_switch_closes_the_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        cx.update(|cx| {
            s.workspace
                .update(cx, |w, cx| w.open_note_in_new_tab(path("b.md"), cx))
        });
        drive(&rt, cx, |cx| tab_count(&s, cx) == 2 && idle(&s, cx));
        press(&s, "f2", cx);
        assert!(menu_open(&s, cx));
        press(&s, "ctrl-tab", cx);
        assert!(
            !menu_open(&s, cx),
            "the menu does not float over a hidden tab"
        );
    }

    #[gpui_kit::test]
    fn an_open_into_the_menu_group_closes_the_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let group = active(&s, cx);
        // The journal chord opens today's entry in the group.
        press(&s, "f2", cx);
        assert!(menu_open(&s, cx));
        press(&s, "ctrl-j", cx);
        assert!(!menu_open(&s, cx), "a chord closes the menu");
        drive(&rt, cx, |cx| {
            note_in(&s, group, cx).is_some_and(|n| n != path("a.md")) && idle(&s, cx)
        });
        // An open that is no chord (FILES, a link) lands in the group.
        press(&s, "f2", cx);
        assert!(menu_open(&s, cx));
        cx.update(|cx| {
            s.workspace
                .update(cx, |w, cx| w.open_note(path("b.md"), cx))
        });
        drive(&rt, cx, |cx| {
            note_in(&s, group, cx) == Some(path("b.md")) && idle(&s, cx)
        });
        assert!(!menu_open(&s, cx), "the menu names the note it showed");
    }

    #[gpui_kit::test]
    fn preferences_leave_no_menu_behind(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let group = active(&s, cx);
        press(&s, "f2", cx);
        assert!(menu_open(&s, cx));
        press(&s, "f4", cx);
        assert!(cx.read(|cx| s.workspace.read(cx).preferences.is_some()));
        assert!(!menu_open(&s, cx));
        // Preferences close: the keyboard goes to the note, not to a menu.
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.drop_preferences();
                w.apply_focus(window, cx);
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!menu_open(&s, cx));
        assert!(editor_focused(&s, group, cx));
    }

    #[gpui_kit::test]
    fn a_drag_starting_closes_the_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s, [_, right]) = two_groups(cx, &rt);
        press(&s, "f2", cx);
        assert!(menu_open(&s, cx));
        // The ghost constructor of a group header's drag: GPUI calls it as
        // the drag starts.
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            let ghost = workspace.update(cx, |w, cx| w.group_drag_ghost("b".into(), cx));
            let _chip = ghost(
                &GroupDrag { group: right },
                point(px(0.), px(0.)),
                window,
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!menu_open(&s, cx));
    }

    fn has(s: &Started, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn a_group_header_has_a_menu_button_that_opens_the_menu_for_its_group(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s, [left, right]) = two_groups(cx, &rt);
        assert!(has(&s, group_element(GROUP_MENU_ID, 0), cx));
        assert!(has(&s, group_element(GROUP_MENU_ID, 1), cx));
        // The left group is active; each button acts on its own group.
        click(&s, group_element(GROUP_MENU_ID, 1), cx);
        assert!(menu_open(&s, cx));
        assert_eq!(menu_group(&s, cx), Some(right));
        press(&s, "escape", cx);
        click(&s, group_element(GROUP_MENU_ID, 0), cx);
        assert_eq!(menu_group(&s, cx), Some(left));
    }

    #[gpui_kit::test]
    fn a_lone_group_shows_a_corner_button_and_a_header_hides_it(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        assert!(has(&s, CORNER_MENU_ID, cx));
        assert!(!has(&s, group_element(GROUP_MENU_ID, 0), cx));
        cx.update(|cx| {
            s.workspace.update(cx, |w, cx| {
                w.open_note_in_split(path("b.md"), Axis::Horizontal, cx)
            })
        });
        drive(&rt, cx, |cx| groups(&s, cx).len() == 2 && idle(&s, cx));
        assert!(!has(&s, CORNER_MENU_ID, cx));
        assert!(has(&s, group_element(GROUP_MENU_ID, 0), cx));
        assert!(has(&s, group_element(GROUP_MENU_ID, 1), cx));
    }

    #[gpui_kit::test]
    fn the_corner_button_opens_the_menu_for_the_lone_group(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        click(&s, CORNER_MENU_ID, cx);
        assert!(menu_open(&s, cx));
        assert_eq!(menu_group(&s, cx), Some(groups(&s, cx)[0]));
    }

    #[gpui_kit::test]
    fn the_button_click_does_not_activate_a_drag_or_close(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s, [left, right]) = two_groups(cx, &rt);
        assert_eq!(active(&s, cx), left);
        click(&s, group_element(GROUP_MENU_ID, 1), cx);
        assert!(menu_open(&s, cx));
        assert_eq!(menu_group(&s, cx), Some(right), "the button's own group");
        assert_eq!(active(&s, cx), right, "opening made the group active");
        assert_eq!(groups(&s, cx).len(), 2, "nothing closed");
        assert!(cx.read(|cx| s.workspace.read(cx).dragged_group(cx).is_none()));
    }

    #[test]
    fn the_corner_button_is_dim_until_the_pointer_is_near() {
        assert!(corner_opacity(false) > 0.0 && corner_opacity(false) < corner_opacity(true));
        assert_eq!(corner_opacity(true), 1.0);
        assert_eq!(corner_opacity(false), CORNER_DIM_OPACITY);
    }
}
