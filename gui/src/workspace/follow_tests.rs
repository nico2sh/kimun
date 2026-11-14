//! UI integration tests for following links (M5b-2 G4): a real `Workspace`
//! on a real, indexed temp vault; the `FollowLink` chord and the editor's
//! double-click, through the window. The system's default program is
//! replaced by a recorder, so a URL never leaves the test; an image or an
//! attachment in the vault shows in the attachment view.
//!
//! Each test owns a current-thread tokio runtime and drives it between
//! GPUI's `run_until_parked` (see `start_tests::drive`).
#![cfg(test)]

use std::cell::RefCell;
use std::ffi::OsString;
use std::rc::Rc;

use gpui_kit::test::{TestEventExt, TestWindowExt};
use gpui_kit::{AppContext, MouseButton, Pixels, Point, TestAppContext, point, px};
use kimun_core::nfs::VaultPath;
use tokio::runtime::Runtime;

use super::start_tests::{Started, drive, open_started_with, runtime, vault_start};
use crate::startup::FirstNote;
use crate::test_vault::{TestVault, test_vault};

fn path(p: &str) -> VaultPath {
    VaultPath::new(p)
}

impl Started {
    fn open_note(&self, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| self.workspace.read(cx).status.note.clone())
    }

    /// Whether the editor is on the note at `p`.
    fn on(&self, p: &str, cx: &TestAppContext) -> bool {
        self.open_note(cx).is_some_and(|n| n.is_like(&path(p)))
    }

    fn chord(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
    }

    /// Window-local centre of editor cell (`col`, `row`), from a fresh frame.
    fn cell(&self, col: u16, row: u16, cx: &mut TestAppContext) -> Point<Pixels> {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            let editor = self.workspace.read(cx).active_state().editor.clone();
            let painted = editor.read(cx).painted.clone().expect("a frame");
            let c = painted.cells;
            point(
                painted.origin.x + px(c.x_of(col) + c.width() / 2.0),
                painted.origin.y + px(c.y_of(row) + c.height() / 2.0),
            )
        })
        .unwrap()
    }

    /// A plain left click (press and release) at `at`.
    fn click(&self, at: Point<Pixels>, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, cx| {
            window.pointer_down(at, MouseButton::Left, cx);
            window.pointer_up(at, MouseButton::Left, cx);
        })
        .unwrap();
    }

    /// Whether the attachment view shows the file at `p`.
    fn shown_attachment_is(&self, p: &str, cx: &TestAppContext) -> bool {
        cx.read(|cx| self.workspace.read(cx).shown_attachment(cx))
            .is_some_and(|shown| shown.is_like(&path(p)))
    }

    /// Hand URLs and files to a recorder instead of the system.
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

/// A vault holding `notes`, started on `first` (vim editor when `vim`),
/// driven until its text shows and the index is built (follow by name needs
/// it).
fn started_on(
    cx: &mut TestAppContext,
    rt: &Runtime,
    notes: &[(&str, &str)],
    first: &str,
    vim: bool,
) -> (TestVault, Started) {
    let v = rt.block_on(test_vault(notes, &[]));
    let started = open_started_with(
        cx,
        vault_start(&v, FirstNote::Note(path(first))),
        v.settings.clone(),
        vim,
    );
    // Two presses are a double-click by the clock between them: freeze it, so a
    // loaded machine cannot put 400 ms between two simulated presses.
    cx.update(|cx| {
        let editor = started.workspace.read(cx).active_state().editor.clone();
        editor.update(cx, |view, _| {
            view.set_clock(crate::editor_view::frozen_clock)
        });
    });
    let want = notes.iter().find(|(p, _)| *p == first).unwrap().1;
    drive(rt, cx, |cx| {
        started.text(cx) == want && !started.indexing(cx)
    });
    (v, started)
}

/// Let tokio and GPUI run a while, for "nothing happens" checks.
fn settle(rt: &Runtime, cx: &mut TestAppContext) {
    let mut rounds = 0;
    drive(rt, cx, |_| {
        rounds += 1;
        rounds > 20
    });
}

