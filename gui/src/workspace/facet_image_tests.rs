//! UI tests for **Inline image**s in the **Facet view** (CONTEXT.md): each
//! chunk is a read-only editor that draws the images its text embeds, from
//! its own note, and never shows an embed line's raw text (ADR-0062). A
//! `FacetView` alone in a headless window over a real temp vault.
#![cfg(test)]

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
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

/// A press on chunk `i`'s cell (`col`, `row`) with `button`, the primary
/// modifier held when `primary`, and its release.
fn press_at(
    view: &Entity<FacetView>,
    (col, row): (u16, u16),
    button: gpui_kit::MouseButton,
    primary: bool,
    cx: &mut VisualTestContext,
) {
    let at = super::facet_tests::cell(view, 0, col, row, cx);
    cx.update(|window, cx| {
        if primary {
            window.change_modifiers(gpui_kit::Modifiers::control(), cx);
        }
        window.pointer_down(at, button, cx);
        window.pointer_up(at, button, cx);
        if primary {
            window.change_modifiers(gpui_kit::Modifiers::none(), cx);
        }
    });
    cx.run_until_parked();
}

/// The follows and Link menus `events` holds.
fn follows(events: &Events) -> Vec<FacetEvent> {
    events
        .borrow()
        .iter()
        .filter(|e| matches!(e, FacetEvent::Follow { .. } | FacetEvent::LinkMenu { .. }))
        .cloned()
        .collect()
}

/// Every press on the embed line itself (the image's first row), inside
/// the hidden link's columns: a right press, a plain click, a primary
/// press. None follows the hidden link or opens its Link menu.
fn assert_no_follow_on_embed_row(text: &str, cx: &mut TestAppContext) {
    use gpui_kit::MouseButton::{Left, Right};
    let v = vault_with(&[("n.md", text)], &[("pic.png", png(10, 60))]);
    let (view, cx, events, _sub) = open_note(cx, &v, "n.md", text);
    let mut followed = Vec::new();
    // Each on its own cell: a press right after a click that followed, on
    // the same cell, is a plain press.
    for (col, button, primary) in [(3, Right, false), (5, Left, false), (7, Left, true)] {
        press_at(&view, (col, 1), button, primary, cx);
        rest(cx);
        if !follows(&events).is_empty() {
            followed.push(format!("{button:?} primary {primary}"));
        }
        events.borrow_mut().clear();
    }
    assert_eq!(followed, Vec::<String>::new(), "these presses followed");
}

#[gpui_kit::test]
fn a_press_on_the_first_row_of_an_image_follows_nothing(cx: &mut TestAppContext) {
    assert_no_follow_on_embed_row(NOTE, cx);
}

#[gpui_kit::test]
fn a_press_on_a_failed_images_note_follows_nothing(cx: &mut TestAppContext) {
    assert_no_follow_on_embed_row("intro\n![[gone.png]]\nafter", cx);
}

/// A link in the chunk's prose still follows and still opens its menu.
#[gpui_kit::test]
fn a_link_in_prose_next_to_an_image_still_follows(cx: &mut TestAppContext) {
    use gpui_kit::MouseButton::{Left, Right};
    let text = "see [[other]] now\n![[pic.png]]";
    let v = vault_with(&[("n.md", text)], &[("pic.png", png(10, 60))]);
    let (view, cx, events, _sub) = open_note(cx, &v, "n.md", text);
    press_at(&view, (6, 0), Right, false, cx);
    assert!(matches!(
        follows(&events)[..],
        [FacetEvent::LinkMenu { .. }]
    ));
    press_at(&view, (6, 0), Left, true, cx);
    assert!(matches!(
        follows(&events)[..],
        [FacetEvent::LinkMenu { .. }, FacetEvent::Follow { .. }]
    ));
}

