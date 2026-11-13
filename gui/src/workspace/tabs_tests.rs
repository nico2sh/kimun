//! The open rules over tabs and groups (ADR-0058): a plain open replaces
//! the active group's note, a new tab and a split each open into a new
//! group, and a note is open in at most one group across the window —
//! opening it again shows its tab and focuses its group.
//!
//! A real `Workspace` on a real temp vault, driven like the note tests
//! (`start_tests::drive`). Nothing here draws more than the active group:
//! the operations are state-level.
#![cfg(test)]

use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor_region::TabId;
use kimun_notes::logic::layout_tree::{Axis, Focus, GroupId, Leaf};
use tokio::runtime::Runtime;

use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

const NOTES: &[(&str, &str)] = &[
    ("a.md", "alpha"),
    ("b.md", "beta"),
    ("c.md", "gamma"),
    ("d.md", "delta"),
    ("e.md", "epsilon"),
];

/// A started window with this file's readers and opens (a wrapper, so
/// they cannot collide with the other test files' `Started` helpers).
struct Win(Started);

impl std::ops::Deref for Win {
    type Target = Started;

    fn deref(&self) -> &Started {
        &self.0
    }
}

/// A vault holding [`NOTES`], started on `a.md` and driven until it shows.
fn started(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Win) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(rt, cx, |cx| started.text(cx) == "alpha");
    (v, Win(started))
}

impl Win {
    fn active(&self, cx: &TestAppContext) -> GroupId {
        cx.read(|cx| self.workspace.read(cx).layout.active_group())
    }

    fn shown_tab(&self, cx: &TestAppContext) -> TabId {
        cx.read(|cx| self.workspace.read(cx).layout.region().shown())
    }

    fn tabs(&self, cx: &TestAppContext) -> Vec<TabId> {
        cx.read(|cx| self.workspace.read(cx).layout.region().tabs())
    }

    fn groups_of(&self, tab: TabId, cx: &TestAppContext) -> Vec<GroupId> {
        cx.read(|cx| self.workspace.read(cx).layout.region().groups(tab))
    }

    /// The note `group` shows.
    fn note_in(&self, group: GroupId, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .and_then(|h| h.note.clone())
        })
    }

    fn text_in(&self, group: GroupId, cx: &TestAppContext) -> String {
        cx.read(|cx| {
            let editor = self.workspace.read(cx).state(group).unwrap().editor.clone();
            editor.read(cx).editor.get_text()
        })
    }

    /// Whether `group` has nothing under way: no open asked for, none
    /// being read, no switch.
    fn idle(&self, group: GroupId, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .is_some_and(|h| {
                    !h.is_switching() && !h.is_open_wanted() && !h.session.is_open_pending()
                })
        })
    }

    fn all_idle(&self, cx: &TestAppContext) -> bool {
        let groups = cx.read(|cx| self.workspace.read(cx).layout.region().all_groups());
        groups.into_iter().all(|g| self.idle(g, cx))
    }

    /// The groups whose state holds `note` (shown, wanted or opening).
    fn holders(&self, note: &VaultPath, cx: &TestAppContext) -> Vec<GroupId> {
        cx.read(|cx| {
            let w = self.workspace.read(cx);
            w.layout
                .region()
                .all_groups()
                .into_iter()
                .filter(|g| {
                    w.state(*g)
                        .and_then(|s| s.notes.as_ref())
                        .is_some_and(|h| h.holds(note))
                })
                .collect()
        })
    }

    /// The groups of the region and the states held, the same set.
    fn assert_states_in_step(&self, cx: &TestAppContext) {
        cx.read(|cx| {
            let w = self.workspace.read(cx);
            let mut groups = w.layout.region().all_groups();
            groups.sort_by_key(|g| format!("{g:?}"));
            let mut states: Vec<GroupId> = w.groups.keys().copied().collect();
            states.sort_by_key(|g| format!("{g:?}"));
            assert_eq!(groups, states, "one state per group of the region");
        });
    }

    fn editor_focused(&self, group: GroupId, cx: &mut TestAppContext) -> bool {
        let workspace = self.workspace.clone();
        cx.update_window(self.handle, |_, window, cx| {
            let editor = workspace.read(cx).state(group).unwrap().editor.clone();
            editor.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap()
    }

    fn plain(&self, note: &str, cx: &mut TestAppContext) {
        let note = path(note);
        cx.update(|cx| self.workspace.update(cx, |w, cx| w.open_note(note, cx)));
    }

    fn new_tab(&self, note: &str, cx: &mut TestAppContext) {
        let note = path(note);
        cx.update(|cx| {
            self.workspace
                .update(cx, |w, cx| w.open_note_in_new_tab(note, cx))
        });
    }

    fn split(&self, note: &str, cx: &mut TestAppContext) {
        let note = path(note);
        cx.update(|cx| {
            self.workspace
                .update(cx, |w, cx| w.open_note_in_split(note, Axis::Horizontal, cx))
        });
    }
}

/// A plain open replaces the note of the active group: no tab, no group
/// is made.
#[gpui_kit::test]
fn plain_open_replaces_the_active_groups_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let group = started.active(cx);
    let tab = started.shown_tab(cx);

    started.plain("b.md", cx);
    drive(&rt, cx, |cx| {
        started.note_in(group, cx) == Some(path("b.md"))
    });

    assert_eq!(started.active(cx), group);
    assert_eq!(started.tabs(cx), vec![tab]);
    assert_eq!(started.groups_of(tab, cx), vec![group]);
    assert_eq!(started.text_in(group, cx), "beta");
    assert_eq!(started.locate(&path("a.md"), cx), None);
    assert_eq!(started.locate(&path("b.md"), cx), Some(group));
    started.assert_states_in_step(cx);
}

