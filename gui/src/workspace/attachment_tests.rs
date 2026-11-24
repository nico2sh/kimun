//! UI integration tests for the attachment view (M7a G3): a real
//! `Workspace` on a real temp vault holding notes and attachments, driven
//! through FILES, the editor's links and the window's keys. The system's
//! default program is replaced by a recorder.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use std::cell::RefCell;
use std::ffi::OsString;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Pixels, TestAppContext, px, size};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::attachment::format_mtime;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::Panel;
use tokio::runtime::Runtime;

use super::attachment::{
    ATTACHMENT_ID, IMAGE_ID, LINE_CHARS, Lines, NO_PREVIEW_ID, NO_PREVIEW_NOTE, NO_PREVIEW_NOTE_ID,
    OPEN_EXTERNALLY_ID, PREVIEW_ID, PaintedImage, Preview, TRUNCATED_ID, display_line, load,
};
use super::files::row_id;
use super::route::NO_NOTE;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
use crate::inline_image::{Failure, ImageState, ImageStore};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// A vault holding `notes` and `attachments` (path, bytes), started on
/// `first`, driven until its text shows, FILES has listed the root and the
/// index has caught up.
fn started_on(
    cx: &mut TestAppContext,
    rt: &Runtime,
    notes: &[(&str, &str)],
    attachments: &[(&str, &[u8])],
    first: &str,
) -> (TestVault, Win) {
    let v = rt.block_on(test_vault(notes, &[]));
    for (p, bytes) in attachments {
        rt.block_on(v.vault.save_attachment(&path(p), bytes))
            .expect("the attachment is saved");
    }
    let started = Win(open_started(
        cx,
        vault_start(&v, FirstNote::Note(path(first))),
        v.settings.clone(),
    ));
    let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx) && started.listed(cx)
    });
    (v, started)
}

/// The note's text on disk.
fn disk(rt: &Runtime, v: &TestVault, p: &str) -> String {
    rt.block_on(v.vault.get_note_text(&path(p))).unwrap()
}

/// Let more than one autosave tick (1 s) pass on the runtime's real clock.
fn past_a_tick(rt: &Runtime, cx: &mut TestAppContext) {
    for _ in 0..30 {
        rt.block_on(tokio::time::sleep(Duration::from_millis(50)));
        cx.run_until_parked();
    }
}

/// The started window, with this file's drivers (the other test files
/// define methods of the same names on `Started`).
struct Win(Started);

impl std::ops::Deref for Win {
    type Target = Started;
    fn deref(&self) -> &Started {
        &self.0
    }
}

