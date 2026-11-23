//! The Preview pane's and the Source reader's **Inline image** rows
//! (CONTEXT.md): how an embed line becomes fixed-height image rows, which
//! embed lines keep their raw text, and what the load reads. The model is
//! tested on its own (`marked_text` on a real temp vault); the load order
//! and the place kept on a height change on a real `Workspace`.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (`start_tests::drive`).
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, px, size};
use kimun_core::nfs::VaultPath;
use tokio::runtime::Runtime;

use super::picker::NotePicker;
use super::search::{
    Mark, PREVIEW_BYTES, PREVIEW_DEBOUNCE, PREVIEW_ROW_ID, PreviewText, RowKind, SearchPreview,
    SectionMark, header_reads, marked_text,
};
use super::start_tests::{Started, drive, open_started, runtime, select_all_key, vault_start};
use crate::attachment_index::AttachmentIndex;
use crate::image_core::ImagePane;
use crate::inline_image::{Failure, fit, rows_for};
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
