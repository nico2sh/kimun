//! The tab strip, its labels and the tab chords (ADR-0058): hidden with one
//! tab, a label per tab naming the note of its active group, and Ctrl+T,
//! Ctrl+W, Ctrl+Tab and Ctrl+Shift+Tab pressed as keys in a real window
//! over a real temp vault (driven like the note tests,
//! `start_tests::drive`).
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, Focusable as _, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor_region::TabId;
use kimun_notes::logic::layout_tree::{Axis, GroupId};
use tokio::runtime::Runtime;

use super::file_ops::FileOpRequest;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use super::tab_strip::{TAB_STRIP_ID, TAB_UNSAVED_ID, tab_dirty, tab_label, tab_label_id};
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
];

/// A started window with this file's readers (a wrapper, so they cannot
/// collide with the other test files' `Started` helpers).
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

fn disk(rt: &Runtime, v: &TestVault, p: &str) -> Option<String> {
    rt.block_on(v.vault.get_note_text(&path(p))).ok()
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

    fn label(&self, tab: TabId, cx: &TestAppContext) -> String {
        cx.read(|cx| tab_label(self.workspace.read(cx), tab, cx).to_string())
    }

    fn labels(&self, cx: &TestAppContext) -> Vec<String> {
        self.tabs(cx)
            .into_iter()
            .map(|tab| self.label(tab, cx))
            .collect()
    }

    fn dirty_tab(&self, tab: TabId, cx: &TestAppContext) -> bool {
        cx.read(|cx| tab_dirty(self.workspace.read(cx), tab, cx))
    }

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

    fn dirty_in(&self, group: GroupId, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let editor = self.workspace.read(cx).state(group).unwrap().editor.clone();
            editor.read(cx).editor.is_dirty()
        })
    }

    /// Empty `group`'s buffer (a clean load), then type `text`: unsaved.
    fn set_text_in(&self, group: GroupId, text: &str, cx: &mut TestAppContext) {
        let editor = cx.read(|cx| self.workspace.read(cx).state(group).unwrap().editor.clone());
        cx.update(|cx| {
            editor.update(cx, |view, cx| {
                view.editor.set_text(String::new());
                view.editor.insert_at_cursor(text);
                cx.notify();
            })
        });
        assert!(self.dirty_in(group, cx), "the text is unsaved");
    }

    fn idle(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let w = self.workspace.read(cx);
            w.layout.region().all_groups().into_iter().all(|g| {
                w.state(g).and_then(|s| s.notes.as_ref()).is_some_and(|h| {
                    !h.is_switching() && !h.is_open_wanted() && !h.session.is_open_pending()
                })
            })
        })
    }

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

    fn drawn(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|e| e.visible())
        })
        .unwrap()
    }

    /// Press `key` as the keyboard does, wherever the focus is.
    fn press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn new_tab_with(&self, note: &str, cx: &mut TestAppContext) {
        let note = path(note);
        cx.update(|cx| {
            self.workspace
                .update(cx, |w, cx| w.open_note_in_new_tab(note, cx))
        });
    }

    fn split_with(&self, note: &str, cx: &mut TestAppContext) {
        let note = path(note);
        cx.update(|cx| {
            self.workspace
                .update(cx, |w, cx| w.open_note_in_split(note, Axis::Horizontal, cx))
        });
    }

    /// `group`'s autosave tick period, `None` while its tick is stopped.
    fn tick_in(&self, group: GroupId, cx: &TestAppContext) -> Option<u64> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .and_then(|h| h.session.autosave_interval())
        })
    }

    /// Whether `group`'s editor refuses typing (a flush holds it).
    fn input_blocked(&self, group: GroupId, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let editor = self.workspace.read(cx).state(group).unwrap().editor.clone();
            editor.read(cx).input_blocked
        })
    }

    /// Whether something holds the notes (a tab close among them).
    fn held(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).note_held())
    }

    fn wanted_in(&self, group: GroupId, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .is_some_and(|h| h.is_open_wanted())
        })
    }

    fn focus_group(&self, group: GroupId, cx: &mut TestAppContext) {
        cx.update(|cx| self.workspace.update(cx, |w, cx| w.focus_group(group, cx)));
    }
}