/// A drag that starts on an image's row still selects.
#[gpui_kit::test]
fn a_drag_from_an_image_row_selects(cx: &mut TestAppContext) {
    let v = vault_with(&[("n.md", NOTE)], &[("pic.png", png(10, 60))]);
    let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", NOTE);
    let rows = image_rows(60, cx) as u16;
    let from = super::facet_tests::cell(&view, 0, 3, 1, cx);
    let to = super::facet_tests::cell(&view, 0, 3, 1 + rows, cx);
    cx.update(|window, cx| {
        window.pointer_down(from, gpui_kit::MouseButton::Left, cx);
        window.pointer_move(to, Some(gpui_kit::MouseButton::Left), cx);
        window.pointer_up(to, gpui_kit::MouseButton::Left, cx);
    });
    cx.run_until_parked();
    let chunk = chunk(&view, 0, cx);
    let selection = cx.update(|_, cx| chunk.read(cx).editor.selection());
    assert!(
        selection.is_some_and(|(start, end)| start != end),
        "{selection:?}"
    );
}

/// A pick that makes chunk 0 come from the other note reuses chunk 0's
/// editor: it now resolves a bare name from its new note, so it draws the
/// other folder's file.
#[gpui_kit::test]
fn a_chunk_editor_reused_for_another_note_resolves_from_its_new_note(cx: &mut TestAppContext) {
    use kimun_core::note::{Facet, FacetKind};
    let (red, blue) = ("# Red\n![[same.png]]", "# Blue\n![[same.png]]");
    let v = vault_with(
        &[("a/n.md", red), ("b/n.md", blue)],
        &[("a/same.png", png(10, 20)), ("b/same.png", png(10, 40))],
    );
    let heading = |key: &str| Facet {
        kind: FacetKind::Heading,
        key: key.into(),
    };
    let vault = v.vault.clone();
    let members = [("a/n.md", red), ("b/n.md", blue)]
        .iter()
        .map(|(path, text)| MemberInit {
            path: VaultPath::new(*path),
            title: path.to_string(),
            text: text.to_string(),
        })
        .collect();
    let (view, cx, _events, _sub) = open_with(cx, move |cx| {
        // `a`'s section picked: chunk 0 comes from `a/n.md`.
        let picks = vec![heading("red")];
        let mut view = FacetView::new_collection_at("q".into(), members, 2, picks, noon, cx);
        view.set_image_vault(Some(vault), cx);
        view
    });
    rest(cx);
    let file = |p: &str| v.vault.path_to_pathbuf(&VaultPath::new(p));
    assert_eq!(shown_files(&view, 0, cx), [file("a/same.png")]);
    let editor = chunk(&view, 0, cx);
    // `b`'s section picked in its place: chunk 0 now comes from
    // `b/n.md`. One pick before and after, so the column keeps its
    // height and nothing but the plan hands the chunk its new note.
    view.update(cx, |view, cx| {
        view.toggle(heading("blue"), cx);
        view.toggle(heading("red"), cx);
    });
    rest(cx);
    assert_eq!(view.read_with(cx, |view, _| view.chunks().len()), 1);
    assert_eq!(chunk(&view, 0, cx), editor, "the same editor, reused");
    assert_eq!(shown_files(&view, 0, cx), [file("b/same.png")]);
    assert_eq!(pinned(cx), [file("b/same.png")]);
}

// ---- Remote images in a chunk ---------------------------------------------------

mod remote {
    //! A **Remote image** (CONTEXT.md) in a facet chunk: the placeholder
    //! through the one placeholder piece, its buttons, the load, a saved
    //! copy, and the tooltip only. The fetcher is a fake and the image cache
    //! a temp folder: no test uses the network.

    use std::time::Duration;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
    use tokio::runtime::Runtime;