/// Opening a note another group holds shows that group's tab and gives
/// its editor the keyboard; nothing is opened again.
#[gpui_kit::test]
fn opening_an_open_note_shows_its_tab_and_focuses_its_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    assert_ne!(started.shown_tab(cx), first_tab);
    // The keyboard leaves the editor for the rail.
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, _| assert!(w.layout.focus(Focus::Rail)))
    });

    started.plain("a.md", cx);
    // Nothing was asked of the first group: it was shown, not reopened.
    assert!(started.idle(first, cx));
    drive(&rt, cx, |cx| started.editor_focused(first, cx));

    assert_eq!(started.active(cx), first);
    assert_eq!(started.shown_tab(cx), first_tab);
    cx.read(|cx| {
        assert_eq!(
            started.workspace.read(cx).layout.focused(),
            Focus::Leaf(Leaf::Editor)
        )
    });
    assert_eq!(started.tabs(cx).len(), 2);
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));
    assert_eq!(started.note_in(second, cx), Some(path("b.md")));
    assert_eq!(started.text(cx), "alpha");
    // The Footer follows the active group.
    cx.read(|cx| assert_eq!(started.workspace.read(cx).status.note, Some(path("a.md"))));
    started.assert_states_in_step(cx);
}

/// A new tab comes after the last one with one group holding the note;
/// that group is active and takes the keyboard.
#[gpui_kit::test]
fn open_in_new_tab_creates_a_tab_with_that_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);

    started.new_tab("b.md", cx);
    let group = started.active(cx);
    // The new group starts empty: the note is on its way, not yet shown.
    assert_eq!(started.note_in(group, cx), None);
    assert_eq!(started.locate(&path("b.md"), cx), Some(group));
    drive(&rt, cx, |cx| {
        started.note_in(group, cx) == Some(path("b.md"))
    });

    let tabs = started.tabs(cx);
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0], first_tab);
    assert_eq!(started.shown_tab(cx), tabs[1]);
    assert_eq!(started.groups_of(tabs[1], cx), vec![group]);
    assert_ne!(group, first);
    assert_eq!(started.text_in(group, cx), "beta");
    assert_eq!(started.text(cx), "beta");
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));
    assert_eq!(started.text_in(first, cx), "alpha");
    drive(&rt, cx, |cx| started.editor_focused(group, cx));
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert_eq!(w.layout.focused(), Focus::Leaf(Leaf::Editor));
        assert_eq!(w.status.note, Some(path("b.md")));
    });
    started.assert_states_in_step(cx);
}

