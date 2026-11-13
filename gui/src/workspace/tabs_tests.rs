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
use kimun_notes::logic::message::AppMessage;
use tokio::runtime::Runtime;

use super::file_ops::FileOpRequest;
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
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

// ── every group shares the window's note lifecycle (Task 6b) ────────────

/// The note's text on disk (`None`: not there).
fn disk(rt: &Runtime, v: &TestVault, p: &str) -> Option<String> {
    rt.block_on(v.vault.get_note_text(&path(p))).ok()
}

fn exists(rt: &Runtime, v: &TestVault, p: &str) -> bool {
    rt.block_on(v.vault.exists(&path(p)))
}

/// Run tokio and GPUI in turn until `done`, or `limit` rounds: whether it
/// got there (a wait that may fail an assertion after it, not a panic).
fn settle(
    rt: &Runtime,
    cx: &mut TestAppContext,
    limit: usize,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) -> bool {
    for _ in 0..limit {
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(5)));
        cx.run_until_parked();
        if done(cx) {
            return true;
        }
    }
    false
}

/// A vault holding `notes`, its autosave tick at one second, started on
/// `first` and driven until it shows.
fn started_with(
    cx: &mut TestAppContext,
    rt: &Runtime,
    notes: &[(&str, &str)],
    first: &str,
) -> (TestVault, Win) {
    let v = rt.block_on(test_vault(notes, &[]));
    v.settings.write().unwrap().autosave_interval_secs = 1;
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path(first))),
        v.settings.clone(),
    );
    let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
    drive(rt, cx, |cx| started.text(cx) == want);
    (v, Win(started))
}

impl Win {
    /// Replace `group`'s text with `text`, as typing would: the buffer is
    /// dirty after it.
    fn set_text_in(&self, group: GroupId, text: &str, cx: &mut TestAppContext) {
        let editor = cx.read(|cx| self.workspace.read(cx).state(group).unwrap().editor.clone());
        cx.update(|cx| {
            editor.update(cx, |view, cx| {
                // Emptied (a load, clean), then typed into: dirty.
                view.editor.set_text(String::new());
                view.editor.insert_at_cursor(text);
                cx.notify();
            })
        });
        assert!(self.dirty_in(group, cx), "the text is unsaved");
    }

    fn dirty_in(&self, group: GroupId, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let editor = self.workspace.read(cx).state(group).unwrap().editor.clone();
            editor.read(cx).editor.is_dirty()
        })
    }

    /// The path `group`'s note session names.
    fn session_in(&self, group: GroupId, cx: &TestAppContext) -> VaultPath {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .unwrap()
                .session
                .path()
                .clone()
        })
    }

    fn running(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).file_task.is_some())
    }

    /// Ask for `request` as a dialog's confirm does.
    fn file_op(&self, request: FileOpRequest, cx: &mut TestAppContext) {
        let workspace = self.workspace.clone();
        cx.update_window(self.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.request_file_op(request, window, cx))
        })
        .unwrap();
    }

    /// [`Self::file_op`], driven until it and every open after it ended.
    fn run_file_op(&self, rt: &Runtime, request: FileOpRequest, cx: &mut TestAppContext) {
        self.file_op(request, cx);
        drive(rt, cx, |cx| !self.running(cx) && self.all_idle(cx));
    }

    fn close(&self, cx: &mut TestAppContext) -> bool {
        gpui_kit::VisualTestContext::from_window(self.handle, cx).simulate_close()
    }

    fn is_closed(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, _, _| ()).is_err()
    }
}

/// Let more than one autosave tick (1 s) pass on the runtime's real clock.
fn past_a_tick(rt: &Runtime, cx: &mut TestAppContext) {
    for _ in 0..30 {
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(50)));
        cx.run_until_parked();
    }
}

/// The tick writes every group's unsaved text, not only the active one's.
#[gpui_kit::test]
fn autosave_tick_saves_every_dirty_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    started.split("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    started.set_text_in(first, "alpha edited", cx);
    started.set_text_in(second, "beta edited", cx);

    send(&started, AppMessage::Autosave, cx);
    settle(&rt, cx, 400, |cx| {
        !started.dirty_in(first, cx) && !started.dirty_in(second, cx)
    });

    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha edited"));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta edited"));
    assert!(!started.dirty_in(first, cx));
    assert!(!started.dirty_in(second, cx));
    assert_eq!(started.active(cx), second);
}