// ── the chord ────────────────────────────────────────────────────────────

/// primary+N on a wikilink opens its note through the session: the old one
/// is saved first, the Footer and the FILES marker follow.
#[gpui_kit::test]
fn the_chord_on_a_wikilink_opens_the_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "[[bravo]] here"), ("sub/bravo.md", "Bravo")],
        "a.md",
        false,
    );
    // An unsaved edit; the caret stays on the link's first bracket.
    cx.update_window(started.handle, |_, window, cx| window.input("X", cx))
        .unwrap();
    started.chord("ctrl-enter", cx);
    drive(&rt, cx, |cx| started.text(cx) == "Bravo");
    assert!(started.on("sub/bravo.md", cx));
    assert_eq!(
        rt.block_on(v.vault.get_note_text(&path("a.md"))).unwrap(),
        "X[[bravo]] here",
        "the note left was saved first"
    );
    let marked = cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .files
            .as_ref()
            .unwrap()
            .read(cx)
            .model()
            .open_note()
            .cloned()
    });
    assert!(
        marked.is_some_and(|m| m.is_like(&path("sub/bravo.md"))),
        "FILES marks the followed note"
    );
}

/// primary+Enter follows too, in the vim editor as well (Normal mode): the
/// vim engine reads no Ctrl+Enter, so the keymap keeps it.
#[gpui_kit::test]
fn primary_enter_follows_under_vim(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "[[bravo]]"), ("bravo.md", "Bravo")],
        "a.md",
        true,
    );
    started.chord("ctrl-enter", cx);
    drive(&rt, cx, |cx| started.text(cx) == "Bravo");
    assert!(started.on("bravo.md", cx));
}

/// The chord off a link does nothing.
#[gpui_kit::test]
fn the_chord_off_a_link_does_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "plain [[bravo]]"), ("bravo.md", "Bravo")],
        "a.md",
        false,
    );
    started.chord("ctrl-enter", cx);
    settle(&rt, cx);
    assert!(started.on("a.md", cx));
    assert_eq!(started.flash(cx), None);
}

/// A wikilink with a folder part is a name suffix, not a path under the
/// following note's folder: `[[work/dup]]` from `journal/` opens
/// `/work/dup`, the one note it ends.
#[gpui_kit::test]
fn a_suffix_wikilink_opens_its_note_from_another_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[
            ("journal/a.md", "[[work/dup]]"),
            ("dup.md", "Root"),
            ("work/dup.md", "Work"),
        ],
        "journal/a.md",
        false,
    );
    started.chord("ctrl-enter", cx);
    drive(&rt, cx, |cx| started.text(cx) == "Work");
    assert!(started.on("work/dup.md", cx));
}

/// A bare markdown `[x](dup.md)` is the same-folder note, exactly: no
/// chooser, though two other notes share the name.
#[gpui_kit::test]
fn a_bare_markdown_link_opens_the_same_folder_note(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[
            ("journal/a.md", "[x](dup.md)"),
            ("journal/dup.md", "Here"),
            ("dup.md", "Root"),
            ("work/dup.md", "Work"),
        ],
        "journal/a.md",
        false,
    );
    started.chord("ctrl-enter", cx);
    drive(&rt, cx, |cx| started.text(cx) == "Here");
    assert!(started.on("journal/dup.md", cx));
}

// ── the double-click ─────────────────────────────────────────────────────

/// Two presses on one cell of a link follow it (ADR-0043).
#[gpui_kit::test]
fn a_double_click_on_a_link_follows_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "see [[bravo]] now"), ("bravo.md", "Bravo")],
        "a.md",
        false,
    );
    let on_link = started.cell(7, 0, cx);
    started.click(on_link, cx);
    started.click(on_link, cx);
    drive(&rt, cx, |cx| started.text(cx) == "Bravo");
    assert!(started.on("bravo.md", cx));
}

