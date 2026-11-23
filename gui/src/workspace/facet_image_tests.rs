//! UI tests for **Inline image**s in the **Facet view** (CONTEXT.md): each
//! chunk is a read-only editor that draws the images its text embeds, from
//! its own note, and never shows an embed line's raw text (ADR-0062). A
//! `FacetView` alone in a headless window over a real temp vault.
#![cfg(test)]

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, point, px};
use kimun_core::nfs::VaultPath;

use super::facet::{FacetEvent, FacetView, MemberInit};
use super::facet_tests::{Events, chunk, open_with, settle};
use crate::editor_element::image_hover_id;
use crate::editor_font;
use crate::editor_view::{EditorView, line_height};
use crate::inline_image::{Failure, ImageStore, fit, rows_for};
use crate::test_vault::{TestVault, test_vault};

/// A PNG of `w`x`h` pixels.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 10, 10, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A vault holding `notes` (path, text) and the attachments `files`.
fn vault_with(notes: &[(&str, &str)], files: &[(&str, Vec<u8>)]) -> TestVault {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let v = test_vault(notes, &[]).await;
        for (path, bytes) in files {
            v.vault
                .save_attachment(&VaultPath::new(*path), bytes)
                .await
                .unwrap();
        }
        v
    })
}

/// A facet view of the note `note` (whose text is `text`) in `v`, with
/// images from `v`'s vault, drawn until it rests.
fn open_note<'a>(
    cx: &'a mut TestAppContext,
    v: &TestVault,
    note: &str,
    text: &str,
) -> (
    Entity<FacetView>,
    &'a mut VisualTestContext,
    Events,
    gpui_kit::Subscription,
) {
    let (note, text, vault) = (VaultPath::new(note), text.to_string(), v.vault.clone());
    let (view, cx, events, sub) = open_with(cx, move |cx| {
        let mut view = FacetView::new(note, text, Vec::new(), cx);
        view.set_image_vault(Some(vault), cx);
        view
    });
    rest(cx);
    (view, cx, events, sub)
}

/// Draw and run what each frame asks for (the attachment walk, the
/// decodes) until nothing is left.
fn rest(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        settle(cx);
    }
}

/// What chunk `i` painted on screen row `row`: its glyphs, joined.
fn row_text(view: &Entity<FacetView>, i: usize, row: usize, cx: &mut VisualTestContext) -> String {
    let chunk = chunk(view, i, cx);
    cx.update(|_, cx| {
        let painted = chunk
            .read(cx)
            .painted
            .clone()
            .expect("the chunk was painted");
        painted.row_paint[row]
            .pieces
            .iter()
            .map(|p| p.text.as_str())
            .collect::<String>()
            .trim()
            .to_string()
    })
}

/// The rows of the image `w`x`h` in the window now: its natural size
/// fits (small), so it takes its height over one line's.
fn image_rows(h: u32, cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| {
        let row_px = f32::from(line_height(editor_font::font_size(cx))) * window.scale_factor();
        let (_, height) = fit((10, h), 10_000.0, 10_000.0);
        rows_for(height, row_px)
    })
}

/// The image files chunk `i` shows (and pins).
fn shown_files(view: &Entity<FacetView>, i: usize, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    let chunk = chunk(view, i, cx);
    cx.update(|_, cx| chunk.read(cx).shown_image_files())
}

fn pinned(cx: &mut VisualTestContext) -> Vec<PathBuf> {
    cx.update(|_, cx| cx.default_global::<ImageStore>().clone().pinned_files())
}

const NOTE: &str = "intro\n![[pic.png]]\nafter";