/// One tab: no strip, the editor Panel is as it always was.
#[gpui_kit::test]
fn strip_is_hidden_with_one_tab(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    assert_eq!(started.tabs(cx).len(), 1);
    assert!(started.drawn("editor-panel", cx));
    assert!(!started.drawn(TAB_STRIP_ID, cx));
    assert!(!started.drawn(tab_label_id(0), cx));
    // A split is still one tab: no strip.
    started.split_with("b.md", cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert!(!started.drawn(TAB_STRIP_ID, cx));
}

/// Two tabs: the strip shows one label per tab, in order, and the plus
/// button. A click on a label shows that tab; its close button closes it.
#[gpui_kit::test]
fn strip_shows_with_two_tabs(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);
    started.new_tab_with("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));

    assert!(started.drawn(TAB_STRIP_ID, cx));
    assert!(started.drawn(tab_label_id(0), cx));
    assert!(started.drawn(tab_label_id(1), cx));
    assert!(!started.drawn(tab_label_id(2), cx));
    assert!(started.drawn("tab-strip-new", cx));
    assert_eq!(started.labels(cx), ["a.md", "b.md"]);

    // A click on the first label shows its tab and focuses its group.
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(tab_label_id(0), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.shown_tab(cx), first_tab);
    assert_eq!(started.active(cx), first);
    assert_eq!(started.text(cx), "alpha");
    drive(&rt, cx, |cx| started.editor_focused(first, cx));

    // The plus button adds a third tab, empty.
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("tab-strip-new", cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(started.labels(cx), ["a.md", "b.md", "Empty"]);

    // The second tab's close button closes it (b.md was clean).
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(ElementId::NamedInteger("tab-close".into(), 1), cx)
    })
    .unwrap();
    drive(&rt, cx, |cx| started.tabs(cx).len() == 2);
    assert_eq!(started.labels(cx), ["a.md", "Empty"]);
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));
    cx.read(|cx| assert!(started.workspace.read(cx).state(second).is_none()));
    started.assert_states_in_step(cx);
}

/// A tab's label is the name of the note of its active group, as the
/// Footer shows it, and follows a plain open.
#[gpui_kit::test]
fn label_is_the_active_notes_title(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first_tab = started.shown_tab(cx);
    assert_eq!(started.label(first_tab, cx), "a.md");
    started.new_tab_with("b.md", cx);
    let second_tab = started.shown_tab(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert_eq!(started.label(second_tab, cx), "b.md");

    let group = started.active(cx);
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_note(path("c.md"), cx))
    });
    drive(&rt, cx, |cx| {
        started.note_in(group, cx) == Some(path("c.md"))
    });
    assert_eq!(started.labels(cx), ["a.md", "c.md"]);
    // The Footer names the same note.
    cx.read(|cx| assert_eq!(started.workspace.read(cx).status.note, Some(path("c.md"))));
}

/// A tab with more groups adds ` +N` (the other groups) to the active
/// group's name; an empty active group reads `Empty`.
#[gpui_kit::test]
fn label_adds_a_count_for_extra_groups(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    let tab = started.shown_tab(cx);
    started.split_with("b.md", cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert_eq!(started.label(tab, cx), "b.md +1");
    started.split_with("c.md", cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert_eq!(started.groups_of(tab, cx).len(), 3);
    assert_eq!(started.label(tab, cx), "c.md +2");
    started.focus_group(first, cx);
    assert_eq!(started.label(tab, cx), "a.md +2");

    // A second tab, empty, counts nothing of the first.
    started.press("ctrl-t", cx);
    let empty_tab = started.shown_tab(cx);
    assert_eq!(started.labels(cx), ["a.md +2", "Empty"]);
    assert_eq!(started.label(empty_tab, cx), "Empty");
}

/// Unsaved text in any group of a tab marks its label with the Footer's
/// unsaved dot; the name stays the same. Saving clears the mark.
#[gpui_kit::test]
fn label_marks_unsaved_text(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);
    started.new_tab_with("b.md", cx);
    let second_tab = started.shown_tab(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert!(!started.dirty_tab(first_tab, cx));
    assert!(!started.dirty_tab(second_tab, cx));
    assert!(!started.drawn(TAB_UNSAVED_ID, cx));

    // The hidden first tab has unsaved text.
    started.set_text_in(first, "alpha edited", cx);
    assert!(started.dirty_tab(first_tab, cx));
    assert!(!started.dirty_tab(second_tab, cx));
    assert_eq!(started.label(first_tab, cx), "a.md");
    assert!(started.drawn((TAB_UNSAVED_ID, 0usize), cx));
    assert!(!started.drawn((TAB_UNSAVED_ID, 1usize), cx));

    // Saved by a visit and the Save chord: the mark goes.
    started.focus_group(first, cx);
    started.press("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.dirty_in(first, cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha edited"));
    assert!(!started.dirty_tab(first_tab, cx));
    assert!(!started.drawn((TAB_UNSAVED_ID, 0usize), cx));
}

/// Ctrl+T opens a tab after the last one holding one empty group, shows it
/// and gives its editor the keyboard; the empty group offers the finder.
#[gpui_kit::test]
fn new_tab_chord_adds_a_tab_with_an_empty_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);

    started.press("ctrl-t", cx);

    let tabs = started.tabs(cx);
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[0], first_tab);
    assert_eq!(started.shown_tab(cx), tabs[1]);
    let groups = started.groups_of(tabs[1], cx);
    assert_eq!(groups.len(), 1);
    let group = groups[0];
    assert_ne!(group, first);
    assert_eq!(started.active(cx), group);
    assert_eq!(started.note_in(group, cx), None);
    assert_eq!(started.text_in(group, cx), "");
    assert_eq!(started.label(tabs[1], cx), "Empty");
    assert_eq!(started.placeholder(cx).as_deref(), Some("No note open"));
    drive(&rt, cx, |cx| started.editor_focused(group, cx));
    // The first tab is untouched.
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));
    assert_eq!(started.text_in(first, cx), "alpha");
    cx.read(|cx| assert_eq!(started.workspace.read(cx).status.note, None));
    started.assert_states_in_step(cx);

    // The finder opens into the empty group.
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_note(path("c.md"), cx))
    });
    drive(&rt, cx, |cx| {
        started.note_in(group, cx) == Some(path("c.md"))
    });
    assert_eq!(started.labels(cx), ["a.md", "c.md"]);
}

