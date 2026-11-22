//! Tests for the **Link menu** ([`super::link_menu`]): which items each kind
//! of target shows, and the menu in a real window: what opens it, where the
//! keyboard goes, and what closes it.
#![cfg(test)]

use super::link_menu::{LinkItem, LinkMenuContext, link_menu_items};
use kimun_notes::logic::follow::TargetClass;

fn items(class: Option<TargetClass>, facet: bool) -> Vec<LinkItem> {
    link_menu_items(&LinkMenuContext { class, facet })
}

#[test]
fn link_menu_items_for_a_note_link() {
    assert_eq!(
        items(Some(TargetClass::Note), false),
        vec![
            LinkItem::Open,
            LinkItem::OpenNewTab,
            LinkItem::OpenSplitRight,
            LinkItem::OpenSplitDown,
            LinkItem::CopyLink,
        ]
    );
}

#[test]
fn link_menu_items_for_an_attachment() {
    assert_eq!(
        items(Some(TargetClass::Attachment), false),
        vec![
            LinkItem::Open,
            LinkItem::OpenNewTab,
            LinkItem::OpenInDefaultApp,
            LinkItem::CopyLink,
        ]
    );
}

#[test]
fn link_menu_items_for_a_web_link() {
    assert_eq!(
        items(Some(TargetClass::Web), false),
        vec![LinkItem::OpenInBrowser, LinkItem::CopyLink]
    );
}

/// A link that cannot be followed offers only what can act: the copy.
#[test]
fn link_menu_items_for_an_invalid_link() {
    assert_eq!(
        items(Some(TargetClass::Invalid), false),
        vec![LinkItem::CopyLink]
    );
    assert_eq!(
        items(Some(TargetClass::Invalid), true),
        vec![LinkItem::CopyLink]
    );
}

/// The context classifies a link from the note that holds it, by the same
/// routing step a follow uses.
#[test]
fn the_context_classifies_from_the_note_that_holds_the_link() {
    use kimun_core::nfs::VaultPath;
    use kimun_core::note::scan::LinkSpanKind;
    use kimun_notes::logic::editor::claim::FollowTarget;
    let from = VaultPath::new("journal/today.md");
    let class = |target: &str, kind| {
        let target = FollowTarget::Link {
            target: target.to_string(),
            kind,
        };
        LinkMenuContext::of(&target, &from, false).class
    };
    assert_eq!(
        class("mailto:a@b.c", LinkSpanKind::Markdown),
        Some(TargetClass::Invalid)
    );
    assert_eq!(
        class("pic.png", LinkSpanKind::WikiLink),
        Some(TargetClass::Attachment)
    );
    assert_eq!(
        class("doc.pdf", LinkSpanKind::WikiLink),
        Some(TargetClass::Invalid)
    );
    assert_eq!(
        class("anton", LinkSpanKind::WikiLink),
        Some(TargetClass::Note)
    );
    assert_eq!(
        LinkMenuContext::of(&FollowTarget::Label("rust".into()), &from, false).class,
        None
    );
}

#[test]
fn link_menu_items_for_a_label() {
    assert_eq!(
        items(None, false),
        vec![LinkItem::SearchLabel, LinkItem::CopyLabel]
    );
}

#[test]
fn a_facet_menu_has_no_plain_open() {
    assert_eq!(
        items(Some(TargetClass::Note), true),
        vec![
            LinkItem::OpenNewTab,
            LinkItem::OpenSplitRight,
            LinkItem::OpenSplitDown,
            LinkItem::CopyLink,
        ]
    );
    assert_eq!(
        items(Some(TargetClass::Attachment), true),
        vec![
            LinkItem::OpenNewTab,
            LinkItem::OpenInDefaultApp,
            LinkItem::CopyLink,
        ]
    );
    // Nothing else had a plain `Open` to drop.
    assert_eq!(
        items(Some(TargetClass::Web), true),
        items(Some(TargetClass::Web), false)
    );
    assert_eq!(items(None, true), items(None, false));
}

