//! UI integration tests for the shared folder chooser ([`folder_chooser`]):
//! its Location field and the ranked list, on a real temp folder, in a
//! window of its own with a bare host around it.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the listings run on
//! tokio's blocking pool, and land only while the runtime is driven.
//!
//! The keys go through the window (`press`, `input`), as a person's do:
//! Enter, Esc and the arrows reach the chooser as the kit's field actions.
//!
//! [`folder_chooser`]: super::folder_chooser
#![cfg(test)]

use std::cell::RefCell;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::input::MoveToStart;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyBinding, KeyDownEvent, ParentElement as _, Pixels, Render, Styled as _,
    TestAppContext, Window, WindowOptions, div, px, size,
};
use kimun_core::system::{self, SystemPath};
use tokio::runtime::Runtime;

use super::folder_chooser::{
    FolderChooser, FolderChooserEvent, LIST_FRACTION, LIST_MAX_REMS, LIST_MIN_REMS, Listing,
    file_row_id, list_height, row_id,
};
use super::start_tests::{drive, runtime};
use crate::input;
use crate::keymap::{self, Keymap, Platform};

const PREFIX: &str = "test";
const LOCATION_ID: &str = "test-chooser-location";
const LOCATION_ERROR_ID: &str = "test-chooser-location-error";
const ROWS_ID: &str = "test-chooser-rows";

/// A bare host, as the real ones: its root hands the keys to the chooser
/// when it has the keyboard, never while a chooser field has them.
struct Host {
    chooser: Entity<FolderChooser>,
    focus: FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.chooser.read(cx).field_focused(window, cx) {
                    return;
                }
                let Some(key) = input::key_down_input(event) else {
                    return;
                };
                if this.chooser.update(cx, |c, cx| c.key(key, window, cx)) {
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .child(self.chooser.clone())
            // The command row, as the hosts place it in their footer.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .children(super::folder_chooser::actions(&self.chooser, cx)),
            )
    }
}

struct Opened {
    handle: AnyWindowHandle,
    chooser: Entity<FolderChooser>,
    /// What the chooser told its host, in order.
    events: Rc<RefCell<Vec<String>>>,
}

/// A window with a chooser on `start` (files mode with `files`), not yet
/// listed. Enter the runtime first.
fn open(cx: &mut TestAppContext, start: &Path, files: Option<&'static str>) -> Opened {
    cx.update(|cx| {
        gpui_kit::init(cx);
        keymap::init(cx, Keymap::defaults(Platform::LinuxWindows));
    });
    let start = SystemPath::try_absolute(start).unwrap();
    let (handle, host) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    let chooser = cx.new(|cx| {
                        let chooser = FolderChooser::new(PREFIX, start, window, cx);
                        match files {
                            Some(ext) => chooser.files(ext),
                            None => chooser,
                        }
                    });
                    Host { chooser, focus }
                })
            })
        })
        .expect("open test window");
    let chooser = cx.read(|cx| host.read(cx).chooser.clone());
    let events: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = events.clone();
    cx.update(|cx| {
        cx.subscribe(&chooser, move |_, event: &FolderChooserEvent, _| {
            log.borrow_mut().push(match event {
                FolderChooserEvent::Chosen(p) => format!("chosen:{}", system::path_to_string(p)),
                FolderChooserEvent::ChosenFile(p) => {
                    format!("file:{}", system::path_to_string(p))
                }
                FolderChooserEvent::Cancelled => "cancelled".into(),
            });
        })
        .detach();
    });
    Opened {
        handle,
        chooser,
        events,
    }
}

impl Opened {
    fn read<R>(&self, cx: &TestAppContext, f: impl FnOnce(&FolderChooser) -> R) -> R {
        cx.read(|cx| f(self.chooser.read(cx)))
    }

    /// Drive until the listing is in.
    fn listed(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| {
            self.read(cx, |c| !matches!(c.listing(), Listing::Loading))
        });
    }

    fn click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Type `text` into the Location field, clicking it first when it does
    /// not have the keyboard yet (the caret stays at the end).
    fn type_location(&self, text: &str, cx: &mut TestAppContext) {
        let focused = cx
            .update_window(self.handle, |_, window, cx| {
                self.chooser.read(cx).location_focused(window, cx)
            })
            .unwrap();
        if !focused {
            self.click(LOCATION_ID, cx);
        }
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    /// Enter in the Location field (the real key, through the window),
    /// then drive until the location is gone to.
    fn go(&self, rt: &Runtime, cx: &mut TestAppContext) {
        assert!(self.location_focused(cx), "Enter is pressed in the field");
        self.press("enter", cx);
        drive(rt, cx, |cx| self.read(cx, |c| !c.resolving()));
        self.listed(rt, cx);
    }

    fn location_focused(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            self.chooser.read(cx).location_focused(window, cx)
        })
        .unwrap()
    }

    fn events(&self) -> Vec<String> {
        self.events.borrow().clone()
    }

    fn location_error(&self, cx: &TestAppContext) -> Option<String> {
        self.read(cx, |c| c.location_error().map(str::to_string))
    }

    fn dir(&self, cx: &TestAppContext) -> SystemPath {
        self.read(cx, |c| c.dir().clone())
    }

    fn location_text(&self, cx: &TestAppContext) -> String {
        let input = self.read(cx, |c| c.location_input());
        cx.read(|cx| input.read(cx).value().to_string())
    }

    fn placeholder(&self, cx: &TestAppContext) -> String {
        let input = self.read(cx, |c| c.location_input());
        cx.read(|cx| input.read(cx).presentation().placeholder().to_string())
    }

    /// The names of the rows shown, in their order.
    fn shown(&self, cx: &TestAppContext) -> Vec<String> {
        self.read(cx, |c| {
            c.shown_rows()
                .into_iter()
                .map(|(path, _)| super::folder_chooser::folder_name(&path))
                .collect()
        })
    }

    /// The aria label of the element `id`, if it is drawn.
    fn label(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> Option<String> {
        let id = id.into();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id)
                .and_then(|e| e.label().map(str::to_string))
        })
        .unwrap()
    }
}