/// One press on a link only places the caret; two presses on neighbouring
/// cells are no double-click either.
#[gpui_kit::test]
fn a_single_click_on_a_link_only_places_the_caret(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(
        cx,
        &rt,
        &[("a.md", "see [[bravo]] now"), ("bravo.md", "Bravo")],
        "a.md",
        false,
    );
    let on_link = started.cell(7, 0, cx);
    started.click(on_link, cx);
    settle(&rt, cx);
    assert!(started.on("a.md", cx));
    let caret = cx.read(|cx| {
        started
            .workspace
            .read(cx)
            .active_state()
            .editor
            .read(cx)
            .editor
            .cursor()
    });
    // The link's brackets were concealed under the press: cell 7 is its `v`.
    assert_eq!(caret, (0, 9));
    let next = started.cell(8, 0, cx);
    started.click(next, cx);
    settle(&rt, cx);
    assert!(started.on("a.md", cx), "another cell");
}

// ── outcomes ─────────────────────────────────────────────────────────────

// A link to no note (the create dialog) and one several notes answer to
// (the link chooser) are tested with the picker, in `picker_tests`.

/// A URL goes to the system, never to the editor.
#[gpui_kit::test]
fn a_url_opens_externally_not_in_the_editor(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let text = "[site](https://example.com/x)";
    let (_v, started) = started_on(cx, &rt, &[("a.md", text)], "a.md", false);
    let opened = started.record_opens(cx);
    started.chord("ctrl-enter", cx);
    drive(&rt, cx, |_| !opened.borrow().is_empty());
    assert_eq!(
        *opened.borrow(),
        vec![OsString::from("https://example.com/x")]
    );
    assert!(started.on("a.md", cx));
    assert_eq!(started.text(cx), text);
    let flash = started.flash(cx).unwrap();
    assert!(
        flash.starts_with("Opening https://example.com/x"),
        "{flash}"
    );
}

/// An image shows in the attachment view, resolved against the note's
/// folder; nothing goes to the system.
#[gpui_kit::test]
fn an_image_shows_from_the_notes_folder(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("sub/a.md", "![](img/pic.png)")],
        "sub/a.md",
        false,
    );
    rt.block_on(
        v.vault
            .save_attachment(&path("sub/img/pic.png"), b"\x89PNG\x00"),
    )
    .unwrap();
    let opened = started.record_opens(cx);
    started.chord("ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_attachment_is("sub/img/pic.png", cx)
    });
    assert!(opened.borrow().is_empty());
    assert_eq!(
        started.open_note(cx),
        None,
        "no note is open while it shows"
    );
}

/// A markdown link to an attachment shows it, resolved against the note's
/// folder (its `#fragment` dropped); nothing goes to the system.
#[gpui_kit::test]
fn an_attachment_link_shows_the_attachment(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (v, started) = started_on(
        cx,
        &rt,
        &[("sub/a.md", "[doc](../files/doc.pdf#page=2)")],
        "sub/a.md",
        false,
    );
    rt.block_on(v.vault.save_attachment(&path("files/doc.pdf"), b"%PDF\x00"))
        .unwrap();
    let opened = started.record_opens(cx);
    started.chord("ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_attachment_is("files/doc.pdf", cx)
    });
    assert!(opened.borrow().is_empty());
}

/// A label opens the search browser on it (M7b G4); nothing opens, nothing
/// goes to the system.
#[gpui_kit::test]
fn a_label_opens_the_search_browser_on_it(cx: &mut TestAppContext) {
    let rt = runtime();
    let _guard = rt.enter();
    let (_v, started) = started_on(cx, &rt, &[("a.md", "#rust")], "a.md", false);
    let opened = started.record_opens(cx);
    started.chord("ctrl-enter", cx);
    drive(&rt, cx, |cx| {
        started.shown_modal(cx) == Some(super::modal::ModalKind::Search)
    });
    assert_eq!(started.flash(cx), None);
    assert!(opened.borrow().is_empty());
    assert!(started.on("a.md", cx));
}