impl Win {
    fn listed(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let files = self.workspace.read(cx).files.clone().unwrap();
            files
                .read(cx)
                .model()
                .core()
                .is_some_and(|core| !core.is_loading() && core.visible_len() > 0)
        })
    }

    fn hit(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn typed(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.input(text, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn click_id(&self, id: &'static str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Double-click FILES' row for the entry at `p`.
    fn open_row(&self, p: &str, cx: &mut TestAppContext) {
        let id = cx.read(|cx| {
            let files = self.workspace.read(cx).files.clone().unwrap();
            let pane = files.read(cx);
            let core = pane.model().core().expect("a listing");
            core.visible_rows()
                .into_iter()
                .find(|r| r.path().is_like(&path(p)))
                .map(|r| row_id(r).to_string())
                .unwrap_or_else(|| panic!("no row {p}"))
        });
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.double_click(id, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// The attachment the editor Panel shows.
    fn shown(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| self.workspace.read(cx).shown_attachment(cx))
    }

    fn showing(&self, p: &str, cx: &TestAppContext) -> bool {
        self.shown(cx).is_some_and(|s| s.is_like(&path(p)))
    }

    /// Open the attachment at `p` from its FILES row and wait for it.
    fn show(&self, rt: &Runtime, p: &str, cx: &mut TestAppContext) {
        self.open_row(p, cx);
        drive(rt, cx, |cx| self.showing(p, cx));
    }

    fn with_view<R>(
        &self,
        cx: &TestAppContext,
        f: impl FnOnce(&super::attachment::AttachmentView) -> R,
    ) -> R {
        cx.read(|cx| {
            let view = self
                .workspace
                .read(cx)
                .active_state()
                .attachment
                .clone()
                .expect("a view");
            f(view.read(cx))
        })
    }

    fn note_open(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .active_state()
                .notes
                .as_ref()?
                .note
                .clone()
        })
    }

    fn is_dirty(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            self.workspace
                .read(cx)
                .active_state()
                .editor
                .read(cx)
                .editor
                .is_dirty()
        })
    }

    /// FILES' rows marked open.
    fn marked(&self, cx: &TestAppContext) -> Vec<VaultPath> {
        cx.read(|cx| {
            let files = self.workspace.read(cx).files.clone().unwrap();
            let pane = files.read(cx);
            pane.model()
                .core()
                .map(|core| {
                    core.visible_rows()
                        .into_iter()
                        .filter(|r| {
                            matches!(
                                r,
                                FileListEntry::Note { is_open: true, .. }
                                    | FileListEntry::Attachment { is_open: true, .. }
                            )
                        })
                        .map(|r| r.path().clone())
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn focused(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused() == Some(true)
        })
        .unwrap()
    }

    /// Hand files and URLs to a recorder instead of the system.
    fn record_opens(&self, cx: &mut TestAppContext) -> Rc<RefCell<Vec<OsString>>> {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let record = opened.clone();
        cx.update(|cx| {
            self.workspace.update(cx, |workspace, _| {
                workspace.opener = Rc::new(move |what| {
                    record.borrow_mut().push(what.to_owned());
                    Ok(())
                });
            })
        });
        opened
    }
}

const TEXT: &[u8] = b"first line\nsecond\tline\n";
/// Not text: a NUL byte.
const PDF: &[u8] = b"%PDF-1.4\x00\x01binary";

fn same(a: &[VaultPath], b: &[&str]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.is_like(&path(b)))
}

// ── the lines (no window) ────────────────────────────────────────────────

#[test]
fn lines_split_once_and_read_without_their_breaks() {
    let lines = Lines::of("a\r\nb\n\nc\n".to_string());
    assert_eq!(lines.len(), 4);
    let all: Vec<&str> = (0..lines.len()).map(|i| lines.line(i)).collect();
    assert_eq!(all, ["a", "b", "", "c"]);
    assert_eq!(lines.line(9), "");
    let one = Lines::of(String::new());
    assert_eq!((one.len(), one.line(0)), (1, ""));
    let open_end = Lines::of("x\ny".to_string());
    assert_eq!((open_end.len(), open_end.line(1)), (2, "y"));
}

#[test]
fn a_long_line_is_cut_and_tabs_widen() {
    assert_eq!(display_line("a\tb").as_ref(), "a    b");
    let long = "é".repeat(LINE_CHARS + 10);
    let shown = display_line(&long);
    assert_eq!(shown.chars().count(), LINE_CHARS + 1);
    assert!(shown.ends_with('\u{2026}'));
    let exact = "x".repeat(LINE_CHARS);
    assert_eq!(display_line(&exact).as_ref(), exact);
}

// ── showing one ──────────────────────────────────────────────────────────

/// A FILES attachment row shows the view in place of the editor: its
/// metadata, its text, no open note, the FILES marker on the attachment,
/// the keyboard in the view, the Footer on its path.
#[gpui_kit::test]
fn a_files_attachment_row_shows_the_view(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("notes.txt", TEXT)],
        "a.md",
    );
    started.show(&rt, "notes.txt", cx);

    let details = rt
        .block_on(v.vault.get_attachment_details(&path("notes.txt")))
        .unwrap();
    started.with_view(cx, |view| {
        assert_eq!(view.loaded.name().as_ref(), "notes.txt");
        assert_eq!(view.loaded.size.as_ref(), "23 B");
        assert_eq!(view.loaded.kind.as_ref(), "TXT");
        assert_eq!(
            view.loaded.modified.as_ref(),
            format_mtime(details.modified_secs)
        );
        let Preview::Text { lines, truncated } = &view.loaded.preview else {
            panic!("a text preview");
        };
        assert!(!truncated);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines.line(1), "second\tline");
    });
    assert!(started.shows(ATTACHMENT_ID, cx));
    assert!(started.shows(PREVIEW_ID, cx));
    assert!(!started.shows(TRUNCATED_ID, cx));
    assert!(!started.shows(NO_PREVIEW_ID, cx));
    assert_eq!(started.note_open(cx), None, "no open note");
    assert!(same(&started.marked(cx), &["notes.txt"]));
    assert!(
        started.focused(ATTACHMENT_ID, cx),
        "the view has the keyboard"
    );
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert_eq!(w.status.note, None);
        assert!(
            w.status
                .attachment
                .as_ref()
                .unwrap()
                .is_like(&path("notes.txt"))
        );
        assert_eq!(w.focused_panel(), Panel::Editor);
    });
    // The Footer has no caret, save state or counts for it.
    let facts = cx.read(|cx| started.workspace.read(cx).doc_facts(false, cx));
    assert_eq!((facts.backlinks, facts.properties), (None, None));
}