/// A temp folder holding `dirs` (made in order) and `files` (empty).
fn tree(dirs: &[&str], files: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for d in dirs {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    for f in files {
        std::fs::write(root.join(f), "").unwrap();
    }
    (dir, root)
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// A name in the field ranks the folders, best first, and selects the
/// best: `notes` beats `kimun-notes-old`, though it lists after it.
#[gpui_kit::test]
fn typing_a_name_ranks_the_listing_and_selects_the_best(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "kimun-notes-old", "notes", "zebra"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.press("down", cx);
    s.type_location("notes", cx);
    assert_eq!(s.shown(cx), names(&["notes", "kimun-notes-old"]));
    assert_eq!(s.read(cx, |c| c.selected()), Some(0));
    // The row keeps its id and its label.
    let notes = SystemPath::try_absolute(root.join("notes")).unwrap();
    assert_eq!(
        s.label(row_id(PREFIX, &notes), cx).as_deref(),
        Some("notes, folder")
    );
    let zebra = SystemPath::try_absolute(root.join("zebra")).unwrap();
    assert!(s.label(row_id(PREFIX, &zebra), cx).is_none());
}

/// With nothing typed (or the text deleted), the listing shows in its
/// alphabetical order, the first row selected.
#[gpui_kit::test]
fn an_empty_field_shows_the_listing_in_order(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "kimun-notes-old", "notes", "zebra"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let all = names(&["archive", "kimun-notes-old", "notes", "zebra"]);
    assert_eq!(s.shown(cx), all);
    s.type_location("zeb", cx);
    assert_eq!(s.shown(cx), names(&["zebra"]));
    for _ in 0..3 {
        s.press("backspace", cx);
    }
    assert_eq!(s.location_text(cx), "");
    assert_eq!(s.shown(cx), all);
    assert_eq!(s.read(cx, |c| c.selected()), Some(0));
}

/// A path in the field does not filter: the list stays as it was until
/// Enter. Nothing is selected then: Enter goes to the path.
#[gpui_kit::test]
fn a_path_in_the_field_does_not_filter(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "notes", "zebra"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.press("down", cx);
    s.type_location(&format!("zz{MAIN_SEPARATOR}qq"), cx);
    assert_eq!(s.shown(cx), names(&["archive", "notes", "zebra"]));
    assert_eq!(s.read(cx, |c| c.selected()), None);
    // Back to `zz`: a filter again, with no match.
    for _ in 0..3 {
        s.press("backspace", cx);
    }
    assert!(s.shown(cx).is_empty());
    // A filter narrowed the list: a path typed after it keeps that list.
    for _ in 0..2 {
        s.press("backspace", cx);
    }
    s.type_location("note", cx);
    assert_eq!(s.shown(cx), names(&["notes"]));
    s.type_location(&format!("s{MAIN_SEPARATOR}"), cx);
    assert_eq!(s.shown(cx), names(&["notes"]));
    assert_eq!(s.read(cx, |c| c.selected()), None);
}

/// Browsing to another folder (a click on a row, `Up`) empties the field
/// and shows the new folder's listing whole.
#[gpui_kit::test]
fn browsing_to_another_folder_empties_the_field(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "notes/a", "notes/b"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location("not", cx);
    let notes = SystemPath::try_absolute(root.join("notes")).unwrap();
    s.click(row_id(PREFIX, &notes), cx);
    s.listed(&rt, cx);
    assert_eq!(s.read(cx, |c| c.dir().clone()), notes);
    assert_eq!(s.location_text(cx), "");
    assert_eq!(s.shown(cx), names(&["a", "b"]));
    s.type_location("b", cx);
    s.click("test-chooser-up", cx);
    s.listed(&rt, cx);
    assert_eq!(s.location_text(cx), "");
    assert_eq!(s.shown(cx), names(&["archive", "notes"]));
}

/// Text typed while the listing runs stays, and ranks the listing once it
/// lands.
#[gpui_kit::test]
fn a_late_listing_keeps_the_filter(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "kimun-notes-old", "notes", "zebra"], &[]);
    let s = open(cx, &root, None);
    assert!(s.read(cx, |c| matches!(c.listing(), Listing::Loading)));
    s.type_location("notes", cx);
    s.listed(&rt, cx);
    assert_eq!(s.location_text(cx), "notes");
    assert_eq!(s.shown(cx), names(&["notes", "kimun-notes-old"]));
    assert_eq!(s.read(cx, |c| c.selected()), Some(0));
}

