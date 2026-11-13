//! Splits of the editor region (ADR-0058): the split and close chords on
//! the focused leaf (a slot, or the editor's active group), every group of
//! the shown tab drawn side by side with its own note, the group header,
//! the target marker, the region's minimum width, and the guard against a
//! group added to a tab whose close is under way. A real window over a
//! real temp vault, driven like the note tests (`start_tests::drive`).
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, ElementId, Focusable as _, Pixels, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor_region::{GroupShape, TabId};
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::{Panel, Side};
use kimun_notes::logic::layout_tree::{Axis, Focus, GroupId, Leaf, UnitRect};
use kimun_notes::logic::nav::DrawerView;
use tokio::runtime::Runtime;

use super::file_ops::FileOpRequest;
use super::group_header::{
    GROUP_CLOSE_ID, GROUP_HEADER_ID, GROUP_ID, GROUP_TARGET_ID, GROUP_UNSAVED_ID, group_element,
    group_title,
};
use super::modal::ModalKind;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use super::tab_strip::{TAB_CLOSING, TAB_STRIP_ID};
use super::tree_view::group_divider_id;
use crate::shell_layout::EDITOR_MIN_REMS;
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

/// The split chords and the close chord with the Linux/Windows keymap.
const SPLIT_RIGHT: &str = "alt-shift-r";
const SPLIT_DOWN: &str = "alt-shift-d";
const CLOSE: &str = "alt-shift-w";

/// A started window with this file's readers (a wrapper, so they cannot
/// collide with the other test files' `Started` helpers).
struct Win(Started);

impl std::ops::Deref for Win {
    type Target = Started;

    fn deref(&self) -> &Started {
        &self.0
    }
}

