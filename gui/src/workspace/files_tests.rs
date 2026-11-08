//! UI integration tests for the FILES drawer view (M5b-2 G3): a real
//! `Workspace` on a real temp vault, driven through the window's keys and
//! pointer, checked through the model the view draws and on disk.
//!
//! Like the startup tests, each test owns a current-thread tokio runtime and
//! drives it between GPUI's `run_until_parked` (see `start_tests::drive`).
#![cfg(test)]

use gpui_kit::Focusable as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::{Panel, Region, Side};
use kimun_notes::logic::message::{AppMessage, FileOp};
use kimun_notes::logic::nav::DrawerView;
use tokio::runtime::Runtime;

use super::files::{FILES_ID, row_id};
use super::modal::ModalKind;
use super::start_tests::{Started, drive, open_started, runtime, send, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// A short name for a row, as the tests write them: `..`, `name/`, a
/// note's file name, `+name` for the create row.
fn short(row: &FileListEntry) -> String {
    match row {
        FileListEntry::Up { .. } => "..".into(),
        FileListEntry::Directory { name, .. } => format!("{name}/"),
        FileListEntry::Note { filename, .. } => filename.clone(),
        FileListEntry::Attachment { filename, .. } => filename.clone(),
        FileListEntry::CreateNote { filename, .. } => format!("+{filename}"),
    }
}

impl Started {
    fn files<R>(&self, cx: &TestAppContext, f: impl FnOnce(&super::FilesPane) -> R) -> R {
        cx.read(|cx| {
            let files = self.workspace.read(cx).files.clone().expect("FILES");
            f(files.read(cx))
        })
    }

    /// The rows FILES shows, top to bottom.
    fn rows(&self, cx: &TestAppContext) -> Vec<String> {
        self.files(cx, |pane| {
            pane.model()
                .core()
                .map(|core| core.visible_rows().into_iter().map(short).collect())
                .unwrap_or_default()
        })
    }

    fn selected(&self, cx: &TestAppContext) -> Option<String> {
        self.files(cx, |pane| {
            pane.model()
                .core()
                .and_then(|core| core.selected_row())
                .map(short)
        })
    }

    fn dir(&self, cx: &TestAppContext) -> VaultPath {
        self.files(cx, |pane| pane.current_dir().clone())
    }

    fn query(&self, cx: &TestAppContext) -> String {
        cx.read(|cx| {
            let files = self.workspace.read(cx).files.clone().unwrap();
            files.read(cx).query(cx)
        })
    }

    /// The note rows marked open.
    fn marked(&self, cx: &TestAppContext) -> Vec<String> {
        self.files(cx, |pane| {
            pane.model()
                .core()
                .map(|core| {
                    core.visible_rows()
                        .into_iter()
                        .filter(|r| matches!(r, FileListEntry::Note { is_open: true, .. }))
                        .map(short)
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn title_of(&self, filename: &str, cx: &TestAppContext) -> Option<String> {
        self.files(cx, |pane| {
            pane.model().core().and_then(|core| {
                core.rows().iter().find_map(|r| match r {
                    FileListEntry::Note {
                        filename: f, title, ..
                    } if f == filename => Some(title.clone()),
                    _ => None,
                })
            })
        })
    }

    /// The element id of the row `name` (see [`short`]).
    fn row(&self, name: &str, cx: &TestAppContext) -> String {
        self.files(cx, |pane| {
            pane.model()
                .core()
                .and_then(|core| {
                    core.visible_rows()
                        .into_iter()
                        .find(|r| short(r) == name)
                        .map(|r| row_id(r).to_string())
                })
                .unwrap_or_else(|| panic!("no row {name}"))
        })
    }

    fn focused(&self, cx: &TestAppContext) -> Panel {
        cx.read(|cx| self.workspace.read(cx).focused_panel())
    }

    fn press_key(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
    }

    fn type_in(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
    }

    fn click_row(&self, name: &str, cx: &mut TestAppContext) {
        let id = self.row(name, cx);
        cx.update_window(self.handle, |_, window, cx| window.click(id, cx))
            .unwrap();
    }

    fn double_click_row(&self, name: &str, cx: &mut TestAppContext) {
        let id = self.row(name, cx);
        cx.update_window(self.handle, |_, window, cx| window.double_click(id, cx))
            .unwrap();
    }

    fn open_in_editor(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| self.workspace.read(cx).status.note.clone())
    }

    fn listed(&self, cx: &TestAppContext) -> bool {
        self.files(cx, |pane| {
            pane.model()
                .core()
                .is_some_and(|core| !core.is_loading() && core.visible_len() > 0)
        })
    }
}

const NOTES: &[(&str, &str)] = &[
    ("top.md", "Top"),
    ("sub/a.md", "Alpha"),
    ("sub/b.md", "Bravo"),
    ("sub/deeper/c.md", "Charlie"),
];

/// Started on `first` (a note named on the command line), driven until its
/// text shows and FILES has listed its folder.
fn started_on(cx: &mut TestAppContext, rt: &Runtime, first: &str) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(NOTES, &[]));
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path(first))),
        v.settings.clone(),
    );
    let want = NOTES.iter().find(|(p, _)| *p == first).unwrap().1;
    drive(rt, cx, |cx| started.text(cx) == want && started.listed(cx));
    (v, started)
}