/// The open note is flushed before the attachment shows: its edits are on
/// disk, and nothing writes while the attachment shows — not a tick, not
/// the Save chord, not typing.
#[gpui_kit::test]
fn the_dirty_note_is_flushed_and_nothing_saves_while_shown(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("notes.txt", TEXT)],
        "a.md",
    );
    started.typed("X", cx);
    assert!(started.is_dirty(cx));
    started.show(&rt, "notes.txt", cx);
    assert_eq!(disk(&rt, &v, "a.md"), "Xalpha", "flushed before it showed");
    assert!(!started.is_dirty(cx));
    assert_eq!(started.text(cx), "", "the buffer is let go");
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        assert!(!w.can_save(), "no note target");
        assert!(
            w.active_state()
                .notes
                .as_ref()
                .unwrap()
                .session
                .path()
                .is_root_or_empty()
        );
    });

    let file = v.workspace.as_path().join("a.md");
    let stamp = std::fs::metadata(&file).unwrap().modified().unwrap();
    started.typed("zzz", cx);
    started.hit("ctrl-s", cx);
    past_a_tick(&rt, cx);
    assert_eq!(started.text(cx), "", "typing reaches no buffer");
    assert!(!started.is_dirty(cx));
    assert_eq!(disk(&rt, &v, "a.md"), "Xalpha");
    assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), stamp);
    assert_eq!(
        rt.block_on(v.vault.get_attachment_details(&path("notes.txt")))
            .unwrap()
            .size,
        23
    );
    assert!(started.showing("notes.txt", cx));
}

/// A flush that fails refuses the attachment: the note stays open with its
/// edits, its keyboard and its tick.
#[cfg(unix)]
#[gpui_kit::test]
fn a_failed_flush_refuses_the_attachment(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("notes.txt", TEXT)],
        "a.md",
    );
    let file = v.workspace.as_path().join("a.md");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
    started.typed("X", cx);
    started.open_row("notes.txt", cx);
    drive(&rt, cx, |cx| started.flash(cx).is_some());
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Couldn't save a.md:") && flash.ends_with("notes.txt was not opened."),
        "{flash}"
    );
    assert_eq!(started.shown(cx), None);
    assert!(
        started
            .note_open(cx)
            .is_some_and(|n| n.is_like(&path("a.md")))
    );
    assert_eq!(started.text(cx), "Xalpha");
    assert!(started.is_dirty(cx));
    assert!(same(&started.marked(cx), &["a.md"]));
    // The note takes typing again, and saves once it can.
    started.typed("Y", cx);
    assert_eq!(started.text(cx), "XYalpha");
    started.hit("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.is_dirty(cx));
    assert_eq!(disk(&rt, &v, "a.md"), "XYalpha");
}

/// `Open externally` and the `FollowLink` chord hand the file to the
/// system's default program.
#[gpui_kit::test]
fn open_externally_hands_the_file_to_the_system(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("notes.txt", TEXT)],
        "a.md",
    );
    started.show(&rt, "notes.txt", cx);
    let opened = started.record_opens(cx);
    started.click_id(OPEN_EXTERNALLY_ID, cx);
    let want: OsString = v.vault.path_to_pathbuf(&path("notes.txt")).into_os_string();
    assert_eq!(*opened.borrow(), vec![want.clone()]);
    assert_eq!(started.flash(cx).as_deref(), Some("Opening notes.txt…"));
    started.hit("ctrl-enter", cx);
    assert_eq!(*opened.borrow(), vec![want.clone(), want]);
    assert!(started.showing("notes.txt", cx));
}

/// Following a link to an attachment in the vault shows it (a binary file:
/// its metadata only); the note was saved first.
#[gpui_kit::test]
fn following_a_link_to_a_vault_attachment_shows_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("sub/a.md", "[doc](../files/doc.pdf)")],
        &[("files/doc.pdf", PDF)],
        "sub/a.md",
    );
    let opened = started.record_opens(cx);
    started.hit("ctrl-enter", cx);
    drive(&rt, cx, |cx| started.showing("files/doc.pdf", cx));
    assert!(opened.borrow().is_empty(), "not handed to the system");
    started.with_view(cx, |view| {
        assert!(matches!(view.loaded.preview, Preview::None));
        assert_eq!(view.loaded.kind.as_ref(), "PDF");
    });
    assert!(started.shows(NO_PREVIEW_ID, cx));
    assert!(!started.shows(PREVIEW_ID, cx));
    assert_eq!(started.note_open(cx), None);
}

