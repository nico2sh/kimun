//! The Preview pane's and the Source reader's **Inline image** rows
//! (CONTEXT.md): how an embed line becomes fixed-height image rows, which
//! embed lines keep their raw text, and what the load reads. The model is
//! tested on its own (`marked_text` on a real temp vault); the load order
//! and the place kept on a height change on a real `Workspace`.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, px, size};
use kimun_core::nfs::VaultPath;
use tokio::runtime::Runtime;

use super::picker::NotePicker;
use super::search::{
    Mark, PREVIEW_BYTES, PREVIEW_DEBOUNCE, PREVIEW_IMAGE_ID, PREVIEW_ROW_ID, PaintedSlice,
    PreviewText, RowKind, SearchPreview, SectionMark, header_reads, marked_text,
};
use super::start_tests::{Started, drive, open_started, runtime, select_all_key, vault_start};
use crate::attachment_index::AttachmentIndex;
use crate::image_core::ImagePane;
use crate::inline_image::{Failure, ImageStore, fit, rows_for};
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

/// The room the model tests fit to: an image 120 px tall is 6 rows.
const PANE: ImagePane = ImagePane {
    column_px: 800.0,
    pane_px: 1000.0,
    row_px: 20.0,
};

/// A vault holding `n.md` (`text`) and the attachments `files`, and its
/// attachment list.
fn vault_with(text: &str, files: &[(&str, Vec<u8>)]) -> (TestVault, Vec<VaultPath>) {
    let rt = runtime();
    let v = rt.block_on(async {
        let v = test_vault(&[("n.md", text)], &[]).await;
        for (name, bytes) in files {
            v.vault
                .save_attachment(&path(name), bytes)
                .await
                .expect("the attachment is saved");
        }
        v
    });
    let mut attachments = v.vault.list_attachments();
    attachments.sort();
    (v, attachments)
}

/// `text` laid out for the preview with `mark`, its images resolved.
fn laid_out(
    v: &TestVault,
    attachments: &[VaultPath],
    text: &str,
    mark: &Mark,
    pane: ImagePane,
) -> PreviewText {
    marked_text(
        path("n.md"),
        text.to_string(),
        mark,
        80,
        pane,
        Some((&v.vault, attachments)),
    )
}

fn needles(words: &[&str]) -> Mark {
    Mark::Needles(
        words
            .iter()
            .map(|w| w.to_string())
            .collect::<Vec<_>>()
            .into(),
    )
}

fn text_rows(n: usize) -> Vec<RowKind> {
    vec![RowKind::Text; n]
}

fn image_rows(image: usize, n: usize) -> Vec<RowKind> {
    (0..n)
        .map(|index| RowKind::Image { image, index })
        .collect()
}

/// The image 10 x 120 px: 6 rows of 20 px in `PANE`.
const TALL: (u32, u32) = (10, 120);

/// The rows an image of `natural` takes in `pane`.
fn rows_in(natural: (u32, u32), pane: ImagePane) -> usize {
    rows_for(fit(natural, pane.column_px, pane.pane_px).1, pane.row_px)
}

const NOTE: &str = "intro\n![a cat](pic.png)\nafter";

#[test]
fn an_embed_line_with_no_hit_becomes_only_image_rows() {
    let (v, list) = vault_with(NOTE, &[("pic.png", png(TALL.0, TALL.1))]);
    let shown = laid_out(&v, &list, NOTE, &needles(&["after"]), PANE);
    let rows = rows_in(TALL, PANE);
    assert_eq!(rows, 6);
    let want: Vec<RowKind> = [text_rows(1), image_rows(0, rows), text_rows(1)].concat();
    assert_eq!(shown.kinds(), want);
    let lines = shown.row_lines();
    assert!(lines[1..=rows].iter().all(|&l| l == 1), "{lines:?}");
    let image = &shown.images()[0];
    assert_eq!(image.rows, rows);
    assert_eq!(image.first_row, 1);
    assert_eq!(image.fitted, (10.0, 120.0));
    assert_eq!(
        (image.name.as_str(), image.alt.as_str()),
        ("pic.png", "a cat")
    );
    assert!(image.file.as_ref().is_some_and(|f| f.ends_with("pic.png")));
    assert_eq!(image.failed, None);
    // An image row holds no text and no hits.
    assert!(
        shown.lines_and_hits()[1..=rows]
            .iter()
            .all(|(t, h)| t.is_empty() && h.is_empty())
    );
}

#[test]
fn an_embed_line_with_a_hit_keeps_its_text_rows_then_image_rows() {
    let (v, list) = vault_with(NOTE, &[("pic.png", png(TALL.0, TALL.1))]);
    let shown = laid_out(&v, &list, NOTE, &needles(&["cat"]), PANE);
    let want: Vec<RowKind> = [text_rows(2), image_rows(0, 6), text_rows(1)].concat();
    assert_eq!(shown.kinds(), want);
    let rows = shown.lines_and_hits();
    assert_eq!(
        rows[1],
        ("![a cat](pic.png)".to_string(), vec!["cat".to_string()])
    );
    assert_eq!(shown.images()[0].first_row, 2);
    assert_eq!(shown.summary(), (1, Some(1), false));
}

#[test]
fn first_match_row_lands_on_the_line_after_a_tall_image() {
    let text = "intro\n![a](pic.png)\nthe needle here";
    let (v, list) = vault_with(text, &[("pic.png", png(TALL.0, TALL.1))]);
    let shown = laid_out(&v, &list, text, &needles(&["needle"]), PANE);
    // intro, 6 image rows, then the match.
    assert_eq!(shown.summary(), (1, Some(7), false));
    assert_eq!(shown.lines_and_hits()[7].1, vec!["needle".to_string()]);
}

#[test]
fn a_failed_image_is_one_note_row_and_a_raw_failed_one_adds_nothing() {
    let text = "![b](bad.png)\n![g](gone.png)\nend";
    let (v, list) = vault_with(text, &[("bad.png", b"not a png".to_vec())]);
    // `bad` holds the hit: its raw line stays, nothing added; `gone` is a note.
    let shown = laid_out(&v, &list, text, &needles(&["bad"]), PANE);
    assert_eq!(
        shown.kinds(),
        vec![RowKind::Text, RowKind::Note { image: 0 }, RowKind::Text]
    );
    assert_eq!(shown.images().len(), 1);
    let gone = &shown.images()[0];
    assert_eq!(
        (gone.name.as_str(), gone.failed),
        ("gone.png", Some(Failure::Missing))
    );
    assert_eq!((gone.rows, gone.first_row), (0, 1));
    assert_eq!(shown.lines_and_hits()[1], (String::new(), vec![]));

    // The other way round: `bad` is a note (it does not decode), `gone` raw.
    let shown = laid_out(&v, &list, text, &needles(&["gone"]), PANE);
    assert_eq!(
        shown.kinds(),
        vec![RowKind::Note { image: 0 }, RowKind::Text, RowKind::Text]
    );
    assert_eq!(shown.images()[0].failed, Some(Failure::Undecodable));
}

#[test]
fn a_rewrap_at_a_new_height_changes_image_rows_and_reads_no_file() {
    let (v, list) = vault_with(NOTE, &[("pic.png", png(TALL.0, TALL.1))]);
    let before = header_reads();
    let mut shown = laid_out(&v, &list, NOTE, &needles(&["after"]), PANE);
    assert_eq!(header_reads() - before, 1, "the load reads the one header");
    assert_eq!(shown.images()[0].rows, 6);

    let short = ImagePane {
        pane_px: 100.0,
        ..PANE
    };
    let read = header_reads();
    shown.rewrap_at(80, short);
    assert_eq!(header_reads(), read, "a re-wrap reads no file");
    let rows = rows_in(TALL, short);
    assert!(rows < 6, "a shorter pane, a shorter image: {rows}");
    let want: Vec<RowKind> = [text_rows(1), image_rows(0, rows), text_rows(1)].concat();
    assert_eq!(shown.kinds(), want);
    assert_eq!(
        shown.images()[0].fitted,
        fit(TALL, short.column_px, short.pane_px)
    );
}

#[test]
fn the_hundred_first_embeds_only() {
    let text = "![[a.png]]\n".repeat(101);
    let (v, list) = vault_with(&text, &[("a.png", png(4, 4))]);
    let shown = laid_out(&v, &list, &text, &needles(&[]), PANE);
    assert_eq!(shown.images().len(), 100);
    let kinds = shown.kinds();
    // One row each for the first hundred; the hundred-and-first is text.
    assert_eq!(kinds.len(), 101);
    assert_eq!(
        kinds[99],
        RowKind::Image {
            image: 99,
            index: 0
        }
    );
    assert_eq!(kinds[100], RowKind::Text);
    assert_eq!(shown.lines_and_hits()[100].0, "![[a.png]]");
}