/// A vault holding [`NOTES`], started on `a.md` and driven until it shows
/// and FILES has listed the root.
fn started(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Win) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    let started = Win(open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    ));
    drive(rt, cx, |cx| {
        started.text(cx) == "alpha" && started.files_listed(cx)
    });
    (v, started)
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

    fn groups(&self, cx: &TestAppContext) -> Vec<GroupId> {
        cx.read(|cx| {
            let region = self.workspace.read(cx).layout.region();
            region.groups(region.shown())
        })
    }

    fn shape(&self, cx: &TestAppContext) -> GroupShape {
        cx.read(|cx| self.workspace.read(cx).layout.region().shape())
    }

    fn rects(&self, cx: &TestAppContext) -> Vec<(GroupId, UnitRect)> {
        cx.read(|cx| self.workspace.read(cx).layout.region().regions())
    }

    fn focus(&self, cx: &TestAppContext) -> Focus {
        cx.read(|cx| self.workspace.read(cx).layout.focused())
    }

    fn slots(&self, cx: &TestAppContext) -> usize {
        cx.read(|cx| self.workspace.read(cx).layout.slots().len())
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

    fn has_state(&self, group: GroupId, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).state(group).is_some())
    }

    fn empty_title_in(&self, group: GroupId, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .note_empty_state_in(group)
                .map(|e| e.title)
        })
    }

    fn title(&self, group: GroupId, cx: &TestAppContext) -> String {
        cx.read(|cx| group_title(self.workspace.read(cx), group, cx).to_string())
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
    }

    fn idle(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let w = self.workspace.read(cx);
            !w.note_held()
                && w.layout.region().all_groups().into_iter().all(|g| {
                    w.state(g).and_then(|s| s.notes.as_ref()).is_some_and(|h| {
                        !h.is_switching() && !h.is_open_wanted() && !h.session.is_open_pending()
                    })
                })
        })
    }

    fn held(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).note_held())
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

    fn panel(&self, cx: &TestAppContext) -> Panel {
        cx.read(|cx| self.workspace.read(cx).focused_panel())
    }

    /// The element `id` as last drawn (a fresh frame first), if visible.
    fn element(
        &self,
        id: impl Into<ElementId>,
        cx: &mut TestAppContext,
    ) -> Option<gpui_kit::test::ElementSnapshot> {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).filter(|e| e.visible())
        })
        .unwrap()
    }

    fn drawn(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        self.element(id, cx).is_some()
    }

    fn bounds(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> Bounds<Pixels> {
        self.element(id, cx).expect("drawn").bounds()
    }

    /// The header's spoken label: its title, and ", unsaved" with the dot.
    fn header_label(&self, at: usize, cx: &mut TestAppContext) -> Option<String> {
        self.element(group_element(GROUP_HEADER_ID, at), cx)
            .and_then(|e| e.label().map(str::to_string))
    }

    /// How many editors are drawn, and how many inside the group at `at`.
    fn editors_drawn(&self, within: Option<usize>, cx: &mut TestAppContext) -> usize {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            match within {
                None => window.find_all("editor").len(),
                Some(at) => window
                    .within(group_element(GROUP_ID, at))
                    .find_all("editor")
                    .len(),
            }
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

    fn click(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn split_with(&self, note: &str, cx: &mut TestAppContext) {
        let note = path(note);
        cx.update(|cx| {
            self.workspace
                .update(cx, |w, cx| w.open_note_in_split(note, Axis::Horizontal, cx))
        });
    }

    fn new_tab_with(&self, note: &str, cx: &mut TestAppContext) {
        let note = path(note);
        cx.update(|cx| {
            self.workspace
                .update(cx, |w, cx| w.open_note_in_new_tab(note, cx))
        });
    }

    fn focus_group(&self, group: GroupId, cx: &mut TestAppContext) {
        cx.update(|cx| self.workspace.update(cx, |w, cx| w.focus_group(group, cx)));
        cx.run_until_parked();
    }

    fn focus_editor(&self, cx: &mut TestAppContext) {
        let workspace = self.workspace.clone();
        cx.update_window(self.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.focus_panel(Panel::Editor, window, cx))
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn files_listed(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let Some(files) = self.workspace.read(cx).files.clone() else {
                return false;
            };
            files
                .read(cx)
                .model()
                .core()
                .is_some_and(|core| !core.is_loading() && core.visible_len() > 0)
        })
    }

    /// The name of FILES' selected row, for a note.
    fn files_selected(&self, cx: &TestAppContext) -> Option<String> {
        cx.read(|cx| {
            let files = self.workspace.read(cx).files.clone()?;
            let pane = files.read(cx);
            match pane.model().core()?.selected_row()? {
                FileListEntry::Note { filename, .. } => Some(filename.clone()),
                _ => None,
            }
        })
    }

    fn run_file_op(&self, rt: &Runtime, request: FileOpRequest, cx: &mut TestAppContext) {
        let workspace = self.workspace.clone();
        cx.update_window(self.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.request_file_op(request, window, cx))
        })
        .unwrap();
        drive(rt, cx, |cx| {
            cx.read(|cx| self.workspace.read(cx).file_task.is_none()) && self.idle(cx)
        });
    }

    fn rem(&self, cx: &mut TestAppContext) -> f32 {
        cx.update_window(self.handle, |_, window, _| f32::from(window.rem_size()))
            .unwrap()
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.5
}

