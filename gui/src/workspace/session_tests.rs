//! The session record (ADR-0058): the tabs, their arrangements, the note in
//! each group and the active tab and group are written to the workspace's
//! session file shortly after a change and at once on quit, and restored
//! when the workspace opens.
//!
//! A real `Workspace` on a real temp vault, driven like the tab tests. The
//! record's writer writes inline under test, so the file can be read right
//! after the debounce's timer fires.
#![cfg(test)]

use gpui_kit::{AppContext as _, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout_tree::{Axis, GroupId};
use kimun_notes::logic::message::AppMessage;
use kimun_notes::settings::session::{
    AxisRecord, GroupTreeRecord, SESSION_VERSION, SessionFile, SessionRecord, TabRecord,
};
use tokio::runtime::Runtime;

use super::reopen_tests::{open, pair, set_current};
use super::session::SESSION_DEBOUNCE;
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use super::{LayoutStore, Workspace};
use crate::startup::{FirstNote, Start};
use crate::test_vault::{NAME, TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

const NOTES: &[(&str, &str)] = &[
    ("a.md", "alpha"),
    ("b.md", "beta"),
    ("c.md", "gamma"),
    ("d.md", "delta"),
];

fn leaf(note: Option<&str>) -> GroupTreeRecord {
    GroupTreeRecord::Leaf {
        note: note.map(VaultPath::new),
    }
}

/// The session file the window writes for the test vault's workspace.
fn session_of(v: &TestVault) -> SessionFile {
    v.settings.read().unwrap().session_for(NAME)
}

/// A started window with this file's readers (a wrapper, so they cannot
/// collide with the other test files' `Started` helpers).
struct Win(Started);

impl std::ops::Deref for Win {
    type Target = Started;

    fn deref(&self) -> &Started {
        &self.0
    }
}

impl Win {
    fn with<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Workspace, &mut gpui_kit::Context<Workspace>) -> R,
    ) -> R {
        self.workspace.update(cx, f)
    }

    /// The window's own record of what it shows now.
    fn record(&self, cx: &TestAppContext) -> SessionRecord {
        cx.read(|cx| self.workspace.read(cx).session_record())
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

    /// Every group has nothing under way.
    fn all_idle(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let w = self.workspace.read(cx);
            w.layout.region().all_groups().into_iter().all(|g| {
                w.state(g).and_then(|s| s.notes.as_ref()).is_some_and(|h| {
                    !h.is_switching() && !h.is_open_wanted() && !h.session.is_open_pending()
                })
            })
        })
    }

    /// The notes each tab's groups show, tab by tab, in reading order.
    fn notes_by_tab(&self, cx: &TestAppContext) -> Vec<Vec<Option<String>>> {
        let tabs = cx.read(|cx| self.workspace.read(cx).layout.region().tabs());
        tabs.into_iter()
            .map(|tab| {
                let groups = cx.read(|cx| self.workspace.read(cx).layout.region().groups(tab));
                groups
                    .into_iter()
                    .map(|g| self.note_in(g, cx).map(|n| n.to_string()))
                    .collect()
            })
            .collect()
    }

    fn active(&self, cx: &TestAppContext) -> GroupId {
        cx.read(|cx| self.workspace.read(cx).layout.active_group())
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

    /// The user quits the app.
    fn quit(&self, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            self.workspace
                .update(cx, |w, cx| w.request_quit(window, cx))
        })
        .unwrap();
    }

    /// Let the debounce's timer fire and what it starts run.
    fn after_debounce(&self, cx: &mut TestAppContext) {
        cx.executor().advance_clock(SESSION_DEBOUNCE);
        cx.run_until_parked();
    }
}

/// A vault holding [`NOTES`] with `history`, the window started on `first`.
fn started(
    cx: &mut TestAppContext,
    rt: &Runtime,
    history: &[&str],
    record: Option<&SessionRecord>,
    first: FirstNote,
) -> (TestVault, Win) {
    let v = rt.block_on(test_vault(NOTES, history));
    if let Some(record) = record {
        session_of(&v).save(record).unwrap();
    }
    let started = open_started(cx, vault_start(&v, first), v.settings.clone());
    (v, Win(started))
}