/// The field's placeholder names the folder shown, and follows a browse.
#[gpui_kit::test]
fn the_placeholder_shows_the_current_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let start = SystemPath::try_absolute(&root).unwrap();
    assert_eq!(s.placeholder(cx), system::path_to_string(&start));
    let notes = SystemPath::try_absolute(root.join("notes")).unwrap();
    s.click(row_id(PREFIX, &notes), cx);
    s.listed(&rt, cx);
    assert_eq!(s.placeholder(cx), system::path_to_string(&notes));
    // The field has an accessible name of its own.
    assert_eq!(s.label(LOCATION_ID, cx).as_deref(), Some("Location"));
}

/// In files mode the filter covers the folders and the files; a file row
/// keeps its kind, its id and its label.
#[gpui_kit::test]
fn files_mode_filters_folders_and_files(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["keys", "notes"], &["keys.toml", "notes.toml"]);
    let s = open(cx, &root, Some("toml"));
    s.listed(&rt, cx);
    s.type_location("keys", cx);
    let shown = s.read(cx, |c| c.shown_rows());
    let kinds: Vec<(String, bool)> = shown
        .iter()
        .map(|(p, file)| (super::folder_chooser::folder_name(p), *file))
        .collect();
    assert_eq!(
        kinds,
        [("keys".to_string(), false), ("keys.toml".to_string(), true)]
    );
    let file = SystemPath::try_absolute(root.join("keys.toml")).unwrap();
    assert_eq!(
        s.label(file_row_id(PREFIX, &file), cx).as_deref(),
        Some("keys.toml, file")
    );
}

fn sys(path: impl AsRef<Path>) -> SystemPath {
    SystemPath::try_absolute(path.as_ref()).unwrap()
}

/// Enter on a full path to a folder opens it in the list, empties the
/// field, and chooses nothing.
#[gpui_kit::test]
fn enter_on_a_folder_path_opens_it_without_choosing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a", "notes/b", "zebra"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let notes = sys(root.join("notes"));
    s.type_location(&system::path_to_string(&notes), cx);
    s.go(&rt, cx);
    assert_eq!(s.dir(cx), notes);
    assert_eq!(s.location_text(cx), "");
    assert_eq!(s.shown(cx), names(&["a", "b"]));
    assert!(s.events.borrow().is_empty(), "nothing chosen");
    assert_eq!(s.location_error(cx), None);
}