/// A link to a file that is not there says so; the note stays.
#[gpui_kit::test]
fn a_link_to_a_missing_attachment_keeps_the_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "[doc](doc.pdf)")], &[], "a.md");
    started.hit("ctrl-enter", cx);
    drive(&rt, cx, |cx| started.flash(cx).is_some());
    let flash = started.flash(cx).unwrap();
    assert!(flash.starts_with("Couldn't open "), "{flash}");
    assert_eq!(started.shown(cx), None);
    assert!(
        started
            .note_open(cx)
            .is_some_and(|n| n.is_like(&path("a.md")))
    );
}

/// Opening a note leaves the view: the editor is back with the note, the
/// FILES marker moves to it, the note has the keyboard.
#[gpui_kit::test]
fn opening_a_note_returns_to_the_editor(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha"), ("b.md", "bravo")],
        &[("notes.txt", TEXT)],
        "a.md",
    );
    started.show(&rt, "notes.txt", cx);
    started.open_row("b.md", cx);
    drive(&rt, cx, |cx| {
        started.text(cx) == "bravo" && !started.indexing(cx)
    });
    assert_eq!(started.shown(cx), None);
    assert!(!started.shows(ATTACHMENT_ID, cx));
    assert!(
        started
            .note_open(cx)
            .is_some_and(|n| n.is_like(&path("b.md")))
    );
    assert!(same(&started.marked(cx), &["b.md"]));
    assert!(started.focused("editor", cx), "the note has the keyboard");
    cx.read(|cx| assert_eq!(started.workspace.read(cx).status.attachment, None));
    // It saves again.
    started.typed("Z", cx);
    started.hit("ctrl-s", cx);
    drive(&rt, cx, |cx| !started.is_dirty(cx));
}

/// Note-scoped commands say there is no note open while an attachment
/// shows (the leader's rename and properties, F2); none touches the file.
#[gpui_kit::test]
fn note_commands_say_no_note_is_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("notes.txt", TEXT)],
        "a.md",
    );
    started.show(&rt, "notes.txt", cx);
    for keys in [["ctrl-shift-b", "n", "r"], ["ctrl-shift-b", "n", "p"]] {
        for key in keys {
            started.hit(key, cx);
        }
        assert_eq!(started.flash(cx).as_deref(), Some(NO_NOTE), "{keys:?}");
        assert_eq!(started.shown_modal(cx), None);
    }
    started.hit("f2", cx);
    assert_eq!(started.flash(cx).as_deref(), Some(NO_NOTE));
    assert_eq!(started.shown_modal(cx), None);
    // The routes agree (the palette's rows read them).
    cx.read(|cx| {
        let w = started.workspace.read(cx);
        for action in [LeaderAction::NoteRename, LeaderAction::NoteProperties] {
            assert_eq!(
                w.route_of(action),
                kimun_notes::logic::leader_route::LeaderRoute::NoNoteOpen
            );
        }
    });
    assert_eq!(
        rt.block_on(v.vault.get_attachment_details(&path("notes.txt")))
            .unwrap()
            .size,
        23
    );
}

/// A text file longer than core's preview cap shows its first part and
/// says the preview is truncated.
#[gpui_kit::test]
fn a_large_text_preview_is_truncated(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    // 11 MiB of text, past core's 10 MiB cap.
    let big = "0123456789abcde\n".repeat(11 * 1024 * 1024 / 16);
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("big.log", big.as_bytes())],
        "a.md",
    );
    started.show(&rt, "big.log", cx);
    started.with_view(cx, |view| {
        let Preview::Text { lines, truncated } = &view.loaded.preview else {
            panic!("a text preview");
        };
        assert!(truncated);
        assert_eq!(lines.len(), 10 * 1024 * 1024 / 16);
        assert_eq!(view.loaded.size.as_ref(), "11.0 MB");
    });
    assert!(started.shows(TRUNCATED_ID, cx));
    assert!(started.shows(PREVIEW_ID, cx));
}

/// The preview scrolls with the keys while the view has the keyboard.
#[gpui_kit::test]
fn the_preview_scrolls_with_the_keys(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let many = (0..500).map(|i| format!("line {i}\n")).collect::<String>();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("many.txt", many.as_bytes())],
        "a.md",
    );
    started.show(&rt, "many.txt", cx);
    assert!(started.shows(PREVIEW_ID, cx));
    let top = |cx: &TestAppContext| {
        started.with_view(cx, |view| {
            f32::from(view.scroll.0.borrow().base_handle.offset().y)
        })
    };
    assert_eq!(top(cx), 0.0);
    started.hit("pagedown", cx);
    assert!(started.shows(PREVIEW_ID, cx));
    let paged = top(cx);
    assert!(paged < 0.0, "scrolled down: {paged}");
    started.hit("down", cx);
    assert!(top(cx) < paged);
    started.hit("home", cx);
    assert_eq!(top(cx), 0.0);
    started.hit("end", cx);
    assert!(top(cx) < paged);
}