/// Ctrl+W writes the unsaved text of every group of the shown tab, then
/// closes it; the tab before it shows, and the closed groups' states go.
#[gpui_kit::test]
fn close_tab_flushes_every_group_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);
    started.new_tab_with("b.md", cx);
    let b = started.active(cx);
    started.split_with("c.md", cx);
    let c = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    let closing = started.shown_tab(cx);
    assert_eq!(started.groups_of(closing, cx), [b, c]);
    started.set_text_in(b, "beta edited", cx);
    started.set_text_in(c, "gamma edited", cx);
    // Nothing written yet.
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
    assert_eq!(disk(&rt, &v, "c.md").as_deref(), Some("gamma"));

    started.press("ctrl-w", cx);
    drive(&rt, cx, |cx| started.tabs(cx).len() == 1);

    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta edited"));
    assert_eq!(disk(&rt, &v, "c.md").as_deref(), Some("gamma edited"));
    assert_eq!(started.tabs(cx), [first_tab]);
    assert_eq!(started.shown_tab(cx), first_tab);
    assert_eq!(started.active(cx), first);
    assert_eq!(started.text(cx), "alpha");
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert!(w.state(b).is_none());
        assert!(w.state(c).is_none());
        assert_eq!(w.status.note, Some(path("a.md")));
        assert!(!w.status.dirty);
    });
    started.assert_states_in_step(cx);
    drive(&rt, cx, |cx| started.editor_focused(first, cx));
    // One tab again: the strip is gone.
    assert!(!started.drawn(TAB_STRIP_ID, cx));
}

/// Ctrl+W on the last tab writes its text and leaves one empty group:
/// never zero groups, never zero tabs.
#[gpui_kit::test]
fn closing_the_last_tab_leaves_one_empty_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    let tab = started.shown_tab(cx);
    started.split_with("d.md", cx);
    let d = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    started.set_text_in(first, "alpha edited", cx);
    started.set_text_in(d, "delta edited", cx);

    started.press("ctrl-w", cx);
    drive(&rt, cx, |cx| {
        started.groups_of(tab, cx).len() == 1 && started.note_in(first, cx).is_none()
    });

    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha edited"));
    assert_eq!(disk(&rt, &v, "d.md").as_deref(), Some("delta edited"));
    assert_eq!(started.tabs(cx), [tab]);
    assert_eq!(started.groups_of(tab, cx), [first]);
    assert_eq!(started.active(cx), first);
    assert_eq!(started.text_in(first, cx), "");
    assert!(!started.dirty_in(first, cx));
    assert_eq!(started.label(tab, cx), "Empty");
    assert_eq!(started.placeholder(cx).as_deref(), Some("No note open"));
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert!(w.state(d).is_none());
        assert_eq!(w.status.note, None);
    });
    started.assert_states_in_step(cx);

    // Again: still one empty group.
    started.press("ctrl-w", cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert_eq!(started.tabs(cx), [tab]);
    assert_eq!(started.groups_of(tab, cx), [first]);
    started.assert_states_in_step(cx);
}