/// FILES gives the keyboard to its filter (`OpenFileBrowser`).
fn focus_files(started: &Started, cx: &mut TestAppContext) {
    started.press_key("ctrl-e", cx);
    assert_eq!(started.focused(cx), Panel::Drawer(Side::Left));
}

// ── the first listing and the marker ─────────────────────────────────────

/// The first note opened picks FILES' folder, and its row is marked open;
/// `..` leads (the test config does not group folders first).
#[gpui_kit::test]
fn the_first_note_opened_lists_its_folder_and_is_marked(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "sub/a.md");
    assert!(started.dir(cx).is_like(&path("sub")));
    assert_eq!(started.rows(cx), vec!["..", "a.md", "b.md", "deeper/"]);
    assert_eq!(started.marked(cx), vec!["a.md"]);
    assert!(started.shows(FILES_ID, cx));
    let a = started.row("a.md", cx);
    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(a.clone()).visible(), "the row is drawn");
    })
    .unwrap();
}

/// An empty history lands on the root: FILES lists it.
#[gpui_kit::test]
fn an_empty_history_lists_the_root(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(NOTES, &[]));
    let started = open_started(cx, vault_start(&v, FirstNote::Restore), v.settings.clone());
    drive(&rt, cx, |cx| started.listed(cx));
    assert!(started.dir(cx).is_root_or_empty());
    assert_eq!(started.rows(cx), vec!["sub/", "top.md"]);
}

// ── keys ─────────────────────────────────────────────────────────────────

/// Up/Down move the selection; Enter on a note opens it, gives the editor
/// the keyboard, and the marker follows the open note.
#[gpui_kit::test]
fn the_keys_select_and_enter_opens_the_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "sub/a.md");
    focus_files(&started, cx);
    assert_eq!(started.selected(cx).as_deref(), Some(".."));
    started.press_key("down", cx);
    started.press_key("down", cx);
    started.press_key("down", cx);
    assert_eq!(started.selected(cx).as_deref(), Some("deeper/"));
    started.press_key("down", cx);
    assert_eq!(
        started.selected(cx).as_deref(),
        Some("deeper/"),
        "stops at the end"
    );
    started.press_key("up", cx);
    assert_eq!(started.selected(cx).as_deref(), Some("b.md"));
    started.press_key("enter", cx);
    assert_eq!(started.focused(cx), Panel::Editor);
    drive(&rt, cx, |cx| started.text(cx) == "Bravo");
    assert!(
        started
            .open_in_editor(cx)
            .unwrap()
            .is_like(&path("sub/b.md"))
    );
    assert_eq!(started.marked(cx), vec!["b.md"], "the marker follows");
}