#[test]
fn link_menu_labels_read_in_order() {
    let labels = |class| {
        items(class, false)
            .into_iter()
            .map(LinkItem::label)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(Some(TargetClass::Note)),
        vec![
            "Open",
            "Open in new tab",
            "Open in split right",
            "Open in split down",
            "Copy link",
        ]
    );
    assert_eq!(labels(None), vec!["Search for label", "Copy label"]);
}

// ── the menu in the workspace ───────────────────────────────────────────

mod in_workspace {
    use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
    use gpui_kit::{
        AppContext as _, Entity, Focusable as _, MouseButton, Pixels, Point, TestAppContext, point,
        px,
    };
    use kimun_core::nfs::VaultPath;
    use kimun_core::note::scan::LinkSpanKind;
    use kimun_notes::logic::editor::claim::FollowTarget;
    use kimun_notes::logic::layout_tree::GroupId;
    use tokio::runtime::Runtime;

    use super::super::facet_tests::in_a_group::{facet, open_facet};
    use super::super::link_menu::{LINK_MENU_ID, LINK_MENU_LAYER_ID, LinkMenuPick};
    use super::super::note_menu::NOTE_MENU_ID;
    use super::super::start_tests::{Started, drive, open_started, runtime, vault_start};
    use crate::editor_view::EditorView;
    use crate::startup::FirstNote;
    use crate::test_vault::{TestVault, test_vault};