#[test]
fn an_embed_past_the_byte_cap_is_not_drawn() {
    let filler = "x".repeat(PREVIEW_BYTES);
    let text = format!("![[a.png]]\n{filler}\n![[a.png]]\n");
    let (v, list) = vault_with(&text, &[("a.png", png(4, 4))]);
    let shown = laid_out(&v, &list, &text, &needles(&[]), PANE);
    assert!(shown.summary().2, "cut");
    assert_eq!(shown.images().len(), 1, "only the embed inside the cap");
    assert_eq!(shown.kinds()[0], RowKind::Image { image: 0, index: 0 });
    assert!(shown.kinds()[1..].iter().all(|k| *k == RowKind::Text));
}

#[test]
fn row_of_orders_text_rows_before_image_rows_of_the_same_line() {
    let (v, list) = vault_with(NOTE, &[("pic.png", png(TALL.0, TALL.1))]);
    let shown = laid_out(&v, &list, NOTE, &needles(&["pic"]), PANE);
    let line = "![a cat](pic.png)".len();
    // Line 1: its text row (1), then six image rows (2..=7); line 2 at 8.
    assert_eq!(shown.row_at(1, 0), 1);
    assert_eq!(shown.row_at(1, line - 1), 1);
    assert_eq!(shown.row_at(1, line + 1), 2);
    assert_eq!(shown.row_at(1, line + 6), 7);
    assert_eq!(shown.row_at(2, 0), 8);
    // The first match (in the text row) is the text row, not an image row.
    assert_eq!(shown.summary().1, Some(1));
}

#[test]
fn the_section_rule_shows_raw_for_an_embed_inside_the_section_only() {
    let text = "# a\n![x](pic.png)\n# b\nbody b\n![y](pic.png)\n";
    let (v, list) = vault_with(text, &[("pic.png", png(TALL.0, TALL.1))]);
    let mark = Mark::Section(SectionMark {
        ordinal: 0,
        heading: "b".into(),
        chunk: "# b\nbody b\n![y](pic.png)".into(),
    });
    let shown = laid_out(&v, &list, text, &mark, PANE);
    let want: Vec<RowKind> = [
        text_rows(1),
        image_rows(0, 6),
        text_rows(3),
        image_rows(1, 6),
    ]
    .concat();
    assert_eq!(shown.kinds(), want);
    // The section's first line, right after the first image.
    assert_eq!(shown.summary().1, Some(7));
    assert_eq!(shown.lines_and_hits()[9].0, "![y](pic.png)");
}

#[test]
fn a_note_without_embeds_wraps_exactly_as_before() {
    let text = "intro\nxx needle needle and needle\n![not an image](x.md)\n";
    let (v, list) = vault_with(text, &[("pic.png", png(4, 4))]);
    let mark = needles(&["needle"]);
    let with = marked_text(
        path("n.md"),
        text.into(),
        &mark,
        10,
        PANE,
        Some((&v.vault, &list)),
    );
    let without = marked_text(path("n.md"), text.into(), &mark, 10, PANE, None);
    assert_eq!(with.lines_and_hits(), without.lines_and_hits());
    assert_eq!(with.summary(), without.summary());
    assert_eq!(with.row_lines(), without.row_lines());
    assert!(with.kinds().iter().all(|k| *k == RowKind::Text));
    assert!(with.images().is_empty());

    // No attachment list: an embed line is plain text, as before.
    let none = marked_text(path("n.md"), NOTE.into(), &mark, 80, PANE, None);
    assert_eq!(none.kinds(), text_rows(3));
    assert_eq!(none.lines_and_hits()[1].0, "![a cat](pic.png)");
}

// ---- On a real workspace ------------------------------------------------------

/// A vault holding `notes` and the attachments `files`, started on `a.md`,
/// driven until its index is built.
fn started_with(
    cx: &mut TestAppContext,
    rt: &Runtime,
    notes: &[(&str, &str)],
    files: &[(&str, Vec<u8>)],
) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(notes, &[]));
    for (name, bytes) in files {
        rt.block_on(v.vault.save_attachment(&path(name), bytes))
            .expect("the attachment is saved");
    }
    let started = open_started(
        cx,
        vault_start(&v, FirstNote::Note(path("a.md"))),
        v.settings.clone(),
    );
    let want = notes.iter().find(|(p, _)| *p == "a.md").unwrap().1;
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx)
    });
    (v, started)
}

fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.press(key, cx))
        .unwrap();
    cx.run_until_parked();
}

fn frame(s: &Started, cx: &mut TestAppContext) {
    cx.update_window(s.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn picker(s: &Started, cx: &TestAppContext) -> Entity<NotePicker> {
    cx.read(|cx| {
        s.workspace
            .read(cx)
            .picker
            .as_ref()
            .and_then(|p| p.upgrade())
            .expect("a picker is open")
    })
}

fn preview(s: &Started, cx: &TestAppContext) -> Entity<SearchPreview> {
    let view = picker(s, cx);
    cx.read(|cx| view.read(cx).preview().expect("the search browser"))
}

/// Open the search browser on `query` and wait for its rows.
fn search(s: &Started, rt: &Runtime, query: &str, cx: &mut TestAppContext) {
    press(s, "ctrl-k", cx);
    press(s, select_all_key(), cx);
    press(s, "backspace", cx);
    cx.update_window(s.handle, |_, window, cx| window.input(query, cx))
        .unwrap();
    cx.run_until_parked();
    let view = picker(s, cx);
    drive(rt, cx, |cx| !cx.read(|cx| view.read(cx).is_loading()));
}

/// Let the preview land and wrap to the pane's measured size.
fn settle(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
    let view = preview(s, cx);
    drive(rt, cx, |cx| {
        cx.update_window(s.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.executor().advance_clock(PREVIEW_DEBOUNCE);
        cx.run_until_parked();
        cx.read(|cx| {
            let preview = view.read(cx);
            let (_, wrapped, measured) = preview.wrap_state();
            let (fitted, pane) = preview.panes();
            !preview.is_pending() && measured > 0 && wrapped == Some(measured) && fitted == pane
        })
    });
}

/// The first preview row drawn (the top of what the reader sees).
fn top_row(s: &Started, rows: usize, cx: &mut TestAppContext) -> usize {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        (0..rows)
            .find(|&ix| window.try_find((PREVIEW_ROW_ID, ix)).is_some())
            .expect("a row is drawn")
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_load_waits_for_the_attachment_list(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let note = "needle\n![a](pic.png)\n";
    let (_v, started) = started_with(
        cx,
        &rt,
        &[("a.md", "alpha"), ("n.md", note)],
        &[("pic.png", png(10, 40))],
    );
    search(&started, &rt, "needle", cx);
    settle(&started, &rt, cx);
    let view = preview(&started, cx);
    // The first read already resolves the image: it waited for the list.
    let images = cx.read(|cx| view.read(cx).shown_images());
    assert_eq!(images.len(), 1, "{images:?}");
    assert!(images[0].file.is_some() && images[0].failed.is_none());
    let kinds = cx.read(|cx| view.read(cx).shown_kinds());
    assert!(
        kinds.contains(&RowKind::Image { image: 0, index: 0 }),
        "{kinds:?}"
    );
    let index = cx.update(|cx| cx.default_global::<AttachmentIndex>().clone());
    assert_eq!(index.walks(), 1, "one walk for the vault");
    let reads = cx.read(|cx| view.read(cx).reads);
    assert_eq!(reads, 1, "one read");
}

#[gpui_kit::test]
fn a_height_change_keeps_the_top_line_in_view(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    // An image every ten lines: a shorter pane shrinks each one.
    let text: String = (0..120)
        .map(|i| match i % 10 {
            5 => "![p](tall.png)\n".to_string(),
            _ => format!("line {i:03}\n"),
        })
        .collect();
    let text = format!("needle\n{text}");
    let (_v, started) = started_with(
        cx,
        &rt,
        &[("a.md", "alpha"), ("n.md", &text)],
        &[("tall.png", png(10, 2000))],
    );
    cx.simulate_window_resize(started.handle, size(px(1400.), px(1000.)));
    frame(&started, cx);
    search(&started, &rt, "needle", cx);
    settle(&started, &rt, cx);
    let view = preview(&started, cx);
    let tall_rows = cx.read(|cx| view.read(cx).row_lines()).len();

    cx.update_window(started.handle, |_, window, cx| {
        window.render_frame(cx);
        let delta = gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-3000.)));
        window.scroll(super::search::PREVIEW_ID, delta, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let top = top_row(&started, tall_rows, cx);
    let lines = cx.read(|cx| view.read(cx).row_lines());
    let line = lines[top];
    assert!(line > 30, "scrolled well past the first images: {line}");

    cx.simulate_window_resize(started.handle, size(px(1400.), px(600.)));
    frame(&started, cx);
    settle(&started, &rt, cx);
    let short_rows = cx.read(|cx| view.read(cx).row_lines()).len();
    assert!(
        short_rows < tall_rows,
        "images shrank: {short_rows} vs {tall_rows}"
    );
    let top = top_row(&started, short_rows, cx);
    assert_eq!(
        cx.read(|cx| view.read(cx).row_lines())[top],
        line,
        "the same note line leads the pane"
    );
}

// ---- The attachment list a load waits for -------------------------------------

/// A preview on its own over `v`'s vault (no window: nothing is drawn).
fn lone_preview(v: &TestVault, cx: &mut TestAppContext) -> Entity<SearchPreview> {
    let vault = v.vault.clone();
    cx.update(|cx| cx.new(|cx| SearchPreview::new(vault, cx)))
}

/// Show `note` and let its debounce, its list and its read land.
fn show_settled(
    preview: &Entity<SearchPreview>,
    note: &str,
    rt: &Runtime,
    cx: &mut TestAppContext,
) {
    preview.update(cx, |p, cx| {
        p.show(Some(path(note)), Arc::from(Vec::new()), cx)
    });
    settle_lone(preview, rt, cx);
}

fn settle_lone(preview: &Entity<SearchPreview>, rt: &Runtime, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| {
        cx.executor().advance_clock(PREVIEW_DEBOUNCE);
        cx.run_until_parked();
        cx.read(|cx| !preview.read(cx).is_pending())
    });
}

fn walks(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.default_global::<AttachmentIndex>().walks())
}

/// Two notes embedding `new.png`, which is not in the vault yet.
const ONE: &str = "![a](new.png)\n";
const TWO: &str = "![a](new.png)\nmore\n";

#[gpui_kit::test]
fn a_refreshed_list_is_walked_again_before_the_next_load(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("one.md", ONE), ("two.md", TWO)], &[]));
    let preview = lone_preview(&v, cx);
    show_settled(&preview, "one.md", &rt, cx);
    assert_eq!(
        cx.read(|cx| preview.read(cx).shown_kinds()),
        vec![RowKind::Note { image: 0 }],
        "not there yet"
    );
    assert_eq!(walks(cx), 1);

    // Added from outside the app, then the window activates (a refresh).
    rt.block_on(v.vault.save_attachment(&path("new.png"), &png(10, 40)))
        .expect("the attachment is saved");
    let vault = v.vault.clone();
    cx.update(|cx| cx.default_global::<AttachmentIndex>().refresh(&vault));

    show_settled(&preview, "two.md", &rt, cx);
    let kinds = cx.read(|cx| preview.read(cx).shown_kinds());
    assert_eq!(kinds[0], RowKind::Image { image: 0, index: 0 }, "{kinds:?}");
    assert_eq!(*kinds.last().unwrap(), RowKind::Text);
    assert!(!kinds.iter().any(|k| matches!(k, RowKind::Note { .. })));
    assert_eq!(walks(cx), 2, "walked again after the refresh");
}