/// Started on `a.md` from the command line: no restore, the record is
/// written as the window changes.
fn started_on_a(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Win) {
    let (v, win) = started(cx, rt, &[], None, FirstNote::Note(path("a.md")));
    drive(rt, cx, |cx| win.text(cx) == "alpha" && !win.indexing(cx));
    (v, win)
}

/// Tab one: `a.md` beside `b.md` (active), 0.25 / 0.75. Tab two (shown):
/// `c.md` over an empty group.
fn two_tab_record() -> SessionRecord {
    SessionRecord {
        version: SESSION_VERSION,
        tabs: vec![
            TabRecord {
                tree: GroupTreeRecord::Split {
                    axis: AxisRecord::Horizontal,
                    weights: vec![0.25, 0.75],
                    children: vec![leaf(Some("a.md")), leaf(Some("b.md"))],
                },
                active: 1,
            },
            TabRecord {
                tree: GroupTreeRecord::Split {
                    axis: AxisRecord::Vertical,
                    weights: vec![0.5, 0.5],
                    children: vec![leaf(Some("c.md")), leaf(None)],
                },
                active: 0,
            },
        ],
        shown: 1,
    }
}

// ── Saving ────────────────────────────────────────────────────────────────

#[gpui_kit::test]
fn a_change_writes_the_record_after_the_debounce(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, win) = started_on_a(cx, &rt);
    win.after_debounce(cx);
    assert_eq!(
        session_of(&v).load(),
        None,
        "the launch's own note is no change"
    );

    win.with(cx, |w, cx| w.open_note_in_new_tab(path("b.md"), cx));
    drive(&rt, cx, |cx| win.text(cx) == "beta" && win.all_idle(cx));
    assert_eq!(
        session_of(&v).load(),
        None,
        "nothing is written before the debounce"
    );
    cx.executor().advance_clock(SESSION_DEBOUNCE / 2);
    cx.run_until_parked();
    assert_eq!(session_of(&v).load(), None);
    cx.executor().advance_clock(SESSION_DEBOUNCE / 2);
    cx.run_until_parked();
    assert_eq!(
        session_of(&v).load(),
        Some(SessionRecord {
            version: SESSION_VERSION,
            tabs: vec![
                TabRecord {
                    tree: leaf(Some("a.md")),
                    active: 0,
                },
                TabRecord {
                    tree: leaf(Some("b.md")),
                    active: 0,
                },
            ],
            shown: 1,
        })
    );
}

/// A divider drag, a tab switch and a split each count as a change, and a
/// burst of them ends with the newest on disk.
#[gpui_kit::test]
fn a_burst_of_changes_ends_with_the_newest_record(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, win) = started_on_a(cx, &rt);
    win.with(cx, |w, cx| {
        w.open_note_in_split(path("b.md"), Axis::Horizontal, cx)
    });
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "beta");
    let first = cx.read(|cx| {
        win.workspace
            .read(cx)
            .layout
            .region()
            .groups(win.workspace.read(cx).layout.region().shown())[0]
    });
    for share in [0.3, 0.4, 0.6] {
        win.with(cx, |w, cx| {
            w.layout.region_mut().resize_after(first, share);
            cx.notify();
        });
        cx.run_until_parked();
    }
    win.after_debounce(cx);
    let saved = session_of(&v).load().expect("a record");
    assert_eq!(saved, win.record(cx));
    let GroupTreeRecord::Split { weights, .. } = &saved.tabs[0].tree else {
        panic!("a split: {saved:?}");
    };
    assert!((weights[0] - 0.6).abs() < 1e-9, "{weights:?}");
}