/// A save's report marks the group that holds its note saved, whichever
/// group is active; a failed one is flashed and leaves it dirty.
#[gpui_kit::test]
fn autosave_completed_clears_the_dirty_state_of_the_group_that_saved(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    started.set_text_in(first, "alpha edited", cx);
    started.set_text_in(second, "beta edited", cx);
    let revision = cx.read(|cx| {
        let editor = started
            .workspace
            .read(cx)
            .state(first)
            .unwrap()
            .editor
            .clone();
        editor.read(cx).editor.content_revision()
    });

    send(
        &started,
        AppMessage::AutosaveCompleted {
            path: path("b.md"),
            saved_revision: None,
            title: None,
            error: Some("disk full".into()),
        },
        cx,
    );
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("Couldn't save b.md: disk full")
    );
    send(
        &started,
        AppMessage::AutosaveCompleted {
            path: path("a.md"),
            saved_revision: Some(revision),
            title: Some("alpha edited".into()),
            error: None,
        },
        cx,
    );

    assert!(
        !started.dirty_in(first, cx),
        "the group that saved is clean"
    );
    assert!(
        started.dirty_in(second, cx),
        "the active group still is not"
    );
    cx.read(|cx| assert!(started.workspace.read(cx).status.dirty));
    assert_eq!(started.active(cx), second);
}

/// Closing the window writes the unsaved text of every group first, the
/// hidden one included, and only then closes.
#[gpui_kit::test]
fn close_flushes_every_group_before_the_window_closes(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    started.split("c.md", cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    // Only groups the user is not looking at hold unsaved text.
    started.set_text_in(first, "alpha edited", cx);
    started.set_text_in(second, "beta edited", cx);

    assert!(!started.close(cx), "the close waits for the writes");
    drive(&rt, cx, |cx| started.is_closed(cx));

    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha edited"));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta edited"));
    assert_eq!(disk(&rt, &v, "c.md").as_deref(), Some("gamma"));
}

/// A vault switch writes every group's text to the old vault, then every
/// group is on the new one: a note session over it, the editor on the new
/// channel, completing from it, of the engine the settings name. The new
/// workspace's groups are its own (its session record's): the old ones go.
#[gpui_kit::test]
fn leave_vault_and_set_channel_reach_every_group(cx: &mut TestAppContext) {
    use super::reopen_tests::{BETA, open_on_a, pair, set_current};
    use kimun_notes::settings::EditorBackendSetting;
    use kimun_notes::settings::session::{AxisRecord, GroupTreeRecord, SessionRecord, TabRecord};

    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    // B left two groups side by side.
    let leaf = |note: &str| GroupTreeRecord::Leaf {
        note: Some(path(note)),
    };
    let b_record = SessionRecord {
        tabs: vec![TabRecord {
            tree: GroupTreeRecord::Split {
                axis: AxisRecord::Horizontal,
                weights: vec![0.5, 0.5],
                children: vec![leaf("bfriend.md"), leaf("beta.md")],
            },
            active: 1,
        }],
        shown: 0,
    };
    p.settings
        .read()
        .unwrap()
        .session_for("b")
        .save(&b_record)
        .unwrap();
    let started = Win(open_on_a(cx, &rt, &p, super::LayoutStore::Off));
    let first = started.active(cx);
    started.new_tab("afriend.md", cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    started.set_text_in(first, "alpha edited", cx);
    let old_tx = cx.read(|cx| started.workspace.read(cx).tx.clone().unwrap());

    set_current(&p.settings, "b");
    p.settings.write().unwrap().editor_backend = EditorBackendSetting::Vim;
    started.workspace.update(cx, |w, cx| w.reopen(cx));
    drive(&rt, cx, |cx| {
        !cx.read(|cx| started.workspace.read(cx).reopen_running())
            && !started.indexing(cx)
            && started.all_idle(cx)
            && started.text(cx) == BETA
    });

    assert_eq!(
        std::fs::read_to_string(p.a.join("alpha.md")).unwrap(),
        "alpha edited",
        "the hidden group's text went to the old vault first"
    );
    let groups = cx.read(|cx| started.workspace.read(cx).layout.region().all_groups());
    assert_eq!(groups.len(), 2, "B's own groups");
    assert_eq!(started.note_in(groups[0], cx), Some(path("bfriend.md")));
    assert_eq!(started.note_in(groups[1], cx), Some(path("beta.md")));
    assert!(
        started.holders(&path("afriend.md"), cx).is_empty(),
        "nothing of A is held"
    );
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        let vault = w.vault().unwrap();
        let tx = w.tx.clone().unwrap();
        assert!(!tx.same_channel(&old_tx), "a fresh channel");
        for group in groups {
            let state = w.state(group).unwrap();
            let host = state.notes.as_ref().expect("a note session");
            assert!(
                std::sync::Arc::ptr_eq(host.session.vault(), &vault),
                "{group:?} is on the new vault"
            );
            let editor = state.editor.read(cx);
            assert!(
                editor.reports_on(&tx),
                "{group:?} reports on the new channel"
            );
            assert!(!editor.reports_on(&old_tx));
            assert!(editor.completes(), "{group:?} completes from the vault");
            assert!(editor.editor.is_vim(), "{group:?} runs the engine set");
        }
    });
    started.assert_states_in_step(cx);
}