#[gpui_kit::test]
fn a_selection_moved_while_the_list_is_awaited_reads_only_the_new_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("one.md", ONE), ("two.md", TWO)], &[]));
    let preview = lone_preview(&v, cx);
    preview.update(cx, |p, cx| {
        p.show(Some(path("one.md")), Arc::from(Vec::new()), cx);
        // Its debounce ends: it asks for the list, not listed yet.
        p.debounce_ended(cx);
        // The selection moves before the walk lands.
        p.show(Some(path("two.md")), Arc::from(Vec::new()), cx);
    });
    settle_lone(&preview, &rt, cx);
    let read = cx.read(|cx| preview.read(cx).read_paths.clone());
    assert_eq!(read, vec![path("two.md")], "one.md's load never starts");
    // one.md's late list does not start two.md early: two.md's own debounce
    // ran out and asked for the list (two debounce ends in all).
    assert_eq!(cx.read(|cx| preview.read(cx).reads), 2);
    let shown = cx.read(|cx| preview.read(cx).shown());
    assert!(shown.is_some_and(|(p, current)| p.is_like(&path("two.md")) && current));
    assert_eq!(walks(cx), 1);
}

#[gpui_kit::test]
fn a_fresh_list_serves_two_loads_with_one_walk(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let v = rt.block_on(test_vault(&[("one.md", ONE), ("two.md", TWO)], &[]));
    let preview = lone_preview(&v, cx);
    show_settled(&preview, "one.md", &rt, cx);
    show_settled(&preview, "two.md", &rt, cx);
    let read = cx.read(|cx| preview.read(cx).read_paths.clone());
    assert_eq!(read, vec![path("one.md"), path("two.md")]);
    assert_eq!(walks(cx), 1, "no second walk");
}

// ---- The image rows painted ---------------------------------------------------

/// The image rows' slices the preview painted at its last render.
fn slices(s: &Started, view: &Entity<SearchPreview>, cx: &mut TestAppContext) -> Vec<PaintedSlice> {
    frame(s, cx);
    cx.read(|cx| view.read(cx).painted_slices())
}

fn scale(s: &Started, cx: &mut TestAppContext) -> f32 {
    cx.update_window(s.handle, |_, window, _| window.scale_factor())
        .unwrap()
}

/// The window bounds of preview row `ix`.
fn row_bounds(
    s: &Started,
    ix: usize,
    cx: &mut TestAppContext,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    cx.update_window(s.handle, |_, window, cx| {
        window.render_frame(cx);
        window.find((PREVIEW_ROW_ID, ix)).bounds()
    })
    .unwrap()
}

/// Open the search browser on `query` over a vault holding `n.md`
/// (`note`) and `files`, and let its preview land, every visible image
/// ready.
fn browse(
    cx: &mut TestAppContext,
    rt: &Runtime,
    note: &str,
    files: &[(&str, Vec<u8>)],
    query: &str,
) -> (TestVault, Started, Entity<SearchPreview>) {
    let (v, started) = started_with(cx, rt, &[("a.md", "alpha"), ("n.md", note)], files);
    search(&started, rt, query, cx);
    settle(&started, rt, cx);
    let view = preview(&started, cx);
    (v, started, view)
}

/// Drive until every image slice on screen paints its pixels.
fn until_ready(s: &Started, view: &Entity<SearchPreview>, rt: &Runtime, cx: &mut TestAppContext) {
    drive(rt, cx, |cx| {
        let painted = slices(s, view, cx);
        !painted.is_empty() && painted.iter().all(|p| p.ready)
    });
}

/// Scroll the preview so row `to` is the top row.
fn scroll_to_row(s: &Started, view: &Entity<SearchPreview>, to: usize, cx: &mut TestAppContext) {
    let rows = cx.read(|cx| view.read(cx).row_lines()).len();
    let top = top_row(s, rows, cx);
    view.update(cx, |p, cx| p.scroll_rows(to as i32 - top as i32, cx));
    frame(s, cx);
}

fn store(cx: &mut TestAppContext) -> ImageStore {
    cx.update(|cx| cx.default_global::<ImageStore>().clone())
}

/// A PNG whose header reads but whose pixels do not decode: its image
/// data corrupted, the header kept.
fn corrupt_png(w: u32, h: u32) -> Vec<u8> {
    let mut bytes = png(w, h);
    let n = bytes.len();
    for b in &mut bytes[n - 20..n - 12] {
        *b ^= 0xFF;
    }
    bytes
}

const SLICED: &str = "needle\n![a cat](pic.png)\nafter";

#[gpui_kit::test]
fn an_image_row_paints_its_slice_at_the_right_offset(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s, view) = browse(
        cx,
        &rt,
        SLICED,
        &[("pic.png", png(TALL.0, TALL.1))],
        "needle",
    );
    until_ready(&s, &view, &rt, cx);
    let image = cx.read(|cx| view.read(cx).shown_images())[0].clone();
    assert!(image.rows >= 3, "{} rows", image.rows);
    let scale = scale(&s, cx);
    let painted = slices(&s, &view, cx);
    assert_eq!(painted.len(), image.rows, "every slice in view paints");
    let row = image.first_row + 2;
    let slice = painted
        .iter()
        .find(|p| p.index == 2)
        .expect("slice 2 painted")
        .clone();
    assert_eq!((slice.row, slice.image), (row, 0));
    let bounds = row_bounds(&s, row, cx);
    assert_eq!(slice.clip, bounds, "clipped to its own row");
    assert!(
        bounds.size.width >= slice.rect.size.width,
        "the row is as wide as the pane, not empty: {bounds:?}"
    );
    assert_eq!(slice.rect.origin.x, bounds.origin.x, "left aligned");
    assert_eq!(
        slice.rect.origin.y,
        bounds.origin.y - bounds.size.height * 2.,
        "the image origin two rows above"
    );
    // The fitted size of the model, device pixels to logical ones.
    assert_eq!(
        slice.rect.size,
        size(px(image.fitted.0 / scale), px(image.fitted.1 / scale))
    );
    assert!(slice.ready, "its pixels");
    // Every slice paints the one image at the same place.
    assert!(painted.iter().all(|p| p.rect == slice.rect), "{painted:?}");
}