    use super::super::facet::{FacetEvent, FacetView};
    use super::super::facet_tests::{Events, chunk, press};
    use super::super::remote_image_tests::{Env, install};
    use super::super::search::{PREVIEW_ROW_ID, SearchPreview};
    use super::{image_rows, open_note, png, rest, vault_with};
    use crate::editor_element::image_hover_id;
    use crate::image_core::{Actions, RemoteLook, remote_rows};
    use crate::remote_image::{Hop, RemoteImages, fetch_count};
    use crate::remote_placeholder::RemoteAction;

    const HOST: &str = "img.example.com";
    const URL: &str = "https://img.example.com/pic.png";
    const NOTE: &str = "intro\n![a cat](https://img.example.com/pic.png)\nafter";

    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// Draw and let the fetches run until `done`.
    fn drive(
        rt: &Runtime,
        cx: &mut VisualTestContext,
        mut done: impl FnMut(&mut VisualTestContext) -> bool,
    ) {
        for _ in 0..500 {
            rest(cx);
            if done(cx) {
                return;
            }
            rt.block_on(tokio::time::sleep(Duration::from_millis(2)));
        }
        panic!("never settled");
    }

    fn allow(cx: &mut TestAppContext, hosts: &[&str]) {
        let hosts: Vec<String> = hosts.iter().map(|h| h.to_string()).collect();
        cx.update(|cx| {
            cx.default_global::<RemoteImages>()
                .clone()
                .set_allowed_hosts(&hosts)
        });
    }