/// An open asked of a group while an operation held the notes waits; when
/// the operation ends it goes in that group even though another group is
/// active by then, so the group does not hold the note forever.
#[gpui_kit::test]
fn file_op_ended_resumes_a_parked_open_in_every_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);

    // An operation on another note holds every open until it ends.
    started.file_op(
        FileOpRequest::Rename {
            from: path("e.md"),
            to: path("f.md"),
        },
        cx,
    );
    assert!(started.running(cx));
    started.new_tab("c.md", cx);
    let second = started.active(cx);
    assert_ne!(second, first);
    assert!(
        cx.read(|cx| started
            .workspace
            .read(cx)
            .state(second)
            .and_then(|s| s.notes.as_ref())
            .is_some_and(|h| h.is_open_wanted())),
        "the open waits for the operation"
    );
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.focus_group(first, cx))
    });
    assert_eq!(started.active(cx), first);

    settle(&rt, cx, 1000, |cx| {
        !started.running(cx) && started.all_idle(cx)
    });

    assert!(exists(&rt, &v, "f.md"));
    assert_eq!(started.note_in(second, cx), Some(path("c.md")));
    assert_eq!(started.text_in(second, cx), "gamma");
    assert_eq!(started.holders(&path("c.md"), cx), vec![second]);
    assert!(started.idle(second, cx));
    assert_eq!(started.active(cx), first);
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));
}

/// Renaming a note open in a tab that is not shown: that group follows it
/// to its new name with its unsaved text written there, and the shown tab
/// is left as it was.
#[gpui_kit::test]
fn rename_updates_a_group_in_a_hidden_tab(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    let second_tab = started.shown_tab(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    started.set_text_in(first, "alpha edited", cx);

    started.run_file_op(
        &rt,
        FileOpRequest::Rename {
            from: path("a.md"),
            to: path("r.md"),
        },
        cx,
    );

    assert!(!exists(&rt, &v, "a.md"));
    assert_eq!(disk(&rt, &v, "r.md").as_deref(), Some("alpha edited"));
    assert_eq!(started.note_in(first, cx), Some(path("r.md")));
    assert_eq!(started.session_in(first, cx), path("r.md"));
    assert_eq!(started.text_in(first, cx), "alpha edited");
    assert!(
        !started.dirty_in(first, cx),
        "reloaded clean at the new name"
    );
    assert_eq!(started.locate(&path("r.md"), cx), Some(first));
    assert_eq!(started.locate(&path("a.md"), cx), None);
    // The shown tab is untouched.
    assert_eq!(started.tabs(cx), vec![first_tab, second_tab]);
    assert_eq!(started.shown_tab(cx), second_tab);
    assert_eq!(started.active(cx), second);
    assert_eq!(started.note_in(second, cx), Some(path("b.md")));
    cx.read(|cx| assert_eq!(started.workspace.read(cx).status.note, Some(path("b.md"))));

    // Later saves of the followed group land on the new name.
    started.set_text_in(first, "alpha again", cx);
    send(&started, AppMessage::Autosave, cx);
    drive(&rt, cx, |cx| !started.dirty_in(first, cx));
    assert_eq!(disk(&rt, &v, "r.md").as_deref(), Some("alpha again"));
    assert!(!exists(&rt, &v, "a.md"));
}

/// Moving a folder: every group holding a note below it follows, at any
/// depth, in any tab; a group holding a note elsewhere is left alone.
#[gpui_kit::test]
fn move_of_a_folder_updates_every_group_below_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let notes = [
        ("d/x.md", "ex"),
        ("d/sub/y.md", "why"),
        ("z/k.md", "kay"),
        ("top.md", "top"),
    ];
    let (v, started) = started_with(cx, &rt, &notes, "d/x.md");
    let first = started.active(cx);
    started.split("d/sub/y.md", cx);
    let second = started.active(cx);
    started.new_tab("top.md", cx);
    let third = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    started.set_text_in(second, "why edited", cx);

    started.run_file_op(
        &rt,
        FileOpRequest::Move {
            from: path("d"),
            to: path("z/d"),
        },
        cx,
    );

    assert!(!exists(&rt, &v, "d"));
    assert_eq!(disk(&rt, &v, "z/d/x.md").as_deref(), Some("ex"));
    assert_eq!(disk(&rt, &v, "z/d/sub/y.md").as_deref(), Some("why edited"));
    let is = |got: Option<VaultPath>, want: &str| got.is_some_and(|n| n.is_like(&path(want)));
    assert!(is(started.note_in(first, cx), "z/d/x.md"));
    assert!(is(started.note_in(second, cx), "z/d/sub/y.md"));
    assert!(
        started
            .session_in(second, cx)
            .is_like(&path("z/d/sub/y.md"))
    );
    assert_eq!(started.text_in(first, cx), "ex");
    assert_eq!(started.text_in(second, cx), "why edited");
    assert!(!started.dirty_in(second, cx));
    assert_eq!(started.note_in(third, cx), Some(path("top.md")));
    assert_eq!(started.active(cx), third);
    started.assert_states_in_step(cx);
}