/// The split chord with the editor focused splits the active group: an
/// empty group comes right of it (down for the other chord), becomes the
/// active group with the keyboard, and offers the note finder; the other
/// group keeps its note. Still one tab, so no strip.
#[gpui_kit::test]
fn split_chord_on_a_group_adds_an_empty_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    assert_eq!(started.panel(cx), Panel::Editor);

    started.press(SPLIT_RIGHT, cx);
    let groups = started.groups(cx);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0], first);
    let right = groups[1];
    assert_eq!(started.active(cx), right);
    assert_eq!(
        started.shape(cx),
        GroupShape::Split {
            axis: Axis::Horizontal,
            children: vec![
                (0.5, GroupShape::Group(first)),
                (0.5, GroupShape::Group(right))
            ],
        }
    );
    assert_eq!(started.note_in(right, cx), None);
    assert_eq!(started.text_in(right, cx), "");
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));
    assert_eq!(started.text_in(first, cx), "alpha");
    assert_eq!(started.focus(cx), Focus::Leaf(Leaf::Editor));
    drive(&rt, cx, |cx| started.editor_focused(right, cx));
    // The empty group's hint offers the finder.
    assert_eq!(
        started.empty_title_in(right, cx).as_deref(),
        Some("No note open")
    );
    cx.read(|cx| {
        let hint = started.workspace.read(cx).chord_labels.no_note_hint.clone();
        assert!(hint.contains("Ctrl+O"), "{hint}");
    });
    assert_eq!(started.tabs(cx).len(), 1);
    assert!(!started.drawn(TAB_STRIP_ID, cx));
    // The Footer names no note: the empty group is active.
    cx.read(|cx| assert_eq!(started.workspace.read(cx).status.note, None));

    started.press(SPLIT_DOWN, cx);
    let groups = started.groups(cx);
    assert_eq!(groups.len(), 3);
    let below = groups[2];
    assert_eq!(started.active(cx), below);
    assert_eq!(
        started.shape(cx),
        GroupShape::Split {
            axis: Axis::Horizontal,
            children: vec![
                (0.5, GroupShape::Group(first)),
                (
                    0.5,
                    GroupShape::Split {
                        axis: Axis::Vertical,
                        children: vec![
                            (0.5, GroupShape::Group(right)),
                            (0.5, GroupShape::Group(below))
                        ],
                    }
                ),
            ],
        }
    );
    drive(&rt, cx, |cx| started.editor_focused(below, cx));
    started.assert_states_in_step(cx);
}

/// On a slot the split chord still splits the slot (the chooser asks what
/// the new slot holds) and the close chord closes it; the editor's groups
/// do not change. A regression lock: it passed before splits existed.
#[gpui_kit::test]
fn split_chord_on_a_slot_still_splits_the_slot(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    let slots = started.slots(cx);
    started.press("ctrl-e", cx);
    assert_eq!(started.panel(cx), Panel::Drawer(Side::Left));

    started.press(SPLIT_DOWN, cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::ViewChooser));
    started.press("enter", cx);
    assert_eq!(started.slots(cx), slots + 1);
    assert_eq!(started.groups(cx), vec![first]);
    assert!(matches!(started.focus(cx), Focus::Leaf(Leaf::Slot(_))));

    started.press(CLOSE, cx);
    assert_eq!(started.slots(cx), slots);
    assert_eq!(started.groups(cx), vec![first]);
    assert_eq!(started.note_in(first, cx), Some(path("a.md")));

    // From the rail the chords do nothing.
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, _| w.layout.focus(Focus::Rail))
    });
    started.press(SPLIT_RIGHT, cx);
    assert_eq!(started.groups(cx), vec![first]);
    assert_eq!(started.slots(cx), slots);
}

/// With one group nothing changes: no header, no group cell, no target
/// marker even with the keyboard in a slot, and one editor drawn. A
/// regression lock: one group looks as it did before splits.
#[gpui_kit::test]
fn header_hidden_with_one_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    assert!(started.drawn("editor-panel", cx));
    assert!(!started.drawn(group_element(GROUP_HEADER_ID, 0), cx));
    assert!(!started.drawn(group_element(GROUP_ID, 0), cx));
    assert!(!started.drawn(group_element(GROUP_CLOSE_ID, 0), cx));
    assert!(!started.drawn(group_divider_id(0), cx));
    assert_eq!(started.editors_drawn(None, cx), 1);
    started.press("ctrl-e", cx);
    assert!(!started.drawn(group_element(GROUP_TARGET_ID, 0), cx));
    // A second tab of one group: still no header.
    started.new_tab_with("b.md", cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert!(started.drawn(TAB_STRIP_ID, cx));
    assert!(!started.drawn(group_element(GROUP_HEADER_ID, 0), cx));
}

