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
use crate::inline_image::ImageStore;
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

/// A vault holding `journal/n.md`, which embeds `![[pic.png]]`, and the
/// file in a root `assets` folder, started on the note, driven until the
/// image is drawn (the closest-file rule found it).
fn started_in_journal(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
    const TEXT: &str = "intro\n![[pic.png]]\nafter";
    let v = rt.block_on(test_vault(&[("journal/n.md", TEXT)], &[]));
    rt.block_on(
        v.vault
            .save_attachment(&path("assets/pic.png"), &png(10, 60)),
    )
    .expect("the attachment is saved");
    let s = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("journal/n.md"))),
        v.settings.clone(),
    );
    drive(rt, cx, |cx| {
        s.text(cx) == TEXT && !s.indexing(cx) && drawn(&s, cx)
    });
    (v, s)
}

/// The `FollowLink` chord on a bare `![[name]]` whose file is in another
/// folder shows the file the image draws, not a file of that name beside
/// the note.
#[gpui_kit::test]
fn ctrl_enter_on_a_bare_embed_shows_the_file_it_draws(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started_in_journal(cx, &rt);
    let opened = record_opens(&s, cx);
    hit(&s, "down", cx);
    hit(&s, "ctrl-enter", cx);
    drive(&rt, cx, |cx| shown(&s, cx).is_some());
    let shown = shown(&s, cx).unwrap();
    assert!(shown.is_like(&path("assets/pic.png")), "{shown}");
    assert!(opened.borrow().is_empty(), "not handed to the system");
}

/// A double-click on that image shows the same file.
#[gpui_kit::test]
fn a_double_click_on_a_bare_embed_shows_the_file_it_draws(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started_in_journal(cx, &rt);
    let view = editor(&s, cx);
    cx.update(|cx| view.update(cx, |v, _| v.set_clock(crate::editor_view::frozen_clock)));
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.double_click(image_hover_id(1), cx);
    })
    .unwrap();
    cx.run_until_parked();
    drive(&rt, cx, |cx| shown(&s, cx).is_some());
    let shown = shown(&s, cx).unwrap();
    assert!(shown.is_like(&path("assets/pic.png")), "{shown}");
}

/// The user leaves the window and comes back to it (the app has no file
/// watcher: what changed on disk meanwhile is read then).
fn reactivate(s: &Started, cx: &mut TestAppContext) {
    gpui_kit::VisualTestContext::from_window(s.handle, cx).deactivate_window();
    cx.update_window(s.handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
}

fn store(cx: &TestAppContext) -> ImageStore {
    cx.read(|cx| cx.global::<ImageStore>().clone())
}

/// The lines the editor's image lines take under row 1, the image's.
fn image_finds(s: &Started, cx: &TestAppContext) -> usize {
    let view = editor(s, cx);
    cx.read(|cx| view.read(cx).image_finds())
}

/// An image file saved into the vault from outside the app while its note
/// is open, the embed already written: it is drawn once the window is
/// active again.
#[gpui_kit::test]
fn an_image_saved_from_outside_is_drawn_after_an_activation(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    const TEXT: &str = "intro\n![](pic.png)\nafter";
    let v = rt.block_on(test_vault(&[("a.md", TEXT)], &[]));
    let s = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| {
        s.text(cx) == TEXT
            && !s.indexing(cx)
            && notes(&s, cx) == ["image not found: pic.png".to_string()]
    });
    rt.block_on(v.vault.save_attachment(&path("pic.png"), &png(10, 60)))
        .expect("the attachment is saved");
    reactivate(&s, cx);
    drive(&rt, cx, |cx| drawn(&s, cx));
    assert!(notes(&s, cx).is_empty());
}

/// An image file rewritten from outside the app (a diagram exported again,
/// here twice as tall): after an activation it is decoded again and takes
/// its new room.
#[gpui_kit::test]
fn an_image_rewritten_from_outside_is_decoded_again_after_an_activation(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, s) = started(cx, &rt);
    let view = editor(&s, cx);
    let pads = |cx: &TestAppContext| {
        cx.read(|cx| {
            view.read(cx)
                .editor
                .session()
                .display
                .layout
                .visual_lines()
                .iter()
                .filter(|l| l.pad && l.logical_row == 1)
                .count()
        })
    };
    let (before, decodes) = (pads(cx), store(cx).decodes_started());
    rt.block_on(v.vault.save_attachment(&path("pic.png"), &png(10, 120)))
        .expect("the attachment is saved");
    reactivate(&s, cx);
    drive(&rt, cx, |cx| drawn(&s, cx));
    assert_eq!(store(cx).decodes_started(), decodes + 1, "decoded again");
    assert!(pads(cx) > before, "{} pad lines, were {before}", pads(cx));
    let size = cx.read(|cx| {
        view.read(cx)
            .painted
            .as_ref()
            .and_then(|p| p.images.iter().find(|i| i.row == 1).map(|i| i.rect.size))
    });
    let scale = cx
        .update_window(s.handle, |_, window, _| window.scale_factor())
        .unwrap();
    assert_eq!(
        size.map(|s| (s.width * scale, s.height * scale)),
        Some((gpui_kit::px(10.), gpui_kit::px(120.)))
    );
}