/// Deleting a note turns its group into an empty group in the same place:
/// the split stays, nothing holds the note, no tick brings it back. The
/// active group's delete says so in its Panel.
#[gpui_kit::test]
fn delete_empties_the_group_in_place_and_keeps_the_split(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, NOTES, "a.md");
    let first = started.active(cx);
    let tab = started.shown_tab(cx);
    started.split("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));

    started.run_file_op(&rt, FileOpRequest::Delete(path("a.md")), cx);

    assert!(!exists(&rt, &v, "a.md"));
    assert_eq!(started.groups_of(tab, cx), vec![first, second]);
    assert_eq!(started.note_in(first, cx), None);
    assert_eq!(started.session_in(first, cx), VaultPath::root());
    assert_eq!(started.text_in(first, cx), "");
    assert!(!started.dirty_in(first, cx));
    assert_eq!(started.locate(&path("a.md"), cx), None);
    // The active group keeps its note and the Panel says nothing.
    assert_eq!(started.active(cx), second);
    assert_eq!(started.note_in(second, cx), Some(path("b.md")));
    assert_eq!(started.text(cx), "beta");
    assert_eq!(started.placeholder(cx), None);

    started.run_file_op(&rt, FileOpRequest::Delete(path("b.md")), cx);

    assert!(!exists(&rt, &v, "b.md"));
    assert_eq!(started.groups_of(tab, cx), vec![first, second]);
    assert_eq!(started.active(cx), second);
    assert_eq!(started.note_in(second, cx), None);
    assert_eq!(started.text_in(second, cx), "");
    assert_eq!(started.placeholder(cx).as_deref(), Some("b.md was deleted"));
    past_a_tick(&rt, cx);
    assert!(!exists(&rt, &v, "a.md"), "no tick recreates it");
    assert!(!exists(&rt, &v, "b.md"), "no tick recreates it");
    started.assert_states_in_step(cx);
}

/// Unsaved text in a hidden group is written before its note is deleted,
/// as for the open note: a write that fails refuses the delete, and the
/// group keeps its note and its text.
#[cfg(unix)]
#[gpui_kit::test]
fn delete_with_unsaved_text_writes_before_it(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_with(cx, &rt, NOTES, "a.md");
    let first = started.active(cx);
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    started.set_text_in(first, "alpha edited", cx);
    let file = v.workspace.as_path().join("a.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();

    started.run_file_op(&rt, FileOpRequest::Delete(path("a.md")), cx);
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save a.md:") && flash.ends_with("Nothing was deleted."),
        "{flash}"
    );
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha"));
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));
    assert_eq!(started.text_in(first, cx), "alpha edited");
    assert!(started.dirty_in(first, cx), "the text is still unsaved");
    assert_eq!(started.active(cx), second);

    // Writable again: the text is written, then the note goes.
    started.run_file_op(&rt, FileOpRequest::Delete(path("a.md")), cx);
    assert!(!exists(&rt, &v, "a.md"));
    assert_eq!(started.note_in(first, cx), None);
    assert!(!started.dirty_in(first, cx));
    past_a_tick(&rt, cx);
    assert!(!exists(&rt, &v, "a.md"), "no tick recreates it");
}