#[gpui_kit::test]
fn a_chunk_with_an_embed_draws_the_image_and_grows_by_its_rows(cx: &mut TestAppContext) {
    let v = vault_with(&[("n.md", NOTE)], &[("pic.png", png(10, 60))]);
    let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", NOTE);
    let rows = image_rows(60, cx);
    assert!(rows > 1, "the image takes more than its line: {rows}");
    let chunk = chunk(&view, 0, cx);
    let (measured, painted) = cx.update(|_, cx| {
        let editor = chunk.read(cx);
        (
            editor.measured_rows,
            editor.painted.clone().expect("painted"),
        )
    });
    // Two text rows and the image, which takes the embed line and its pads.
    assert_eq!(measured, Some(2 + rows));
    let images: Vec<_> = painted.images.iter().map(|i| (i.row, i.ready)).collect();
    assert_eq!(images, [(1, true)], "the image of row 1 is drawn");
    assert_eq!(row_text(&view, 0, 2 + rows - 1, cx), "after");
}

#[gpui_kit::test]
fn an_embed_line_in_a_chunk_never_shows_raw_text(cx: &mut TestAppContext) {
    // The embed is the chunk's first line: the caret's row.
    let text = "![[pic.png]]\nafter";
    let v = vault_with(&[("n.md", text)], &[("pic.png", png(10, 60))]);
    let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", text);
    let chunk = chunk(&view, 0, cx);
    let cursor = cx.update(|_, cx| chunk.read(cx).editor.cursor());
    assert_eq!(cursor, (0, 0), "the hidden caret sits on the embed line");
    assert_eq!(
        row_text(&view, 0, 0, cx),
        "",
        "no raw text on the embed row"
    );
    let painted = cx.update(|_, cx| chunk.read(cx).painted.clone().expect("painted"));
    assert_eq!(painted.images.len(), 1, "the image is drawn there");
}

#[gpui_kit::test]
fn a_collection_resolves_a_bare_name_by_its_own_note(cx: &mut TestAppContext) {
    let text = "![[same.png]]";
    let v = vault_with(
        &[("a/n.md", text), ("b/n.md", text)],
        &[("a/same.png", png(10, 20)), ("b/same.png", png(10, 40))],
    );
    let vault = v.vault.clone();
    let members = ["a/n.md", "b/n.md"]
        .iter()
        .map(|path| MemberInit {
            path: VaultPath::new(*path),
            title: path.to_string(),
            text: text.to_string(),
        })
        .collect();
    let (view, cx, _events, _sub) = open_with(cx, move |cx| {
        let mut view = FacetView::new_collection_at("q".into(), members, 2, Vec::new(), noon, cx);
        view.set_image_vault(Some(vault), cx);
        view
    });
    rest(cx);
    let file = |p: &str| v.vault.path_to_pathbuf(&VaultPath::new(p));
    assert_eq!(shown_files(&view, 0, cx), [file("a/same.png")]);
    assert_eq!(shown_files(&view, 1, cx), [file("b/same.png")]);
}

fn noon() -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap()
}

#[gpui_kit::test]
fn a_failed_image_shows_its_note_in_the_chunk(cx: &mut TestAppContext) {
    let text = "intro\n![[gone.png]]\nafter";
    let v = vault_with(&[("n.md", text)], &[("pic.png", png(10, 60))]);
    let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", text);
    let chunk = chunk(&view, 0, cx);
    let (painted, measured) = cx.update(|_, cx| {
        let editor = chunk.read(cx);
        (
            editor.painted.clone().expect("painted"),
            editor.measured_rows,
        )
    });
    assert_eq!(
        painted.image_notes,
        [(1, Failure::Missing.note("gone.png", ""))]
    );
    assert!(painted.images.is_empty());
    assert_eq!(row_text(&view, 0, 1, cx), "", "the note, not the markdown");
    assert_eq!(measured, Some(3), "the note takes the embed line only");
}

#[gpui_kit::test]
fn a_chunk_drops_its_pins_when_it_is_dropped(cx: &mut TestAppContext) {
    let v = vault_with(&[("n.md", NOTE)], &[("pic.png", png(10, 60))]);
    let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", NOTE);
    let file = v.vault.path_to_pathbuf(&VaultPath::new("pic.png"));
    assert_eq!(pinned(cx), [file]);
    // New text with nothing in it: the chunk's editor goes.
    view.update(cx, |view, cx| view.set_text(String::new(), cx));
    rest(cx);
    assert_eq!(view.read_with(cx, |view, _| view.chunks().len()), 0);
    assert!(pinned(cx).is_empty(), "the dropped chunk pins nothing");
}