#[gpui_kit::test]
fn a_slice_row_scrolled_in_alone_paints_its_slice(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let note = format!("needle\n![p](tall.png)\n{}", "line\n".repeat(80));
    let (_v, s, view) = browse(cx, &rt, &note, &[("tall.png", png(10, 2000))], "needle");
    until_ready(&s, &view, &rt, cx);
    let image = cx.read(|cx| view.read(cx).shown_images())[0].clone();
    assert!(image.rows > 8, "a tall image: {} rows", image.rows);
    let k = image.rows / 2;
    scroll_to_row(&s, &view, image.first_row + k, cx);
    let rows = cx.read(|cx| view.read(cx).row_lines()).len();
    assert_eq!(top_row(&s, rows, cx), image.first_row + k);
    let painted = slices(&s, &view, cx);
    assert!(
        painted.iter().all(|p| p.index >= k),
        "the slices above are off screen: {painted:?}"
    );
    let slice = painted
        .iter()
        .find(|p| p.index == k)
        .expect("the top slice paints")
        .clone();
    let bounds = row_bounds(&s, image.first_row + k, cx);
    assert_eq!(slice.clip, bounds);
    assert_eq!(
        slice.rect.origin.y,
        bounds.origin.y - bounds.size.height * k as f32
    );
    assert!(slice.ready);
}

#[gpui_kit::test]
fn every_image_row_has_the_tooltip_with_alt_or_name(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let note = "needle\n![a red square](a.png)\n![[b.png]]\n![](c.png)\nafter";
    let files = [
        ("a.png", png(10, 50)),
        ("b.png", png(10, 50)),
        ("c.png", png(10, 50)),
    ];
    let (_v, s, view) = browse(cx, &rt, note, &files, "needle");
    let scale = scale(&s, cx);
    let images = cx.read(|cx| view.read(cx).shown_images());
    assert_eq!(images.len(), 3);
    for (image, tip) in images.iter().zip(["a red square", "b.png", "c.png"]) {
        assert!(image.rows >= 2, "{} rows", image.rows);
        for ix in image.first_row..image.first_row + image.rows {
            let (label, target, row) = cx
                .update_window(s.handle, |_, window, cx| {
                    window.render_frame(cx);
                    let row = window.find((PREVIEW_ROW_ID, ix)).bounds();
                    let target = window.within((PREVIEW_ROW_ID, ix)).find(PREVIEW_IMAGE_ID);
                    (target.label().map(str::to_string), target.bounds(), row)
                })
                .unwrap();
            assert_eq!(label.as_deref(), Some(tip), "row {ix}");
            // Over the image's width and the row's height, at its left.
            assert_eq!(target.origin, row.origin, "row {ix}");
            assert_eq!(target.size.height, row.size.height);
            assert_eq!(target.size.width, px(image.fitted.0 / scale));
            crate::editor_element::BUILT_TIPS.with(|b| b.borrow_mut().clear());
            cx.update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .within((PREVIEW_ROW_ID, ix))
                    .hover(PREVIEW_IMAGE_ID, cx);
            })
            .unwrap();
            cx.executor()
                .advance_clock(std::time::Duration::from_secs(2));
            cx.run_until_parked();
            frame(&s, cx);
            let built = crate::editor_element::BUILT_TIPS.with(|b| b.borrow().clone());
            assert_eq!(
                built.last().map(|t| t.as_ref()),
                Some(tip),
                "the tooltip over row {ix}"
            );
        }
    }
}

#[gpui_kit::test]
fn visible_images_are_pinned_and_decoded_once_and_a_completion_requests_nothing(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let note = format!(
        "needle\n![a](a.png)\n{}![b](b.png)\nend",
        "line\n".repeat(200)
    );
    let files = [("a.png", png(10, 60)), ("b.png", png(10, 60))];
    let (v, started) = started_with(cx, &rt, &[("a.md", "alpha"), ("n.md", &note)], &files);
    let store = ImageStore::default();
    cx.update(|cx| cx.set_global(store.clone()));
    search(&started, &rt, "needle", cx);
    settle(&started, &rt, cx);
    let view = preview(&started, cx);
    until_ready(&started, &view, &rt, cx);
    let file = |name: &str| v.vault.path_to_pathbuf(&path(name));
    assert_eq!(store.decodes_started(), 1, "only the image on screen");
    assert_eq!(store.pinned_files(), vec![file("a.png")]);
    for _ in 0..5 {
        frame(&started, cx);
    }
    assert_eq!(store.decodes_started(), 1, "a completion requests nothing");

    // To the end: `b` comes into view, `a` leaves.
    view.update(cx, |p, cx| p.scroll_rows(10_000, cx));
    until_ready(&started, &view, &rt, cx);
    assert_eq!(store.decodes_started(), 2, "the new image decodes");
    assert_eq!(store.pinned_files(), vec![file("b.png")]);
    for _ in 0..5 {
        frame(&started, cx);
    }
    assert_eq!(store.decodes_started(), 2, "settled");

    // Back: `a` is still cached, nothing decodes again.
    view.update(cx, |p, cx| p.scroll_rows(-10_000, cx));
    until_ready(&started, &view, &rt, cx);
    assert_eq!(store.decodes_started(), 2, "cached: no second decode");
    assert_eq!(store.pinned_files(), vec![file("a.png")]);
    let painted = slices(&started, &view, cx);
    assert!(painted.iter().all(|p| p.image == 0 && p.ready));

    // The preview gone, its pins go with it.
    press(&started, "escape", cx);
    started.settle_dialog(cx);
    drop(view);
    for _ in 0..3 {
        frame(&started, cx);
    }
    assert!(
        store.pinned_files().is_empty(),
        "{:?}",
        store.pinned_files()
    );
}

#[gpui_kit::test]
fn two_big_visible_images_over_the_budget_both_end_up_ready(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    // One 10x60 image decodes to 2400 bytes: one fits, two do not.
    let note = "needle\n![a](a.png)\n![b](b.png)\nafter";
    let files = [("a.png", png(10, 60)), ("b.png", png(10, 60))];
    let (_v, started) = started_with(cx, &rt, &[("a.md", "alpha"), ("n.md", note)], &files);
    let store = ImageStore::with_test_budget(3000);
    cx.update(|cx| cx.set_global(store.clone()));
    search(&started, &rt, "needle", cx);
    settle(&started, &rt, cx);
    let view = preview(&started, cx);
    until_ready(&started, &view, &rt, cx);
    let painted = slices(&started, &view, cx);
    assert!(painted.iter().any(|p| p.image == 0) && painted.iter().any(|p| p.image == 1));
    assert_eq!(store.decodes_started(), 2, "one decode per image");
    for _ in 0..5 {
        frame(&started, cx);
    }
    assert_eq!(
        store.decodes_started(),
        2,
        "an eviction decodes nothing again"
    );
    assert!(slices(&started, &view, cx).iter().all(|p| p.ready));
}

#[gpui_kit::test]
fn a_failed_decode_makes_one_note_row_and_settles(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let note = "needle\n![b](bad.png)\nafter";
    let (v, started) = started_with(
        cx,
        &rt,
        &[("a.md", "alpha"), ("n.md", note)],
        &[("bad.png", corrupt_png(10, 60))],
    );
    let store = ImageStore::default();
    cx.update(|cx| cx.set_global(store.clone()));
    search(&started, &rt, "needle", cx);
    settle(&started, &rt, cx);
    let view = preview(&started, cx);
    // The header reads: the load lays it out as image rows.
    let file = v.vault.path_to_pathbuf(&path("bad.png"));
    assert!(matches!(
        ImageStore::default().size_of(&file),
        crate::inline_image::ImageState::Sized { .. }
    ));
    drive(&rt, cx, |cx| {
        frame(&started, cx);
        cx.read(|cx| view.read(cx).shown_kinds())
            .contains(&RowKind::Note { image: 0 })
    });
    let want = vec![RowKind::Text, RowKind::Note { image: 0 }, RowKind::Text];
    assert_eq!(cx.read(|cx| view.read(cx).shown_kinds()), want);
    let images = cx.read(|cx| view.read(cx).shown_images());
    assert_eq!(images[0].failed, Some(Failure::Undecodable));
    assert_eq!(cx.read(|cx| view.read(cx).failure_rewraps()), 1);
    assert_eq!(store.decodes_started(), 1);
    for _ in 0..10 {
        frame(&started, cx);
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(2)));
        cx.run_until_parked();
    }
    assert_eq!(cx.read(|cx| view.read(cx).shown_kinds()), want, "settled");
    assert_eq!(cx.read(|cx| view.read(cx).failure_rewraps()), 1, "once");
    assert_eq!(store.decodes_started(), 1, "no decode again");
    assert!(slices(&started, &view, cx).is_empty(), "no slice paints");
    assert!(cx.read(|cx| !view.read(cx).is_pending()));
}