/// Two groups: each draws a header with its title (the note's name, or
/// `Empty`), the unsaved dot while its text is unsaved, and a close
/// button that closes that group (its note written first).
#[gpui_kit::test]
fn header_shows_title_unsaved_marker_and_close_with_two_groups(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    started.split_with("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));

    assert_eq!(started.header_label(0, cx).as_deref(), Some("a.md"));
    assert_eq!(started.header_label(1, cx).as_deref(), Some("b.md"));
    assert_eq!(started.title(first, cx), "a.md");
    assert!(started.drawn(group_element(GROUP_CLOSE_ID, 0), cx));
    assert!(started.drawn(group_element(GROUP_CLOSE_ID, 1), cx));
    assert!(!started.drawn(group_element(GROUP_UNSAVED_ID, 0), cx));
    assert!(!started.drawn(group_element(GROUP_UNSAVED_ID, 1), cx));

    started.set_text_in(first, "alpha edited", cx);
    assert_eq!(
        started.header_label(0, cx).as_deref(),
        Some("a.md, unsaved")
    );
    assert!(started.drawn(group_element(GROUP_UNSAVED_ID, 0), cx));
    assert!(!started.drawn(group_element(GROUP_UNSAVED_ID, 1), cx));

    // An empty group is called `Empty`.
    started.focus_editor(cx);
    started.press(SPLIT_RIGHT, cx);
    let third = started.active(cx);
    assert_eq!(started.header_label(2, cx).as_deref(), Some("Empty"));
    assert_eq!(started.title(third, cx), "Empty");

    // The first group's close button closes it, its text written, and
    // nothing else; the active group stays active.
    started.click(group_element(GROUP_CLOSE_ID, 0), cx);
    drive(&rt, cx, |cx| {
        started.groups(cx).len() == 2 && started.idle(cx)
    });
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha edited"));
    assert_eq!(started.groups(cx), vec![second, third]);
    assert!(!started.has_state(first, cx));
    assert_eq!(started.active(cx), third);
    assert_eq!(started.header_label(0, cx).as_deref(), Some("b.md"));
    started.assert_states_in_step(cx);

    // A click on a header makes its group active.
    started.click(group_element(GROUP_HEADER_ID, 0), cx);
    assert_eq!(started.active(cx), second);

    // Down to one group: the header goes.
    started.click(group_element(GROUP_CLOSE_ID, 1), cx);
    drive(&rt, cx, |cx| {
        started.groups(cx).len() == 1 && started.idle(cx)
    });
    assert_eq!(started.groups(cx), vec![second]);
    assert!(!started.drawn(group_element(GROUP_HEADER_ID, 0), cx));
}

/// Every group of the shown tab is drawn, side by side, each with its own
/// editor showing its own note; a hairline divider between them resizes
/// the pair.
#[gpui_kit::test]
fn every_group_draws_its_own_note_side_by_side(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    started.split_with("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));

    assert_eq!(started.editors_drawn(None, cx), 2);
    assert_eq!(started.editors_drawn(Some(0), cx), 1);
    assert_eq!(started.editors_drawn(Some(1), cx), 1);
    assert_eq!(started.text_in(first, cx), "alpha");
    assert_eq!(started.text_in(second, cx), "beta");
    let left = started.bounds(group_element(GROUP_ID, 0), cx);
    let right = started.bounds(group_element(GROUP_ID, 1), cx);
    let panel = started.bounds("editor-panel", cx);
    // Side by side, the same height, sharing the Panel by weight (0.5 each,
    // the 1 px divider between them).
    assert!(left.right() <= right.left(), "{left:?} {right:?}");
    assert!(close(
        f32::from(left.size.width),
        f32::from(right.size.width)
    ));
    assert!(close(
        f32::from(left.size.height),
        f32::from(right.size.height)
    ));
    // The Panel's margin (4) and border (1) on each side.
    assert!(close(
        f32::from(left.size.width + right.size.width) + 1.0,
        f32::from(panel.size.width) - 10.0
    ));
    assert!(started.drawn(group_divider_id(0), cx));

    // A drag on the divider moves the boundary: the left group takes 0.3
    // of the pair.
    let target = f64::from(f32::from(left.left()))
        + 0.3 * f64::from(f32::from(left.size.width + right.size.width));
    let workspace = started.workspace.clone();
    cx.update_window(started.handle, |_, window, cx| {
        workspace.update(cx, |w, cx| {
            w.on_divider_drag(
                super::tree_view::DividerAt::Group(first),
                target,
                window,
                cx,
            )
        })
    })
    .unwrap();
    let rects = started.rects(cx);
    assert_eq!(rects[0].0, first);
    assert!((rects[0].1.width() - 0.3).abs() < 0.01, "{:?}", rects[0].1);
    assert!((rects[1].1.width() - 0.7).abs() < 0.01);
}

/// Closing a group (the close chord on the editor) writes its note first;
/// a neighbour takes its space, and the group before it becomes active
/// with the keyboard.
#[gpui_kit::test]
fn closing_a_group_gives_its_space_to_a_neighbor(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    started.split_with("b.md", cx);
    let second = started.active(cx);
    started.split_with("c.md", cx);
    let third = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert_eq!(started.groups(cx), vec![first, second, third]);
    let before = started.rects(cx);
    started.focus_group(second, cx);
    started.set_text_in(second, "beta edited", cx);

    started.press(CLOSE, cx);
    drive(&rt, cx, |cx| {
        started.groups(cx).len() == 2 && started.idle(cx)
    });

    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta edited"));
    assert_eq!(started.groups(cx), vec![first, third]);
    assert!(!started.has_state(second, cx));
    assert_eq!(started.active(cx), first);
    drive(&rt, cx, |cx| started.editor_focused(first, cx));
    cx.read(|cx| assert_eq!(started.workspace.read(cx).status.note, Some(path("a.md"))));
    // Before: a half and two quarters. The closed quarter went to its
    // neighbour before it; the last group did not move.
    let widths: Vec<f64> = before.iter().map(|(_, r)| r.width()).collect();
    assert_eq!(widths, vec![0.5, 0.25, 0.25]);
    let rects = started.rects(cx);
    assert_eq!(rects.len(), 2);
    assert_eq!((rects[0].0, rects[1].0), (first, third));
    assert_eq!(rects[0].1, UnitRect::new(0.0, 0.0, 0.75, 1.0));
    assert_eq!(rects[1].1, before[2].1);
    started.assert_states_in_step(cx);
}

/// Closing the last group of a tab closes the tab (its note written
/// first); closing the last group of the last tab leaves it, empty.
#[gpui_kit::test]
fn closing_the_last_group_closes_the_tab(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first = started.active(cx);
    let first_tab = started.shown_tab(cx);
    started.new_tab_with("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    started.set_text_in(second, "beta edited", cx);

    started.press(CLOSE, cx);
    drive(&rt, cx, |cx| {
        started.tabs(cx).len() == 1 && started.idle(cx)
    });
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta edited"));
    assert_eq!(started.tabs(cx), vec![first_tab]);
    assert!(!started.has_state(second, cx));
    assert_eq!(started.active(cx), first);
    assert_eq!(started.text(cx), "alpha");

    started.focus_editor(cx);
    started.set_text_in(first, "alpha edited", cx);
    started.press(CLOSE, cx);
    drive(&rt, cx, |cx| {
        started.note_in(first, cx).is_none() && started.idle(cx)
    });
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha edited"));
    assert_eq!(started.tabs(cx), vec![first_tab]);
    assert_eq!(started.groups(cx), vec![first]);
    assert_eq!(started.text_in(first, cx), "");
    assert_eq!(
        started.empty_title_in(first, cx).as_deref(),
        Some("No note open")
    );
    started.assert_states_in_step(cx);
}

/// While the keyboard is in a slot, the active group of a split tab wears
/// the target marker; with the keyboard in the editor nothing does.
#[gpui_kit::test]
fn target_marker_shows_on_the_active_group_while_focus_is_in_a_slot(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    started.split_with("b.md", cx);
    drive(&rt, cx, |cx| started.idle(cx));
    assert_eq!(started.focus(cx), Focus::Leaf(Leaf::Editor));
    assert!(!started.drawn(group_element(GROUP_TARGET_ID, 0), cx));
    assert!(!started.drawn(group_element(GROUP_TARGET_ID, 1), cx));

    started.press("ctrl-e", cx);
    assert_eq!(started.panel(cx), Panel::Drawer(Side::Left));
    assert!(!started.drawn(group_element(GROUP_TARGET_ID, 0), cx));
    let marker = started.bounds(group_element(GROUP_TARGET_ID, 1), cx);
    let group = started.bounds(group_element(GROUP_ID, 1), cx);
    // A bar along the top of the group.
    assert!(close(f32::from(marker.top()), f32::from(group.top())));
    assert!(close(
        f32::from(marker.size.width),
        f32::from(group.size.width)
    ));

    // The keyboard back in the editor: no marker.
    started.press("escape", cx);
    assert_eq!(started.panel(cx), Panel::Editor);
    assert!(!started.drawn(group_element(GROUP_TARGET_ID, 1), cx));
}

/// The marker bends with the Panel's rounded corners instead of sticking
/// out past them: it is the top border of a box over the whole group, and
/// only the top corners the group shares with the Panel are rounded (none
/// below the tab strip, none for a group under another). GPUI's test
/// snapshot exposes no corner radius, so the decision is checked here and
/// the box's extent in the window; the curve itself is checked by eye.
#[gpui_kit::test]
fn target_marker_rounds_only_the_panel_corners(cx: &mut TestAppContext) {
    use super::group_header::marker_corners;
    let full = UnitRect::new(0.0, 0.0, 1.0, 1.0);
    let left = UnitRect::new(0.0, 0.0, 0.5, 1.0);
    let right = UnitRect::new(0.5, 0.0, 0.5, 1.0);
    let lower_left = UnitRect::new(0.0, 0.5, 0.5, 0.5);
    assert_eq!(marker_corners(full, false), (true, true));
    assert_eq!(marker_corners(left, false), (true, false));
    assert_eq!(marker_corners(right, false), (false, true));
    assert_eq!(marker_corners(lower_left, false), (false, false));
    assert_eq!(
        marker_corners(left, true),
        (false, false),
        "under the strip"
    );

    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    started.split_with("b.md", cx);
    drive(&rt, cx, |cx| started.idle(cx));
    started.press("ctrl-e", cx);
    let marker = started.bounds(group_element(GROUP_TARGET_ID, 1), cx);
    let group = started.bounds(group_element(GROUP_ID, 1), cx);
    assert_eq!(marker, group, "the box covers the group, inside the Panel");
}

/// A plain open from FILES replaces the note of the marked (active)
/// group; the other group keeps its note. The open is a regression lock
/// (it passed before); the marker check is new.
#[gpui_kit::test]
fn plain_open_from_files_replaces_the_marked_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    started.split_with("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));

    started.press("ctrl-e", cx);
    assert!(started.drawn(group_element(GROUP_TARGET_ID, 1), cx));
    for _ in 0..8 {
        if started.files_selected(cx).as_deref() == Some("c.md") {
            break;
        }
        started.press("down", cx);
    }
    assert_eq!(started.files_selected(cx).as_deref(), Some("c.md"));
    started.press("enter", cx);
    // FILES opens by the row's (absolute) path.
    let holds = |group: GroupId, note: &str, cx: &TestAppContext| {
        started
            .note_in(group, cx)
            .is_some_and(|p| p.is_like(&path(note)))
    };
    drive(&rt, cx, |cx| holds(second, "c.md", cx) && started.idle(cx));
    assert_eq!(started.groups(cx), vec![first, second]);
    assert!(holds(first, "a.md", cx));
    assert_eq!(started.text_in(second, cx), "gamma");
    assert_eq!(started.active(cx), second);
    assert_eq!(started.panel(cx), Panel::Editor);
    cx.read(|cx| {
        assert_eq!(
            started.workspace.read(cx).locate_note(&path("b.md")),
            None,
            "b.md is open nowhere"
        )
    });
}

/// The region's minimum width is one editor's minimum per group of the
/// widest row: a split tab collapses the slots first, and in a window too
/// narrow for its groups they share the width; none collapses.
#[gpui_kit::test]
fn groups_never_collapse_in_a_narrow_window(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let rem = started.rem(cx);
    let height = 700.;
    cx.simulate_window_resize(
        started.handle,
        gpui_kit::size(gpui_kit::px(700.), gpui_kit::px(height)),
    );
    // One group: FILES fits beside it.
    assert!(started.drawn(super::slot_id(DrawerView::Files), cx));
    cx.read(|cx| assert!(started.workspace.read(cx).collapsed.is_empty()));

    started.press(SPLIT_RIGHT, cx);
    assert_eq!(started.groups(cx).len(), 2);
    // Two groups need 2 x 20 rems: FILES collapses, both groups keep
    // their minimum.
    assert!(!started.drawn(super::slot_id(DrawerView::Files), cx));
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        let files = w.layout.view_slot(&DrawerView::Files).unwrap();
        assert!(w.collapsed.contains(&Leaf::Slot(files)));
        assert!(!w.collapsed.contains(&Leaf::Editor));
    });
    for at in 0..2 {
        let group = started.bounds(group_element(GROUP_ID, at), cx);
        assert!(
            f32::from(group.size.width) >= EDITOR_MIN_REMS * rem - 0.5,
            "{at}: {group:?}"
        );
    }

    // Narrower than both minimums: both still drawn, sharing the width.
    cx.simulate_window_resize(
        started.handle,
        gpui_kit::size(gpui_kit::px(500.), gpui_kit::px(height)),
    );
    let left = started.bounds(group_element(GROUP_ID, 0), cx);
    let right = started.bounds(group_element(GROUP_ID, 1), cx);
    assert!(f32::from(left.size.width) > 150., "{left:?}");
    assert!(close(
        f32::from(left.size.width),
        f32::from(right.size.width)
    ));
    assert!(started.drawn(group_element(GROUP_HEADER_ID, 1), cx));
}

/// While a tab's close writes its notes, no group is added to it: the
/// split chord and an open into a split are refused with a flash, and the
/// tab closes with exactly the groups it had.
#[gpui_kit::test]
fn a_tab_mid_close_takes_no_new_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let first_tab = started.shown_tab(cx);
    started.new_tab_with("b.md", cx);
    let second = started.active(cx);
    drive(&rt, cx, |cx| started.idle(cx));
    started.set_text_in(second, "beta edited", cx);

    started.press("ctrl-w", cx);
    assert!(started.held(cx), "the close's flush runs");
    assert_eq!(started.tabs(cx).len(), 2);

    started.press(SPLIT_RIGHT, cx);
    assert_eq!(started.flash(cx).as_deref(), Some(TAB_CLOSING));
    assert_eq!(started.groups(cx), vec![second]);
    started.split_with("c.md", cx);
    assert_eq!(started.groups(cx), vec![second]);
    assert_eq!(started.flash(cx).as_deref(), Some(TAB_CLOSING));

    drive(&rt, cx, |cx| {
        started.tabs(cx).len() == 1 && started.idle(cx)
    });
    assert_eq!(started.tabs(cx), vec![first_tab]);
    assert_eq!(disk(&rt, &v, "b.md").as_deref(), Some("beta edited"));
    cx.read(|cx| {
        assert_eq!(started.workspace.read(cx).locate_note(&path("c.md")), None);
    });
    started.assert_states_in_step(cx);
}

