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
use gpui_kit::{AppContext as _, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::attachment::format_mtime;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::Panel;
use tokio::runtime::Runtime;

use super::attachment::{
    ATTACHMENT_ID, LINE_CHARS, Lines, NO_PREVIEW_ID, OPEN_EXTERNALLY_ID, PREVIEW_ID, Preview,
    TRUNCATED_ID, display_line,
};
use super::files::row_id;
use super::route::NO_NOTE;
use super::start_tests::{Started, drive, open_started, runtime, vault_start};
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
            let view = self.workspace.read(cx).attachment.clone().expect("a view");
            f(view.read(cx))
        })
    }

    fn note_open(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| self.workspace.read(cx).notes.as_ref()?.note.clone())
    }

    fn is_dirty(&self, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).editor.read(cx).editor.is_dirty())
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
        assert!(w.notes.as_ref().unwrap().session.path().is_root_or_empty());
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
    drive(&rt, cx, |cx| started.text(cx) == "bravo");
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