#[gpui_kit::test]
fn quit_writes_the_record_at_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, win) = started_on_a(cx, &rt);
    win.with(cx, |w, cx| {
        w.open_note_in_split(path("b.md"), Axis::Vertical, cx)
    });
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "beta");
    win.quit(cx);
    cx.run_until_parked();
    let record = session_of(&v).load().expect("written without waiting");
    assert_eq!(
        record,
        SessionRecord {
            version: SESSION_VERSION,
            tabs: vec![TabRecord {
                tree: GroupTreeRecord::Split {
                    axis: AxisRecord::Vertical,
                    weights: vec![0.5, 0.5],
                    children: vec![leaf(Some("a.md")), leaf(Some("b.md"))],
                },
                active: 1,
            }],
            shown: 0,
        }
    );
}

/// A window close (the app stays in the dock) writes it at once too.
#[gpui_kit::test]
fn closing_the_window_writes_the_record_at_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, win) = started_on_a(cx, &rt);
    win.with(cx, |w, cx| w.open_note_in_new_tab(path("c.md"), cx));
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "gamma");
    let expected = win.record(cx);
    assert!(gpui_kit::VisualTestContext::from_window(win.handle, cx).simulate_close());
    assert_eq!(session_of(&v).load(), Some(expected));
}

/// A note still opening is recorded as the note it opens.
#[gpui_kit::test]
fn a_note_still_opening_is_recorded_as_its_target(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, win) = started_on_a(cx, &rt);
    win.with(cx, |w, cx| w.open_note(path("d.md"), cx));
    // Not driven: the read has not landed.
    assert_eq!(
        win.record(cx).tabs[0].tree,
        leaf(Some("d.md")),
        "the group's target, not the note it is leaving"
    );
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "delta");
    assert_eq!(win.record(cx).tabs[0].tree, leaf(Some("d.md")));
}

/// No workspace: nothing to write, ever.
#[gpui_kit::test]
fn no_workspace_writes_no_record(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(NOTES, &[]));
    let win = Win(open_started(
        cx,
        Start::OpenFailed("not in this test".into()),
        v.settings.clone(),
    ));
    cx.run_until_parked();
    win.after_debounce(cx);
    win.quit(cx);
    cx.run_until_parked();
    assert!(!session_of(&v).exists());
}

// ── Restoring ─────────────────────────────────────────────────────────────

/// The record wins over the history's newest note: every tab, split,
/// weight, note, the shown tab and each tab's active group come back.
#[gpui_kit::test]
fn the_record_restores_at_start_over_the_history(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (_v, win) = started(cx, &rt, &["d.md"], Some(&record), FirstNote::Restore);
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.notes_by_tab(cx).len() == 2
    });
    win.assert_states_in_step(cx);
    assert_eq!(
        win.notes_by_tab(cx),
        vec![
            vec![Some("a.md".to_string()), Some("b.md".to_string())],
            vec![Some("c.md".to_string()), None],
        ]
    );
    assert_eq!(win.record(cx), record, "the same arrangement, weights too");
    assert_eq!(win.text(cx), "gamma", "the shown tab's active group");
    let active = win.active(cx);
    assert_eq!(win.note_in(active, cx), Some(path("c.md")));
    assert!(
        cx.read(|cx| win.workspace.read(cx).locate_note(&path("d.md")))
            .is_none(),
        "the history's note does not open"
    );
}

/// A note deleted between runs leaves its group empty; the rest returns,
/// and the window says which note is gone.
#[gpui_kit::test]
fn a_missing_note_restores_as_an_empty_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let mut record = two_tab_record();
    let GroupTreeRecord::Split { children, .. } = &mut record.tabs[0].tree else {
        unreachable!();
    };
    children[0] = leaf(Some("gone.md"));
    let (_v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.notes_by_tab(cx).len() == 2
    });
    win.assert_states_in_step(cx);
    assert_eq!(
        win.notes_by_tab(cx),
        vec![
            vec![None, Some("b.md".to_string())],
            vec![Some("c.md".to_string()), None],
        ]
    );
    let flash = win.flash(cx).expect("the gone note is said");
    assert!(flash.contains("gone.md"), "{flash}");
}