/// A relative path is read from the folder shown, `..` included.
#[gpui_kit::test]
fn enter_on_a_relative_path_reads_from_the_current_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a/deep", "notes/b"], &[]);
    let s = open(cx, &root.join("notes"), None);
    s.listed(&rt, cx);
    s.type_location(&format!("a{MAIN_SEPARATOR}deep"), cx);
    s.go(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes").join("a").join("deep")));
    // `..` alone is a path, not a filter: it goes up.
    s.type_location("..", cx);
    s.go(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes").join("a")));
    // `.` stays, with the field emptied and no message.
    s.type_location(" . ", cx);
    s.go(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes").join("a")));
    assert_eq!(s.location_text(cx), "");
    assert_eq!(s.location_error(cx), None);
}

/// `~` goes to the home folder. The home is the process's own (`HOME`):
/// changing it would race the other tests, which read it too.
#[gpui_kit::test]
fn enter_on_a_tilde_path_goes_home(cx: &mut TestAppContext) {
    let Ok(home) = system::home() else {
        return;
    };
    let home = SystemPath::resolve("", &home);
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location("~", cx);
    s.go(&rt, cx);
    assert_eq!(s.dir(cx), home);
    assert_eq!(s.location_text(cx), "");
    // `~user` is not expanded: read from the folder shown, it is missing.
    s.type_location("~nobody-here", cx);
    s.go(&rt, cx);
    assert_eq!(s.dir(cx), home);
    assert_eq!(
        s.location_error(cx).as_deref(),
        Some("That folder does not exist")
    );
}

/// In files mode, a path to a file with the extension picks it.
#[gpui_kit::test]
fn enter_on_a_file_path_picks_it_in_files_mode(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["keys"], &["keys/mine.toml"]);
    let s = open(cx, &root, Some("toml"));
    s.listed(&rt, cx);
    let file = sys(root.join("keys").join("mine.toml"));
    s.type_location(&format!("keys{MAIN_SEPARATOR}mine.toml"), cx);
    s.go(&rt, cx);
    assert_eq!(
        *s.events.borrow(),
        vec![format!("file:{}", system::path_to_string(&file))]
    );
    assert_eq!(s.dir(cx), sys(&root));
    assert_eq!(s.location_error(cx), None);
}

/// In folder mode, a path to a file shows the message under the field and
/// the chooser stays where it was.
#[gpui_kit::test]
fn enter_on_a_file_path_in_folder_mode_shows_the_message(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &["notes/a.toml"]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let typed = format!("notes{MAIN_SEPARATOR}a.toml");
    s.type_location(&typed, cx);
    s.go(&rt, cx);
    assert_eq!(
        s.location_error(cx).as_deref(),
        Some("That is a file, not a folder")
    );
    assert_eq!(
        s.label(LOCATION_ERROR_ID, cx).as_deref(),
        Some("That is a file, not a folder")
    );
    assert_eq!(s.dir(cx), sys(&root));
    assert_eq!(s.location_text(cx), typed);
    assert!(s.events.borrow().is_empty());
}

/// A missing path shows the message; the chooser, its listing and the text
/// stay. A very long path ends in a message too, never a stuck listing.
#[gpui_kit::test]
fn enter_on_a_missing_path_shows_the_message_and_stays(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let typed = format!("gone{MAIN_SEPARATOR}away ");
    s.type_location(&typed, cx);
    s.go(&rt, cx);
    assert_eq!(
        s.location_error(cx).as_deref(),
        Some("That folder does not exist")
    );
    assert_eq!(
        s.label(LOCATION_ERROR_ID, cx).as_deref(),
        Some("That folder does not exist")
    );
    assert_eq!(s.dir(cx), sys(&root));
    assert_eq!(s.location_text(cx), typed);
    assert_eq!(s.shown(cx), names(&["notes"]));
    // A path past any name length: some message, the listing still there.
    let long = format!("{}{MAIN_SEPARATOR}x", "a".repeat(4096));
    let input = s.read(cx, |c| c.location_input());
    cx.update_window(s.handle, |_, window, cx| {
        input.update(cx, |i, cx| i.set_value(long.clone(), window, cx))
    })
    .unwrap();
    s.go(&rt, cx);
    let message = s.location_error(cx).expect("a message");
    assert_ne!(message, "That folder does not exist");
    assert_eq!(s.dir(cx), sys(&root));
    assert_eq!(s.shown(cx), names(&["notes"]));
}

/// The message goes at the next key in the field, and when the chooser
/// browses elsewhere.
#[gpui_kit::test]
fn the_message_goes_at_the_next_key(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location(&format!("gone{MAIN_SEPARATOR}"), cx);
    s.go(&rt, cx);
    assert!(s.location_error(cx).is_some());
    s.type_location("x", cx);
    assert_eq!(s.location_error(cx), None);
    assert!(s.label(LOCATION_ERROR_ID, cx).is_none());
    // Again, then a browse (a click on a row) takes it away.
    s.go(&rt, cx);
    assert!(s.location_error(cx).is_some());
    s.click(row_id(PREFIX, &sys(root.join("notes"))), cx);
    s.listed(&rt, cx);
    assert_eq!(s.location_error(cx), None);
}

/// A name in the field (a filter) or an empty field: Enter opens the
/// selected row, the best match. A path drops the selection, so no row
/// looks chosen for Enter.
#[gpui_kit::test]
fn enter_on_a_filter_opens_the_selected_row(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "kimun-notes-old/x", "notes/a"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location("notes", cx);
    s.go(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes")));
    assert_eq!(s.location_text(cx), "");
    // Empty field: the first row.
    s.go(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes").join("a")));
    assert!(s.events.borrow().is_empty());
}

/// A path in the field drops the selection; the list keeps its rows.
#[gpui_kit::test]
fn a_path_in_the_field_drops_the_selection(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location("..", cx);
    assert_eq!(s.read(cx, |c| c.selected()), None);
    assert_eq!(s.shown(cx), names(&["archive", "notes"]));
}

/// The list's accessible name says which folder it lists: the path line
/// is gone, and the placeholder hides once text is typed.
#[gpui_kit::test]
fn the_list_label_names_the_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &["a.toml"]);
    let path = system::path_to_string(sys(&root));
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    assert_eq!(
        s.label("test-chooser-rows", cx),
        Some(format!("Folders in {path}"))
    );
    let f = open(cx, &root, Some("toml"));
    f.listed(&rt, cx);
    assert_eq!(
        f.label("test-chooser-rows", cx),
        Some(format!("Folders and files in {path}"))
    );
}

// ── the keys ────────────────────────────────────────────────────────────────

/// The chooser takes the keyboard as it opens: the Location field has it,
/// so a name typed at once filters.
#[gpui_kit::test]
fn the_location_field_has_the_keyboard_on_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    assert!(s.location_focused(cx));
    s.press("n", cx);
    assert_eq!(s.location_text(cx), "n");
    assert_eq!(s.shown(cx), names(&["notes"]));
}