/// An activation with nothing changed on disk finds the lines again for
/// nothing and decodes nothing.
#[gpui_kit::test]
fn an_activation_with_nothing_changed_redoes_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    let (finds, decodes) = (image_finds(&s, cx), store(cx).decodes_started());
    for _ in 0..3 {
        reactivate(&s, cx);
        drive(&rt, cx, |cx| drawn(&s, cx));
    }
    assert_eq!(image_finds(&s, cx), finds, "no lines found again");
    assert_eq!(store(cx).decodes_started(), decodes, "no decode");
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

/// Whether the image's entry menu is open.
fn entry_menu_open(s: &Started, cx: &mut TestAppContext) -> bool {
    let any_menu = cx
        .update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            !window.find_all("popup-menu").is_empty()
        })
        .unwrap();
    any_menu
        && menu_labels(s, cx)
            .iter()
            .any(|l| l == "Open in default app")
}

/// With a selection a right press on the image keeps its text meaning (it
/// copies the selection): no entry menu.
#[gpui_kit::test]
fn a_right_click_on_an_image_with_a_selection_opens_no_entry_menu(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    hit(&s, "shift-down", cx);
    let view = editor(&s, cx);
    assert!(
        cx.read(|cx| view.read(cx).editor.selection())
            .is_some_and(|(start, end)| start != end),
        "a selection"
    );
    right_click_image(&s, cx);
    assert!(!entry_menu_open(&s, cx));
}

/// The find bar owns the pointer while open: no entry menu.
#[gpui_kit::test]
fn a_right_click_on_an_image_with_the_find_bar_open_opens_no_entry_menu(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started(cx, &rt);
    hit(&s, "ctrl-f", cx);
    let view = editor(&s, cx);
    assert!(cx.read(|cx| view.read(cx).editor.find_bar().is_some()));
    right_click_image(&s, cx);
    assert!(!entry_menu_open(&s, cx));
}

/// A failed image's note is no image: a right press on it opens the Link
/// menu of the embed's link, as before images were drawn, not an entry
/// menu.
#[gpui_kit::test]
fn a_right_click_on_a_failed_image_note_opens_the_link_menu(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    const TEXT: &str = "intro\n![a](gone.png)\nafter";
    let v = rt.block_on(test_vault(&[("a.md", TEXT)], &[]));
    let s = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    drive(&rt, cx, |cx| {
        s.text(cx) == TEXT
            && !s.indexing(cx)
            && notes(&s, cx) == ["image not found: gone.png (a)".to_string()]
    });
    let view = editor(&s, cx);
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(image_hover_id(1)).is_none(), "no image");
        let p = view.read(cx).painted.clone().expect("painted");
        let editor = window.find("editor").bounds();
        let at = gpui_kit::point(
            p.origin.x + gpui_kit::px(4.),
            p.origin.y + gpui_kit::px(p.cells.y_of(1) + p.cells.height() / 2.),
        );
        window.click_with_options(
            "editor",
            gpui_kit::test::ClickOptions::new()
                .with_button(gpui_kit::MouseButton::Right)
                .with_offset(at - editor.origin),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!entry_menu_open(&s, cx));
    let (note_menu, link_menu) = cx.read(|cx| {
        let w = s.workspace.read(cx);
        (w.note_menu.is_some(), w.link_menu.is_some())
    });
    assert!(link_menu && !note_menu, "the Link menu of its link");
}

/// A read-only editor (a facet view's piece, the Ask source reader) never
/// opens an entry menu, even handed one.
#[gpui_kit::test]
fn a_read_only_editor_has_no_image_menu(cx: &mut TestAppContext) {
    let view = cx.new(|cx| EditorView::new_read_only("![a](pic.png)", cx));
    let menu: crate::editor_view::EntryMenu = Rc::new(|menu, _, _, _| menu);
    let open = cx.update(|cx| {
        view.update(cx, |view, _| {
            view.set_entry_menu(Some(menu));
            view.image_menu().is_some()
        })
    });
    assert!(!open);
}