/// Ctrl+Tab shows the next tab and wraps from the last to the first;
/// Ctrl+Shift+Tab goes back and wraps the other way. The Footer and the
/// keyboard follow the shown tab's active group.
#[gpui_kit::test]
fn next_tab_chord_wraps(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let a = started.active(cx);
    started.new_tab_with("b.md", cx);
    let b = started.active(cx);
    started.new_tab_with("c.md", cx);
    let c = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    let tabs = started.tabs(cx);
    assert_eq!(tabs.len(), 3);
    assert_eq!(started.shown_tab(cx), tabs[2]);

    let note = |cx: &TestAppContext| cx.read(|cx| started.workspace.read(cx).status.note.clone());
    started.press("ctrl-tab", cx);
    assert_eq!(started.shown_tab(cx), tabs[0], "wraps to the first");
    assert_eq!(started.active(cx), a);
    assert_eq!(note(cx), Some(path("a.md")));
    assert_eq!(started.text(cx), "alpha");
    drive(&rt, cx, |cx| started.editor_focused(a, cx));
    started.press("ctrl-tab", cx);
    assert_eq!(started.shown_tab(cx), tabs[1]);
    assert_eq!(started.active(cx), b);
    assert_eq!(note(cx), Some(path("b.md")));

    started.press("ctrl-shift-tab", cx);
    assert_eq!(started.shown_tab(cx), tabs[0]);
    started.press("ctrl-shift-tab", cx);
    assert_eq!(started.shown_tab(cx), tabs[2], "wraps to the last");
    assert_eq!(started.active(cx), c);
    assert_eq!(note(cx), Some(path("c.md")));
    drive(&rt, cx, |cx| started.editor_focused(c, cx));
    started.assert_states_in_step(cx);
}

/// More tabs than fit: the strip scrolls sideways and keeps the shown
/// tab's label in view, at either end.
#[gpui_kit::test]
fn the_strip_keeps_the_shown_tab_in_view(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    for _ in 0..30 {
        started.press("ctrl-t", cx);
    }
    assert_eq!(started.tabs(cx).len(), 31);
    let in_view = |at: usize, cx: &mut TestAppContext| {
        cx.update_window(started.handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            let strip = window.find("tab-strip-labels").bounds();
            let label = window.find(tab_label_id(at)).bounds();
            label.left() >= strip.left() && label.right() <= strip.right()
        })
        .unwrap()
    };
    assert!(in_view(30, cx), "the newest tab, at the far end");
    assert!(
        !in_view(0, cx),
        "the strip is full: the first is scrolled away"
    );
    started.press("ctrl-tab", cx);
    assert_eq!(started.shown_tab(cx), started.tabs(cx)[0]);
    assert!(in_view(0, cx), "wrapped to the first: back in view");
    started.press("ctrl-shift-tab", cx);
    assert!(in_view(30, cx));
}

/// A write that fails during the close keeps the tab: the text stays in
/// the editor, unsaved; the flash says why; the group's tick and keyboard
/// come back; and the hold on the notes ends, so an open parked meanwhile
/// goes on.
#[cfg(unix)]
#[gpui_kit::test]
fn closing_a_tab_whose_flush_fails_keeps_the_tab_and_its_text(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let tick = v.settings.read().unwrap().autosave_interval_secs;
    let a = started.active(cx);
    started.new_tab_with("b.md", cx);
    let b = started.active(cx);
    let tab = started.shown_tab(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    let file = v.workspace.as_path().join("b.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    started.set_text_in(b, "beta edited", cx);

    started.press("ctrl-w", cx);
    // The flush runs: the notes are held, the group's tick stopped and its
    // keyboard blocked. An open asked for meanwhile waits.
    assert!(started.held(cx));
    assert_eq!(started.tick_in(b, cx), None);
    assert!(started.input_blocked(b, cx));
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_note_in(a, path("d.md"), cx))
    });
    assert!(started.wanted_in(a, cx), "the open waits for the close");
    drive(&rt, cx, |cx| !started.held(cx));
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save b.md:") && flash.ends_with("The tab stays open."),
        "{flash}"
    );
    assert_eq!(started.tabs(cx).len(), 2);
    assert_eq!(started.shown_tab(cx), tab);
    assert_eq!(started.active(cx), b);
    assert_eq!(started.note_in(b, cx), Some(path("b.md")));
    assert_eq!(started.text_in(b, cx), "beta edited");
    assert!(started.dirty_in(b, cx), "the text is still unsaved");
    assert!(started.dirty_tab(tab, cx));
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
    assert_eq!(started.tick_in(b, cx), Some(tick), "the tick is back");
    assert!(!started.input_blocked(b, cx), "the keyboard is back");
    // The parked open went on once the hold ended.
    drive(&rt, cx, |cx| started.note_in(a, cx) == Some(path("d.md")));
    started.assert_states_in_step(cx);

    // Writable again: the next close writes the text and closes the tab.
    started.press("ctrl-w", cx);
    drive(&rt, cx, |cx| started.tabs(cx).len() == 1);
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta edited"));
    started.assert_states_in_step(cx);
}