    /// The remote looks chunk 0 painted on its last frame, by row.
    fn looks(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Vec<(usize, RemoteLook)> {
        let chunk = chunk(view, 0, cx);
        cx.update(|_, cx| {
            chunk
                .read(cx)
                .painted
                .as_ref()
                .map(|p| {
                    p.remote_looks
                        .iter()
                        .map(|l| (l.row, l.look.clone()))
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn measured(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> Option<usize> {
        let chunk = chunk(view, 0, cx);
        cx.update(|_, cx| chunk.read(cx).measured_rows)
    }

    fn ready(view: &Entity<FacetView>, cx: &mut VisualTestContext) -> bool {
        let chunk = chunk(view, 0, cx);
        cx.update(|_, cx| {
            chunk
                .read(cx)
                .painted
                .as_ref()
                .is_some_and(|p| p.images.iter().any(|i| i.row == 1 && i.ready))
        })
    }

    /// Press the `action` button of the placeholder of `url` in chunk 0.
    fn press_button(
        view: &Entity<FacetView>,
        url: &str,
        action: RemoteAction,
        cx: &mut VisualTestContext,
    ) {
        rest(cx);
        let chunk = chunk(view, 0, cx);
        let at = cx.update(|_, cx| {
            chunk
                .read(cx)
                .remote_hits
                .iter()
                .filter(|hit| hit.url == url)
                .flat_map(|hit| hit.buttons.iter())
                .find(|(_, a)| *a == action)
                .map(|(bounds, _)| bounds.center())
                .unwrap_or_else(|| panic!("no {action:?} button for {url}"))
        });
        press(cx, at, false);
    }

    fn both() -> RemoteLook {
        RemoteLook::Placeholder {
            host: HOST.to_string(),
            actions: Actions {
                load: true,
                always: true,
            },
        }
    }

    #[gpui_kit::test]
    fn a_facet_chunk_with_a_remote_embed_on_an_unallowed_host_starts_no_fetch(
        cx: &mut TestAppContext,
    ) {
        let env: Env = install(cx, &[(URL, Hop::Body(png(10, 60)))]);
        let v = vault_with(&[("n.md", NOTE)], &[]);
        let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", NOTE);
        for _ in 0..3 {
            rest(cx);
        }
        assert_eq!(
            looks(&view, cx),
            [(1, both())],
            "the placeholder, both actions"
        );
        // Two text rows and the placeholder's two rows.
        assert_eq!(measured(&view, cx), Some(4));
        assert_eq!(cx.update(|_, cx| fetch_count(cx)), 0, "no fetch");
        assert!(env.fake.calls().is_empty());
    }

    #[gpui_kit::test]
    fn a_facet_chunk_draws_the_placeholder_and_load_swaps_in_the_image(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 200)))]);
        let v = vault_with(&[("n.md", NOTE)], &[]);
        let (view, cx, events, _sub) = open_note(cx, &v, "n.md", NOTE);
        assert_eq!(measured(&view, cx), Some(4));
        let caret = chunk(&view, 0, cx).read_with(cx, |c, _| c.editor.cursor());
        let mut heights = vec![measured(&view, cx)];
        press_button(&view, URL, RemoteAction::Load, cx);
        let rows = image_rows(200, cx);
        assert!(
            rows > remote_rows(),
            "the image is taller than the placeholder"
        );
        drive(&rt, cx, |cx| {
            let h = measured(&view, cx);
            if heights.last() != Some(&h) {
                heights.push(h);
            }
            ready(&view, cx)
        });
        assert_eq!(measured(&view, cx), Some(2 + rows), "the image's rows");
        // The chunk grew once: from the placeholder to the image.
        assert_eq!(heights, [Some(4), Some(2 + rows)], "grew once");
        assert!(looks(&view, cx).is_empty(), "no placeholder any more");
        assert_eq!(cx.update(|_, cx| fetch_count(cx)), 1);
        // The press moved no caret and followed nothing.
        let after = chunk(&view, 0, cx).read_with(cx, |c, _| c.editor.cursor());
        assert_eq!(after, caret);
        assert!(no_follow(&events), "{:?}", events.borrow());
    }

    fn no_follow(events: &Events) -> bool {
        !events
            .borrow()
            .iter()
            .any(|e| matches!(e, FacetEvent::Follow { .. } | FacetEvent::LinkMenu { .. }))
    }

    #[gpui_kit::test]
    fn always_load_from_in_a_facet_chunk_asks_the_host_with_the_chunk(cx: &mut TestAppContext) {
        let _env = install(cx, &[]);
        let v = vault_with(&[("n.md", NOTE)], &[]);
        let (view, cx, events, _sub) = open_note(cx, &v, "n.md", NOTE);
        press_button(&view, URL, RemoteAction::AlwaysLoad, cx);
        let editor = chunk(&view, 0, cx);
        let asked: Vec<_> = events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                FacetEvent::AlwaysLoadFrom { chunk, host, url } => {
                    Some((chunk.clone(), host.clone(), url.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(asked, [(editor, HOST.to_string(), URL.to_string())]);
        assert_eq!(
            cx.update(|_, cx| fetch_count(cx)),
            0,
            "the host loads it after the write"
        );
    }

    #[gpui_kit::test]
    fn a_facet_chunk_with_a_saved_copy_draws_the_image_with_no_fetch(cx: &mut TestAppContext) {
        let env = install(cx, &[]);
        env.cache.save(URL, &png(10, 60)).unwrap();
        let v = vault_with(&[("n.md", NOTE)], &[]);
        let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", NOTE);
        let rows = image_rows(60, cx);
        assert!(ready(&view, cx), "the saved copy is drawn");
        assert_eq!(
            measured(&view, cx),
            Some(2 + rows),
            "its rows from its header"
        );
        let saved = env.cache.find(URL).unwrap();
        assert_eq!(
            super::shown_files(&view, 0, cx),
            [saved],
            "the copy is pinned"
        );
        assert_eq!(cx.update(|_, cx| fetch_count(cx)), 0);
    }

    #[gpui_kit::test]
    fn a_loaded_remote_image_in_a_read_only_view_has_the_tooltip_and_no_menu(
        cx: &mut TestAppContext,
    ) {
        let env = install(cx, &[]);
        env.cache.save(URL, &png(10, 60)).unwrap();
        let v = vault_with(&[("n.md", NOTE)], &[]);
        let (view, cx, events, _sub) = open_note(cx, &v, "n.md", NOTE);
        assert!(ready(&view, cx));
        let editor = chunk(&view, 0, cx);
        crate::editor_element::BUILT_TIPS.with(|b| b.borrow_mut().clear());
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.hover(image_hover_id(1), cx);
        });
        cx.executor().advance_clock(Duration::from_secs(2));
        rest(cx);
        let built = crate::editor_element::BUILT_TIPS.with(|b| b.borrow().clone());
        assert_eq!(
            built.last().map(|t| t.as_ref()),
            Some("a cat"),
            "the tooltip"
        );
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.right_click(image_hover_id(1), cx);
            window.click(image_hover_id(1), cx);
        });
        rest(cx);
        let menus = cx.update(|window, cx| {
            window.render_frame(cx);
            window.find_all("popup-menu").len()
        });
        assert_eq!(menus, 0, "no menu");
        let menu = editor.update(cx, |e, cx| e.remote_menu(URL, cx).is_some());
        assert!(!menu, "a read-only editor builds no remote menu");
        assert!(no_follow(&events));
        assert_eq!(cx.update(|_, cx| fetch_count(cx)), 0);
    }

    /// A chunk taller than the screen: an allowed-host embed below the
    /// visible area starts no fetch, even though the chunk is drawn. It
    /// loads once it scrolls into view.
    #[gpui_kit::test]
    fn an_allowed_embed_below_the_visible_part_of_a_chunk_starts_no_fetch(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let env = install(cx, &[(URL, Hop::Body(png(10, 20)))]);
        allow(cx, &[HOST]);
        let text = format!("intro\n{}![a]({URL})\nafter", "line\n".repeat(300));
        let v = vault_with(&[("n.md", &text)], &[]);
        let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", &text);
        for _ in 0..5 {
            rest(cx);
            rt.block_on(tokio::time::sleep(Duration::from_millis(2)));
        }
        assert_eq!(
            view.read_with(cx, |v, _| v.chunks().len()),
            1,
            "one tall chunk"
        );
        assert!(
            !looks(&view, cx).is_empty(),
            "the placeholder is drawn off screen"
        );
        assert_eq!(
            cx.update(|_, cx| fetch_count(cx)),
            0,
            "off screen: no fetch"
        );
        assert!(env.fake.calls().is_empty());
        let max = super::super::facet_window_view_tests::max_offset(&view, cx);
        super::super::facet_window_view_tests::scroll_to(&view, max, cx);
        drive(&rt, cx, |cx| fetch_count_of(cx) >= 1);
        assert_eq!(env.fake.calls(), [URL.to_string()], "on screen: one fetch");
    }

    fn fetch_count_of(cx: &mut VisualTestContext) -> usize {
        cx.update(|_, cx| fetch_count(cx))
    }

    /// "Always load from" in a facet chunk, through the workspace: the host
    /// is written to the settings and allowed, and the chunk loads the image.
    #[gpui_kit::test]
    fn always_load_from_in_a_facet_chunk_allows_the_host_and_loads(cx: &mut TestAppContext) {
        use super::super::facet_tests::in_a_group::{active, facet, open_facet};
        use super::super::start_tests::{drive as drive_ws, open_started, vault_start};
        use gpui_kit::test::TestEventExt as _;
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 60)))]);
        let v = rt.block_on(crate::test_vault::test_vault(&[("a.md", NOTE)], &[]));
        let s = open_started(
            cx,
            vault_start(
                &v,
                crate::startup::FirstNote::Note(kimun_core::nfs::VaultPath::new("a.md")),
            ),
            v.settings.clone(),
        );
        drive_ws(&rt, cx, |cx| s.text(cx) == NOTE && !s.indexing(cx));
        let group = active(&s, cx);
        open_facet(&s, &rt, group, "a.md", Vec::new(), cx);
        let view = facet(&s, group, cx).expect("a facet view");
        let render = |cx: &mut TestAppContext| {
            cx.update_window(s.handle, |_, window, cx| {
                window.refresh();
                window.render_frame(cx)
            })
            .unwrap();
            cx.run_until_parked();
        };
        render(cx);
        render(cx);
        let editor = cx.read(|cx| view.read(cx).chunks()[0].clone());
        let at = cx.read(|cx| {
            editor
                .read(cx)
                .remote_hits
                .iter()
                .flat_map(|hit| hit.buttons.iter())
                .find(|(_, a)| *a == RemoteAction::AlwaysLoad)
                .map(|(bounds, _)| bounds.center())
                .expect("the Always load from button")
        });
        cx.update_window(s.handle, |_, window, cx| {
            window.pointer_down(at, gpui_kit::MouseButton::Left, cx);
            window.pointer_up(at, gpui_kit::MouseButton::Left, cx);
        })
        .unwrap();
        drive_ws(&rt, cx, |cx| {
            render(cx);
            cx.read(|cx| {
                editor
                    .read(cx)
                    .painted
                    .as_ref()
                    .is_some_and(|p| p.images.iter().any(|i| i.row == 1 && i.ready))
            })
        });
        let hosts = v.settings.read().unwrap().remote_image_hosts.clone();
        assert_eq!(hosts, [HOST.to_string()], "written to the settings");
        assert!(cx.read(|cx| cx.global::<RemoteImages>().is_allowed(HOST)));
        assert_eq!(cx.update(|cx| fetch_count(cx)), 1);
    }

    /// A dead address on an allowed host: the preview asks once, it fails,
    /// and the facet view that shows it later asks nothing.
    #[gpui_kit::test]
    fn a_dead_address_on_an_allowed_host_is_fetched_once_in_the_preview_and_the_facet(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        // No script: the fake answers 404.
        let env = install(cx, &[]);
        allow(cx, &[HOST]);
        let v = vault_with(&[("n.md", NOTE)], &[]);
        let vault = v.vault.clone();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::editor_view::init(cx);
        });
        let window = cx.add_window(|_, cx| SearchPreview::new(vault, cx));
        let preview = window.root(cx).unwrap();
        preview.update(cx, |p, cx| {
            p.show(
                Some(kimun_core::nfs::VaultPath::new("n.md")),
                std::sync::Arc::from(Vec::new()),
                cx,
            )
        });
        let failed = |cx: &mut TestAppContext| {
            cx.read(|cx| {
                matches!(
                    cx.global::<RemoteImages>().state(URL),
                    crate::remote_image::RemoteState::Failed(_)
                )
            })
        };
        for _ in 0..500 {
            cx.update_window(window.into(), |_, window, cx| {
                window.refresh();
                window.render_frame(cx)
            })
            .unwrap();
            cx.executor()
                .advance_clock(super::super::search::PREVIEW_DEBOUNCE);
            cx.run_until_parked();
            if failed(cx) {
                break;
            }
            rt.block_on(tokio::time::sleep(Duration::from_millis(2)));
        }
        assert!(failed(cx), "the preview asked and the fetch failed");
        let drawn = cx
            .update_window(window.into(), |_, window, cx| {
                window.refresh();
                window.render_frame(cx);
                window.try_find((PREVIEW_ROW_ID, 1usize)).is_some()
            })
            .unwrap();
        assert!(drawn, "the placeholder row is on screen");
        assert_eq!(env.fake.calls(), [URL.to_string()]);

        let (view, cx, _events, _sub) = open_note(cx, &v, "n.md", NOTE);
        for _ in 0..5 {
            rest(cx);
            rt.block_on(tokio::time::sleep(Duration::from_millis(2)));
        }
        let looks = looks(&view, cx);
        assert!(
            matches!(&looks[..], [(1, RemoteLook::Failed { retry: true, .. })]),
            "{looks:?}"
        );
        assert_eq!(cx.update(|_, cx| fetch_count(cx)), 1, "asked once in all");
    }
}