    const A: &str = "plain text\n\nsee [[beta]] here";
    const NOTES: &[(&str, &str)] = &[("a.md", A), ("beta.md", "beta")];

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
        drive(rt, cx, |cx| started.text(cx) == A && !started.indexing(cx));
        (v, started)
    }

    fn active(s: &Started, cx: &TestAppContext) -> GroupId {
        cx.read(|cx| s.workspace.read(cx).layout.active_group())
    }

    fn active_editor(s: &Started, cx: &TestAppContext) -> Entity<EditorView> {
        cx.read(|cx| s.workspace.read(cx).active_state().editor.clone())
    }

    /// The window position of the first cell of `needle` in `editor`'s last
    /// frame, after a fresh one.
    fn cell_of(
        s: &Started,
        editor: &Entity<EditorView>,
        needle: &str,
        cx: &mut TestAppContext,
    ) -> Option<Point<Pixels>> {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            let painted = editor.read(cx).painted.clone()?;
            let (row, col) = painted
                .row_paint
                .iter()
                .enumerate()
                .find_map(|(row, paint)| {
                    let text = paint.text();
                    text.find(needle)
                        .map(|at| (row, text[..at].chars().count() as u16))
                })?;
            let c = painted.cells;
            Some(point(
                painted.origin.x + px(c.x_of(col) + c.width() / 2.0),
                painted.origin.y + px(c.y_of(row as u16) + c.height() / 2.0),
            ))
        })
        .unwrap()
    }

    fn right_click_at(s: &Started, at: Point<Pixels>, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            window.pointer_down(at, MouseButton::Right, cx);
            window.pointer_up(at, MouseButton::Right, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn drawn(s: &Started, id: &'static str, cx: &mut TestAppContext) -> bool {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    fn link_menu_open(s: &Started, cx: &mut TestAppContext) -> bool {
        let open = cx.read(|cx| s.workspace.read(cx).link_menu.is_some());
        assert_eq!(open, drawn(s, LINK_MENU_ID, cx), "drawn while open");
        open
    }

    fn note_menu_open(s: &Started, cx: &mut TestAppContext) -> bool {
        let open = cx.read(|cx| s.workspace.read(cx).note_menu.is_some());
        assert_eq!(open, drawn(s, NOTE_MENU_ID, cx), "drawn while open");
        open
    }

    fn link_menu_focused(s: &Started, cx: &mut TestAppContext) -> bool {
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace
                .read(cx)
                .link_menu
                .as_ref()
                .map(|m| m.view.clone())
                .is_some_and(|menu| menu.focus_handle(cx).is_focused(window))
        })
        .unwrap()
    }

    fn editor_focused(s: &Started, editor: &Entity<EditorView>, cx: &mut TestAppContext) -> bool {
        let editor = editor.clone();
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            editor.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap()
    }

    /// The open menu's group, source note, target and facet flag.
    fn link_menu_state(
        s: &Started,
        cx: &TestAppContext,
    ) -> Option<(GroupId, VaultPath, FollowTarget, bool)> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .link_menu
                .as_ref()
                .map(|m| (m.group, m.from.clone(), m.target.clone(), m.facet))
        })
    }

    fn beta() -> FollowTarget {
        FollowTarget::Link {
            target: "beta".to_string(),
            kind: LinkSpanKind::WikiLink,
        }
    }

    /// What a pick of the menu on `target`, in `group` and the note `from`,
    /// acts on.
    fn pick_of(group: GroupId, from: &str, target: FollowTarget, facet: bool) -> LinkMenuPick {
        LinkMenuPick {
            group,
            from: path(from),
            target,
            facet,
        }
    }

    /// Right-click the link in the active editor.
    fn right_click_link(s: &Started, cx: &mut TestAppContext) {
        let editor = active_editor(s, cx);
        let at = cell_of(s, &editor, "beta", cx).expect("the link is painted");
        right_click_at(s, at, cx);
    }

    #[gpui_kit::test]
    fn right_click_on_a_link_opens_the_link_menu_and_not_the_note_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let group = active(&s, cx);
        right_click_link(&s, cx);
        assert!(link_menu_open(&s, cx));
        assert!(!note_menu_open(&s, cx));
        assert!(link_menu_focused(&s, cx), "the menu has the keyboard");
        assert_eq!(
            link_menu_state(&s, cx),
            Some((group, path("a.md"), beta(), false))
        );
        // A note link in an editor group: the open items show.
        assert!(drawn(&s, "link-menu-open", cx));
        assert!(drawn(&s, "link-menu-open-new-tab", cx));
        assert!(drawn(&s, "link-menu-copy-link", cx));
        assert!(!drawn(&s, "link-menu-open-in-browser", cx));
    }

    #[gpui_kit::test]
    fn right_click_on_plain_text_opens_the_note_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let editor = active_editor(&s, cx);
        let at = cell_of(&s, &editor, "plain", cx).expect("painted");
        right_click_at(&s, at, cx);
        assert!(note_menu_open(&s, cx));
        assert!(!link_menu_open(&s, cx));
    }

    #[gpui_kit::test]
    fn escape_closes_the_link_menu_and_returns_the_keyboard(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let editor = active_editor(&s, cx);
        right_click_link(&s, cx);
        assert!(link_menu_open(&s, cx));
        press(&s, "escape", cx);
        assert!(!link_menu_open(&s, cx));
        assert!(editor_focused(&s, &editor, cx));
        assert_eq!(s.text(cx), A, "nothing typed or changed");
    }

    #[gpui_kit::test]
    fn a_press_outside_closes_the_link_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let editor = active_editor(&s, cx);
        right_click_link(&s, cx);
        assert!(link_menu_open(&s, cx));
        assert!(drawn(&s, LINK_MENU_LAYER_ID, cx));
        let outside = point(px(4.), px(4.));
        cx.update_window(s.handle, |_, window, cx| {
            window.pointer_down(outside, MouseButton::Left, cx);
            window.pointer_up(outside, MouseButton::Left, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!link_menu_open(&s, cx));
        assert!(editor_focused(&s, &editor, cx));
    }

    #[gpui_kit::test]
    fn no_link_menu_opens_while_a_modal_is_open(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let group = active(&s, cx);
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_palette(window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        assert!(s.shown_modal(cx).is_some());
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.open_link_menu(
                    pick_of(group, "a.md", beta(), false),
                    point(px(100.), px(100.)),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!link_menu_open(&s, cx));
    }

    #[gpui_kit::test]
    fn opening_the_note_menu_closes_the_link_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let editor = active_editor(&s, cx);
        right_click_link(&s, cx);
        assert!(link_menu_open(&s, cx));
        press(&s, "f2", cx);
        assert!(note_menu_open(&s, cx));
        assert!(!link_menu_open(&s, cx));
        // The keyboard goes back to the editor, not to the gone Link menu.
        press(&s, "escape", cx);
        assert!(!note_menu_open(&s, cx));
        assert!(editor_focused(&s, &editor, cx));

        // And the reverse: the Link menu takes the Note menu's place.
        press(&s, "f2", cx);
        assert!(note_menu_open(&s, cx));
        let group = active(&s, cx);
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.open_link_menu(
                    pick_of(group, "a.md", beta(), false),
                    point(px(100.), px(100.)),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        cx.run_until_parked();
        assert!(link_menu_open(&s, cx));
        assert!(!note_menu_open(&s, cx));
        press(&s, "escape", cx);
        assert!(!link_menu_open(&s, cx));
        assert!(editor_focused(&s, &editor, cx));
    }

    #[gpui_kit::test]
    fn a_right_click_on_a_link_in_a_facet_view_opens_the_link_menu(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", vec![], cx);
        let view = facet(&s, group, cx).expect("a facet view");
        let chunks: Vec<_> = cx.read(|cx| view.read(cx).chunks().to_vec());
        let at = chunks
            .iter()
            .find_map(|chunk| cell_of(&s, chunk, "beta", cx))
            .expect("a piece paints the link");
        right_click_at(&s, at, cx);
        assert!(link_menu_open(&s, cx));
        assert!(!note_menu_open(&s, cx));
        assert_eq!(
            link_menu_state(&s, cx),
            Some((group, path("a.md"), beta(), true)),
            "from the piece's source note, as a facet menu"
        );
        assert!(!drawn(&s, "link-menu-open", cx), "no plain Open");
        assert!(drawn(&s, "link-menu-open-new-tab", cx));
    }

    // ── what a pick does ─────────────────────────────────────────────────

    mod picks {
        use std::cell::RefCell;
        use std::ffi::OsString;
        use std::rc::Rc;

        use kimun_notes::logic::layout_tree::Axis as TreeAxis;
        use kimun_notes::settings::session::{AxisRecord, GroupTreeRecord};

        use super::super::super::link_menu::{LABEL_COPIED, LINK_COPIED, LinkItem};
        use super::super::super::modal::ModalKind;
        use super::super::super::note_menu::MENU_NOTE_CHANGED;
        use super::super::super::tab_strip::TAB_CLOSING;
        use super::*;

        /// A vault of `notes`, started on `first`, driven until its text
        /// shows and the index is built (a follow by name needs it).
        fn start_on(
            cx: &mut TestAppContext,
            rt: &Runtime,
            notes: &[(&str, &str)],
            first: &str,
        ) -> (TestVault, Started) {
            let v = rt.block_on(test_vault(notes, &[]));
            let started = open_started(
                cx,
                vault_start(&v, FirstNote::Note(path(first))),
                v.settings.clone(),
            );
            let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
            drive(rt, cx, |cx| {
                started.text(cx) == want && !started.indexing(cx)
            });
            (v, started)
        }

        fn run(s: &Started, pick: LinkMenuPick, item: LinkItem, cx: &mut TestAppContext) {
            let workspace = s.workspace.clone();
            cx.update_window(s.handle, |_, window, cx| {
                workspace.update(cx, |w, cx| w.run_link_menu_pick(pick, item, window, cx))
            })
            .unwrap();
            cx.run_until_parked();
        }

        fn tabs(s: &Started, cx: &TestAppContext) -> usize {
            cx.read(|cx| s.workspace.read(cx).layout.region().tabs().len())
        }

        /// The groups of the shown tab.
        fn groups(s: &Started, cx: &TestAppContext) -> Vec<GroupId> {
            cx.read(|cx| {
                let region = s.workspace.read(cx).layout.region();
                region.groups(region.shown())
            })
        }

        fn note_in(s: &Started, g: GroupId, cx: &TestAppContext) -> Option<VaultPath> {
            cx.read(|cx| {
                s.workspace
                    .read(cx)
                    .host_in(g)
                    .and_then(|host| host.note.clone())
            })
        }

        /// Whether the active group holds the note at `p`.
        fn on(s: &Started, p: &str, cx: &TestAppContext) -> bool {
            note_in(s, active(s, cx), cx).is_some_and(|n| n.is_like(&path(p)))
        }

        fn flash(s: &Started, cx: &TestAppContext) -> Option<String> {
            s.flash(cx)
        }

        /// Hand URLs and files to a recorder instead of the system.
        fn record_opens(s: &Started, cx: &mut TestAppContext) -> Rc<RefCell<Vec<OsString>>> {
            let opened = Rc::new(RefCell::new(Vec::new()));
            let record = opened.clone();
            cx.update(|cx| {
                s.workspace.update(cx, |workspace, _| {
                    workspace.opener = Rc::new(move |what| {
                        record.borrow_mut().push(what.to_owned());
                        Ok(())
                    });
                })
            });
            opened
        }

        fn link(target: &str, kind: LinkSpanKind) -> FollowTarget {
            FollowTarget::Link {
                target: target.to_string(),
                kind,
            }
        }

        /// The split axis of the shown tab's top split, if it has one.
        fn split_axis(s: &Started, cx: &TestAppContext) -> Option<AxisRecord> {
            let record = cx.read(|cx| s.workspace.read(cx).session_record(cx));
            match &record.tabs[record.shown].tree {
                GroupTreeRecord::Split { axis, .. } => Some(*axis),
                _ => None,
            }
        }

        #[gpui_kit::test]
        fn picking_open_in_new_tab_opens_the_note_in_a_new_tab(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start(cx, &rt);
            let first = active(&s, cx);
            // Through the menu, as the user picks: the second row.
            right_click_link(&s, cx);
            assert!(link_menu_open(&s, cx));
            press(&s, "down", cx);
            press(&s, "enter", cx);
            assert!(!link_menu_open(&s, cx));
            drive(&rt, cx, |cx| tabs(&s, cx) == 2 && on(&s, "beta.md", cx));
            assert_ne!(active(&s, cx), first);
            assert_eq!(note_in(&s, first, cx), Some(path("a.md")));
        }

        #[gpui_kit::test]
        fn picking_open_in_new_tab_for_an_open_note_focuses_its_tab(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start(cx, &rt);
            let first = active(&s, cx);
            cx.update(|cx| {
                s.workspace
                    .update(cx, |w, cx| w.open_note_in_new_tab(path("beta.md"), cx))
            });
            drive(&rt, cx, |cx| on(&s, "beta.md", cx));
            let second = active(&s, cx);
            cx.update(|cx| s.workspace.update(cx, |w, cx| w.focus_group(first, cx)));
            cx.run_until_parked();
            run(
                &s,
                pick_of(first, "a.md", beta(), false),
                LinkItem::OpenNewTab,
                cx,
            );
            drive(&rt, cx, |cx| active(&s, cx) == second);
            assert_eq!(tabs(&s, cx), 2, "no third tab");
        }

        #[gpui_kit::test]
        fn picking_split_right_and_split_down_open_the_note_in_the_matching_split(
            cx: &mut TestAppContext,
        ) {
            let rt = runtime();
            let _guard = rt.enter();
            for (item, axis) in [
                (LinkItem::OpenSplitRight, AxisRecord::Horizontal),
                (LinkItem::OpenSplitDown, AxisRecord::Vertical),
            ] {
                let (_v, s) = start(cx, &rt);
                let first = active(&s, cx);
                run(&s, pick_of(first, "a.md", beta(), false), item, cx);
                drive(&rt, cx, |cx| {
                    groups(&s, cx).len() == 2 && on(&s, "beta.md", cx)
                });
                assert_eq!(split_axis(&s, cx), Some(axis), "{item:?}");
                assert_eq!(note_in(&s, first, cx), Some(path("a.md")));
                assert_eq!(tabs(&s, cx), 1);
            }
        }

        #[gpui_kit::test]
        fn picking_open_on_a_missing_link_shows_the_create_dialog(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start_on(cx, &rt, &[("a.md", "[[nobody]]")], "a.md");
            let group = active(&s, cx);
            run(
                &s,
                pick_of(group, "a.md", link("nobody", LinkSpanKind::WikiLink), false),
                LinkItem::Open,
                cx,
            );
            drive(&rt, cx, |cx| {
                s.shown_modal(cx) == Some(ModalKind::CreateNote)
            });
        }

        #[gpui_kit::test]
        fn picking_open_on_an_ambiguous_link_shows_the_chooser(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start_on(
                cx,
                &rt,
                &[
                    ("a.md", "[[dup]]"),
                    ("dup.md", "Dup one"),
                    ("sub/dup.md", "Dup two"),
                ],
                "a.md",
            );
            let group = active(&s, cx);
            run(
                &s,
                pick_of(group, "a.md", link("dup", LinkSpanKind::WikiLink), false),
                LinkItem::Open,
                cx,
            );
            drive(&rt, cx, |cx| {
                s.shown_modal(cx) == Some(ModalKind::LinkChooser)
            });
        }

        #[gpui_kit::test]
        fn picking_open_in_default_app_hands_the_attachment_to_the_opener(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (v, s) = start_on(
                cx,
                &rt,
                &[("sub/a.md", "[doc](../files/doc.pdf)")],
                "sub/a.md",
            );
            rt.block_on(v.vault.save_attachment(&path("files/doc.pdf"), b"%PDF\x00"))
                .unwrap();
            let opened = record_opens(&s, cx);
            let group = active(&s, cx);
            run(
                &s,
                pick_of(
                    group,
                    "sub/a.md",
                    link("../files/doc.pdf", LinkSpanKind::Markdown),
                    false,
                ),
                LinkItem::OpenInDefaultApp,
                cx,
            );
            drive(&rt, cx, |_| !opened.borrow().is_empty());
            let opened = opened.borrow();
            assert_eq!(opened.len(), 1);
            assert!(
                std::path::Path::new(&opened[0]).ends_with("files/doc.pdf"),
                "{:?}",
                opened[0]
            );
            assert!(on(&s, "sub/a.md", cx), "the editor stays");
        }

        #[gpui_kit::test]
        fn picking_open_in_browser_hands_the_url_to_the_opener(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let url = "https://example.com/page";
            let (_v, s) = start_on(
                cx,
                &rt,
                &[("a.md", "[site](https://example.com/page)")],
                "a.md",
            );
            let opened = record_opens(&s, cx);
            let group = active(&s, cx);
            run(
                &s,
                pick_of(group, "a.md", link(url, LinkSpanKind::Markdown), false),
                LinkItem::OpenInBrowser,
                cx,
            );
            drive(&rt, cx, |_| !opened.borrow().is_empty());
            assert_eq!(*opened.borrow(), vec![OsString::from(url)]);
            assert!(on(&s, "a.md", cx), "the editor stays");
            assert_eq!(tabs(&s, cx), 1);
        }

        #[gpui_kit::test]
        fn picking_copy_link_puts_the_target_on_the_clipboard(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start(cx, &rt);
            let group = active(&s, cx);
            run(
                &s,
                pick_of(group, "a.md", beta(), false),
                LinkItem::CopyLink,
                cx,
            );
            let copied = cx.read_from_clipboard().and_then(|item| item.text());
            assert_eq!(copied.as_deref(), Some("beta"));
            assert_eq!(flash(&s, cx).as_deref(), Some(LINK_COPIED));
            assert!(on(&s, "a.md", cx), "nothing opens");
            assert_eq!(tabs(&s, cx), 1);

            // A markdown link's target, as written: not resolved.
            run(
                &s,
                pick_of(
                    group,
                    "a.md",
                    link("../files/doc.pdf", LinkSpanKind::Markdown),
                    false,
                ),
                LinkItem::CopyLink,
                cx,
            );
            let copied = cx.read_from_clipboard().and_then(|item| item.text());
            assert_eq!(copied.as_deref(), Some("../files/doc.pdf"));
            assert!(on(&s, "a.md", cx), "nothing opens");
        }

        #[gpui_kit::test]
        fn picking_copy_label_puts_the_label_on_the_clipboard(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start(cx, &rt);
            let group = active(&s, cx);
            run(
                &s,
                pick_of(group, "a.md", FollowTarget::Label("rust".into()), false),
                LinkItem::CopyLabel,
                cx,
            );
            let copied = cx.read_from_clipboard().and_then(|item| item.text());
            // As written in the note: with its `#`.
            assert_eq!(copied.as_deref(), Some("#rust"));
            assert_eq!(flash(&s, cx).as_deref(), Some(LABEL_COPIED));
        }

        #[gpui_kit::test]
        fn picking_search_for_label_opens_the_search_on_it(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start(cx, &rt);
            let opened = record_opens(&s, cx);
            let group = active(&s, cx);
            run(
                &s,
                pick_of(group, "a.md", FollowTarget::Label("rust".into()), false),
                LinkItem::SearchLabel,
                cx,
            );
            drive(&rt, cx, |cx| s.shown_modal(cx) == Some(ModalKind::Search));
            assert!(opened.borrow().is_empty());
            assert!(on(&s, "a.md", cx));
        }

        #[gpui_kit::test]
        fn a_pick_after_the_group_shows_another_note_flashes_and_does_nothing(
            cx: &mut TestAppContext,
        ) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start_on(
                cx,
                &rt,
                &[("a.md", "[[beta]]"), ("beta.md", "beta"), ("c.md", "gamma")],
                "a.md",
            );
            let group = active(&s, cx);
            // The group moves on to c.md before the pick runs.
            cx.update(|cx| {
                s.workspace
                    .update(cx, |w, cx| w.open_note(path("c.md"), cx))
            });
            drive(&rt, cx, |cx| on(&s, "c.md", cx) && s.text(cx) == "gamma");
            run(
                &s,
                pick_of(group, "a.md", beta(), false),
                LinkItem::OpenNewTab,
                cx,
            );
            assert_eq!(flash(&s, cx).as_deref(), Some(MENU_NOTE_CHANGED));
            // Give a follow the time it would take: nothing opens.
            for _ in 0..20 {
                rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(5)));
                cx.run_until_parked();
            }
            assert_eq!(tabs(&s, cx), 1);
            assert!(on(&s, "c.md", cx));
        }

        #[gpui_kit::test]
        fn a_split_pick_while_the_tab_is_closing_flashes_tab_closing(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s) = start_on(
                cx,
                &rt,
                &[("a.md", "alpha"), ("beta.md", "beta"), ("c.md", "[[beta]]")],
                "a.md",
            );
            cx.update(|cx| {
                s.workspace
                    .update(cx, |w, cx| w.open_note_in_new_tab(path("c.md"), cx))
            });
            drive(&rt, cx, |cx| on(&s, "c.md", cx) && s.text(cx) == "[[beta]]");
            let second = active(&s, cx);
            // Unsaved text: the tab's close writes it first, and waits.
            let editor = active_editor(&s, cx);
            cx.update(|cx| {
                editor.update(cx, |view, cx| {
                    view.editor.insert_at_cursor("x");
                    cx.notify();
                })
            });
            press(&s, "ctrl-w", cx);
            assert!(
                cx.read(|cx| s.workspace.read(cx).note_held()),
                "the close's flush runs"
            );
            run(
                &s,
                pick_of(second, "c.md", beta(), false),
                LinkItem::OpenSplitRight,
                cx,
            );
            assert_eq!(flash(&s, cx).as_deref(), Some(TAB_CLOSING));
            assert_eq!(groups(&s, cx), vec![second]);
            drive(&rt, cx, |cx| tabs(&s, cx) == 1);
            assert_eq!(groups(&s, cx).len(), 1, "no split anywhere");
            assert!(on(&s, "a.md", cx));
        }

        /// Two groups in one tab: a facet view of `a.md` first, then an
        /// editor on `c.md`, which is active.
        fn facet_beside_an_editor(
            cx: &mut TestAppContext,
            rt: &Runtime,
        ) -> (TestVault, Started, GroupId, GroupId) {
            let (v, s) = start_on(
                cx,
                rt,
                &[("a.md", A), ("beta.md", "beta"), ("c.md", "gamma")],
                "a.md",
            );
            let facet_group = active(&s, cx);
            open_facet(&s, rt, facet_group, "a.md", vec![], cx);
            cx.update(|cx| {
                s.workspace.update(cx, |w, cx| {
                    w.open_note_in_split(path("c.md"), TreeAxis::Horizontal, cx)
                })
            });
            drive(rt, cx, |cx| groups(&s, cx).len() == 2 && on(&s, "c.md", cx));
            let editor_group = active(&s, cx);
            assert_ne!(editor_group, facet_group);
            assert!(facet(&s, facet_group, cx).is_some());
            (v, s, facet_group, editor_group)
        }

        #[gpui_kit::test]
        fn a_facet_pick_in_a_group_that_is_not_active_focuses_the_facet_group_first(
            cx: &mut TestAppContext,
        ) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s, facet_group, editor_group) = facet_beside_an_editor(cx, &rt);
            run(
                &s,
                pick_of(facet_group, "a.md", beta(), true),
                LinkItem::OpenSplitRight,
                cx,
            );
            drive(&rt, cx, |cx| {
                groups(&s, cx).len() == 3 && on(&s, "beta.md", cx)
            });
            // The split is of the facet group, not of the editor that was
            // active: the new group sits right after the facet group.
            let new = active(&s, cx);
            assert_eq!(groups(&s, cx), vec![facet_group, new, editor_group]);
            assert!(facet(&s, facet_group, cx).is_some(), "the view stays");
            assert_eq!(note_in(&s, editor_group, cx), Some(path("c.md")));
            assert_eq!(tabs(&s, cx), 1);
        }

        #[gpui_kit::test]
        fn a_copy_pick_in_a_facet_view_leaves_the_active_group(cx: &mut TestAppContext) {
            let rt = runtime();
            let _guard = rt.enter();
            let (_v, s, facet_group, editor_group) = facet_beside_an_editor(cx, &rt);
            run(
                &s,
                pick_of(facet_group, "a.md", beta(), true),
                LinkItem::CopyLink,
                cx,
            );
            let copied = cx.read_from_clipboard().and_then(|item| item.text());
            assert_eq!(copied.as_deref(), Some("beta"));
            assert_eq!(flash(&s, cx).as_deref(), Some(LINK_COPIED));
            assert_eq!(active(&s, cx), editor_group, "copy moves no focus");
            assert_eq!(groups(&s, cx), vec![facet_group, editor_group]);
        }

        #[gpui_kit::test]
        fn a_facet_view_pick_of_open_in_new_tab_opens_the_note_from_the_chunks_source_note(
            cx: &mut TestAppContext,
        ) {
            let rt = runtime();
            let _guard = rt.enter();
            // A relative link resolves from the note it is written in: from
            // sub/c.md it leads to sub/beta.md, from a.md to beta.md.
            let (_v, s) = start_on(
                cx,
                &rt,
                &[
                    ("a.md", "alpha"),
                    ("beta.md", "root beta"),
                    ("sub/beta.md", "sub beta"),
                    ("sub/c.md", "#x see [bee](beta.md) here"),
                ],
                "a.md",
            );
            let group = active(&s, cx);
            cx.update(|cx| {
                s.workspace.update(cx, |w, cx| {
                    w.open_collection(group, "#x".into(), vec![], cx)
                })
            });
            drive(&rt, cx, |cx| {
                facet(&s, group, cx).is_some_and(|view| {
                    cx.read(|cx| {
                        view.read(cx)
                            .member_paths()
                            .iter()
                            .any(|m| m.is_like(&path("sub/c.md")))
                    })
                })
            });
            let view = facet(&s, group, cx).expect("a collection view");
            let chunks: Vec<_> = cx.read(|cx| view.read(cx).chunks().to_vec());
            let at = chunks
                .iter()
                .find_map(|chunk| cell_of(&s, chunk, "bee", cx))
                .expect("a piece paints the link");
            right_click_at(&s, at, cx);
            assert!(link_menu_open(&s, cx));
            let state = link_menu_state(&s, cx).expect("a menu");
            assert!(state.1.is_like(&path("sub/c.md")), "{:?}", state.1);
            assert!(state.3, "a facet menu");
            // The first row of a facet menu: Open in new tab.
            press(&s, "enter", cx);
            drive(&rt, cx, |cx| {
                tabs(&s, cx) == 2 && note_in(&s, active(&s, cx), cx).is_some()
            });
            assert!(
                on(&s, "sub/beta.md", cx),
                "{:?}",
                note_in(&s, active(&s, cx), cx)
            );
            assert!(facet(&s, group, cx).is_some(), "the view stays");
        }
    }
}