/// Deleting the shown attachment from its FILES row takes the view down and
/// says so; a rename follows it.
#[gpui_kit::test]
fn the_view_follows_a_rename_and_leaves_on_delete(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("notes.txt", TEXT)],
        "a.md",
    );
    started.show(&rt, "notes.txt", cx);
    let workspace = started.workspace.clone();
    let run = |request: super::file_ops::FileOpRequest, cx: &mut TestAppContext| {
        let workspace = workspace.clone();
        cx.update_window(started.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.request_file_op(request, window, cx))
        })
        .unwrap();
    };
    run(
        super::file_ops::FileOpRequest::Rename {
            from: path("notes.txt"),
            to: path("renamed.txt"),
        },
        cx,
    );
    drive(&rt, cx, |cx| started.showing("renamed.txt", cx));
    run(
        super::file_ops::FileOpRequest::Delete(path("renamed.txt")),
        cx,
    );
    drive(&rt, cx, |cx| started.shown(cx).is_none());
    assert_eq!(
        started.placeholder(cx).as_deref(),
        Some("renamed.txt was deleted")
    );
    assert!(started.marked(cx).is_empty());
}

// ── an image ─────────────────────────────────────────────────────────────

/// A PNG of `w`x`h` pixels.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 10, 10, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A fresh image store as the app's, its decodes counted.
fn fresh_store(cx: &mut TestAppContext) -> ImageStore {
    let store = ImageStore::default();
    cx.update(|cx| cx.set_global(store.clone()));
    store
}

fn scale(s: &Started, cx: &mut TestAppContext) -> f32 {
    cx.update_window(s.handle, |_, window, _| window.scale_factor())
        .unwrap()
}

/// `a` is `b` logical pixels, to rounding.
fn near(a: Pixels, b: f32) -> bool {
    (f32::from(a) - b).abs() < 0.01
}

impl Win {
    /// Show the attachment at `p` (no FILES row needed) and wait for it.
    fn show_path(&self, rt: &Runtime, p: &str, cx: &mut TestAppContext) {
        cx.update(|cx| {
            self.workspace
                .update(cx, |w, cx| w.open_attachment(&path(p), cx))
        });
        drive(rt, cx, |cx| self.showing(p, cx));
    }

    /// Draw a frame; the image as painted in it.
    fn painted(&self, cx: &mut TestAppContext) -> Option<PaintedImage> {
        cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
        self.with_view(cx, |view| view.painted_image())
    }

    /// Redraw the window, as a hover or a resize would.
    fn redraw(&self, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            window.refresh();
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Drive until the image paints its pixels; the paint.
    fn until_ready(&self, rt: &Runtime, cx: &mut TestAppContext) -> PaintedImage {
        drive(rt, cx, |cx| self.painted(cx).is_some_and(|p| p.ready));
        self.painted(cx).unwrap()
    }

    fn label_of(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id)
                .and_then(|e| e.label().map(str::to_string))
        })
        .unwrap()
    }
}

/// `rect` is centered in `area`, to a device pixel.
fn is_centered(p: &PaintedImage, scale: f32) -> bool {
    let off = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() <= 1.0 / scale + 0.01;
    off(p.rect.center().x, p.area.center().x) && off(p.rect.center().y, p.area.center().y)
}

/// The area in device pixels.
fn area_px(p: &PaintedImage, scale: f32) -> (f32, f32) {
    (
        f32::from(p.area.size.width) * scale,
        f32::from(p.area.size.height) * scale,
    )
}

const PIC: &str = "pic.png";

