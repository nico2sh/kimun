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
    drive(&rt, cx, |cx| {
        started.text(cx) == "Bravo" && !started.indexing(cx)
    });
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
    drive(&rt, cx, |cx| {
        started.text(cx) == "Bravo" && !started.indexing(cx)
    });
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
    drive(&rt, cx, |cx| {
        started.text(cx) == "Work" && !started.indexing(cx)
    });
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
    drive(&rt, cx, |cx| {
        started.text(cx) == "Here" && !started.indexing(cx)
    });
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
    drive(&rt, cx, |cx| {
        started.text(cx) == "Bravo" && !started.indexing(cx)
    });
    assert!(started.on("bravo.md", cx));
}

/// One press with the platform's follow modifier (Cmd on macOS, Ctrl
/// elsewhere) follows the link, with no first press.
#[gpui_kit::test]
fn a_modifier_click_on_a_link_follows_it(cx: &mut TestAppContext) {
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
    // The harness installs the Linux keymap, so the primary key is Ctrl on every OS.
    let held = gpui_kit::Modifiers {
        control: true,
        ..Default::default()
    };
    cx.update_window(started.handle, |_, window, cx| {
        window.change_modifiers(held, cx)
    })
    .unwrap();
    started.click(on_link, cx);
    drive(&rt, cx, |cx| {
        started.text(cx) == "Bravo" && !started.indexing(cx)
    });
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

// ── where a follow opens (`OpenAt`) ───────────────────────────────────────

mod open_at {
    use gpui_kit::TestAppContext;
    use kimun_core::note::scan::LinkSpanKind;
    use kimun_notes::logic::layout_tree::{Axis, GroupId};

    use super::super::modal::ModalKind;
    use super::super::note::OpenAt;
    use super::*;

    fn groups(s: &Started, cx: &TestAppContext) -> Vec<GroupId> {
        cx.read(|cx| {
            let region = s.workspace.read(cx).layout.region();
            region.groups(region.shown())
        })
    }

    fn tabs(s: &Started, cx: &TestAppContext) -> usize {
        cx.read(|cx| s.workspace.read(cx).layout.region().tabs().len())
    }

    fn active(s: &Started, cx: &TestAppContext) -> GroupId {
        cx.read(|cx| s.workspace.read(cx).layout.active_group())
    }

    /// The note the group `g` holds, if any.
    fn note_in(s: &Started, g: GroupId, cx: &TestAppContext) -> Option<VaultPath> {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .host_in(g)
                .and_then(|host| host.note.clone())
        })
    }

    fn follow(
        s: &Started,
        from: &str,
        target: &str,
        kind: LinkSpanKind,
        at: OpenAt,
        cx: &mut TestAppContext,
    ) {
        let from = path(from);
        let target = target.to_string();
        cx.update(|cx| {
            s.workspace
                .update(cx, |w, cx| w.follow_link_from(&from, target, kind, at, cx))
        });
    }

    fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx)
        })
        .unwrap();
        cx.run_until_parked();
    }

    const SPLIT: OpenAt = OpenAt::Split(Axis::Horizontal);

    #[gpui_kit::test]
    fn following_into_a_split_opens_the_note_in_a_group_beside_the_active_one(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = started_on(
            cx,
            &rt,
            &[("a.md", "[[bravo]]"), ("bravo.md", "Bravo")],
            "a.md",
            false,
        );
        let first = active(&s, cx);
        follow(&s, "a.md", "bravo", LinkSpanKind::WikiLink, SPLIT, cx);
        drive(&rt, cx, |cx| {
            groups(&s, cx).len() == 2 && s.text(cx) == "Bravo"
        });
        assert_eq!(groups(&s, cx).first(), Some(&first));
        assert_ne!(active(&s, cx), first, "the new group has the keyboard");
        assert_eq!(note_in(&s, first, cx), Some(path("a.md")));
        assert!(s.on("bravo.md", cx));
    }

    #[gpui_kit::test]
    fn following_into_a_new_tab_focuses_the_tab_of_a_note_that_is_open(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = started_on(
            cx,
            &rt,
            &[("a.md", "[[bravo]]"), ("bravo.md", "Bravo")],
            "a.md",
            false,
        );
        let first = active(&s, cx);
        cx.update(|cx| {
            s.workspace
                .update(cx, |w, cx| w.open_note_in_new_tab(path("bravo.md"), cx))
        });
        drive(&rt, cx, |cx| s.text(cx) == "Bravo");
        let second = active(&s, cx);
        assert_eq!(tabs(&s, cx), 2);
        cx.update(|cx| s.workspace.update(cx, |w, cx| w.focus_group(first, cx)));
        cx.run_until_parked();
        assert_eq!(active(&s, cx), first);
        follow(
            &s,
            "a.md",
            "bravo",
            LinkSpanKind::WikiLink,
            OpenAt::NewTab,
            cx,
        );
        drive(&rt, cx, |cx| active(&s, cx) == second);
        assert_eq!(tabs(&s, cx), 2, "no third tab");
    }

    #[gpui_kit::test]
    fn a_missing_link_followed_into_a_split_creates_the_note_and_opens_it_in_the_split(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = started_on(cx, &rt, &[("a.md", "[[nobody]]")], "a.md", false);
        let first = active(&s, cx);
        follow(&s, "a.md", "nobody", LinkSpanKind::WikiLink, SPLIT, cx);
        drive(&rt, cx, |cx| {
            s.shown_modal(cx) == Some(ModalKind::CreateNote)
        });
        press(&s, "enter", cx);
        drive(&rt, cx, |cx| {
            s.on("nobody.md", cx) && groups(&s, cx).len() == 2
        });
        assert_ne!(active(&s, cx), first);
        assert_eq!(note_in(&s, first, cx), Some(path("a.md")));
    }

    #[gpui_kit::test]
    fn an_ambiguous_link_followed_into_a_split_opens_the_chosen_note_in_the_split(
        cx: &mut TestAppContext,
    ) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = started_on(
            cx,
            &rt,
            &[
                ("a.md", "[[dup]]"),
                ("dup.md", "Dup one"),
                ("sub/dup.md", "Dup two"),
            ],
            "a.md",
            false,
        );
        let first = active(&s, cx);
        follow(&s, "a.md", "dup", LinkSpanKind::WikiLink, SPLIT, cx);
        drive(&rt, cx, |cx| {
            s.shown_modal(cx) == Some(ModalKind::LinkChooser)
        });
        let view = cx.read(|cx| {
            s.workspace
                .read(cx)
                .picker
                .as_ref()
                .and_then(|p| p.upgrade())
                .expect("a picker is open")
        });
        drive(&rt, cx, |cx| {
            cx.read(|cx| !view.read(cx).is_loading() && !view.read(cx).rows().is_empty())
        });
        press(&s, "enter", cx);
        drive(&rt, cx, |cx| {
            groups(&s, cx).len() == 2
                && note_in(&s, active(&s, cx), cx).is_some_and(|n| n.get_name() == "dup.md")
        });
        assert_eq!(note_in(&s, first, cx), Some(path("a.md")));
    }

    #[gpui_kit::test]
    fn following_an_attachment_externally_hands_the_file_to_the_opener(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s) = started_on(
            cx,
            &rt,
            &[("sub/a.md", "[doc](../files/doc.pdf)")],
            "sub/a.md",
            false,
        );
        rt.block_on(v.vault.save_attachment(&path("files/doc.pdf"), b"%PDF\x00"))
            .unwrap();
        let opened = s.record_opens(cx);
        follow(
            &s,
            "sub/a.md",
            "../files/doc.pdf",
            LinkSpanKind::Markdown,
            OpenAt::Externally,
            cx,
        );
        drive(&rt, cx, |_| !opened.borrow().is_empty());
        let opened = opened.borrow();
        assert_eq!(opened.len(), 1);
        assert!(
            std::path::Path::new(&opened[0]).ends_with("files/doc.pdf"),
            "{:?}",
            opened[0]
        );
        assert!(s.on("sub/a.md", cx), "the editor stays");
        assert!(!s.shown_attachment_is("files/doc.pdf", cx));
    }
}

/// A note never goes to the default app: a follow that lands on a note
/// (a missing one, several matches) opens it in place instead.
#[test]
fn externally_becomes_here_for_a_note() {
    use super::follow::for_a_note;
    use super::note::OpenAt;
    use kimun_notes::logic::layout_tree::Axis;
    assert_eq!(for_a_note(OpenAt::Externally), OpenAt::Here);
    for at in [
        OpenAt::Here,
        OpenAt::NewTab,
        OpenAt::Split(Axis::Horizontal),
        OpenAt::Split(Axis::Vertical),
    ] {
        assert_eq!(for_a_note(at), at);
    }
}