/// Every recorded note gone: a fresh start, so the history's newest note
/// opens in one group, as with no record at all.
#[gpui_kit::test]
fn every_note_missing_falls_back_to_the_history(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = SessionRecord {
        version: SESSION_VERSION,
        tabs: vec![
            TabRecord {
                tree: leaf(Some("x.md")),
                active: 0,
            },
            TabRecord {
                tree: leaf(Some("y.md")),
                active: 0,
            },
        ],
        shown: 1,
    };
    let (_v, win) = started(cx, &rt, &["d.md"], Some(&record), FirstNote::Restore);
    drive(&rt, cx, |cx| win.text(cx) == "delta" && win.all_idle(cx));
    win.assert_states_in_step(cx);
    assert_eq!(win.notes_by_tab(cx), vec![vec![Some("d.md".to_string())]]);
}

/// The restore does not write the record back: what it opened is what the
/// file says (a note it dropped stays in the file until the user changes
/// something).
#[gpui_kit::test]
fn the_restore_does_not_write_the_record_back(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let mut record = two_tab_record();
    record.tabs[1].tree = leaf(Some("gone.md"));
    let (v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    let before = std::fs::read(session_of(&v).path().as_path()).unwrap();
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.notes_by_tab(cx).len() == 2
    });
    win.after_debounce(cx);
    assert_eq!(
        std::fs::read(session_of(&v).path().as_path()).unwrap(),
        before,
        "untouched"
    );
    // A change after the restore is written.
    win.with(cx, |w, cx| w.open_note_in_new_tab(path("d.md"), cx));
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "delta");
    win.after_debounce(cx);
    assert_eq!(session_of(&v).load(), Some(win.record(cx)));
}

/// A quit before the restore landed keeps the record on disk as it was.
#[gpui_kit::test]
fn a_quit_before_the_restore_keeps_the_record(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    let before = std::fs::read(session_of(&v).path().as_path()).unwrap();
    // Not driven: indexing (and so the restore) has not finished.
    win.quit(cx);
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(session_of(&v).path().as_path()).unwrap(),
        before
    );
    // The restore still lands (the test platform does not end the app);
    // a second quit with nothing changed writes nothing either.
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.notes_by_tab(cx).len() == 2
    });
    win.quit(cx);
    cx.run_until_parked();
    assert_eq!(
        std::fs::read(session_of(&v).path().as_path()).unwrap(),
        before
    );
}