/// Up and Down move the selection from inside the field, and keep its text.
#[gpui_kit::test]
fn up_and_down_move_the_selection_from_the_field(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "notes", "zebra"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    assert_eq!(s.read(cx, |c| c.selected()), Some(0));
    s.press("down", cx);
    s.press("down", cx);
    assert_eq!(s.read(cx, |c| c.selected()), Some(2));
    s.press("down", cx);
    assert_eq!(s.read(cx, |c| c.selected()), Some(2), "stays at the end");
    s.press("up", cx);
    assert_eq!(s.read(cx, |c| c.selected()), Some(1));
    assert!(s.location_focused(cx));
    // Enter opens the row selected.
    s.press("enter", cx);
    s.listed(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes")));
    assert!(s.events().is_empty(), "nothing chosen");
}

/// Esc with text clears it (the whole listing is back); Esc from the empty
/// field closes the chooser.
#[gpui_kit::test]
fn esc_clears_the_text_then_closes_the_chooser(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location("zz", cx);
    assert!(s.shown(cx).is_empty());
    s.press("escape", cx);
    assert_eq!(s.location_text(cx), "");
    assert_eq!(s.shown(cx), names(&["archive", "notes"]));
    assert_eq!(s.read(cx, |c| c.selected()), Some(0));
    assert!(s.events().is_empty(), "not closed yet");
    // A message goes with the text.
    s.type_location(&format!("gone{MAIN_SEPARATOR}"), cx);
    s.go(&rt, cx);
    assert!(s.location_error(cx).is_some());
    s.press("escape", cx);
    assert_eq!(s.location_error(cx), None);
    assert!(s.events().is_empty());
    s.press("escape", cx);
    assert_eq!(s.events(), ["cancelled"]);
}

/// Backspace with text deletes a character; only in the empty field does
/// it go up one folder.
#[gpui_kit::test]
fn backspace_edits_text_and_goes_up_only_when_empty(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a"], &[]);
    let s = open(cx, &root.join("notes"), None);
    s.listed(&rt, cx);
    s.type_location("ab", cx);
    s.press("backspace", cx);
    assert_eq!(s.location_text(cx), "a");
    assert_eq!(s.dir(cx), sys(root.join("notes")));
    s.press("backspace", cx);
    assert_eq!(s.location_text(cx), "");
    assert_eq!(s.dir(cx), sys(root.join("notes")), "the text went first");
    s.press("backspace", cx);
    s.listed(&rt, cx);
    assert_eq!(s.dir(cx), sys(&root));
    assert!(s.location_focused(cx), "the field keeps the keyboard");
}

/// Alt+Up goes up one folder, with text in the field too (it empties).
/// Left and Right stay the caret's.
#[gpui_kit::test]
fn alt_up_goes_up_one_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a/deep"], &[]);
    let s = open(cx, &root.join("notes").join("a"), None);
    s.listed(&rt, cx);
    s.type_location("de", cx);
    s.press("left", cx);
    s.press("right", cx);
    assert_eq!(s.dir(cx), sys(root.join("notes").join("a")), "the caret's");
    assert_eq!(s.location_text(cx), "de");
    s.press("alt-up", cx);
    s.listed(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes")));
    assert_eq!(s.location_text(cx), "");
    assert!(s.location_focused(cx));
}

/// On macOS (the keymap's platform) it is Cmd+Up; Alt+Up does nothing.
/// Here Cmd+Up goes through the chooser's key table (`on_key_down`): this
/// Linux build has no kit binding for it, as when the host's root hands the
/// keys over. The kit's own binding on a Mac is the next test.
#[gpui_kit::test]
fn cmd_up_goes_up_on_macos_through_the_key_table(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a"], &[]);
    let s = open(cx, &root.join("notes").join("a"), None);
    cx.update(|cx| keymap::init(cx, Keymap::defaults(Platform::MacOs)));
    s.listed(&rt, cx);
    s.press("alt-up", cx);
    assert_eq!(s.dir(cx), sys(root.join("notes").join("a")));
    s.press("cmd-up", cx);
    s.listed(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes")));
}

/// On a Mac the kit binds Cmd+Up in its field to `MoveToStart` (only under
/// `cfg(target_os = "macos")`, so a Linux run never sees it). The binding is
/// made here as the kit makes it, in its `Input` key context: Cmd+Up then
/// reaches the chooser as that action, not as a key, and still goes up.
#[gpui_kit::test]
fn cmd_up_goes_up_on_macos_through_the_kit_binding(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a"], &[]);
    let s = open(cx, &root.join("notes").join("a"), None);
    cx.update(|cx| {
        keymap::init(cx, Keymap::defaults(Platform::MacOs));
        // The kit's key context for its field (`input::CONTEXT`, private).
        cx.bind_keys([KeyBinding::new("cmd-up", MoveToStart, Some("Input"))]);
    });
    s.listed(&rt, cx);
    s.type_location("x", cx);
    s.press("cmd-up", cx);
    s.listed(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes")));
    assert!(s.location_focused(cx));
}

/// A click on a row keeps the keyboard in the Location field: the rows
/// take no focus, and text typed next lands in the field. A double-click
/// still chooses.
#[gpui_kit::test]
fn a_click_on_a_row_keeps_the_keyboard_in_the_field(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.click(row_id(PREFIX, &sys(root.join("notes"))), cx);
    s.listed(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes")));
    assert!(s.location_focused(cx), "the field keeps the keyboard");
    cx.update_window(s.handle, |_, window, cx| window.input("a", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(s.location_text(cx), "a");
    // A double-click: the first click enters `a`, the second takes it.
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(row_id(PREFIX, &sys(root.join("notes").join("a"))), cx)
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        s.events(),
        [format!(
            "chosen:{}",
            system::path_to_string(sys(root.join("notes").join("a")))
        )]
    );
}

/// The list's height in the window, in a fresh frame.
fn list_bounds(s: &Opened, cx: &mut TestAppContext) -> gpui_kit::Bounds<Pixels> {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(ROWS_ID).bounds()
    })
    .unwrap()
}

fn rem_size(s: &Opened, cx: &mut TestAppContext) -> Pixels {
    cx.update_window(s.handle, |_, window, _| window.rem_size())
        .unwrap()
}

#[test]
fn list_height_clamps_to_the_floor_and_the_ceiling() {
    for rem in [px(16.), px(20.)] {
        assert_eq!(list_height(px(200.), rem), rem * LIST_MIN_REMS);
        assert_eq!(list_height(px(0.), rem), rem * LIST_MIN_REMS);
        assert_eq!(list_height(px(4000.), rem), rem * LIST_MAX_REMS);
    }
}

#[test]
fn list_height_is_a_fraction_of_the_window_between() {
    // 45 % of 1000 px is 450 px: above 14 rem (224 px), under 36 (576 px).
    assert_eq!(list_height(px(1000.), px(16.)), px(1000.) * LIST_FRACTION);
    assert_eq!(list_height(px(1000.), px(16.)), px(450.));
}

/// The list follows the window's height: a taller window gives a taller
/// list, up to the ceiling.
#[gpui_kit::test]
fn a_tall_window_gives_a_taller_list(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let rem = rem_size(&s, cx);
    cx.simulate_window_resize(s.handle, size(px(800.), px(700.)));
    cx.run_until_parked();
    let short = list_bounds(&s, cx).size.height;
    assert_eq!(short, list_height(px(700.), rem));
    cx.simulate_window_resize(s.handle, size(px(800.), px(1000.)));
    cx.run_until_parked();
    let tall = list_bounds(&s, cx).size.height;
    assert!(tall > short, "{tall:?} > {short:?}");
    assert_eq!(tall, list_height(px(1000.), rem));
    cx.simulate_window_resize(s.handle, size(px(800.), px(4000.)));
    cx.run_until_parked();
    assert_eq!(list_bounds(&s, cx).size.height, rem * LIST_MAX_REMS);
}

/// A short window keeps the floor.
#[gpui_kit::test]
fn a_short_window_keeps_the_floor(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let rem = rem_size(&s, cx);
    cx.simulate_window_resize(s.handle, size(px(800.), px(300.)));
    cx.run_until_parked();
    assert_eq!(list_bounds(&s, cx).size.height, rem * LIST_MIN_REMS);
}

/// A plain N in the field types an `n`: it opens no New folder field.
#[gpui_kit::test]
fn typing_n_types_an_n(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.press("n", cx);
    s.type_location("ew", cx);
    assert_eq!(s.location_text(cx), "new");
    assert!(!s.read(cx, |c| c.new_folder_open()));
}

/// Ctrl+N opens the New folder field (it takes the keyboard) in folder
/// mode only; Esc there closes it and gives the keyboard back to the
/// Location field.
#[gpui_kit::test]
fn ctrl_n_opens_new_folder_in_folder_mode_only(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &["a.toml"]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.press("ctrl-n", cx);
    assert!(s.read(cx, |c| c.new_folder_open()));
    assert_eq!(s.location_text(cx), "", "no `n` typed");
    let typing = cx
        .update_window(s.handle, |_, window, cx| {
            s.chooser.read(cx).new_folder_focused(window, cx)
        })
        .unwrap();
    assert!(typing, "the new folder field has the keyboard");
    s.press("escape", cx);
    assert!(!s.read(cx, |c| c.new_folder_open()));
    assert!(s.location_focused(cx));
    assert!(s.events().is_empty(), "Esc closed only the field");
    let f = open(cx, &root, Some("toml"));
    f.listed(&rt, cx);
    f.press("ctrl-n", cx);
    assert!(!f.read(cx, |c| c.new_folder_open()));
}

/// Ctrl+Enter chooses the folder shown, from the field, with text in it;
/// files mode chooses no folder.
#[gpui_kit::test]
fn ctrl_enter_chooses_the_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &["a.toml"]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location("no", cx);
    s.press("ctrl-enter", cx);
    assert_eq!(
        s.events(),
        [format!("chosen:{}", system::path_to_string(sys(&root)))]
    );
    let f = open(cx, &root, Some("toml"));
    f.listed(&rt, cx);
    f.press("ctrl-enter", cx);
    assert!(f.events().is_empty());
    assert_eq!(f.dir(cx), sys(&root));
}

// ── choosing a typed path ───────────────────────────────────────────────────

const CHOOSE_ID: &str = "test-chooser-choose";

impl Opened {
    /// Drive until a typed path is looked up.
    fn looked_up(&self, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| self.read(cx, |c| !c.resolving()));
    }

    fn new_folder_focused(&self, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            self.chooser.read(cx).new_folder_focused(window, cx)
        })
        .unwrap()
    }
}

