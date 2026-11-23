//! UI integration tests for an **Inline image** in a note's editor
//! (CONTEXT.md): what the pointer and the `FollowLink` chord do on one, and
//! what a file operation on its attachment leaves. A real `Workspace` on a
//! real temp vault; the system's default program is replaced by a recorder.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use std::cell::RefCell;
use std::ffi::OsString;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops::ValidationState;
use tokio::runtime::Runtime;

use super::file_ops::FileDialogView;
use super::modal::ModalKind;
use super::row_menu_tests::in_workspace::{menu_labels, pick};
use super::start_tests::{Started, drive, open_started, runtime, select_all_key, vault_start};
use crate::editor_element::image_hover_id;
use crate::editor_view::EditorView;
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

/// A PNG of `w`x`h` pixels.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 10, 10, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// The note: an image between two lines of text.
const NOTE: &str = "intro\n![a](pic.png)\nafter";

/// A vault holding `a.md` (`NOTE`) and `pic.png`, started on `a.md`, driven
/// until the image is drawn.
fn started(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(&[("a.md", NOTE)], &[]));
    rt.block_on(v.vault.save_attachment(&path("pic.png"), &png(10, 60)))
        .expect("the attachment is saved");
    let s = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(rt, cx, |cx| {
        s.text(cx) == NOTE && !s.indexing(cx) && drawn(&s, cx)
    });
    (v, s)
}

fn editor(s: &Started, cx: &TestAppContext) -> Entity<EditorView> {
    cx.read(|cx| s.workspace.read(cx).active_state().editor.clone())
}

/// Draw a frame; whether the image of row 1 is painted.
fn drawn(s: &Started, cx: &mut TestAppContext) -> bool {
    render(s, cx);
    let view = editor(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .painted
            .as_ref()
            .is_some_and(|p| p.images.iter().any(|i| i.row == 1 && i.ready))
    })
}

/// Draw a frame; the notes of failed images it painted.
fn notes(s: &Started, cx: &mut TestAppContext) -> Vec<String> {
    render(s, cx);
    let view = editor(s, cx);
    cx.read(|cx| {
        view.read(cx)
            .painted
            .as_ref()
            .map(|p| p.image_notes.iter().map(|(_, n)| n.clone()).collect())
            .unwrap_or_default()
    })
}

fn render(s: &Started, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.refresh();
        window.render_frame(cx);
    })
    .unwrap();
}

fn hit(s: &Started, key: &str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

/// Hand files and URLs to a recorder instead of the system.
fn record_opens(s: &Started, cx: &mut TestAppContext) -> Rc<RefCell<Vec<OsString>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let record = opened.clone();
    cx.update(|cx| {
        s.workspace.update(cx, |workspace, _| {
            workspace.opener = Rc::new(move |what| {
                record.borrow_mut().push(what.to_owned());
                Ok(())
            });
        })
    });
    opened
}

fn shown(s: &Started, cx: &TestAppContext) -> Option<VaultPath> {
    cx.read(|cx| s.workspace.read(cx).shown_attachment(cx))
}