/// A switch writes the old workspace's record under the old workspace,
/// starts the new one from its own (here none: its history), and a switch
/// back restores the old arrangement.
#[gpui_kit::test]
fn a_switch_saves_the_old_record_and_restores_the_new(cx: &mut TestAppContext) {
    use super::reopen_tests::{ALPHA, BETA};
    let rt = runtime();
    let _guard = rt.enter();
    let p = rt.block_on(pair());
    let win = Win(open(cx, &rt, &p, LayoutStore::Off));
    drive(&rt, cx, |cx| win.text(cx) == ALPHA && !win.indexing(cx));
    win.with(cx, |w, cx| {
        w.open_note_in_split(path("afriend.md"), Axis::Horizontal, cx)
    });
    drive(&rt, cx, |cx| {
        win.all_idle(cx) && win.notes_by_tab(cx)[0].len() == 2
    });
    let a_record = win.record(cx);

    set_current(&p.settings, "b");
    win.sw_reopen_to(&rt, Some(BETA), cx);
    drive(&rt, cx, |cx| win.all_idle(cx));
    let a_file = p.settings.read().unwrap().session_for("a");
    assert_eq!(
        a_file.load(),
        Some(a_record.clone()),
        "written on the way out"
    );
    assert_eq!(
        win.notes_by_tab(cx),
        vec![vec![Some("beta.md".to_string())]],
        "B starts from its own history, in one group"
    );
    win.assert_states_in_step(cx);
    // In B: bfriend below beta.
    win.with(cx, |w, cx| {
        w.open_note_in_split(path("bfriend.md"), Axis::Vertical, cx)
    });
    drive(&rt, cx, |cx| {
        win.all_idle(cx) && win.notes_by_tab(cx)[0].len() == 2
    });

    set_current(&p.settings, "a");
    win.sw_reopen_to(&rt, None, cx);
    drive(&rt, cx, |cx| {
        win.all_idle(cx) && win.notes_by_tab(cx).first().is_some_and(|t| t.len() == 2)
    });
    win.assert_states_in_step(cx);
    assert_eq!(win.record(cx), a_record, "A's arrangement comes back");
    let b_file = p.settings.read().unwrap().session_for("b");
    assert_eq!(
        b_file.load(),
        Some(SessionRecord {
            version: SESSION_VERSION,
            tabs: vec![TabRecord {
                tree: GroupTreeRecord::Split {
                    axis: AxisRecord::Vertical,
                    weights: vec![0.5, 0.5],
                    children: vec![leaf(Some("beta.md")), leaf(Some("bfriend.md"))],
                },
                active: 1,
            }],
            shown: 0,
        }),
        "B's record was written under B, every group and weight"
    );
    assert_eq!(
        a_file.load(),
        Some(a_record),
        "A's restore wrote nothing back"
    );
}

// ── Command-line starts ───────────────────────────────────────────────────

/// The file's bytes, to see that nothing touched it.
fn bytes(v: &TestVault) -> Vec<u8> {
    std::fs::read(session_of(v).path().as_path()).unwrap()
}

/// A note named on the command line opens alone, and the stored record
/// survives the launch: it is replaced only once the arrangement differs
/// from that one group.
#[gpui_kit::test]
fn a_command_line_note_keeps_the_record_until_a_change(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Note(path("d.md")));
    let before = bytes(&v);
    drive(&rt, cx, |cx| {
        win.text(cx) == "delta" && !win.indexing(cx) && win.all_idle(cx)
    });
    assert_eq!(win.notes_by_tab(cx), vec![vec![Some("d.md".to_string())]]);
    win.after_debounce(cx);
    assert_eq!(bytes(&v), before, "the launch's note is no change");

    win.with(cx, |w, cx| {
        w.open_note_in_split(path("a.md"), Axis::Horizontal, cx)
    });
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "alpha");
    win.after_debounce(cx);
    assert_eq!(session_of(&v).load(), Some(win.record(cx)), "a real change");
}

/// A quit right after a command-line launch leaves the record as it was;
/// so does one whose note will not open (the group stays empty).
#[gpui_kit::test]
fn quitting_after_a_command_line_launch_keeps_the_record(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Note(path("a.md")));
    let before = bytes(&v);
    drive(&rt, cx, |cx| win.text(cx) == "alpha" && win.all_idle(cx));
    win.quit(cx);
    cx.run_until_parked();
    assert_eq!(bytes(&v), before);
    drive(&rt, cx, |cx| !win.indexing(cx));

    let (v, win) = started(
        cx,
        &rt,
        &[],
        Some(&record),
        FirstNote::Note(path("missing.md")),
    );
    let before = bytes(&v);
    drive(&rt, cx, |cx| !win.indexing(cx) && win.all_idle(cx));
    win.after_debounce(cx);
    win.quit(cx);
    cx.run_until_parked();
    assert_eq!(bytes(&v), before, "an empty launch is no change either");
}

/// A refused argument (nothing to open) restores the record as a plain
/// start does; its error flash writes nothing.
#[gpui_kit::test]
fn a_refused_argument_restores_the_record_and_keeps_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(
        cx,
        &rt,
        &["d.md"],
        Some(&record),
        FirstNote::Rejected("bad?name is not a note".into()),
    );
    let before = bytes(&v);
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.notes_by_tab(cx).len() == 2
    });
    assert_eq!(win.record(cx), record);
    win.after_debounce(cx);
    win.quit(cx);
    cx.run_until_parked();
    assert_eq!(bytes(&v), before);
}