/// `Choose this folder` with a path to a folder typed in the field chooses
/// that folder, as a system dialog does, not the folder shown.
#[gpui_kit::test]
fn choose_this_folder_takes_a_typed_folder_path(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location(&format!("notes{MAIN_SEPARATOR}a"), cx);
    s.click(CHOOSE_ID, cx);
    s.looked_up(&rt, cx);
    assert_eq!(
        s.events(),
        [format!(
            "chosen:{}",
            system::path_to_string(sys(root.join("notes").join("a")))
        )]
    );
    assert_eq!(s.location_error(cx), None);
}

/// Ctrl+Enter does the same with a full path.
#[gpui_kit::test]
fn ctrl_enter_takes_a_typed_folder_path(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let notes = sys(root.join("notes"));
    s.type_location(&system::path_to_string(&notes), cx);
    s.press("ctrl-enter", cx);
    s.looked_up(&rt, cx);
    assert_eq!(
        s.events(),
        [format!("chosen:{}", system::path_to_string(&notes))]
    );
}

/// A typed path that is missing, or a file, shows the usual message under
/// the field and chooses nothing.
#[gpui_kit::test]
fn choosing_a_missing_path_or_a_file_shows_the_message(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &["notes/a.toml"]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location(&format!("gone{MAIN_SEPARATOR}away"), cx);
    s.click(CHOOSE_ID, cx);
    s.looked_up(&rt, cx);
    assert_eq!(
        s.label(LOCATION_ERROR_ID, cx).as_deref(),
        Some("That folder does not exist")
    );
    assert!(s.events().is_empty(), "nothing chosen");
    s.press("escape", cx);
    s.type_location(&format!("notes{MAIN_SEPARATOR}a.toml"), cx);
    s.press("ctrl-enter", cx);
    s.looked_up(&rt, cx);
    assert_eq!(
        s.label(LOCATION_ERROR_ID, cx).as_deref(),
        Some("That is a file, not a folder")
    );
    assert!(s.events().is_empty(), "nothing chosen");
    assert_eq!(s.dir(cx), sys(&root));
}

