//! UI integration tests for the shared folder chooser ([`folder_chooser`]):
//! its Location field and the ranked list, on a real temp folder, in a
//! window of its own with a bare host around it.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`): the listings run on
//! tokio's blocking pool, and land only while the runtime is driven.
//!
//! [`folder_chooser`]: super::folder_chooser
#![cfg(test)]

use std::cell::RefCell;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, Styled as _, TestAppContext, Window,
    WindowOptions, div,
};
use kimun_core::system::{self, SystemPath};
use tokio::runtime::Runtime;

use super::folder_chooser::{FolderChooser, FolderChooserEvent, Listing, file_row_id, row_id};
use super::start_tests::{drive, runtime};
use crate::input;

const PREFIX: &str = "test";
const LOCATION_ID: &str = "test-chooser-location";
const LOCATION_ERROR_ID: &str = "test-chooser-location-error";

/// A bare host, as the real ones: its root holds the keyboard and hands
/// the keys to the chooser, unless a chooser field has them.
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
    cx.update(gpui_kit::init);
    let start = SystemPath::try_absolute(start).unwrap();
    let (handle, host) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    let host_focus = focus.clone();
                    let chooser = cx.new(|cx| {
                        let chooser = FolderChooser::new(PREFIX, start, host_focus, window, cx);
                        match files {
                            Some(ext) => chooser.files(ext),
                            None => chooser,
                        }
                    });
                    window.focus(&focus, cx);
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

    /// Enter on the Location field's value (the keys come later: called
    /// directly), then drive until the location is gone to.
    fn go(&self, rt: &Runtime, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            self.chooser
                .update(cx, |c, cx| c.go_to_location(window, cx))
        })
        .unwrap();
        drive(rt, cx, |cx| self.read(cx, |c| !c.resolving()));
        self.listed(rt, cx);
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