// ── An open during the restore ────────────────────────────────────────────

/// The user opens a note while the vault still indexes: the restore wins,
/// and the note opens in its active group afterwards (a plain open, which
/// replaces that group's note), and that change is written.
#[gpui_kit::test]
fn an_open_during_the_restore_lands_in_the_restored_active_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    // Not driven: the restore is on its way. Two opens: the newest wins.
    win.with(cx, |w, cx| w.open_note(path("b.md"), cx));
    win.with(cx, |w, cx| w.open_note(path("d.md"), cx));
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.text(cx) == "delta"
    });
    win.assert_states_in_step(cx);
    assert_eq!(
        win.notes_by_tab(cx),
        vec![
            vec![Some("a.md".to_string()), Some("b.md".to_string())],
            vec![Some("d.md".to_string()), None],
        ],
        "the restored arrangement, d.md in place of the active group's c.md"
    );
    win.after_debounce(cx);
    assert_eq!(session_of(&v).load(), Some(win.record(cx)));
}

/// The note opened meanwhile is one the record restores: its group is
/// focused (one copy), nothing is opened twice.
#[gpui_kit::test]
fn an_open_during_the_restore_of_a_restored_note_focuses_its_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (_v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    win.with(cx, |w, cx| w.open_note(path("a.md"), cx));
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.notes_by_tab(cx).len() == 2
    });
    win.assert_states_in_step(cx);
    assert_eq!(
        win.notes_by_tab(cx),
        vec![
            vec![Some("a.md".to_string()), Some("b.md".to_string())],
            vec![Some("c.md".to_string()), None],
        ]
    );
    assert_eq!(win.note_in(win.active(cx), cx), Some(path("a.md")));
    assert_eq!(win.text(cx), "alpha");
}

/// A restore that fails (its job panicked) lands on the history's newest
/// note, so the editor does not wait for it for good.
#[gpui_kit::test]
fn a_failed_restore_lands_on_the_history(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(cx, &rt, &["d.md"], Some(&record), FirstNote::Restore);
    win.with(cx, |w, cx| {
        let job = tokio::spawn(async { panic!("the restore job fails in this test") });
        w.await_session_restore(job, cx);
    });
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.text(cx) == "delta"
    });
    assert!(!cx.read(|cx| win.workspace.read(cx).restore_pending));
    assert_eq!(win.notes_by_tab(cx), vec![vec![Some("d.md".to_string())]]);
    // The stored record was never applied: the history's note does not
    // overwrite it, at the debounce or at a quit.
    win.after_debounce(cx);
    win.quit(cx);
    cx.run_until_parked();
    assert_eq!(session_of(&v).load(), Some(record));
    // A real change does.
    win.with(cx, |w, cx| w.open_note_in_new_tab(path("a.md"), cx));
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "alpha");
    win.after_debounce(cx);
    assert_eq!(session_of(&v).load(), Some(win.record(cx)));
}

// ── Which opens wait ──────────────────────────────────────────────────────

/// With no stored record nothing restores from one: an open while the
/// vault indexes goes through at once, as before the session file.
#[gpui_kit::test]
fn with_no_record_an_open_during_indexing_opens_at_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, win) = started(cx, &rt, &["d.md"], None, FirstNote::Restore);
    assert!(win.indexing(cx));
    win.with(cx, |w, cx| w.open_note(path("b.md"), cx));
    assert!(!cx.read(|cx| win.workspace.read(cx).session_restoring()));
    assert_eq!(
        win.record(cx).tabs[0].tree,
        leaf(Some("b.md")),
        "the group is on its way to it now"
    );
    drive(&rt, cx, |cx| !win.indexing(cx) && win.all_idle(cx));
}