/// Enter on a folder shows it; Left with an empty filter goes back up,
/// and Backspace there leaves the folder alone.
#[gpui_kit::test]
fn enter_goes_into_a_folder_and_left_goes_back_up(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "sub/a.md");
    focus_files(&started, cx);
    for _ in 0..3 {
        started.press_key("down", cx);
    }
    assert_eq!(started.selected(cx).as_deref(), Some("deeper/"));
    started.press_key("enter", cx);
    assert!(started.dir(cx).is_like(&path("sub/deeper")));
    assert_eq!(
        started.focused(cx),
        Panel::Drawer(Side::Left),
        "FILES keeps the keyboard"
    );
    drive(&rt, cx, |cx| started.listed(cx));
    assert_eq!(started.rows(cx), vec!["..", "c.md"]);
    started.press_key("left", cx);
    assert!(started.dir(cx).is_like(&path("sub")));
    started.press_key("backspace", cx);
    assert!(
        started.dir(cx).is_like(&path("sub")),
        "Backspace on an empty filter stays put"
    );
    started.press_key("left", cx);
    assert!(started.dir(cx).is_root_or_empty());
    drive(&rt, cx, |cx| started.listed(cx));
    // At the root there is nowhere to go up to.
    started.press_key("left", cx);
    assert!(started.dir(cx).is_root_or_empty());
    assert_eq!(started.rows(cx), vec!["sub/", "top.md"]);
}

/// Typing filters (Up stays, filter-exempt) and offers `Create:`; Backspace
/// then edits the filter rather than going up; Escape clears it, and a
/// second Escape gives the editor the keyboard back.
#[gpui_kit::test]
fn typing_filters_and_escape_clears_then_leaves(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "sub/a.md");
    focus_files(&started, cx);
    started.type_in("bm", cx);
    assert_eq!(started.query(cx), "bm");
    let rows = started.rows(cx);
    assert!(
        rows[0].starts_with("+") && rows[0].contains("bm"),
        "{rows:?}"
    );
    assert!(
        rows.contains(&"..".to_string()),
        "Up is filter-exempt: {rows:?}"
    );
    assert!(rows.contains(&"b.md".to_string()), "{rows:?}");
    assert!(!rows.contains(&"a.md".to_string()), "{rows:?}");
    started.press_key("backspace", cx);
    assert_eq!(started.query(cx), "b");
    assert!(
        started.dir(cx).is_like(&path("sub")),
        "not up: the filter had text"
    );
    started.press_key("escape", cx);
    assert_eq!(started.query(cx), "");
    assert_eq!(started.rows(cx), vec!["..", "a.md", "b.md", "deeper/"]);
    assert_eq!(started.focused(cx), Panel::Drawer(Side::Left));
    started.press_key("escape", cx);
    assert_eq!(started.focused(cx), Panel::Editor);
}

/// The `Create:` row makes the note through core, opens it, and FILES lists
/// it (the `Created` refresh).
#[gpui_kit::test]
fn the_create_row_makes_and_opens_the_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, "sub/a.md");
    focus_files(&started, cx);
    started.type_in("zulu", cx);
    assert_eq!(started.selected(cx).as_deref(), Some("+sub/zulu.md"));
    started.press_key("enter", cx);
    assert_eq!(started.focused(cx), Panel::Editor);
    drive(&rt, cx, |cx| {
        started
            .open_in_editor(cx)
            .is_some_and(|n| n.is_like(&path("sub/zulu.md")))
            && started.rows(cx).contains(&"zulu.md".to_string())
    });
    assert_eq!(started.query(cx), "", "the filter did its job");
    assert!(
        rt.block_on(v.vault.get_note_text(&path("sub/zulu.md")))
            .is_ok()
    );
    assert_eq!(started.marked(cx), vec!["zulu.md"]);
}

// ── the pointer ──────────────────────────────────────────────────────────

/// A click opens a note at once, as Enter does (C1: no click-click).
#[gpui_kit::test]
fn a_click_opens_a_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "sub/a.md");
    assert_eq!(started.text(cx), "Alpha");
    started.click_row("b.md", cx);
    assert_eq!(started.selected(cx).as_deref(), Some("b.md"));
    drive(&rt, cx, |cx| started.text(cx) == "Bravo");
    assert_eq!(started.focused(cx), Panel::Editor);
}

/// A click on a folder shows it.
#[gpui_kit::test]
fn a_click_opens_a_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "top.md");
    assert_eq!(started.rows(cx), vec!["sub/", "top.md"]);
    started.click_row("sub/", cx);
    assert!(started.dir(cx).is_like(&path("sub")));
    drive(&rt, cx, |cx| started.listed(cx));
    assert_eq!(started.rows(cx), vec!["..", "a.md", "b.md", "deeper/"]);
}