/// A new tab or split whose note is not there (or would not read) keeps
/// its group, empty: it holds nothing, so the note opens elsewhere later.
#[gpui_kit::test]
fn a_failed_open_in_a_new_tab_leaves_an_empty_group_that_holds_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);

    started.new_tab("missing.md", cx);
    let second = started.active(cx);
    let second_tab = started.shown_tab(cx);
    assert_eq!(started.holders(&path("missing.md"), cx), vec![second]);
    drive(&rt, cx, |cx| started.all_idle(cx));

    assert_eq!(started.tabs(cx), vec![first_tab, second_tab]);
    assert_eq!(started.groups_of(second_tab, cx), vec![second]);
    assert_eq!(started.note_in(second, cx), None);
    assert_eq!(started.session_in(second, cx), VaultPath::root());
    assert_eq!(started.text_in(second, cx), "");
    assert_eq!(
        started.holders(&path("missing.md"), cx),
        Vec::<GroupId>::new()
    );
    assert_eq!(started.locate(&path("missing.md"), cx), None);

    // A folder named like a note does not read: the split stays empty too.
    std::fs::create_dir(v.workspace.as_path().join("odd.md")).unwrap();
    started.split("odd.md", cx);
    let third = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    assert_eq!(started.groups_of(second_tab, cx), vec![second, third]);
    assert_eq!(started.note_in(third, cx), None);
    assert_eq!(started.holders(&path("odd.md"), cx), Vec::<GroupId>::new());

    // Made later, the note opens where it is asked for, not in a group that
    // once tried it.
    rt.block_on(v.vault.create_note(&path("missing.md"), "found"))
        .unwrap();
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.focus_group(first, cx))
    });
    started.plain("missing.md", cx);
    drive(&rt, cx, |cx| {
        started.note_in(first, cx) == Some(path("missing.md"))
    });
    assert_eq!(started.text_in(first, cx), "found");
    assert_eq!(started.holders(&path("missing.md"), cx), vec![first]);
    started.assert_states_in_step(cx);
}

// ── review round 1: moves follow, properties and attachments find their group

/// Moving a note by itself while a group in a hidden tab holds it: that
/// group follows it to the new place with its unsaved text written there;
/// the tabs, the split and the shown tab stay as they were.
#[gpui_kit::test]
fn move_of_a_single_note_updates_a_group_in_a_hidden_tab(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let notes = [
        ("a.md", "alpha"),
        ("b.md", "beta"),
        ("c.md", "gamma"),
        ("z/k.md", "kay"),
    ];
    let (v, started) = started_with(cx, &rt, &notes, "a.md");
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    let second_tab = started.shown_tab(cx);
    started.split("c.md", cx);
    let third = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    started.set_text_in(first, "alpha edited", cx);

    started.run_file_op(
        &rt,
        FileOpRequest::Move {
            from: path("a.md"),
            to: path("z/a.md"),
        },
        cx,
    );

    assert!(!exists(&rt, &v, "a.md"));
    assert_eq!(disk(&rt, &v, "z/a.md").as_deref(), Some("alpha edited"));
    assert_eq!(started.note_in(first, cx), Some(path("z/a.md")));
    assert_eq!(started.session_in(first, cx), path("z/a.md"));
    assert_eq!(started.text_in(first, cx), "alpha edited");
    assert!(
        !started.dirty_in(first, cx),
        "reloaded clean at the new place"
    );
    assert_eq!(started.locate(&path("z/a.md"), cx), Some(first));
    assert_eq!(started.locate(&path("a.md"), cx), None);
    assert_eq!(started.tabs(cx), vec![first_tab, second_tab]);
    assert_eq!(started.groups_of(first_tab, cx), vec![first]);
    assert_eq!(started.groups_of(second_tab, cx), vec![second, third]);
    assert_eq!(started.shown_tab(cx), second_tab);
    assert_eq!(started.active(cx), third);
    assert_eq!(started.placeholder(cx), None, "nothing says it was moved");

    // Later saves of the followed group land at the new place.
    started.set_text_in(first, "alpha again", cx);
    send(&started, AppMessage::Autosave, cx);
    drive(&rt, cx, |cx| !started.dirty_in(first, cx));
    assert_eq!(disk(&rt, &v, "z/a.md").as_deref(), Some("alpha again"));
    past_a_tick(&rt, cx);
    assert!(!exists(&rt, &v, "a.md"), "no tick recreates the old place");
    started.assert_states_in_step(cx);
}