/// With a stored record the open waits for it, and the Panel keeps saying
/// the last session is on its way (even for an `OpenPath` on the channel).
#[gpui_kit::test]
fn with_a_record_an_open_during_indexing_is_queued(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (_v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    assert!(win.indexing(cx));
    win.with(cx, |w, cx| w.open_note(path("b.md"), cx));
    send(&win, AppMessage::open(path("d.md")), cx);
    assert!(cx.read(|cx| win.workspace.read(cx).session_restoring()));
    assert_eq!(
        win.record(cx).tabs[0].tree,
        leaf(None),
        "nothing opened yet"
    );
    assert_eq!(
        win.placeholder(cx).as_deref(),
        Some("Opening your last note…")
    );
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.text(cx) == "delta"
    });
    assert_eq!(
        win.notes_by_tab(cx),
        vec![
            vec![Some("a.md".to_string()), Some("b.md".to_string())],
            vec![Some("d.md".to_string()), None],
        ],
        "the newest open, in the restored active group"
    );
}

/// A new tab (or split) asked for during the restore waits too: no tab is
/// made for the restore to throw away; it is made after, with its note.
#[gpui_kit::test]
fn a_new_tab_during_the_restore_waits_for_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (_v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    win.with(cx, |w, cx| w.open_note_in_new_tab(path("d.md"), cx));
    assert_eq!(win.notes_by_tab(cx).len(), 1, "no tab yet");
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && win.all_idle(cx) && win.text(cx) == "delta"
    });
    win.assert_states_in_step(cx);
    assert_eq!(
        win.notes_by_tab(cx),
        vec![
            vec![Some("a.md".to_string()), Some("b.md".to_string())],
            vec![Some("c.md".to_string()), None],
            vec![Some("d.md".to_string())],
        ]
    );
}

/// A new tab, a split, a group close and a tab step asked for during the
/// restore wait for it, as an open does, and then run in the order they
/// were asked for on the restored region: none is thrown away by it.
#[gpui_kit::test]
fn structural_commands_wait_for_a_pending_restore(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (_v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    assert!(cx.read(|cx| win.workspace.read(cx).session_restoring()));
    win.with(cx, |w, cx| {
        w.new_tab(cx);
        w.split_active_group(Axis::Horizontal, cx);
        let active = w.layout.active_group();
        w.close_editor_group(active, cx);
        w.step_tab(true, cx);
    });
    assert_eq!(
        win.notes_by_tab(cx),
        vec![vec![None]],
        "nothing changed yet"
    );
    drive(&rt, cx, |cx| {
        !win.indexing(cx)
            && !cx.read(|cx| win.workspace.read(cx).session_restoring())
            && win.all_idle(cx)
    });
    win.assert_states_in_step(cx);
    // The record, then a new tab (split, its new group closed), then the
    // step from it wraps to the first tab.
    assert_eq!(
        win.notes_by_tab(cx),
        vec![
            vec![Some("a.md".to_string()), Some("b.md".to_string())],
            vec![Some("c.md".to_string()), None],
            vec![None],
        ]
    );
    let (shown, first) = cx.read(|cx| {
        let region = win.workspace.read(cx).layout.region();
        (region.shown(), region.tabs()[0])
    });
    assert_eq!(shown, first, "the step ran last");
}

/// A restore that cannot be applied (a close was running) leaves the stored
/// record as it is: what the window holds instead is no change.
#[gpui_kit::test]
fn a_restore_that_cannot_apply_writes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Restore);
    let before = bytes(&v);
    win.with(cx, |w, _| {
        let group = w.layout.active_group();
        w.groups_closing = Some((
            super::tab_strip::CloseTarget::Group(group),
            gpui_kit::Task::ready(()),
        ));
    });
    drive(&rt, cx, |cx| {
        !win.indexing(cx) && !cx.read(|cx| win.workspace.read(cx).session_restoring())
    });
    win.with(cx, |w, _| w.groups_closing = None);
    assert_eq!(win.notes_by_tab(cx), vec![vec![None]], "not applied");
    win.after_debounce(cx);
    win.quit(cx);
    cx.run_until_parked();
    assert_eq!(bytes(&v), before);
}