/// An empty field and a filter still choose the folder shown, with the
/// button too.
#[gpui_kit::test]
fn choose_this_folder_with_a_filter_or_nothing_takes_the_folder_shown(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    let shown = format!("chosen:{}", system::path_to_string(sys(&root)));
    s.click(CHOOSE_ID, cx);
    assert_eq!(s.events(), std::slice::from_ref(&shown));
    s.type_location("no", cx);
    s.click(CHOOSE_ID, cx);
    assert_eq!(s.events(), [shown.clone(), shown]);
}

/// Ctrl+Enter right after Enter on the same path: the choose takes over the
/// lookup, so the folder is chosen and not only opened.
#[gpui_kit::test]
fn choose_replaces_a_lookup_from_enter(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location(&format!("notes{MAIN_SEPARATOR}"), cx);
    s.press("enter", cx);
    s.press("ctrl-enter", cx);
    s.looked_up(&rt, cx);
    s.listed(&rt, cx);
    assert_eq!(
        s.events(),
        [format!(
            "chosen:{}",
            system::path_to_string(sys(root.join("notes")))
        )]
    );
    assert_eq!(s.dir(cx), sys(&root), "the Enter lookup was dropped");
}

/// Esc in the Location field while the New folder field is open closes
/// that field first (the keyboard stays in the Location field); the
/// chooser stays open.
#[gpui_kit::test]
fn esc_in_the_location_field_closes_new_folder_first(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.press("ctrl-n", cx);
    assert!(s.new_folder_focused(cx));
    s.click(LOCATION_ID, cx);
    assert!(s.location_focused(cx));
    s.press("escape", cx);
    assert!(!s.read(cx, |c| c.new_folder_open()));
    assert!(s.events().is_empty(), "the chooser stays open");
    assert!(s.location_focused(cx));
}