/// A double-click on a folder shows it once: its second press lands on a
/// row of the folder it entered, which it must not open (not two levels,
/// no note).
#[gpui_kit::test]
fn a_double_click_opens_a_folder_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "top.md");
    assert_eq!(started.rows(cx), vec!["sub/", "top.md"]);
    started.double_click_row("sub/", cx);
    assert!(started.dir(cx).is_like(&path("sub")));
    drive(&rt, cx, |cx| started.listed(cx));
    assert_eq!(started.rows(cx), vec!["..", "a.md", "b.md", "deeper/"]);
    assert!(started.dir(cx).is_like(&path("sub")), "one level");
    assert!(started.open_in_editor(cx).unwrap().is_like(&path("top.md")));
}

/// A right click is the entry menu's: it never opens the row.
#[gpui_kit::test]
fn a_right_click_never_opens_a_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "top.md");
    let id = started.row("sub/", cx);
    cx.update_window(started.handle, |_, window, cx| window.right_click(id, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(started.dir(cx).is_root_or_empty(), "the folder stays shut");
    started.press_key("escape", cx);
    cx.run_until_parked();
    assert!(started.dir(cx).is_root_or_empty());
}

/// The pointer says what a click does: the pointing hand over a row that
/// opens on a click, the default arrow over one that only selects or
/// cannot open now. (The kit's test window cannot read the cursor shown,
/// so the policy's mapping is what is checked.)
#[test]
fn the_click_policy_names_the_pointer() {
    use super::files::ClickPolicy;
    use gpui_kit::CursorStyle;
    assert_eq!(
        ClickPolicy::Open.cursor(true),
        Some(CursorStyle::PointingHand)
    );
    assert_eq!(ClickPolicy::Open.cursor(false), None);
    assert_eq!(ClickPolicy::Select.cursor(true), None);
    assert_eq!(ClickPolicy::Select.cursor(false), None);
    assert_eq!(ClickPolicy::default(), ClickPolicy::Open);
}

/// What one click does under each policy: `Open` acts on a double-click's
/// first press only, `Select` opens on its second; a right click is never
/// a row click.
#[test]
fn the_click_policy_reads_each_press() {
    use super::files::{ClickPolicy, RowClick};
    use gpui_kit::{ClickEvent, MouseButton, MouseClickEvent, MouseDownEvent, MouseUpEvent};
    let press = |button: MouseButton, click_count: usize| {
        ClickEvent::Mouse(MouseClickEvent {
            down: MouseDownEvent {
                button,
                click_count,
                ..MouseDownEvent::default()
            },
            up: MouseUpEvent {
                button,
                click_count,
                ..MouseUpEvent::default()
            },
        })
    };
    let left = MouseButton::Left;
    let open = ClickPolicy::Open;
    let select = ClickPolicy::Select;
    assert_eq!(open.click(&press(left, 1)), RowClick::Activate);
    assert_eq!(open.click(&press(left, 2)), RowClick::Ignore);
    assert_eq!(open.click(&press(left, 3)), RowClick::Ignore);
    assert_eq!(select.click(&press(left, 1)), RowClick::Select);
    assert_eq!(select.click(&press(left, 2)), RowClick::Activate);
    assert_eq!(select.click(&press(left, 3)), RowClick::Select);
    for policy in [open, select] {
        assert_eq!(
            policy.click(&press(MouseButton::Right, 1)),
            RowClick::Ignore
        );
    }
}

// ── the app channel ──────────────────────────────────────────────────────

/// A note created elsewhere (`FileOp::Created`) shows up in the folder FILES
/// lists; a creation in another folder leaves it alone.
#[gpui_kit::test]
fn a_created_note_refreshes_the_listing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(cx, &rt, "sub/a.md");
    rt.block_on(v.vault.create_note(&path("sub/echo.md"), "Echo"))
        .unwrap();
    rt.block_on(v.vault.create_note(&path("other.md"), "Other"))
        .unwrap();
    send(
        &started,
        AppMessage::FileOp(FileOp::Created(path("other.md"))),
        cx,
    );
    drive(&rt, cx, |_| true);
    assert!(!started.rows(cx).contains(&"echo.md".to_string()));
    send(
        &started,
        AppMessage::FileOp(FileOp::Created(path("sub/echo.md"))),
        cx,
    );
    drive(&rt, cx, |cx| {
        started.rows(cx).contains(&"echo.md".to_string())
    });
    assert_eq!(
        started.marked(cx),
        vec!["a.md"],
        "the marker survives the reload"
    );
}