// ── A stored file that does not load ─────────────────────────────────────

/// Regression lock (final review I2): a stored file this build cannot load
/// (a newer build's format here; a read that failed is the same) is kept
/// byte for byte through the start, the debounce and a quit, whether the
/// history lands a note or the editor stays empty, and with a refused
/// argument too. A real change then writes.
#[gpui_kit::test]
fn a_stored_file_that_does_not_load_is_kept_until_a_real_change(cx: &mut TestAppContext) {
    const NEWER: &[u8] = br#"{"version":2,"future":true}"#;
    let rt = runtime();
    let _guard = rt.enter();
    for (history, first, text) in [
        (&["d.md"][..], FirstNote::Restore, "delta"),
        (&[][..], FirstNote::Restore, ""),
        (
            &["d.md"][..],
            FirstNote::Rejected("bad?name is not a note".into()),
            "",
        ),
    ] {
        let v = rt.block_on(test_vault(NOTES, history));
        std::fs::write(session_of(&v).path().as_path(), NEWER).unwrap();
        let win = Win(open_started(
            cx,
            vault_start(&v, first.clone()),
            v.settings.clone(),
        ));
        drive(&rt, cx, |cx| {
            !win.indexing(cx) && win.all_idle(cx) && win.text(cx) == text
        });
        win.after_debounce(cx);
        win.quit(cx);
        cx.run_until_parked();
        assert_eq!(bytes(&v), NEWER, "kept: {first:?}");

        win.with(cx, |w, cx| w.open_note_in_new_tab(path("a.md"), cx));
        drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "alpha");
        win.after_debounce(cx);
        assert_eq!(session_of(&v).load(), Some(win.record(cx)), "{first:?}");
    }
}

// ── A command-line launch keeps the arrangement ──────────────────────────

/// Final-review ruling: after a launch with a note on the command line,
/// opening another note in its one group (a click in FILES) is no change:
/// the stored arrangement stays, through the debounce and a quit.
#[gpui_kit::test]
fn a_command_line_launch_keeps_the_record_when_another_note_opens(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Note(path("d.md")));
    let before = bytes(&v);
    drive(&rt, cx, |cx| {
        win.text(cx) == "delta" && !win.indexing(cx) && win.all_idle(cx)
    });
    win.with(cx, |w, cx| w.open_note(path("a.md"), cx));
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "alpha");
    win.after_debounce(cx);
    assert_eq!(bytes(&v), before, "a note replaced in the one group");
    win.quit(cx);
    cx.run_until_parked();
    assert_eq!(bytes(&v), before, "nor at a quit");
}

/// A structural change after a command-line launch (a tab added) replaces
/// the stored record, and from then on every change is written, a note
/// replaced in a group as well.
#[gpui_kit::test]
fn a_command_line_launch_writes_once_a_tab_is_added(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let record = two_tab_record();
    let (v, win) = started(cx, &rt, &[], Some(&record), FirstNote::Note(path("d.md")));
    drive(&rt, cx, |cx| {
        win.text(cx) == "delta" && !win.indexing(cx) && win.all_idle(cx)
    });
    win.with(cx, |w, cx| w.open_note_in_new_tab(path("b.md"), cx));
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "beta");
    win.after_debounce(cx);
    assert_eq!(session_of(&v).load(), Some(win.record(cx)), "replaced");
    assert_eq!(win.notes_by_tab(cx).len(), 2);

    win.with(cx, |w, cx| w.open_note(path("c.md"), cx));
    drive(&rt, cx, |cx| win.all_idle(cx) && win.text(cx) == "gamma");
    win.after_debounce(cx);
    assert_eq!(
        session_of(&v).load(),
        Some(win.record(cx)),
        "normal writes resume"
    );
}