/// Right-click the image of row 1.
fn right_click_image(s: &Started, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.right_click(image_hover_id(1), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// A right-click on a drawn image opens the attachment's entry menu: the
/// ways to open it, `Open in default app` among them (the menu's "Open
/// externally"), the helpers and the file operations. The note menu stays
/// shut and the caret does not move.
#[gpui_kit::test]
fn a_right_click_on_an_image_opens_the_entry_menu_with_open_externally(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, s) = started(cx, &rt);
    let opened = record_opens(&s, cx);
    right_click_image(&s, cx);
    assert_eq!(
        menu_labels(&s, cx),
        [
            "Open",
            "Open in new tab",
            "Open in default app",
            "-",
            "Copy path",
            "Reveal in file browser",
            "-",
            "Rename…",
            "Move…",
            "Delete…",
        ]
    );
    assert!(
        cx.read(|cx| s.workspace.read(cx).note_menu.is_none()),
        "no note menu"
    );
    let view = editor(&s, cx);
    assert_eq!(cx.read(|cx| view.read(cx).editor.cursor()), (0, 0));
    pick(&s, "Open in default app", cx);
    drive(&rt, cx, |_| !opened.borrow().is_empty());
    let want: OsString = v.vault.path_to_pathbuf(&path("pic.png")).into_os_string();
    assert_eq!(*opened.borrow(), vec![want]);
    assert_eq!(shown(&s, cx), None, "the note stays");
}

/// `Open` from the image's menu shows the image in the attachment view.
#[gpui_kit::test]
fn open_from_the_image_menu_shows_the_attachment(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    right_click_image(&s, cx);
    pick(&s, "Open", cx);
    drive(&rt, cx, |cx| {
        shown(&s, cx).is_some_and(|p| p.is_like(&path("pic.png")))
    });
}

/// The `FollowLink` chord on an embed line does what it does on any link
/// to an attachment in the vault: it shows the image in the attachment
/// view (not the default program).
#[gpui_kit::test]
fn ctrl_enter_on_an_image_line_shows_the_attachment(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    let opened = record_opens(&s, cx);
    hit(&s, "down", cx);
    hit(&s, "home", cx);
    let view = editor(&s, cx);
    assert_eq!(cx.read(|cx| view.read(cx).editor.cursor()), (1, 0));
    hit(&s, "ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        shown(&s, cx).is_some_and(|p| p.is_like(&path("pic.png")))
    });
    assert!(opened.borrow().is_empty(), "not handed to the system");
}

/// A double-click on an image keeps the editor's double-click: the first
/// press puts the caret on the embed line, the second follows the link
/// there, as the chord does.
#[gpui_kit::test]
fn a_double_click_on_an_image_follows_its_link(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    let view = editor(&s, cx);
    cx.update(|cx| view.update(cx, |v, _| v.set_clock(crate::editor_view::frozen_clock)));
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(image_hover_id(1), cx);
    })
    .unwrap();
    cx.run_until_parked();
    drive(&rt, cx, |cx| {
        shown(&s, cx).is_some_and(|p| p.is_like(&path("pic.png")))
    });
}

/// Rename from the image's menu renames the file and leaves the note's
/// text alone (the attachment rule): the embed line then says the image is
/// not found, without reopening the note.
#[gpui_kit::test]
fn rename_from_the_menu_leaves_the_embed_text_and_shows_the_not_found_note(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, s) = started(cx, &rt);
    right_click_image(&s, cx);
    pick(&s, "Rename…", cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Rename));
    let dialog = cx.read(|cx| match s.workspace.read(cx).file_dialog.clone() {
        Some(FileDialogView::Rename(view)) => view.upgrade().expect("the rename dialog"),
        _ => panic!("no rename dialog"),
    });
    let target = cx.read(|cx| dialog.read(cx).target().clone());
    assert!(target.is_like(&path("pic.png")), "{target}");
    hit(&s, select_all_key(), cx);
    cx.update_window(s.handle, |_, window, cx| window.input("other.png", cx))
        .unwrap();
    drive(&rt, cx, |cx| {
        cx.read(|cx| dialog.read(cx).state()) != ValidationState::Pending
    });
    assert_eq!(
        cx.read(|cx| dialog.read(cx).state()),
        ValidationState::Available
    );
    hit(&s, "enter", cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| s.workspace.read(cx).file_task.is_none())
    });
    assert!(rt.block_on(v.vault.exists(&path("other.png"))));
    assert_eq!(s.text(cx), NOTE, "the embed text is unchanged");
    drive(&rt, cx, |cx| {
        notes(&s, cx) == ["image not found: pic.png (a)".to_string()]
    });
    assert!(!drawn(&s, cx));
}

/// Delete from the image's menu: the embed line says the image is not
/// found.
#[gpui_kit::test]
fn delete_from_the_menu_shows_the_not_found_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, s) = started(cx, &rt);
    right_click_image(&s, cx);
    pick(&s, "Delete…", cx);
    s.settle_dialog(cx);
    assert_eq!(s.shown_modal(cx), Some(ModalKind::Delete));
    hit(&s, "enter", cx);
    drive(&rt, cx, |cx| {
        cx.read(|cx| s.workspace.read(cx).file_task.is_none())
            && !rt.block_on(v.vault.exists(&path("pic.png")))
    });
    assert_eq!(s.text(cx), NOTE);
    drive(&rt, cx, |cx| {
        notes(&s, cx) == ["image not found: pic.png (a)".to_string()]
    });
}