#[gpui_kit::test]
fn an_embed_inside_a_prose_chunk_line_stays_text(cx: &mut TestAppContext) {
    let text = "see ![[pic.png]] here";
    let v = vault_with(&[("n.md", text)], &[("pic.png", png(10, 60))]);
    let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", text);
    let chunk = chunk(&view, 0, cx);
    let (painted, measured) = cx.update(|_, cx| {
        let editor = chunk.read(cx);
        (
            editor.painted.clone().expect("painted"),
            editor.measured_rows,
        )
    });
    assert!(painted.images.is_empty() && painted.image_notes.is_empty());
    assert_eq!(measured, Some(1));
    assert!(row_text(&view, 0, 0, cx).starts_with("see"));
    assert!(pinned(cx).is_empty());
}

#[gpui_kit::test]
fn a_read_only_editor_opens_no_image_menu_and_no_follow(cx: &mut TestAppContext) {
    let v = vault_with(&[("n.md", NOTE)], &[("pic.png", png(10, 60))]);
    let (view, cx, events, _sub) = open_note(cx, &v, "n.md", NOTE);
    let chunk: Entity<EditorView> = chunk(&view, 0, cx);
    // Even with an entry menu handed over, a read-only editor never opens it.
    let built = Rc::new(Cell::new(0));
    let count = built.clone();
    chunk.update(cx, |editor, _| {
        editor.set_entry_menu(Some(Rc::new(move |menu, _, _, _| {
            count.set(count.get() + 1);
            menu
        })))
    });
    rest(cx);
    let painted = cx.update(|_, cx| chunk.read(cx).painted.clone().expect("painted"));
    assert_eq!(painted.images.len(), 1, "the image is drawn");
    let rect = painted.images[0].rect;
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.right_click(image_hover_id(1), cx);
    });
    cx.run_until_parked();
    assert_eq!(built.get(), 0, "no image menu");
    // A click on the image follows nothing.
    let at = point(
        rect.origin.x + px(4.),
        rect.origin.y + rect.size.height / 2.0,
    );
    super::facet_tests::press(cx, at, false);
    super::facet_tests::press(cx, at, true);
    cx.update(|_, cx| {
        let editor = chunk.read(cx);
        assert_eq!(editor.image_menu().map(|_| ()), None);
        assert_eq!(editor.image_at_cursor(), None, "no follow from the caret");
    });
    assert!(
        !events
            .borrow()
            .iter()
            .any(|e| matches!(e, FacetEvent::Follow { .. } | FacetEvent::LinkMenu { .. })),
        "{:?}",
        events.borrow()
    );
}

/// The window became active again with a file saved from outside the app:
/// the shared list is listed again and the chunk draws the new image.
#[gpui_kit::test]
fn a_refresh_lists_the_attachments_again(cx: &mut TestAppContext) {
    let text = "intro\n![[late.png]]\nafter";
    let v = vault_with(&[("n.md", text)], &[("pic.png", png(10, 60))]);
    let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", text);
    let chunk = chunk(&view, 0, cx);
    let notes = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| chunk.read(cx).painted.clone().expect("painted").image_notes)
    };
    assert_eq!(notes(cx).len(), 1, "not found yet");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(
            v.vault
                .save_attachment(&VaultPath::new("late.png"), &png(10, 60)),
        )
        .unwrap();
    let vault = v.vault.clone();
    cx.update(|_, cx| {
        cx.default_global::<crate::attachment_index::AttachmentIndex>()
            .refresh(&vault)
    });
    view.update(cx, |view, cx| view.refresh_images(cx));
    rest(cx);
    assert!(notes(cx).is_empty());
    let file = v.vault.path_to_pathbuf(&VaultPath::new("late.png"));
    assert_eq!(shown_files(&view, 0, cx), [file]);
}