#[test]
fn an_image_attachment_loads_as_an_image_preview() {
    let rt = runtime();
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="12"/>"#;
    let v = rt.block_on(async {
        let v = test_vault(&[("a.md", "alpha")], &[]).await;
        for (p, bytes) in [
            ("pic.png", png(4, 4)),
            ("PHOTO.JPG", png(4, 4)),
            ("logo.svg", svg.to_vec()),
            ("pic.bmp", png(4, 4)),
            ("a.zip", PDF.to_vec()),
            ("notes.txt", TEXT.to_vec()),
        ] {
            v.vault.save_attachment(&path(p), &bytes).await.unwrap();
        }
        v
    });
    let preview = |p: &str| rt.block_on(load(&v.vault, &path(p))).unwrap().preview;
    for p in ["pic.png", "PHOTO.JPG", "logo.svg"] {
        match preview(p) {
            Preview::Image { file } => assert_eq!(file, v.vault.path_to_pathbuf(&path(p))),
            _ => panic!("{p} is an image preview"),
        }
    }
    assert!(matches!(preview("pic.bmp"), Preview::None));
    assert!(matches!(preview("a.zip"), Preview::None));
    assert!(matches!(preview("notes.txt"), Preview::Text { .. }));
}

#[gpui_kit::test]
fn an_image_attachment_paints_fitted_to_the_whole_area(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("small.png", &png(40, 30))],
        "a.md",
    );
    started.show(&rt, "small.png", cx);
    let small = started.until_ready(&rt, cx);
    let scale = scale(&started, cx);
    // Natural size, centered.
    assert!(near(small.rect.size.width, 40.0 / scale), "{small:?}");
    assert!(near(small.rect.size.height, 30.0 / scale), "{small:?}");
    assert!(is_centered(&small, scale), "{small:?}");
    // The area is the whole room below the header: wider and taller than
    // the image.
    let (aw, ah) = area_px(&small, scale);
    assert!(aw > 200.0 && ah > 200.0, "{small:?}");
    assert!(started.shows(IMAGE_ID, cx));
    assert!(!started.shows(NO_PREVIEW_ID, cx));

    // Wider than the area: the area's width, aspect kept, centered.
    let wide = png((aw * 2.0) as u32, 40);
    rt.block_on(v.vault.save_attachment(&path("wide.png"), &wide))
        .unwrap();
    started.show_path(&rt, "wide.png", cx);
    let painted = started.until_ready(&rt, cx);
    let k = aw / ((aw * 2.0) as u32) as f32;
    assert!(near(painted.rect.size.width, aw / scale), "{painted:?}");
    assert!(
        near(painted.rect.size.height, 40.0 * k / scale),
        "{painted:?}"
    );
    assert!(is_centered(&painted, scale), "{painted:?}");

    // Taller than the area: the area's height.
    let tall_h = (ah * 3.0) as u32;
    rt.block_on(v.vault.save_attachment(&path("tall.png"), &png(30, tall_h)))
        .unwrap();
    started.show_path(&rt, "tall.png", cx);
    let painted = started.until_ready(&rt, cx);
    let k = ah / tall_h as f32;
    assert!(near(painted.rect.size.height, ah / scale), "{painted:?}");
    assert!(
        near(painted.rect.size.width, 30.0 * k / scale),
        "{painted:?}"
    );
    assert!(is_centered(&painted, scale), "{painted:?}");
}

#[gpui_kit::test]
fn a_taller_image_is_not_capped_at_sixty_percent(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("small.png", &png(4, 4))],
        "a.md",
    );
    started.show(&rt, "small.png", cx);
    let small = started.until_ready(&rt, cx);
    let scale = scale(&started, cx);
    let (_, ah) = area_px(&small, scale);
    let h = (ah * 0.9).floor() as u32;
    rt.block_on(v.vault.save_attachment(&path("tall.png"), &png(20, h)))
        .unwrap();
    started.show_path(&rt, "tall.png", cx);
    let painted = started.until_ready(&rt, cx);
    assert_eq!(painted.area, small.area, "the same area");
    assert!(
        near(painted.rect.size.height, h as f32 / scale),
        "{painted:?}"
    );
    assert!(near(painted.rect.size.width, 20.0 / scale), "{painted:?}");
    assert!(is_centered(&painted, scale), "{painted:?}");
}

#[gpui_kit::test]
fn the_image_fits_again_after_a_resize(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[("big.png", &png(3000, 2000))],
        "a.md",
    );
    let store = fresh_store(cx);
    cx.simulate_window_resize(started.handle, size(px(1400.), px(1000.)));
    started.show(&rt, "big.png", cx);
    let before = started.until_ready(&rt, cx);
    let scale = scale(&started, cx);
    let fits = |p: &PaintedImage| {
        let (aw, ah) = area_px(p, scale);
        let k = (aw / 3000.0).min(ah / 2000.0);
        near(p.rect.size.width, 3000.0 * k / scale)
            && near(p.rect.size.height, 2000.0 * k / scale)
            && is_centered(p, scale)
    };
    assert!(fits(&before), "{before:?}");
    assert_eq!(store.decodes_started(), 1);

    cx.simulate_window_resize(started.handle, size(px(900.), px(700.)));
    started.redraw(cx);
    let after = started.painted(cx).expect("painted");
    assert!(after.ready);
    assert!(
        after.area.size.width < before.area.size.width
            && after.area.size.height < before.area.size.height,
        "{before:?} {after:?}"
    );
    assert!(fits(&after), "{after:?}");
    assert!(after.rect.size.width < before.rect.size.width);
    assert_eq!(store.decodes_started(), 1, "a resize decodes nothing");
}