/// "X was deleted" shows only in the group whose note was deleted: a new
/// empty group says "No note open", and the message is still in its own
/// group when that one is active again.
#[gpui_kit::test]
fn the_deleted_message_stays_in_its_own_group(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let first = started.active(cx);
    started.run_file_op(&rt, FileOpRequest::Delete(path("a.md")), cx);
    assert_eq!(started.placeholder(cx).as_deref(), Some("a.md was deleted"));

    started.focus_editor(cx);
    started.press(SPLIT_RIGHT, cx);
    let second = started.active(cx);
    assert_ne!(second, first);
    assert_eq!(started.placeholder(cx).as_deref(), Some("No note open"));
    assert_eq!(
        started.empty_title_in(first, cx).as_deref(),
        Some("a.md was deleted")
    );
    started.focus_group(first, cx);
    assert_eq!(started.placeholder(cx).as_deref(), Some("a.md was deleted"));

    // A note opened there clears it.
    cx.update(|cx| {
        started
            .workspace
            .update(cx, |w, cx| w.open_note(path("b.md"), cx))
    });
    drive(&rt, cx, |cx| {
        started.note_in(first, cx) == Some(path("b.md")) && started.idle(cx)
    });
    cx.read(|cx| assert!(started.workspace.read(cx).note_gone.is_none()));
}