/// Saving retitles the note's row from its first line.
#[gpui_kit::test]
fn a_save_retitles_the_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "sub/a.md");
    assert_eq!(started.title_of("a.md", cx).as_deref(), Some("Alpha"));
    started.type_text_in_editor("New ", cx);
    started.press_key("ctrl-s", cx);
    drive(&rt, cx, |cx| {
        started.title_of("a.md", cx).as_deref() == Some("New Alpha")
    });
}

/// `OpenFileBrowser` shows FILES on the note's folder when it lists another
/// one, and leaves the filter alone when it already lists it.
#[gpui_kit::test]
fn open_file_browser_reveals_the_notes_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "sub/a.md");
    // Hide the slot; the chord shows it again, focused.
    started.press_key("ctrl-t", cx);
    assert!(!started.shows(FILES_ID, cx));
    focus_files(&started, cx);
    assert!(started.shows(FILES_ID, cx));
    // Same folder: the filter stays.
    started.type_in("b", cx);
    started.press_key("ctrl-e", cx);
    assert_eq!(started.query(cx), "b");
    // Elsewhere: back to the note's folder, filter cleared.
    started.press_key("escape", cx);
    started.press_key("left", cx);
    assert!(started.dir(cx).is_root_or_empty());
    started.press_key("ctrl-e", cx);
    assert!(started.dir(cx).is_like(&path("sub")));
    assert_eq!(started.focused(cx), Panel::Drawer(Side::Left));
}

impl Started {
    /// Type `text` into the editor (it takes the keyboard first).
    fn type_text_in_editor(&self, text: &str, cx: &mut TestAppContext) {
        self.workspace.update(cx, |w, cx| {
            w.set_focus_panel(Panel::Editor);
            cx.notify();
        });
        cx.update_window(self.handle, |_, window, cx| {
            let ws = self.workspace.clone();
            ws.update(cx, |w, cx| w.apply_focus(window, cx));
            window.input(text, cx);
        })
        .unwrap();
    }
}

// ── arranging FILES: the shell chords win over the filter's keys ────────

impl Started {
    /// The views the slots of `side` show.
    fn views(&self, side: Side, cx: &TestAppContext) -> Vec<DrawerView> {
        cx.read(|cx| self.workspace.read(cx).shown_on(side))
    }

    /// What has the keyboard, a slot named by its side and its position
    /// among that side's slots.
    fn region(&self, cx: &TestAppContext) -> Region {
        cx.read(|cx| {
            let w = self.workspace.read(cx);
            match (w.focused_slot(), w.focused_panel()) {
                (Some(slot), Panel::Drawer(side)) => Region::Slot {
                    side,
                    index: w
                        .side_slots(side)
                        .iter()
                        .position(|s| *s == slot)
                        .expect("a slot of its side"),
                },
                (_, Panel::Rail) => Region::Rail,
                _ => Region::Group(w.layout.active_group()),
            }
        })
    }