/// The properties of a note a hidden group holds with unsaved text: that
/// group is flushed before the dialog opens, the write lands, the group
/// reloads the written text clean, and its later autosave keeps the write.
#[gpui_kit::test]
fn properties_write_flushes_the_group_that_holds_the_note(cx: &mut TestAppContext) {
    use super::properties::PropertyJob;

    let rt = runtime();
    let _guard = rt.enter();
    let notes = [("a.md", "---\ntag: x\n---\nalpha\n"), ("b.md", "beta")];
    let (v, started) = started_with(cx, &rt, &notes, "a.md");
    let first = started.active(cx);
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));
    let edited = "---\ntag: x\n---\nalpha edited\n";
    started.set_text_in(first, edited, cx);

    let workspace = started.workspace.clone();
    cx.update_window(started.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_properties(path("a.md"), window, cx))
    })
    .unwrap();
    drive(&rt, cx, |cx| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            !w.note_held() && w.properties_dialog().is_some()
        })
    });
    assert_eq!(
        disk(&rt, &v, "a.md").as_deref(),
        Some(edited),
        "the holding group's text is on disk before the dialog opens"
    );
    assert!(!started.dirty_in(first, cx));

    let written = cx.update(|cx| {
        started.workspace.update(cx, |w, cx| {
            w.request_property_write(path("a.md"), PropertyJob::Delete("tag".into()), cx)
        })
    });
    assert_eq!(written, Ok(()));
    drive(&rt, cx, |cx| {
        cx.read(|cx| !started.workspace.read(cx).note_held())
            && !disk(&rt, &v, "a.md").unwrap().contains("tag")
            && !started.text_in(first, cx).contains("tag")
    });
    let on_disk = disk(&rt, &v, "a.md").unwrap();
    assert!(on_disk.contains("alpha edited"), "{on_disk}");
    assert_eq!(started.text_in(first, cx), on_disk, "the holder reloaded");
    assert!(!started.dirty_in(first, cx));

    // The group's later autosave keeps the write.
    started.set_text_in(first, &format!("{on_disk}more\n"), cx);
    send(&started, AppMessage::Autosave, cx);
    drive(&rt, cx, |cx| !started.dirty_in(first, cx));
    let after = disk(&rt, &v, "a.md").unwrap();
    assert!(!after.contains("tag"), "{after}");
    assert!(after.contains("alpha edited"), "{after}");
    assert_eq!(started.active(cx), second);
}

/// An attachment shown in a group of a hidden tab follows a rename and
/// goes on a delete, while the shown tab's group is left alone.
#[gpui_kit::test]
fn attachment_rename_follows_the_group_that_shows_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    rt.block_on(v.vault.save_attachment(&path("notes.txt"), b"some text\n"))
        .unwrap();
    let first = started.active(cx);
    let shown = |group: GroupId, cx: &TestAppContext| -> Option<VaultPath> {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            w.state(group)
                .and_then(|s| s.attachment.as_ref())
                .map(|view| view.read(cx).path().clone())
        })
    };
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_attachment(&path("notes.txt"), cx))
    });
    drive(&rt, cx, |cx| shown(first, cx) == Some(path("notes.txt")));
    started.new_tab("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.all_idle(cx));

    started.run_file_op(
        &rt,
        FileOpRequest::Rename {
            from: path("notes.txt"),
            to: path("renamed.txt"),
        },
        cx,
    );
    settle(&rt, cx, 400, |cx| {
        shown(first, cx).is_some_and(|p| p.is_like(&path("renamed.txt")))
    });
    assert!(
        shown(first, cx).is_some_and(|p| p.is_like(&path("renamed.txt"))),
        "{:?}",
        shown(first, cx)
    );
    assert_eq!(shown(second, cx), None);
    assert_eq!(started.active(cx), second);
    assert_eq!(started.note_in(second, cx), Some(path("b.md")));
    cx.read(|cx| assert_eq!(started.workspace.read(cx).status.attachment, None));

    started.run_file_op(&rt, FileOpRequest::Delete(path("renamed.txt")), cx);
    assert_eq!(shown(first, cx), None);
    assert_eq!(
        started.placeholder(cx),
        None,
        "the shown group says nothing"
    );
    assert_eq!(started.active(cx), second);
    assert_eq!(started.text(cx), "beta");
}