/// Regression lock (final review I1): Alt+Arrow between the groups of a
/// tab makes the other group active AND moves the window to its note, as
/// a click on the group does: the Footer's note and dirty state and FILES'
/// open-note mark follow, zoomed or not.
#[gpui_kit::test]
fn directional_focus_between_groups_syncs_the_window(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started(cx, &rt);
    let left = started.active(cx);
    started.split_with("b.md", cx);
    drive(&rt, cx, |cx| {
        started.note_in(started.active(cx), cx) == Some(path("b.md")) && started.idle(cx)
    });
    let right = started.active(cx);
    assert_ne!(left, right);
    started.set_text_in(right, "beta edited", cx);
    started.focus_editor(cx);
    let footer = |cx: &TestAppContext| {
        cx.read(|cx| {
            let w = started.workspace.read(cx);
            (w.status.note.clone(), w.status.dirty)
        })
    };
    let marked = |cx: &TestAppContext| {
        cx.read(|cx| {
            let files = started.workspace.read(cx).files.clone()?;
            files.read(cx).model().open_note().cloned()
        })
    };
    assert_eq!(footer(cx), (Some(path("b.md")), true));

    for zoomed in [false, true] {
        if zoomed {
            started.press("ctrl-shift-m", cx);
            let zoom = cx.read(|cx| started.workspace.read(cx).layout.zoomed());
            assert_eq!(zoom, Some(Leaf::Editor));
        }
        started.press("alt-left", cx);
        assert_eq!(started.active(cx), left, "zoomed={zoomed}");
        assert_eq!(footer(cx), (Some(path("a.md")), false), "zoomed={zoomed}");
        assert!(
            marked(cx).is_some_and(|m| m.is_like(&path("a.md"))),
            "FILES marks a.md, zoomed={zoomed}"
        );
        started.press("alt-right", cx);
        assert_eq!(started.active(cx), right, "zoomed={zoomed}");
        assert_eq!(footer(cx), (Some(path("b.md")), true), "zoomed={zoomed}");
        assert!(
            marked(cx).is_some_and(|m| m.is_like(&path("b.md"))),
            "FILES marks b.md, zoomed={zoomed}"
        );
    }
}