#[gpui_kit::test]
fn a_pending_image_paints_the_empty_box_of_the_fitted_size_then_the_pixels(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[(PIC, &png(40, 30))],
        "a.md",
    );
    let held = fresh_store(cx);
    held.hold_decodes();
    started.show(&rt, PIC, cx);
    let pending = started.painted(cx).expect("the box paints");
    let scale = scale(&started, cx);
    assert!(!pending.ready, "the empty box");
    assert!(near(pending.rect.size.width, 40.0 / scale), "{pending:?}");
    assert!(near(pending.rect.size.height, 30.0 / scale), "{pending:?}");
    assert!(is_centered(&pending, scale));
    for _ in 0..3 {
        assert!(!started.painted(cx).unwrap().ready);
    }
    assert_eq!(held.decodes_started(), 1, "asked for once while pending");

    // A store that decodes: the pixels land in the same place.
    let store = fresh_store(cx);
    started.redraw(cx);
    let ready = started.until_ready(&rt, cx);
    assert_eq!(
        ready.rect, pending.rect,
        "nothing moves when the pixels land"
    );
    assert_eq!(store.decodes_started(), 1);
}

#[gpui_kit::test]
fn a_failed_decode_shows_no_preview_with_the_failure_note_once(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    // A PNG cut short inside its image data: the header reads, the pixels
    // do not decode.
    let mut bad = png(64, 64);
    bad.truncate(bad.len() - 30);
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[("bad.png", &bad)], "a.md");
    let file = v.vault.path_to_pathbuf(&path("bad.png"));
    assert!(
        matches!(
            ImageStore::default().size_of(&file),
            ImageState::Sized {
                width: 64,
                height: 64
            }
        ),
        "the header reads"
    );
    let store = fresh_store(cx);
    started.show(&rt, "bad.png", cx);
    drive(&rt, cx, |cx| started.shows(NO_PREVIEW_ID, cx));
    let want = Failure::Undecodable.note("bad.png", "");
    for _ in 0..5 {
        assert!(started.shows(NO_PREVIEW_ID, cx));
        assert!(!started.shows(IMAGE_ID, cx));
        assert_eq!(
            started.label_of(NO_PREVIEW_ID, cx).as_deref(),
            Some("No preview")
        );
        assert_eq!(
            started.label_of(NO_PREVIEW_NOTE_ID, cx).as_deref(),
            Some(want.as_str())
        );
        assert!(started.painted(cx).is_none(), "no image paints");
        assert_eq!(store.decodes_started(), 1, "decoded once, never again");
    }
    // The header and the button stay.
    assert!(started.shows(OPEN_EXTERNALLY_ID, cx));
}

#[gpui_kit::test]
fn the_image_is_pinned_while_shown_and_the_pins_drop_with_the_view(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[(PIC, &png(40, 30))],
        "a.md",
    );
    let store = fresh_store(cx);
    assert!(store.pinned_files().is_empty());
    started.show(&rt, PIC, cx);
    started.until_ready(&rt, cx);
    let file = v.vault.path_to_pathbuf(&path(PIC));
    assert_eq!(store.pinned_files(), vec![file.clone()]);
    for _ in 0..3 {
        started.redraw(cx);
    }
    assert_eq!(store.pinned_files(), vec![file]);

    // A note opens: the view goes, and its pin with it.
    started.open_row("a.md", cx);
    drive(&rt, cx, |cx| {
        started.shown(cx).is_none() && started.text(cx) == "alpha"
    });
    assert!(store.pinned_files().is_empty(), "the pins dropped");
}

#[gpui_kit::test]
fn a_finished_decode_requests_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[(PIC, &png(40, 30))],
        "a.md",
    );
    let store = fresh_store(cx);
    assert_eq!(store.decodes_started(), 0);
    started.show(&rt, PIC, cx);
    started.until_ready(&rt, cx);
    assert_eq!(store.decodes_started(), 1);
    for _ in 0..5 {
        started.redraw(cx);
        assert!(started.painted(cx).unwrap().ready);
    }
    assert_eq!(store.decodes_started(), 1, "a ready image asks for nothing");
}

