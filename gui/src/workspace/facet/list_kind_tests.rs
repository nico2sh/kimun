//! Tests for the names and facet kinds of the facet view's lists.
//!
//! Each list kind names itself in one place. These tests check that the
//! chips, the list names and the group labels all use that name, and that
//! the open and close rules of a list follow its facet kinds.

use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use kimun_core::note::FacetKind;

use super::{ListKind, MORE_LIST_ID, list_group_id, menu_id};
use crate::workspace::facet_tests::{click, label, list_open, open, tag};

/// A note with two tags, one link and one heading.
const NOTE: &str = "# Head\n\n#a #b [[x]]\n";

/// The accessible name of the open list.
fn list_name(cx: &mut gpui_kit::VisualTestContext) -> Option<String> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .try_find(MORE_LIST_ID)
            .and_then(|e| e.label().map(str::to_string))
    })
}

#[gpui_kit::test]
fn each_list_kind_has_one_name_used_by_chips_lists_and_groups(cx: &mut TestAppContext) {
    // The labels stay as they were.
    let names: Vec<_> = ListKind::chip_order().iter().map(|k| k.name()).collect();
    assert_eq!(names, ["Tags", "Links", "Sections"]);
    assert_eq!(ListKind::All.name(), "Find");
    let (_view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    // Each chip starts with its list's name, in chip order.
    for kind in ListKind::chip_order() {
        let chip = label(cx, menu_id(kind)).expect("the chip is drawn");
        assert!(chip.starts_with(&format!("{} ", kind.name())), "{chip}");
        // The list the chip opens has the same name.
        click(cx, menu_id(kind));
        assert_eq!(list_name(cx).as_deref(), Some(kind.name()));
        click(cx, menu_id(kind));
    }
    // The all-kinds list labels each group with the name of that kind's list.
    click(cx, menu_id(ListKind::All));
    assert_eq!(list_name(cx).as_deref(), Some(ListKind::All.name()));
    for kind in ListKind::chip_order() {
        let [facet_kind] = kind.facet_kinds() else {
            panic!("a chip list holds one facet kind");
        };
        assert_eq!(
            label(cx, list_group_id(*facet_kind)).as_deref(),
            Some(kind.name())
        );
    }
}

#[gpui_kit::test]
fn list_holds_and_list_drawable_agree_with_the_facet_kinds_of_each_list(cx: &mut TestAppContext) {
    // Tags and a heading, no link. The tag `a` is picked.
    let (view, cx, _events, _sub) = open(cx, "# Head\n\n#a\n", vec![tag("a")]);
    assert_eq!(
        ListKind::All.facet_kinds(),
        [FacetKind::Tag, FacetKind::NoteLink, FacetKind::Heading]
    );
    for kind in ListKind::chip_order() {
        assert_eq!(kind.facet_kinds().len(), 1, "{kind:?}");
    }
    view.read_with(cx, |view, _| {
        let entries = &view.members.merged.entries;
        let picks = view.bar.picks();
        for kind in ListKind::chip_order().into_iter().chain([ListKind::All]) {
            let held =
                |e: &&kimun_core::note::FacetEntry| kind.facet_kinds().contains(&e.facet.kind);
            let holds = entries.iter().any(|e| held(&e));
            let unpicked = entries
                .iter()
                .filter(held)
                .any(|e| !picks.contains(&e.facet));
            assert_eq!(view.list_holds(kind), holds, "{kind:?}");
            assert_eq!(view.list_drawable(kind), unpicked, "{kind:?}");
        }
        // The cases the note sets up.
        assert!(view.list_holds(ListKind::Tags));
        assert!(!view.list_drawable(ListKind::Tags), "its one tag is picked");
        assert!(!view.list_holds(ListKind::Links));
        assert!(view.list_drawable(ListKind::Sections));
        assert!(view.list_drawable(ListKind::All));
    });
}

#[gpui_kit::test]
fn list_changed_ignores_a_close_of_a_list_that_is_no_longer_open(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, NOTE, vec![]);
    click(cx, menu_id(ListKind::Tags));
    assert!(view.read_with(cx, |view, _| view.list_open(ListKind::Tags)));
    // The kit reports the close of the links popover, which is not open.
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.list_changed(ListKind::Links, false, window, cx)
        })
    });
    assert!(list_open(&view, cx), "the tags list stays open");
    assert!(view.read_with(cx, |view, _| view.list_open(ListKind::Tags)));
    // An open of the list already open changes nothing either.
    let input = view.read_with(cx, |view, _| view.more_input());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.list_changed(ListKind::Tags, true, window, cx)
        })
    });
    assert_eq!(view.read_with(cx, |view, _| view.more_input()), input);
    // A close of the open list closes it.
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.list_changed(ListKind::Tags, false, window, cx)
        })
    });
    assert!(!list_open(&view, cx));
}