/// Regression lock (final review I5): the search browser's flush lands on
/// the group it flushed, not on whichever group is active by then. Begun
/// on group A, with group B made active before it lands: A gets its
/// keyboard and its autosave tick back and its text is written.
#[gpui_kit::test]
fn the_search_open_flush_lands_on_the_group_it_flushed(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started(cx, &rt);
    let tick = v.settings.read().unwrap().autosave_interval_secs;
    let a = started.active(cx);
    started.split_with("b.md", cx);
    drive(&rt, cx, |cx| {
        started.note_in(started.active(cx), cx) == Some(path("b.md")) && started.idle(cx)
    });
    let b = started.active(cx);
    started.focus_group(a, cx);
    started.set_text_in(a, "alpha edited", cx);
    let blocked = |g: GroupId, cx: &TestAppContext| {
        cx.read(|cx| {
            let editor = started.workspace.read(cx).state(g).unwrap().editor.clone();
            editor.read(cx).input_blocked
        })
    };
    let tick_of = |g: GroupId, cx: &TestAppContext| {
        cx.read(|cx| {
            started
                .workspace
                .read(cx)
                .state(g)
                .and_then(|s| s.notes.as_ref())
                .and_then(|h| h.session.autosave_interval())
        })
    };
    let workspace = started.workspace.clone();
    let waits = cx
        .update_window(started.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.search_open(path("c.md"), None, window, cx))
        })
        .unwrap();
    assert!(waits, "the browser waits for the flush of a.md");
    assert!(blocked(a, cx));
    // B becomes active before the flush lands.
    cx.update(|cx| started.workspace.update(cx, |w, cx| w.focus_group(b, cx)));
    drive(&rt, cx, |cx| {
        cx.read(|cx| started.workspace.read(cx).search_flush.is_none()) && started.idle(cx)
    });
    assert!(!blocked(a, cx), "A, the flushed group, takes typing again");
    assert_eq!(tick_of(a, cx), Some(tick), "A's tick is back");
    assert!(!blocked(b, cx));
    assert_eq!(disk(&rt, &v, "a.md").as_deref(), Some("alpha edited"));
}