    /// Whether the FILES filter has GPUI focus.
    fn filter_focused(&self, cx: &mut TestAppContext) -> bool {
        let files = cx.read(|cx| self.workspace.read(cx).files.clone().unwrap());
        cx.update_window(self.handle, |_, window, cx| {
            files.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap()
    }
}

/// With the FILES filter holding the keyboard (where the kit's input binds
/// Alt+Shift+Left/Right to word selection and Alt+Shift+Up/Down to extra
/// carets), the move chords still carry FILES between the slots, both
/// ways, and the keyboard stays in its filter.
#[gpui_kit::test]
fn files_moves_both_ways_from_its_filter(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "top.md");
    focus_files(&started, cx);
    assert!(started.filter_focused(cx));

    started.press_key("alt-shift-right", cx);
    assert_eq!(started.views(Side::Right, cx), [DrawerView::Files]);
    assert_eq!(
        started.region(cx),
        Region::Slot {
            side: Side::Right,
            index: 0
        }
    );
    assert!(started.filter_focused(cx));

    started.press_key("alt-shift-left", cx);
    assert_eq!(started.views(Side::Left, cx), [DrawerView::Files]);
    assert!(started.views(Side::Right, cx).is_empty());
    assert_eq!(
        started.region(cx),
        Region::Slot {
            side: Side::Left,
            index: 0
        }
    );
    assert!(started.filter_focused(cx));
    assert_eq!(started.query(cx), "", "the filter saw none of it");

    // Reached by Alt+Arrow on the right, then moved back from there.
    started.press_key("alt-shift-right", cx);
    started.press_key("alt-left", cx);
    started.press_key("alt-right", cx);
    assert_eq!(
        started.region(cx),
        Region::Slot {
            side: Side::Right,
            index: 0
        }
    );
    assert!(started.filter_focused(cx));
    started.press_key("alt-shift-left", cx);
    assert_eq!(started.views(Side::Left, cx), [DrawerView::Files]);

    // From the rail the move is spatial: Alt+Shift+Right brings the
    // cursor's view (FILES, moved right first) to the slot beside the
    // rail; Alt+Shift+Left points at the window's edge and moves nothing.
    started.press_key("alt-shift-right", cx);
    assert_eq!(started.views(Side::Right, cx), [DrawerView::Files]);
    cx.update_window(started.handle, |_, window, cx| window.click("rail", cx))
        .unwrap();
    started.press_key("alt-shift-right", cx);
    assert_eq!(started.views(Side::Left, cx), [DrawerView::Files]);
    assert!(started.views(Side::Right, cx).is_empty());
    started.press_key("alt-shift-left", cx);
    assert_eq!(started.views(Side::Left, cx), [DrawerView::Files]);
    assert_eq!(started.focused(cx), Panel::Rail);
}

/// The swap chords reorder a split side whichever slot holds the
/// keyboard: the FILES filter, or a placeholder view. The keyboard follows
/// the moved view.
#[gpui_kit::test]
fn the_swap_chords_work_from_the_files_filter_and_a_placeholder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, "top.md");
    focus_files(&started, cx);
    // Split from the filter asks what the slot below holds; FIND (the
    // first row) goes there and takes the keyboard.
    started.press_key("alt-shift-d", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::ViewChooser));
    started.press_key("enter", cx);
    assert_eq!(
        started.views(Side::Left, cx),
        [DrawerView::Files, DrawerView::Find]
    );
    assert_eq!(
        started.region(cx),
        Region::Slot {
            side: Side::Left,
            index: 1
        }
    );

    // From the placeholder.
    started.press_key("alt-shift-up", cx);
    assert_eq!(
        started.views(Side::Left, cx),
        [DrawerView::Find, DrawerView::Files]
    );
    assert_eq!(
        started.region(cx),
        Region::Slot {
            side: Side::Left,
            index: 0
        }
    );

    // From the FILES filter, now the bottom slot.
    started.press_key("alt-down", cx);
    assert!(started.filter_focused(cx));
    started.press_key("alt-shift-up", cx);
    assert_eq!(
        started.views(Side::Left, cx),
        [DrawerView::Files, DrawerView::Find]
    );
    assert_eq!(
        started.region(cx),
        Region::Slot {
            side: Side::Left,
            index: 0
        }
    );
    assert!(started.filter_focused(cx));
    started.press_key("alt-shift-down", cx);
    assert_eq!(
        started.views(Side::Left, cx),
        [DrawerView::Find, DrawerView::Files]
    );
    assert!(started.filter_focused(cx));
    assert_eq!(started.query(cx), "");

    // Moved to the empty right side (its top slot) and split there, from
    // the filter: the picked view (TAGS, the first row: FIND shows on the
    // left) fills a new slot below and takes the keyboard; back in the
    // filter, the swap chord trades the two.
    started.press_key("alt-shift-right", cx);
    assert_eq!(started.views(Side::Right, cx), [DrawerView::Files]);
    started.press_key("alt-shift-d", cx);
    assert_eq!(started.shown_modal(cx), Some(ModalKind::ViewChooser));
    started.press_key("enter", cx);
    assert_eq!(
        started.views(Side::Right, cx),
        [DrawerView::Files, DrawerView::Tags]
    );
    assert_eq!(
        started.region(cx),
        Region::Slot {
            side: Side::Right,
            index: 1
        }
    );
    started.press_key("alt-up", cx);
    assert!(started.filter_focused(cx));
    started.press_key("alt-shift-down", cx);
    assert_eq!(
        started.views(Side::Right, cx),
        [DrawerView::Tags, DrawerView::Files],
        "moved from the filter in the right slot"
    );
    assert!(started.filter_focused(cx));
}