#[gpui_kit::test]
fn a_height_change_repaints_the_slices_at_the_new_size(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s) = started_with(
        cx,
        &rt,
        &[("a.md", "alpha"), ("n.md", SLICED)],
        &[("pic.png", png(10, 2000))],
    );
    cx.simulate_window_resize(s.handle, size(px(1400.), px(1000.)));
    frame(&s, cx);
    search(&s, &rt, "needle", cx);
    settle(&s, &rt, cx);
    let view = preview(&s, cx);
    until_ready(&s, &view, &rt, cx);
    let tall = cx.read(|cx| view.read(cx).shown_images())[0].clone();

    cx.simulate_window_resize(s.handle, size(px(1400.), px(600.)));
    frame(&s, cx);
    settle(&s, &rt, cx);
    until_ready(&s, &view, &rt, cx);
    let short = cx.read(|cx| view.read(cx).shown_images())[0].clone();
    assert!(short.fitted.1 < tall.fitted.1, "{short:?} vs {tall:?}");
    let scale = scale(&s, cx);
    let painted = slices(&s, &view, cx);
    assert!(!painted.is_empty());
    for slice in &painted {
        assert_eq!(
            slice.rect.size,
            size(px(short.fitted.0 / scale), px(short.fitted.1 / scale)),
            "the new fitted size"
        );
        let bounds = row_bounds(&s, slice.row, cx);
        assert_eq!(
            slice.rect.origin.y,
            bounds.origin.y - bounds.size.height * slice.index as f32
        );
    }
}

#[gpui_kit::test]
fn a_preview_of_a_note_without_embeds_paints_and_scrolls_as_before(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let lines: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let note = format!("{lines}the needle\n{lines}");
    let (_v, s, view) = browse(cx, &rt, &note, &[("pic.png", png(10, 60))], "needle");
    let expected = super::search::preview_text(path("n.md"), note.clone(), &["needle".into()], 80);
    let kinds = cx.read(|cx| view.read(cx).shown_kinds());
    assert_eq!(kinds.len(), 401, "one row per line");
    assert!(kinds.iter().all(|k| *k == RowKind::Text));
    let (first_hit, _, _) = cx.read(|cx| view.read(cx).wrap_state());
    assert_eq!(first_hit, expected.summary().1);
    assert_eq!(first_hit, Some(200));
    let rows = kinds.len();
    let top = top_row(&s, rows, cx);
    view.update(cx, |p, cx| p.scroll_rows(30, cx));
    frame(&s, cx);
    assert_eq!(top_row(&s, rows, cx), top + 30, "scrolls as before");
    for _ in 0..3 {
        frame(&s, cx);
    }
    let store = store(cx);
    assert_eq!(
        cx.read(|cx| view.read(cx).visible_checks),
        0,
        "no image work"
    );
    assert!(slices(&s, &view, cx).is_empty());
    assert_eq!(store.decodes_started(), 0);
    assert!(store.pinned_files().is_empty());
}

/// What a press on the preview may change: the open modal, the browser's
/// query and selection, the note the editor shows, the preview's note.
fn snapshot(s: &Started, cx: &mut TestAppContext) -> (Option<String>, String, String, usize) {
    let modal = s.shown_modal(cx).map(|m| format!("{m:?}"));
    let view = picker(s, cx);
    let (query, selected) = cx.read(|cx| {
        let p = view.read(cx);
        let selected = p
            .selected()
            .map(|r| super::picker::picker_row_id(&r).to_string());
        (p.query().to_string(), format!("{selected:?}"))
    });
    let shown = preview(s, cx);
    let rows = cx.read(|cx| shown.read(cx).row_lines()).len();
    (modal, format!("{query}|{selected}"), s.text(cx), rows)
}

#[gpui_kit::test]
fn an_image_row_click_behaves_like_a_text_row_of_its_line(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s, view) = browse(
        cx,
        &rt,
        SLICED,
        &[("pic.png", png(TALL.0, TALL.1))],
        "needle",
    );
    until_ready(&s, &view, &rt, cx);
    let image = cx.read(|cx| view.read(cx).shown_images())[0].clone();
    let before = snapshot(&s, cx);
    assert_eq!(before.0.as_deref(), Some("Search"));
    // A text row first, then an image row: the same nothing happens.
    for ix in [0, image.first_row, image.first_row + 1] {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click((PREVIEW_ROW_ID, ix), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(snapshot(&s, cx), before, "a click on row {ix}");
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.double_click((PREVIEW_ROW_ID, ix), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(snapshot(&s, cx), before, "a double-click on row {ix}");
    }
    // On the image itself: no menu, no follow, nothing.
    for ix in [image.first_row, image.first_row + 1] {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            let mut row = window.within((PREVIEW_ROW_ID, ix));
            row.click(PREVIEW_IMAGE_ID, cx);
            row.double_click(PREVIEW_IMAGE_ID, cx);
            row.right_click(PREVIEW_IMAGE_ID, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(snapshot(&s, cx), before, "presses on the image of row {ix}");
        let menus = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window.find_all("popup-menu").len()
            })
            .unwrap();
        assert_eq!(menus, 0, "no menu opens");
    }
    assert!(!s.shown_modal(cx).is_none(), "the browser stays");
}

// ---- After the window is activated again ----------------------------------------