#[gpui_kit::test]
fn an_image_replaced_on_disk_is_decoded_again(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[(PIC, &png(40, 30))],
        "a.md",
    );
    let store = fresh_store(cx);
    started.show(&rt, PIC, cx);
    started.until_ready(&rt, cx);
    assert_eq!(store.decodes_started(), 1);
    let scale = scale(&started, cx);

    // Rewritten from outside the app: another size, so another file key.
    rt.block_on(v.vault.save_attachment(&path(PIC), &png(60, 30)))
        .unwrap();
    started.redraw(cx);
    drive(&rt, cx, |cx| {
        started
            .painted(cx)
            .is_some_and(|p| p.ready && near(p.rect.size.width, 60.0 / scale))
    });
    assert_eq!(store.decodes_started(), 2, "decoded again, once");
    for _ in 0..5 {
        started.redraw(cx);
    }
    assert_eq!(store.decodes_started(), 2, "settled");
    let painted = started.painted(cx).unwrap();
    assert!(painted.ready, "the picture, not the box");
    assert!(near(painted.rect.size.width, 60.0 / scale), "{painted:?}");
}

#[gpui_kit::test]
fn the_header_and_the_open_button_are_unchanged_for_an_image(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let bytes = png(40, 30);
    let (v, started) = started_on(cx, &rt, &[("a.md", "alpha")], &[(PIC, &bytes)], "a.md");
    started.show(&rt, PIC, cx);
    started.until_ready(&rt, cx);
    let details = rt
        .block_on(v.vault.get_attachment_details(&path(PIC)))
        .unwrap();
    started.with_view(cx, |view| {
        assert_eq!(view.loaded.name().as_ref(), PIC);
        assert_eq!(view.loaded.kind.as_ref(), "PNG");
        assert_eq!(
            view.loaded.size.as_ref(),
            kimun_notes::logic::attachment::human_size(bytes.len() as u64)
        );
        assert_eq!(
            view.loaded.modified.as_ref(),
            format_mtime(details.modified_secs)
        );
    });
    assert!(started.shows(ATTACHMENT_ID, cx));
    assert!(started.shows(IMAGE_ID, cx));
    assert!(!started.shows(PREVIEW_ID, cx));
    assert!(!started.shows(NO_PREVIEW_ID, cx));
    assert!(
        started.focused(ATTACHMENT_ID, cx),
        "the view has the keyboard"
    );
    let opened = started.record_opens(cx);
    started.click_id(OPEN_EXTERNALLY_ID, cx);
    let want: OsString = v.vault.path_to_pathbuf(&path(PIC)).into_os_string();
    assert_eq!(*opened.borrow(), vec![want.clone()]);
    assert_eq!(started.flash(cx).as_deref(), Some("Opening pic.png…"));
    started.hit("ctrl-enter", cx);
    assert_eq!(*opened.borrow(), vec![want.clone(), want]);
    // The scroll keys do nothing to an image, and nothing breaks.
    started.hit("pagedown", cx);
    assert!(started.painted(cx).unwrap().ready);
}

#[gpui_kit::test]
fn a_text_attachment_and_a_binary_attachment_render_as_before(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "alpha")],
        &[
            ("notes.txt", TEXT),
            ("doc.pdf", PDF),
            ("pic.bmp", &png(4, 4)),
        ],
        "a.md",
    );
    let store = fresh_store(cx);
    started.show(&rt, "notes.txt", cx);
    assert!(started.shows(PREVIEW_ID, cx));
    assert!(!started.shows(IMAGE_ID, cx));
    assert!(!started.shows(NO_PREVIEW_ID, cx));
    assert!(started.painted(cx).is_none());
    for binary in ["doc.pdf", "pic.bmp"] {
        started.show(&rt, binary, cx);
        started.with_view(cx, |view| {
            assert!(matches!(view.loaded.preview, Preview::None), "{binary}")
        });
        assert!(started.shows(NO_PREVIEW_ID, cx), "{binary}");
        assert!(!started.shows(IMAGE_ID, cx));
        assert!(!started.shows(PREVIEW_ID, cx));
        assert_eq!(
            started.label_of(NO_PREVIEW_NOTE_ID, cx).as_deref(),
            Some(NO_PREVIEW_NOTE)
        );
        assert_eq!(NO_PREVIEW_NOTE, "Open it externally to view it.");
        assert!(started.painted(cx).is_none());
    }
    assert_eq!(store.decodes_started(), 0, "nothing decodes");
    assert!(store.pinned_files().is_empty(), "nothing pinned");
}