/// Backspace in the empty Location field while the New folder field is
/// open does not go up: that would drop the New folder field.
#[gpui_kit::test]
fn backspace_in_the_empty_field_keeps_new_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a"], &[]);
    let s = open(cx, &root.join("notes"), None);
    s.listed(&rt, cx);
    s.press("ctrl-n", cx);
    s.click(LOCATION_ID, cx);
    s.press("backspace", cx);
    s.listed(&rt, cx);
    assert_eq!(s.dir(cx), sys(root.join("notes")));
    assert!(s.read(cx, |c| c.new_folder_open()));
}

/// Up and Down select nothing while the field holds a path: Enter goes to
/// the path, not to a row.
#[gpui_kit::test]
fn up_and_down_do_nothing_with_a_path(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["archive", "notes"], &[]);
    let s = open(cx, &root, None);
    s.listed(&rt, cx);
    s.type_location("..", cx);
    s.press("down", cx);
    assert_eq!(s.read(cx, |c| c.selected()), None);
    s.press("up", cx);
    assert_eq!(s.read(cx, |c| c.selected()), None);
}

/// In files mode, a path to a file with another extension says which kind
/// is wanted.
#[gpui_kit::test]
fn a_file_of_another_kind_says_which_kind_in_files_mode(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&[], &["b.txt"]);
    let s = open(cx, &root, Some("toml"));
    s.listed(&rt, cx);
    s.type_location(&format!(".{MAIN_SEPARATOR}b.txt"), cx);
    s.go(&rt, cx);
    assert_eq!(
        s.label(LOCATION_ERROR_ID, cx).as_deref(),
        Some("That is not a .toml file")
    );
    assert!(s.events().is_empty());
}

/// Each key a hint line names does what it says, on both platforms and in
/// both modes, and each line names exactly the keys tried here.
#[gpui_kit::test]
fn the_hints_match_the_keys(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_dir, root) = tree(&["notes/a/deep", "notes/b"], &["notes/a/k.toml"]);
    for (platform, files) in [
        (Platform::LinuxWindows, None),
        (Platform::LinuxWindows, Some("toml")),
        (Platform::MacOs, None),
        (Platform::MacOs, Some("toml")),
    ] {
        let s = open(cx, &root.join("notes"), files);
        cx.update(|cx| keymap::init(cx, Keymap::defaults(platform)));
        s.listed(&rt, cx);
        let hints = cx.read(|cx| s.chooser.read(cx).hints(cx));
        let keys: Vec<&str> = hints
            .split(" · ")
            .map(|part| part.split_once(": ").expect("key: what it does").0)
            .collect();
        let (up, choose, new) = match platform {
            Platform::MacOs => ("Cmd+Up", "Cmd+Enter", "Cmd+N"),
            Platform::LinuxWindows => ("Alt+Up", "Ctrl+Enter", "Ctrl+N"),
        };
        let mut want = vec!["Up and Down", "Enter", up];
        if files.is_none() {
            want.extend([choose, new]);
        }
        want.push("Esc");
        assert_eq!(keys, want, "{platform:?} {files:?}: {hints}");
        let notes = sys(root.join("notes"));
        // Up and Down: select.
        s.press("down", cx);
        assert_eq!(s.read(cx, |c| c.selected()), Some(1));
        s.press("up", cx);
        assert_eq!(s.read(cx, |c| c.selected()), Some(0));
        // Enter: open (the row selected) or go to the path.
        s.press("enter", cx);
        s.listed(&rt, cx);
        assert_eq!(s.dir(cx), sys(root.join("notes").join("a")));
        // Alt+Up (Cmd+Up): up.
        s.press(&up.to_lowercase().replace('+', "-"), cx);
        s.listed(&rt, cx);
        assert_eq!(s.dir(cx), notes);
        s.type_location(&format!("b{MAIN_SEPARATOR}"), cx);
        s.go(&rt, cx);
        assert_eq!(s.dir(cx), sys(root.join("notes").join("b")));
        s.press("backspace", cx);
        s.listed(&rt, cx);
        assert_eq!(s.dir(cx), notes);
        if files.is_some() {
            // Enter: pick a file (the path to one).
            s.type_location(&format!("a{MAIN_SEPARATOR}k.toml"), cx);
            s.go(&rt, cx);
            assert_eq!(s.events().len(), 1, "{:?}", s.events());
            assert!(s.events()[0].starts_with("file:"));
            s.events.borrow_mut().clear();
        } else {
            // Ctrl+N (Cmd+N): new folder, then Esc closes the field.
            s.press(&new.to_lowercase().replace('+', "-"), cx);
            assert!(s.read(cx, |c| c.new_folder_open()));
            s.press("escape", cx);
            // Ctrl+Enter (Cmd+Enter): choose.
            s.press(&choose.to_lowercase().replace('+', "-"), cx);
            assert_eq!(
                s.events(),
                [format!("chosen:{}", system::path_to_string(&notes))]
            );
            s.events.borrow_mut().clear();
        }
        // Esc: clear, then back.
        s.type_location("x", cx);
        s.press("escape", cx);
        assert_eq!(s.location_text(cx), "");
        assert!(s.events().is_empty());
        s.press("escape", cx);
        assert_eq!(s.events(), ["cancelled"]);
    }
}