/// The user leaves the window and comes back to it: the workspace lists
/// the vault's attachments again (the app has no file watcher).
fn reactivate(s: &Started, cx: &mut TestAppContext) {
    gpui_kit::VisualTestContext::from_window(s.handle, cx).deactivate_window();
    cx.update_window(s.handle, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
}

/// The notes the preview read so far.
fn reads(view: &Entity<SearchPreview>, cx: &TestAppContext) -> usize {
    cx.read(|cx| view.read(cx).read_paths.len())
}

#[gpui_kit::test]
fn a_preview_resolves_a_file_added_after_an_activation_without_a_new_selection(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let note = "needle\n![a](new.png)\nafter";
    let (v, s, view) = browse(cx, &rt, note, &[], "needle");
    assert_eq!(
        cx.read(|cx| view.read(cx).shown_kinds()),
        vec![RowKind::Text, RowKind::Note { image: 0 }, RowKind::Text],
        "not there yet"
    );
    let selected = snapshot(&s, cx).1;
    let before = reads(&view, cx);
    // Saved from outside the app while the window is inactive.
    rt.block_on(v.vault.save_attachment(&path("new.png"), &png(10, 60)))
        .expect("the attachment is saved");
    reactivate(&s, cx);
    drive(&rt, cx, |cx| {
        frame(&s, cx);
        cx.read(|cx| view.read(cx).shown_kinds())
            .contains(&RowKind::Image { image: 0, index: 0 })
    });
    assert_eq!(reads(&view, cx), before + 1, "read again once");
    assert_eq!(snapshot(&s, cx).1, selected, "no new selection");
    until_ready(&s, &view, &rt, cx);
}

#[gpui_kit::test]
fn a_file_replaced_on_disk_is_decoded_again_while_the_preview_keeps_showing_it(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, s) = started_with(
        cx,
        &rt,
        &[("a.md", "alpha"), ("n.md", SLICED)],
        &[("pic.png", png(10, 60))],
    );
    let store = ImageStore::default();
    cx.update(|cx| cx.set_global(store.clone()));
    search(&s, &rt, "needle", cx);
    settle(&s, &rt, cx);
    let view = preview(&s, cx);
    until_ready(&s, &view, &rt, cx);
    assert_eq!(store.decodes_started(), 1);
    // Rewritten from outside the app: another size, so another file key.
    rt.block_on(v.vault.save_attachment(&path("pic.png"), &png(12, 60)))
        .expect("the attachment is saved");
    // A redraw (a hover, a scroll) while it stays in view.
    cx.update_window(s.handle, |_, window, cx| {
        window.refresh();
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    until_ready(&s, &view, &rt, cx);
    assert_eq!(store.decodes_started(), 2, "decoded again, once");
    for _ in 0..5 {
        frame(&s, cx);
    }
    assert_eq!(store.decodes_started(), 2, "settled");
    assert!(
        slices(&s, &view, cx).iter().all(|p| p.ready),
        "the picture, not a box"
    );
}

#[gpui_kit::test]
fn a_refresh_does_not_reload_the_preview_every_frame(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, s, view) = browse(cx, &rt, SLICED, &[("pic.png", png(10, 60))], "needle");
    until_ready(&s, &view, &rt, cx);
    let before = reads(&view, cx);
    reactivate(&s, cx);
    for _ in 0..10 {
        cx.update_window(s.handle, |_, window, cx| {
            window.refresh();
            window.render_frame(cx);
        })
        .unwrap();
        rt.block_on(tokio::time::sleep(std::time::Duration::from_millis(5)));
        cx.run_until_parked();
    }
    assert_eq!(reads(&view, cx), before + 1, "one load for one refresh");
    assert!(cx.read(|cx| !view.read(cx).is_pending()));
    until_ready(&s, &view, &rt, cx);
}

#[gpui_kit::test]
fn a_new_selection_during_a_reread_lands_at_its_own_first_match(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    // `m.md` holds its match far down, so a place taken from `n.md` shows.
    let filler: String = (0..150).map(|i| format!("line {i}\n")).collect();
    let far = format!("{filler}the needle\n{filler}");
    let (_v, s) = started_with(
        cx,
        &rt,
        &[("a.md", "alpha"), ("n.md", SLICED), ("m.md", &far)],
        &[("pic.png", png(10, 60))],
    );
    search(&s, &rt, "needle", cx);
    let view = preview(&s, cx);
    // Select `n.md`, the note with an image, whichever row it is.
    for _ in 0..3 {
        settle(&s, &rt, cx);
        let on_n = cx
            .read(|cx| view.read(cx).shown())
            .is_some_and(|(p, _)| p.is_like(&path("n.md")));
        if on_n {
            break;
        }
        press(&s, "down", cx);
    }
    settle(&s, &rt, cx);
    let shown = cx.read(|cx| view.read(cx).shown());
    assert!(shown.is_some_and(|(p, _)| p.is_like(&path("n.md"))));
    // The refresh starts a re-read of `n.md`; the selection moves on before
    // it lands.
    let before = reads(&view, cx);
    reactivate(&s, cx);
    frame(&s, cx);
    assert!(cx.read(|cx| view.read(cx).is_pending()), "the re-read runs");
    assert_eq!(reads(&view, cx), before + 1);
    press(&s, "down", cx);
    settle(&s, &rt, cx);
    let shown = cx.read(|cx| view.read(cx).shown());
    if !shown.is_some_and(|(p, _)| p.is_like(&path("m.md"))) {
        press(&s, "up", cx);
        settle(&s, &rt, cx);
    }
    let shown = cx.read(|cx| view.read(cx).shown());
    assert!(
        shown
            .as_ref()
            .is_some_and(|(p, current)| p.is_like(&path("m.md")) && *current),
        "{shown:?}"
    );
    let (first_hit, _, _) = cx.read(|cx| view.read(cx).wrap_state());
    assert_eq!(first_hit, Some(150));
    let rows = cx.read(|cx| view.read(cx).row_lines()).len();
    let top = top_row(&s, rows, cx);
    assert!(
        top > 100 && top <= 150,
        "its own first match in view, not n.md's place: top row {top}"
    );
}

// ---- Remote images ----------------------------------------------------------------

mod remote {
    //! A **Remote image** (CONTEXT.md) in the Preview pane and the Source
    //! reader: the placeholder's two rows through the one placeholder piece,
    //! its buttons, the load and the rows made again, a saved copy, and the
    //! embeds that load on their own. The fetcher is a fake and the image
    //! cache a temp folder: no test uses the network.

    use gpui_kit::test::{ClickOptions, TestWindowExt as _};
    use gpui_kit::{AppContext as _, Entity, TestAppContext, px};
    use tokio::runtime::Runtime;

    use super::super::remote_image_tests::{Env, install};
    use super::super::search::{
        Mark, PREVIEW_ID, PREVIEW_IMAGE_ID, PREVIEW_ROW_ID, PaintedRemote, RowKind, SearchPreview,
        SectionMark, marked_text_cached,
    };
    use super::super::start_tests::{Started, drive, open_started, runtime, vault_start};
    use super::{
        NOTE, PANE, TALL, frame, image_rows, laid_out, needles, path, png, preview, rows_in,
        search, settle, slices, snapshot, text_rows, top_row, until_ready, vault_with,
    };
    use crate::image_core::{Actions, RemoteLook, remote_rows};
    use crate::remote_image::{Hop, RemoteImages, fetch_count};
    use crate::remote_placeholder::RemoteAction;
    use crate::startup::FirstNote;
    use crate::test_vault::{TestVault, test_vault};

    const HOST: &str = "img.example.com";
    const URL: &str = "https://img.example.com/pic.png";
    /// A remote embed between two lines, a needle on the first.
    const REMOTE: &str = "needle\n![a cat](https://img.example.com/pic.png)\nafter";

    fn remote_rows_of(image: usize) -> Vec<RowKind> {
        (0..remote_rows())
            .map(|index| RowKind::Remote { image, index })
            .collect()
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

    /// The search browser on `query` over a vault holding `n.md` (`note`),
    /// with `hosts` allowed, its preview landed.
    fn browse(
        cx: &mut TestAppContext,
        rt: &Runtime,
        note: &str,
        hosts: &[&str],
        query: &str,
    ) -> (TestVault, Started, Entity<SearchPreview>) {
        let v = rt.block_on(test_vault(&[("a.md", "alpha"), ("n.md", note)], &[]));
        v.settings.write().unwrap().remote_image_hosts =
            hosts.iter().map(|h| h.to_string()).collect();
        let s = open_started(
            cx,
            vault_start(&v, FirstNote::Note(path("a.md"))),
            v.settings.clone(),
        );
        drive(rt, cx, |cx| s.text(cx) == "alpha" && !s.indexing(cx));
        search(&s, rt, query, cx);
        settle(&s, rt, cx);
        let view = preview(&s, cx);
        (v, s, view)
    }

    fn painted(
        s: &Started,
        view: &Entity<SearchPreview>,
        cx: &mut TestAppContext,
    ) -> Vec<PaintedRemote> {
        frame(s, cx);
        cx.read(|cx| view.read(cx).painted_remote())
    }

    fn kinds(view: &Entity<SearchPreview>, cx: &TestAppContext) -> Vec<RowKind> {
        cx.read(|cx| view.read(cx).shown_kinds())
    }

    fn fetches(cx: &mut TestAppContext) -> usize {
        cx.update(|cx| fetch_count(cx))
    }

    /// A left press at window position `at` on the preview.
    fn press_at(s: &Started, at: gpui_kit::Point<gpui_kit::Pixels>, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            let pane = window.find(PREVIEW_ID).bounds();
            window.click_with_options(
                PREVIEW_ID,
                ClickOptions::new().with_offset(at - pane.origin),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// Press the `action` button of the placeholder of `url`.
    fn press_button(
        s: &Started,
        view: &Entity<SearchPreview>,
        url: &str,
        action: RemoteAction,
        cx: &mut TestAppContext,
    ) {
        let at = painted(s, view, cx)
            .iter()
            .filter(|p| p.url == url)
            .flat_map(|p| p.buttons.iter())
            .find(|(_, a)| *a == action)
            .map(|(bounds, _)| bounds.center())
            .unwrap_or_else(|| panic!("no {action:?} button for {url}"));
        press_at(s, at, cx);
    }

    /// Drive until the image of the remote embed is drawn with its pixels.
    fn until_loaded(
        s: &Started,
        view: &Entity<SearchPreview>,
        rt: &Runtime,
        cx: &mut TestAppContext,
    ) {
        drive(rt, cx, |cx| {
            frame(s, cx);
            kinds(view, cx)
                .iter()
                .any(|k| matches!(k, RowKind::Image { .. }))
        });
        until_ready(s, view, rt, cx);
    }

    #[test]
    fn the_preview_draws_a_placeholder_in_two_rows_for_a_remote_embed() {
        let (v, list) = vault_with(REMOTE, &[]);
        // No hit on the embed line: only the placeholder's rows.
        let shown = laid_out(&v, &list, REMOTE, &needles(&["after"]), PANE);
        let want = [text_rows(1), remote_rows_of(0), text_rows(1)].concat();
        assert_eq!(shown.kinds(), want);
        let image = &shown.images()[0];
        assert_eq!(
            (image.rows, image.first_row, image.file.clone()),
            (2, 1, None)
        );
        assert!(
            shown.lines_and_hits()[1..3]
                .iter()
                .all(|(t, h)| t.is_empty() && h.is_empty())
        );
        // A hit on the embed line: its raw line first, then the placeholder.
        let shown = laid_out(&v, &list, REMOTE, &needles(&["cat"]), PANE);
        let want = [text_rows(2), remote_rows_of(0), text_rows(1)].concat();
        assert_eq!(shown.kinds(), want);
        assert_eq!(
            shown.summary().1,
            Some(1),
            "the first match is the raw line"
        );
    }

    /// The pointing hand shows over each button of a placeholder row, and
    /// nowhere else in it: the hand areas are the press areas. (The test
    /// window cannot read the pointer it shows.)
    #[gpui_kit::test]
    fn the_buttons_of_a_preview_placeholder_show_the_pointing_hand(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env: Env = install(cx, &[(URL, Hop::Body(png(10, 60)))]);
        let (_v, s, view) = browse(cx, &rt, REMOTE, &[], "needle");
        let painted = painted(&s, &view, cx);
        assert_eq!(painted.len(), 2, "{painted:?}");
        for p in &painted {
            let press: Vec<_> = p.buttons.iter().map(|(b, _)| *b).collect();
            assert_eq!(p.hands, press, "row {}", p.row);
        }
        assert!(painted[0].hands.is_empty(), "the text row has no button");
        assert_eq!(painted[1].hands.len(), 2, "Load and Always load from");
    }

    #[gpui_kit::test]
    fn the_preview_paints_the_placeholder_over_its_two_rows_with_no_fetch(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let env: Env = install(cx, &[(URL, Hop::Body(png(10, 60)))]);
        let (_v, s, view) = browse(cx, &rt, REMOTE, &[], "needle");
        assert_eq!(
            kinds(&view, cx),
            [text_rows(1), remote_rows_of(0), text_rows(1)].concat()
        );
        let painted = painted(&s, &view, cx);
        assert_eq!(painted.len(), 2, "{painted:?}");
        let rows: Vec<_> = painted.iter().map(|p| (p.row, p.index)).collect();
        assert_eq!(rows, [(1, 0), (2, 1)]);
        for p in &painted {
            assert_eq!(p.look, both());
            let bounds = s_row_bounds(&s, p.row, cx);
            assert_eq!(p.clip, bounds, "clipped to its own row");
            assert_eq!(
                p.rect.origin.y,
                bounds.origin.y - bounds.size.height * p.index as f32
            );
            assert_eq!(p.rect.size.height, bounds.size.height * 2.);
        }
        // The buttons are in the second row only.
        assert!(painted[0].buttons.is_empty());
        let actions: Vec<_> = painted[1].buttons.iter().map(|(_, a)| *a).collect();
        assert_eq!(actions, [RemoteAction::Load, RemoteAction::AlwaysLoad]);
        for _ in 0..3 {
            frame(&s, cx);
        }
        // A fetch that started shows Loading, so this holds with no timing.
        frame(&s, cx);
        let looks: Vec<_> = cx.read(|cx| view.read(cx).painted_remote());
        let looks: Vec<_> = looks.into_iter().map(|p| p.look).collect();
        assert_eq!(looks, [both(), both()], "still the placeholder");
        assert_eq!(fetches(cx), 0, "an unallowed host loads nothing");
        assert!(env.fake.calls().is_empty());
    }

    fn s_row_bounds(
        s: &Started,
        ix: usize,
        cx: &mut TestAppContext,
    ) -> gpui_kit::Bounds<gpui_kit::Pixels> {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.find((PREVIEW_ROW_ID, ix)).bounds()
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn a_press_on_a_preview_placeholder_button_loads_and_does_not_select_the_result(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 60)))]);
        let (_v, s, view) = browse(cx, &rt, REMOTE, &[], "needle");
        let before = snapshot(&s, cx);
        press_button(&s, &view, URL, RemoteAction::Load, cx);
        until_loaded(&s, &view, &rt, cx);
        let after = snapshot(&s, cx);
        assert_eq!(
            (&after.0, &after.1, &after.2),
            (&before.0, &before.1, &before.2),
            "the browser, its query and selection, and the open note stay"
        );
        assert_eq!(fetches(cx), 1);
        let image = cx.read(|cx| view.read(cx).shown_images())[0].clone();
        assert_eq!(image.rows, rows_in((10, 60), cx_pane(&view, cx)));
        assert!(painted(&s, &view, cx).is_empty(), "no placeholder any more");
    }

    fn cx_pane(view: &Entity<SearchPreview>, cx: &TestAppContext) -> crate::image_core::ImagePane {
        cx.read(|cx| view.read(cx).panes().0.expect("wrapped"))
    }

    #[gpui_kit::test]
    fn a_press_elsewhere_in_the_preview_still_selects(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 60)))]);
        let (_v, s, view) = browse(cx, &rt, REMOTE, &[], "needle");
        let before = snapshot(&s, cx);
        assert_eq!(before.0.as_deref(), Some("Search"));
        let painted = painted(&s, &view, cx);
        // The placeholder's text row, the second row right of the buttons,
        // and a text row: each press keeps its meaning, no load.
        let text = s_row_bounds(&s, 1, cx).center();
        let second = s_row_bounds(&s, 2, cx);
        let past = painted[1].buttons.last().unwrap().0.right() + px(40.);
        let beside = gpui_kit::point(past, second.center().y);
        let line = s_row_bounds(&s, 0, cx).center();
        for at in [text, beside, line] {
            press_at(&s, at, cx);
            assert_eq!(snapshot(&s, cx), before, "a press at {at:?}");
        }
        for _ in 0..3 {
            frame(&s, cx);
        }
        assert_eq!(fetches(cx), 0, "nothing loads");
        assert_eq!(
            kinds(&view, cx),
            [text_rows(1), remote_rows_of(0), text_rows(1)].concat()
        );
    }

    #[gpui_kit::test]
    fn always_load_from_in_the_preview_allows_the_host_and_loads(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 60)))]);
        let (v, s, view) = browse(cx, &rt, REMOTE, &[], "needle");
        press_button(&s, &view, URL, RemoteAction::AlwaysLoad, cx);
        until_loaded(&s, &view, &rt, cx);
        let hosts = v.settings.read().unwrap().remote_image_hosts.clone();
        assert_eq!(hosts, [HOST.to_string()], "written to the settings");
        assert!(cx.read(|cx| cx.global::<RemoteImages>().is_allowed(HOST)));
        assert_eq!(fetches(cx), 1);
    }

    #[gpui_kit::test]
    fn after_a_load_the_preview_keeps_its_first_match_in_view(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 2000)))]);
        let note = format!(
            "intro\n![a](https://img.example.com/pic.png)\n{}the needle\n{}",
            "line\n".repeat(5),
            "line\n".repeat(200)
        );
        let (_v, s, view) = browse(cx, &rt, &note, &[], "needle");
        let (hit, _, _) = cx.read(|cx| view.read(cx).wrap_state());
        assert_eq!(hit, Some(1 + 2 + 5), "after the placeholder's two rows");
        let replans = cx.read(|cx| view.read(cx).replans);
        view.update(cx, |p, cx| {
            p.run_remote_action(URL.into(), None, RemoteAction::Load, cx)
        });
        until_loaded(&s, &view, &rt, cx);
        assert_eq!(
            cx.read(|cx| view.read(cx).replans),
            replans + 1,
            "made again once"
        );
        let rows = cx.read(|cx| view.read(cx).shown_images())[0].rows;
        assert!(rows > 8, "a tall image: {rows} rows");
        let (hit, _, _) = cx.read(|cx| view.read(cx).wrap_state());
        assert_eq!(hit, Some(1 + rows + 5), "the first match moved down");
        let shown = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find((PREVIEW_ROW_ID, hit.unwrap())).is_some()
            })
            .unwrap();
        assert!(shown, "the first match is still in view");
        let lines = cx.read(|cx| view.read(cx).shown_lines());
        assert_eq!(lines[hit.unwrap()].0, "the needle");
    }

    /// The reader never scrolled and the first match is in view: when an
    /// allowed-host image on screen loads and the rows are made again, the
    /// view does not move to center the match.
    #[gpui_kit::test]
    fn an_unscrolled_preview_with_its_first_match_in_view_does_not_move_on_a_load(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 200)))]);
        let note = format!(
            "![a](https://img.example.com/pic.png)\n{}the needle\n{}",
            "line\n".repeat(20),
            "line\n".repeat(200)
        );
        let (_v, s, view) = browse(cx, &rt, &note, &[HOST], "needle");
        let count = kinds(&view, cx).len();
        assert_eq!(top_row(&s, count, cx), 0, "the match is in view at the top");
        let (hit, _, _) = cx.read(|cx| view.read(cx).wrap_state());
        let in_view = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find((PREVIEW_ROW_ID, hit.unwrap())).is_some()
            })
            .unwrap();
        assert!(in_view, "the first match is in the first screen");
        drive(&rt, cx, |cx| {
            frame(&s, cx);
            kinds(&view, cx)
                .iter()
                .any(|k| matches!(k, RowKind::Image { .. }))
        });
        frame(&s, cx);
        let (hit, _, _) = cx.read(|cx| view.read(cx).wrap_state());
        let hit = hit.unwrap();
        let rows = cx.read(|cx| view.read(cx).shown_images())[0].rows;
        assert!(rows > remote_rows(), "the rows grew: {rows}");
        let count = kinds(&view, cx).len();
        assert_eq!(top_row(&s, count, cx), 0, "the view did not move");
        let shown = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find((PREVIEW_ROW_ID, hit)).is_some()
            })
            .unwrap();
        assert!(shown, "the first match is still in view");
    }

    #[gpui_kit::test]
    fn after_a_load_the_preview_keeps_its_top_line(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 2000)))]);
        let note = format!(
            "needle\n![a](https://img.example.com/pic.png)\n{}",
            (0..200)
                .map(|i| format!("line {i:03}\n"))
                .collect::<String>()
        );
        let (_v, s, view) = browse(cx, &rt, &note, &[], "needle");
        let count = kinds(&view, cx).len();
        view.update(cx, |p, cx| p.scroll_rows(40, cx));
        frame(&s, cx);
        let top = top_row(&s, count, cx);
        let line = cx.read(|cx| view.read(cx).row_lines())[top];
        assert!(line > 10, "scrolled past the embed: {line}");
        view.update(cx, |p, cx| {
            p.run_remote_action(URL.into(), None, RemoteAction::Load, cx)
        });
        drive(&rt, cx, |cx| {
            frame(&s, cx);
            kinds(&view, cx)
                .iter()
                .any(|k| matches!(k, RowKind::Image { .. }))
        });
        frame(&s, cx);
        let count = kinds(&view, cx).len();
        let top = top_row(&s, count, cx);
        assert_eq!(
            cx.read(|cx| view.read(cx).row_lines())[top],
            line,
            "the same note line leads the pane"
        );
    }

    #[gpui_kit::test]
    fn a_preview_of_a_note_with_a_saved_copy_draws_the_image_with_no_fetch(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        let env = install(cx, &[]);
        env.cache.save(URL, &png(TALL.0, TALL.1)).unwrap();
        let saved = env.cache.find(URL).unwrap();
        // The model: the copy's rows from its header, before any frame.
        let (v, list) = vault_with(REMOTE, &[]);
        let shown = marked_text_cached(
            path("n.md"),
            REMOTE.to_string(),
            &needles(&["after"]),
            80,
            PANE,
            Some((&v.vault, &list)),
            Some(&env.cache),
        );
        let want = [text_rows(1), image_rows(0, 6), text_rows(1)].concat();
        assert_eq!(shown.kinds(), want);
        assert_eq!(shown.images()[0].file.as_ref(), Some(&saved));
        // On screen: drawn with its pixels and its tooltip, nothing asked.
        let (_v, s, view) = browse(cx, &rt, REMOTE, &[HOST], "needle");
        let images = cx.read(|cx| view.read(cx).shown_images());
        assert_eq!(
            images[0].file.as_ref(),
            Some(&saved),
            "the first read found it"
        );
        assert!(kinds(&view, cx).contains(&RowKind::Image { image: 0, index: 0 }));
        until_ready(&s, &view, &rt, cx);
        let label = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                let ix = images[0].first_row;
                window
                    .within((PREVIEW_ROW_ID, ix))
                    .find(PREVIEW_IMAGE_ID)
                    .label()
                    .map(str::to_string)
            })
            .unwrap();
        assert_eq!(label.as_deref(), Some("a cat"), "the tooltip only");
        assert_eq!(fetches(cx), 0);
        assert!(!slices(&s, &view, cx).is_empty());
    }

    #[gpui_kit::test]
    fn the_preview_does_not_fetch_for_an_embed_past_the_first_100_or_off_screen(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        let off = "https://img.example.com/off.png";
        let past = "https://img.example.com/past.png";
        let env = install(cx, &[(off, Hop::Body(png(10, 20)))]);
        // The first embed is far below the first match; 99 more fill the
        // hundred; the hundred-and-first stays text.
        let note = format!(
            "needle\n{}![o]({off})\n{}![p]({past})\n",
            "line\n".repeat(300),
            "![[gone.png]]\n".repeat(99)
        );
        let (_v, s, view) = browse(cx, &rt, &note, &[HOST], "needle");
        for _ in 0..3 {
            frame(&s, cx);
        }
        assert_eq!(fetches(cx), 0, "the embed is off screen");
        let kinds_now = kinds(&view, cx);
        assert_eq!(
            *kinds_now.last().unwrap(),
            RowKind::Text,
            "the 101st is text"
        );
        let first = cx.read(|cx| view.read(cx).shown_images())[0].first_row;
        super::scroll_to_row(&s, &view, first, cx);
        drive(&rt, cx, |cx| {
            frame(&s, cx);
            fetches(cx) >= 1
        });
        view.update(cx, |p, cx| p.scroll_rows(100_000, cx));
        for _ in 0..3 {
            frame(&s, cx);
        }
        assert_eq!(
            env.fake.calls(),
            [off.to_string()],
            "only the one on screen"
        );
    }

    /// Another view (an editor, a facet chunk) loads the copy: the preview
    /// asked for nothing, and makes its rows again at its next frame.
    #[gpui_kit::test]
    fn a_copy_loaded_elsewhere_makes_the_preview_rows_again(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[(URL, Hop::Body(png(10, 60)))]);
        let (_v, s, view) = browse(cx, &rt, REMOTE, &[], "needle");
        let mut store = cx.read(|cx| cx.global::<RemoteImages>().clone());
        cx.update(|cx| store.load(URL, cx, |_| {}));
        until_loaded(&s, &view, &rt, cx);
        assert_eq!(cx.read(|cx| view.read(cx).replans), 1);
        assert_eq!(fetches(cx), 1);
    }

    #[test]
    fn the_source_reader_draws_the_placeholder_and_keeps_the_section_rule() {
        let text = "# a\n![x](https://img.example.com/x.png)\n# b\nbody b\n![y](https://img.example.com/y.png)\n";
        let (v, list) = vault_with(text, &[]);
        let mark = Mark::Section(SectionMark {
            ordinal: 0,
            heading: "b".into(),
            chunk: "# b\nbody b\n![y](https://img.example.com/y.png)".into(),
        });
        let shown = laid_out(&v, &list, text, &mark, PANE);
        // `x` is outside the section: only its placeholder. `y` is inside:
        // its raw line, then its placeholder.
        let want = [
            text_rows(1),
            remote_rows_of(0),
            text_rows(3),
            remote_rows_of(1),
        ]
        .concat();
        assert_eq!(shown.kinds(), want);
        assert_eq!(shown.summary().1, Some(3), "the section's first line");
        assert_eq!(
            shown.lines_and_hits()[5].0,
            "![y](https://img.example.com/y.png)"
        );
    }

    #[gpui_kit::test]
    fn the_source_reader_paints_the_placeholder(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let _env = install(cx, &[]);
        let text = "# b\nbody b\n![y](https://img.example.com/pic.png)\nend\n";
        let v = rt.block_on(test_vault(&[("n.md", text)], &[]));
        let vault = v.vault.clone();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::editor_view::init(cx);
        });
        let window = cx.add_window(|_, cx| SearchPreview::new(vault, cx));
        let reader = window.root(cx).unwrap();
        reader.update(cx, |p, cx| {
            p.show_marked(
                Some(path("n.md")),
                Mark::Section(SectionMark {
                    ordinal: 0,
                    heading: "b".into(),
                    chunk: "# b\nbody b".into(),
                }),
                cx,
            )
        });
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| {
                window.refresh();
                window.render_frame(cx)
            })
            .unwrap();
        };
        drive(&rt, cx, |cx| {
            draw(cx);
            cx.executor().advance_clock(super::PREVIEW_DEBOUNCE);
            cx.run_until_parked();
            !cx.read(|cx| reader.read(cx).painted_remote()).is_empty()
        });
        let want = [text_rows(2), remote_rows_of(0), text_rows(1)].concat();
        assert_eq!(kinds(&reader, cx), want, "outside the section: no raw line");
        let painted = cx.read(|cx| reader.read(cx).painted_remote());
        assert!(painted.iter().all(|p| p.look == both()), "{painted:?}");
        assert_eq!(fetches(cx), 0);
    }

    #[test]
    fn vault_images_in_the_read_only_views_are_unchanged() {
        let alone = NOTE;
        let both_kinds = format!("{NOTE}\n![r](https://img.example.com/pic.png)");
        let (v, list) = vault_with(&both_kinds, &[("pic.png", png(TALL.0, TALL.1))]);
        let one = laid_out(&v, &list, alone, &needles(&["after"]), PANE);
        let two = laid_out(&v, &list, &both_kinds, &needles(&["after"]), PANE);
        let n = one.kinds().len();
        assert_eq!(
            two.kinds()[..n],
            one.kinds()[..],
            "the vault image's rows as before"
        );
        assert_eq!(two.kinds()[n..], remote_rows_of(1)[..]);
        assert_eq!(two.images()[0], one.images()[0], "the same vault image");
        assert_eq!(two.images()[0].rows, rows_in(TALL, PANE));
    }
}