/// A new tab for a note already open anywhere makes no tab: the note's
/// group is shown and focused.
#[gpui_kit::test]
fn open_in_new_tab_of_an_already_open_note_focuses_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);

    // The active group's own note: nothing is made.
    started.new_tab("a.md", cx);
    assert_eq!(started.tabs(cx), vec![first_tab]);
    assert_eq!(started.active(cx), first);
    assert!(started.idle(first, cx));

    // A note in another tab: that tab shows.
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    let second_tab = started.shown_tab(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    started.plain("a.md", cx);
    assert_eq!(started.shown_tab(cx), first_tab);

    started.new_tab("b.md", cx);
    assert!(started.idle(second, cx));
    assert_eq!(started.tabs(cx), vec![first_tab, second_tab]);
    assert_eq!(started.shown_tab(cx), second_tab);
    assert_eq!(started.active(cx), second);
    assert_eq!(started.note_in(second, cx), Some(path("b.md")));
    drive(&rt, cx, |cx| started.editor_focused(second, cx));
    started.assert_states_in_step(cx);
}

/// A split adds a group right after the active one, in the same tab,
/// holding the note; it becomes the active group.
#[gpui_kit::test]
fn open_in_split_adds_a_group_beside_the_active_one(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    let tab = started.shown_tab(cx);

    started.split("b.md", cx);
    let group = started.active(cx);
    drive(&rt, cx, |cx| {
        started.note_in(group, cx) == Some(path("b.md"))
    });

    assert_ne!(group, first);
    assert_eq!(started.tabs(cx), vec![tab]);
    assert_eq!(started.groups_of(tab, cx), vec![first, group]);
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));
    assert_eq!(started.text_in(group, cx), "beta");
    // Side by side: the new group is the right half.
    cx.read(|cx| {
        let regions = started.workspace.read(cx).layout.region().regions();
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].0, first);
        assert_eq!(regions[1].0, group);
        assert_eq!((regions[0].1.x(), regions[0].1.width()), (0.0, 0.5));
        assert_eq!((regions[1].1.x(), regions[1].1.width()), (0.5, 0.5));
    });

    // Splitting for a note already open makes no group.
    started.split("a.md", cx);
    assert_eq!(started.active(cx), first);
    assert_eq!(started.groups_of(tab, cx), vec![first, group]);
    started.assert_states_in_step(cx);
}

/// Over a run of plain, new-tab and split opens with repeats, no note is
/// ever held by two groups, and the arrangement ends as the rules say.
#[gpui_kit::test]
fn a_note_is_open_in_at_most_one_group(cx: &mut TestAppContext) {
    #[derive(Clone, Copy)]
    enum Op {
        Plain,
        Tab,
        Split,
    }
    use Op::*;

    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let ops = [
        (Tab, "b.md"),
        (Plain, "c.md"),
        (Split, "a.md"),
        (Split, "b.md"),
        (Plain, "c.md"),
        (Tab, "d.md"),
        (Plain, "a.md"),
        (Plain, "b.md"),
        (Plain, "e.md"),
        (Split, "b.md"),
        (Tab, "e.md"),
        (Plain, "e.md"),
        (Plain, "d.md"),
    ];
    let check = |cx: &TestAppContext, step: usize| {
        for (note, _) in NOTES {
            let holders = started.holders(&path(note), cx);
            assert!(
                holders.len() <= 1,
                "step {step}: {note} is held by {holders:?}"
            );
        }
        started.assert_states_in_step(cx);
    };
    for (step, (op, note)) in ops.into_iter().enumerate() {
        match op {
            Plain => started.plain(note, cx),
            Tab => started.new_tab(note, cx),
            Split => started.split(note, cx),
        }
        check(cx, step);
        drive(&rt, cx, |cx| started.all_idle(cx));
        check(cx, step);
    }

    let tabs = started.tabs(cx);
    let notes: Vec<Vec<Option<VaultPath>>> = tabs
        .iter()
        .map(|t| {
            started
                .groups_of(*t, cx)
                .into_iter()
                .map(|g| started.note_in(g, cx))
                .collect()
        })
        .collect();
    let some = |p: &str| Some(path(p));
    assert_eq!(
        notes,
        vec![
            vec![some("a.md"), some("e.md"), some("b.md")],
            vec![some("c.md")],
            vec![some("d.md")],
        ]
    );
    assert_eq!(started.shown_tab(cx), tabs[2]);
    assert_eq!(started.note_in(started.active(cx), cx), some("d.md"));
    assert_eq!(started.text(cx), "delta");
}

impl Win {
    fn locate(&self, note: &VaultPath, cx: &TestAppContext) -> Option<GroupId> {
        cx.read(|cx| self.workspace.read(cx).locate_note(note))
    }
}