/// A note of the tab that cannot be let go yet refuses the close with a
/// flash: one still opening, or one another operation holds. The tab
/// stays and nothing is written.
#[gpui_kit::test]
fn closing_a_tab_while_a_note_is_held_refuses_with_a_flash(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let tick = v.settings.read().unwrap().autosave_interval_secs;

    // Opening: the new tab's note is still being read.
    started.new_tab_with("b.md", cx);
    let b = started.active(cx);
    assert!(!started.idle(cx), "b.md is still opening");
    started.press("ctrl-w", cx);
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("A note in this tab is still opening. Try again in a moment.")
    );
    assert_eq!(started.tabs(cx).len(), 2);
    assert!(!started.held(cx), "the refusal holds nothing");
    drive(&rt, cx, |cx| started.idle(cx));
    assert_eq!(started.note_in(b, cx), Some(path("b.md")));

    // Held: a file operation on another note runs.
    started.set_text_in(b, "beta edited", cx);
    let workspace = started.workspace.clone();
    cx.update_window(started.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| {
            w.request_file_op(
                FileOpRequest::Rename {
                    from: path("d.md"),
                    to: path("e.md"),
                },
                window,
                cx,
            )
        })
    })
    .unwrap();
    assert!(started.held(cx));
    started.press("ctrl-w", cx);
    assert_eq!(
        started.flash(cx).as_deref(),
        Some("Another operation on the note is still running. The tab stays open.")
    );
    assert_eq!(started.tabs(cx).len(), 2);
    assert_eq!(started.active(cx), b);
    assert_eq!(started.text_in(b, cx), "beta edited");
    assert!(started.dirty_in(b, cx));
    assert!(!started.input_blocked(b, cx));
    assert_eq!(
        started.tick_in(b, cx),
        Some(tick),
        "the tick was never stopped"
    );
    drive(&rt, cx, |cx| !started.held(cx));
    assert_eq!(
        disk(&rt, &v, "b.md").as_deref(),
        Some("beta"),
        "nothing written"
    );
    started.assert_states_in_step(cx);
}

/// A tab of two groups whose second flush cannot start (another flush of
/// that note is still out): the first group's flush, already begun, is
/// dropped unrun and the group gets its tick and keyboard back; the tab
/// stays and the flash says why. Once that other flush is gone the close
/// goes through.
#[gpui_kit::test]
fn partial_rollback_when_begin_note_flush_refuses_a_later_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let tick = v.settings.read().unwrap().autosave_interval_secs;
    started.new_tab_with("b.md", cx);
    let b = started.active(cx);
    started.split_with("c.md", cx);
    let c = started.active(cx);
    let tab = started.shown_tab(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert_eq!(started.groups_of(tab, cx), [b, c]);
    started.set_text_in(b, "beta edited", cx);
    started.set_text_in(c, "gamma edited", cx);
    // Another flush of c.md is out, not yet run.
    let outstanding = cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.begin_note_flush_in(c, cx))
    });
    assert!(outstanding.is_some());

    started.press("ctrl-w", cx);

    assert_eq!(
        started.flash(cx).as_deref(),
        Some("Another save is still running. The tab stays open.")
    );
    assert!(!started.held(cx), "no close runs");
    assert_eq!(started.tabs(cx).len(), 2);
    assert_eq!(started.groups_of(tab, cx), [b, c]);
    // The first group, begun and rolled back.
    assert_eq!(started.tick_in(b, cx), Some(tick), "b's tick is back");
    assert!(!started.input_blocked(b, cx), "b's keyboard is back");
    assert_eq!(started.text_in(b, cx), "beta edited");
    assert!(started.dirty_in(b, cx));
    assert_eq!(started.text_in(c, cx), "gamma edited");
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta"));
    assert_eq!(disk(&rt, &v, "c.md").as_deref(), Some("gamma"));

    // The other flush goes away unrun: the close now writes both.
    drop(outstanding);
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.block_input_in(c, false, cx))
    });
    started.press("ctrl-w", cx);
    drive(&rt, cx, |cx| started.tabs(cx).len() == 1);
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta edited"));
    assert_eq!(disk(&rt, &v, "c.md").as_deref(), Some("gamma edited"));
    started.assert_states_in_step(cx);
}
