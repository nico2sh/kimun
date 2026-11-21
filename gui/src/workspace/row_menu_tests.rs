//! Tests for the **row menu** ([`super::row_menu`]): which entries each kind
//! of row shows, and the menu on real rows (FILES and FIND) in a window:
//! what each pick does, and that a right-click opens nothing.
#![cfg(test)]

use super::file_dialogs::FileOpKind;
use super::row_menu::{
    RowAction, RowKind, RowMenuContext, RowMenuEntry, RowPick, row_menu_entries,
};

fn entries(kind: RowKind, in_files: bool, pinned: bool) -> Vec<RowMenuEntry> {
    row_menu_entries(&RowMenuContext {
        kind,
        in_files,
        pinned,
    })
}

fn act(action: RowAction) -> RowMenuEntry {
    RowMenuEntry::Item(RowPick::Act(action))
}

fn file(kind: FileOpKind) -> RowMenuEntry {
    RowMenuEntry::Item(RowPick::File(kind))
}

const SEP: RowMenuEntry = RowMenuEntry::Separator;

/// Rename…, Move…, Delete…: every row ends with them.
fn file_group() -> [RowMenuEntry; 3] {
    [
        file(FileOpKind::Rename),
        file(FileOpKind::Move),
        file(FileOpKind::Delete),
    ]
}

#[test]
fn a_note_row_has_open_helper_and_file_groups_in_order() {
    let mut want = vec![
        act(RowAction::Open),
        act(RowAction::OpenNewTab),
        act(RowAction::OpenSplitRight),
        act(RowAction::OpenSplitDown),
        SEP,
        act(RowAction::CopyPath),
        act(RowAction::Pin),
        act(RowAction::RevealInFiles),
        SEP,
    ];
    want.extend(file_group());
    assert_eq!(entries(RowKind::Note, false, false), want);
}

#[test]
fn the_file_browser_row_has_no_reveal() {
    for kind in [RowKind::Note, RowKind::Attachment, RowKind::Folder] {
        let shown = entries(kind, true, false);
        assert!(
            !shown.contains(&act(RowAction::RevealInFiles)),
            "{kind:?} in FILES"
        );
    }
    assert!(entries(RowKind::Attachment, false, false).contains(&act(RowAction::RevealInFiles)));
}

#[test]
fn a_folder_row_has_copy_path_and_file_items_only() {
    let mut want = vec![act(RowAction::CopyPath), SEP];
    want.extend(file_group());
    assert_eq!(entries(RowKind::Folder, true, false), want);
    assert_eq!(entries(RowKind::Folder, false, false), want);
}

#[test]
fn an_attachment_row_has_no_split_and_no_pin() {
    let mut want = vec![
        act(RowAction::Open),
        act(RowAction::OpenNewTab),
        act(RowAction::OpenInDefaultApp),
        SEP,
        act(RowAction::CopyPath),
        act(RowAction::RevealInFiles),
        SEP,
    ];
    want.extend(file_group());
    assert_eq!(entries(RowKind::Attachment, false, false), want);
    // Only notes pin: a stray `pinned` changes nothing.
    assert_eq!(entries(RowKind::Attachment, false, true), want);
}

#[test]
fn an_unpinned_note_offers_pin_and_a_pinned_note_offers_unpin() {
    let unpinned = entries(RowKind::Note, true, false);
    assert!(unpinned.contains(&act(RowAction::Pin)));
    assert!(!unpinned.contains(&act(RowAction::Unpin)));
    let pinned = entries(RowKind::Note, true, true);
    assert!(pinned.contains(&act(RowAction::Unpin)));
    assert!(!pinned.contains(&act(RowAction::Pin)));
}

#[test]
fn row_menu_labels_read_as_the_menu_shows_them() {
    let labels: Vec<&str> = entries(RowKind::Note, false, true)
        .into_iter()
        .map(|entry| match entry {
            RowMenuEntry::Item(pick) => pick.label(),
            RowMenuEntry::Separator => "-",
        })
        .collect();
    assert_eq!(
        labels,
        vec![
            "Open",
            "Open in new tab",
            "Open in split right",
            "Open in split down",
            "-",
            "Copy path",
            "Unpin note",
            "Reveal in file browser",
            "-",
            "Rename…",
            "Move…",
            "Delete…",
        ]
    );
    assert_eq!(
        RowPick::Act(RowAction::OpenInDefaultApp).label(),
        "Open in default app"
    );
    assert_eq!(RowPick::Act(RowAction::Pin).label(), "Pin note");
}

// ── the menu on real rows ───────────────────────────────────────────────

pub(super) mod in_workspace {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, ClipboardItem, SharedString, TestAppContext};
    use kimun_core::nfs::VaultPath;
    use kimun_notes::logic::files::FileListEntry;
    use kimun_notes::logic::nav::DrawerView;
    use kimun_notes::settings::session::{AxisRecord, GroupTreeRecord};
    use tokio::runtime::Runtime;

    use super::super::files::{FILES_PINNED_TOGGLE_ID, files_pinned_row_id, row_id};
    use super::super::find::find_row_id;
    use super::super::find_tests::open_find_on_backlinks;
    use super::super::modal::ModalKind;
    use super::super::row_menu::{
        ALREADY_PINNED, NOT_PINNED, PATH_COPIED, PinnedPaths, RowAction, RowPick,
    };
    use super::super::start_tests::{Started, drive, open_started, runtime, vault_start};
    use crate::startup::FirstNote;
    use crate::test_vault::{TestVault, test_vault};

    /// `beta` and `sub/gamma` link to `a`, so FIND on `a`'s backlinks lists
    /// them; `doc.pdf` is an attachment beside `a`.
    const NOTES: &[(&str, &str)] = &[
        ("a.md", "alpha"),
        ("beta.md", "# Beta\nback to [[a]]"),
        ("sub/gamma.md", "# Gamma\nalso [[a]]"),
    ];

    fn path(p: &str) -> VaultPath {
        VaultPath::new(p)
    }

    /// The vault, with `doc.pdf`, started on `a.md`, driven until it shows,
    /// FILES has listed its folder and the index is built.
    fn start(cx: &mut TestAppContext, rt: &Runtime) -> (TestVault, Started) {
        start_pinned(cx, rt, &[])
    }

    /// [`start`], with the notes at `pins` pinned first.
    fn start_pinned(cx: &mut TestAppContext, rt: &Runtime, pins: &[&str]) -> (TestVault, Started) {
        let v = rt.block_on(test_vault(NOTES, &[]));
        rt.block_on(v.vault.save_attachment(&path("doc.pdf"), b"%PDF\x00"))
            .unwrap();
        for p in pins {
            rt.block_on(v.vault.toggle_pinned_note(&path(p))).unwrap();
        }
        let started = open_started(
            cx,
            vault_start(&v, FirstNote::Note(path("a.md"))),
            v.settings.clone(),
        );
        drive(rt, cx, |cx| {
            started.text(cx) == "alpha" && !started.indexing(cx) && files_listed(&started, cx)
        });
        (v, started)
    }

    fn files_listed(s: &Started, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            let Some(files) = s.workspace.read(cx).files.clone() else {
                return false;
            };
            files
                .read(cx)
                .model()
                .core()
                .is_some_and(|core| !core.is_loading() && core.visible_len() > 0)
        })
    }

    /// The FILES row for the entry at `p`.
    fn files_row(s: &Started, p: &str, cx: &TestAppContext) -> SharedString {
        cx.read(|cx| {
            let files = s.workspace.read(cx).files.clone().unwrap();
            let pane = files.read(cx);
            pane.model()
                .core()
                .unwrap()
                .visible_rows()
                .into_iter()
                .find(|r| {
                    !matches!(
                        r,
                        FileListEntry::Up { .. } | FileListEntry::CreateNote { .. }
                    ) && r.path().is_like(&path(p))
                })
                .map(row_id)
                .unwrap_or_else(|| panic!("no FILES row {p}"))
        })
    }

    /// The pins FILES shows.
    fn pins_in_files(s: &Started, cx: &TestAppContext) -> Vec<VaultPath> {
        cx.read(|cx| {
            let files = s.workspace.read(cx).files.clone().unwrap();
            files
                .read(cx)
                .pins()
                .iter()
                .map(|pin| pin.path.clone())
                .collect()
        })
    }

    /// Open FILES' pinned section once its pins are read.
    fn open_pinned_section(s: &Started, rt: &Runtime, cx: &mut TestAppContext) {
        drive(rt, cx, |cx| !pins_in_files(s, cx).is_empty());
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(FILES_PINNED_TOGGLE_ID, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.read(|cx| {
            let files = s.workspace.read(cx).files.clone().unwrap();
            files.read(cx).pins_open()
        }));
    }

    /// The pinned section's row for the note at `p`.
    fn pinned_row(s: &Started, p: &str, cx: &TestAppContext) -> SharedString {
        pins_in_files(s, cx)
            .iter()
            .find(|pin| pin.is_like(&path(p)))
            .map(files_pinned_row_id)
            .unwrap_or_else(|| panic!("no pinned row {p}"))
    }

    /// Right-click `row`, pick `Copy path`: what lands on the clipboard.
    fn copy_path_of(s: &Started, row: SharedString, cx: &mut TestAppContext) -> Option<String> {
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
        right_click(s, row, cx);
        pick(s, "Copy path", cx);
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    /// FIND on `a`'s backlinks, its rows landed: the row for `p` and the
    /// path the row names.
    fn find_row(
        s: &Started,
        rt: &Runtime,
        p: &str,
        cx: &mut TestAppContext,
    ) -> (SharedString, VaultPath) {
        open_find_on_backlinks(s, cx);
        let list = cx.read(|cx| {
            s.workspace
                .read(cx)
                .find
                .clone()
                .unwrap()
                .read(cx)
                .list
                .clone()
        });
        drive(rt, cx, |cx| {
            cx.read(|cx| {
                let list = list.read(cx);
                !list.is_loading() && list.rows().iter().any(|r| r.entry.path.is_like(&path(p)))
            })
        });
        let found = cx.read(|cx| {
            list.read(cx)
                .rows()
                .into_iter()
                .find(|r| r.entry.path.is_like(&path(p)))
                .map(|r| r.entry.path)
                .unwrap()
        });
        (find_row_id(&found), found)
    }

    pub(in crate::workspace) fn right_click(
        s: &Started,
        id: SharedString,
        cx: &mut TestAppContext,
    ) {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.right_click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// The open row menu's entries, top to bottom: each item's label, `-`
    /// for a separator.
    pub(in crate::workspace) fn menu_labels(s: &Started, cx: &mut TestAppContext) -> Vec<String> {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            let menu = window.within("popup-menu");
            (0usize..)
                .map_while(|ix| menu.try_find(ix))
                .map(|item| item.label().unwrap_or("-").to_string())
                .collect()
        })
        .unwrap()
    }

    /// Click the open row menu's item `label`.
    pub(in crate::workspace) fn pick(s: &Started, label: &str, cx: &mut TestAppContext) {
        let labels = menu_labels(s, cx);
        let ix = labels
            .iter()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("no {label:?} in {labels:?}"));
        cx.update_window(s.handle, |_, window, cx| {
            window.within("popup-menu").click(ix, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn press(s: &Started, key: &str, cx: &mut TestAppContext) {
        cx.update_window(s.handle, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn menu_open(s: &Started, cx: &mut TestAppContext) -> bool {
        cx.update_window(s.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("popup-menu").is_some()
        })
        .unwrap()
    }

    fn tabs(s: &Started, cx: &TestAppContext) -> usize {
        cx.read(|cx| s.workspace.read(cx).layout.region().tabs().len())
    }

    fn groups(s: &Started, cx: &TestAppContext) -> usize {
        cx.read(|cx| {
            let region = s.workspace.read(cx).layout.region();
            region.groups(region.shown()).len()
        })
    }

    fn split_axis(s: &Started, cx: &TestAppContext) -> Option<AxisRecord> {
        let record = cx.read(|cx| s.workspace.read(cx).session_record(cx));
        match &record.tabs[record.shown].tree {
            GroupTreeRecord::Split { axis, .. } => Some(*axis),
            _ => None,
        }
    }

    /// Whether the active group holds the note at `p`.
    fn on(s: &Started, p: &str, cx: &TestAppContext) -> bool {
        cx.read(|cx| {
            s.workspace
                .read(cx)
                .active_state()
                .notes
                .as_ref()?
                .note
                .clone()
        })
        .is_some_and(|n| n.is_like(&path(p)))
    }

    fn strings(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|l| l.to_string()).collect()
    }

    #[gpui_kit::test]
    fn a_right_click_on_a_file_row_shows_the_new_items(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let cases: [(&str, &[&str]); 3] = [
            (
                "beta.md",
                &[
                    "Open",
                    "Open in new tab",
                    "Open in split right",
                    "Open in split down",
                    "-",
                    "Copy path",
                    "Pin note",
                    "-",
                    "Rename…",
                    "Move…",
                    "Delete…",
                ],
            ),
            ("sub", &["Copy path", "-", "Rename…", "Move…", "Delete…"]),
            (
                "doc.pdf",
                &[
                    "Open",
                    "Open in new tab",
                    "Open in default app",
                    "-",
                    "Copy path",
                    "-",
                    "Rename…",
                    "Move…",
                    "Delete…",
                ],
            ),
        ];
        for (entry, want) in cases {
            let row = files_row(&s, entry, cx);
            right_click(&s, row, cx);
            assert_eq!(menu_labels(&s, cx), strings(want), "{entry}");
            press(&s, "escape", cx);
            assert!(!menu_open(&s, cx), "{entry}");
        }
    }

    #[gpui_kit::test]
    fn a_right_click_on_a_search_row_shows_the_new_items_and_reveal(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let (row, _) = find_row(&s, &rt, "beta.md", cx);
        right_click(&s, row, cx);
        assert_eq!(
            menu_labels(&s, cx),
            strings(&[
                "Open",
                "Open in new tab",
                "Open in split right",
                "Open in split down",
                "-",
                "Copy path",
                "Pin note",
                "Reveal in file browser",
                "-",
                "Rename…",
                "Move…",
                "Delete…",
            ])
        );
    }

    #[gpui_kit::test]
    fn picking_open_in_new_tab_from_a_row_opens_a_new_tab(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let (row, _) = find_row(&s, &rt, "beta.md", cx);
        right_click(&s, row, cx);
        pick(&s, "Open in new tab", cx);
        drive(&rt, cx, |cx| tabs(&s, cx) == 2 && on(&s, "beta.md", cx));
    }

    #[gpui_kit::test]
    fn picking_open_in_split_right_from_a_row_opens_a_split(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let row = files_row(&s, "beta.md", cx);
        right_click(&s, row, cx);
        pick(&s, "Open in split right", cx);
        drive(&rt, cx, |cx| groups(&s, cx) == 2 && on(&s, "beta.md", cx));
        assert_eq!(split_axis(&s, cx), Some(AxisRecord::Horizontal));
        assert_eq!(tabs(&s, cx), 1);
    }

    #[gpui_kit::test]
    fn picking_copy_path_puts_the_path_on_the_clipboard(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        // A folder.
        let row = files_row(&s, "sub", cx);
        assert_eq!(copy_path_of(&s, row, cx).as_deref(), Some("/sub"));
        let (row, _) = find_row(&s, &rt, "beta.md", cx);
        // The vault path as the app shows it: from the vault root.
        assert_eq!(copy_path_of(&s, row, cx).as_deref(), Some("/beta.md"));
        assert_eq!(s.flash(cx).as_deref(), Some(PATH_COPIED));
        assert!(on(&s, "a.md", cx), "nothing opens");
        assert_eq!(tabs(&s, cx), 1);
        // A row whose path is not rooted copies the same text.
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
        let workspace = s.workspace.clone();
        cx.update_window(s.handle, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.run_row_pick(
                    RowPick::Act(RowAction::CopyPath),
                    path("sub/gamma.md"),
                    window,
                    cx,
                )
            })
        })
        .unwrap();
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some("/sub/gamma.md"));
    }

    #[gpui_kit::test]
    fn copy_path_gives_the_same_text_for_a_note_in_every_list(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start_pinned(cx, &rt, &["beta.md"]);
        open_pinned_section(&s, &rt, cx);
        let from_pinned = copy_path_of(&s, pinned_row(&s, "beta.md", cx), cx);
        let from_files = copy_path_of(&s, files_row(&s, "beta.md", cx), cx);
        let (row, _) = find_row(&s, &rt, "beta.md", cx);
        let from_find = copy_path_of(&s, row, cx);
        assert_eq!(from_pinned.as_deref(), Some("/beta.md"));
        assert_eq!(from_files.as_deref(), Some("/beta.md"));
        assert_eq!(from_find.as_deref(), Some("/beta.md"));
    }

    #[gpui_kit::test]
    fn a_right_click_on_a_pinned_row_offers_unpin_and_unpin_drops_the_pin(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s) = start_pinned(cx, &rt, &["beta.md"]);
        open_pinned_section(&s, &rt, cx);
        // A note that is not pinned offers Pin note.
        right_click(&s, files_row(&s, "a.md", cx), cx);
        let labels = menu_labels(&s, cx);
        assert!(labels.contains(&"Pin note".to_string()), "{labels:?}");
        assert!(!labels.contains(&"Unpin note".to_string()), "{labels:?}");
        press(&s, "escape", cx);
        // The pinned row offers Unpin note.
        right_click(&s, pinned_row(&s, "beta.md", cx), cx);
        let labels = menu_labels(&s, cx);
        assert!(labels.contains(&"Unpin note".to_string()), "{labels:?}");
        assert!(!labels.contains(&"Pin note".to_string()), "{labels:?}");
        pick(&s, "Unpin note", cx);
        drive(&rt, cx, |cx| pins_in_files(&s, cx).is_empty());
        let on_disk = rt.block_on(v.vault.list_pinned_notes()).unwrap();
        assert!(on_disk.is_empty(), "{on_disk:?}");
        assert!(on(&s, "a.md", cx), "nothing opens");
    }

    #[gpui_kit::test]
    fn picking_pin_pins_the_note_and_the_pinned_section_follows(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s) = start(cx, &rt);
        let row = files_row(&s, "beta.md", cx);
        right_click(&s, row.clone(), cx);
        pick(&s, "Pin note", cx);
        let pinned_in_files = |cx: &TestAppContext| {
            cx.read(|cx| {
                let files = s.workspace.read(cx).files.clone().unwrap();
                files
                    .read(cx)
                    .pins()
                    .iter()
                    .any(|pin| pin.path.is_like(&path("beta.md")))
            })
        };
        drive(&rt, cx, |cx| pinned_in_files(cx));
        let on_disk = rt.block_on(v.vault.list_pinned_notes()).unwrap();
        assert!(on_disk.iter().any(|p| p.is_like(&path("beta.md"))));
        // The menu knows it now: the same row offers Unpin.
        right_click(&s, row, cx);
        let labels = menu_labels(&s, cx);
        assert!(labels.contains(&"Unpin note".to_string()), "{labels:?}");
        pick(&s, "Unpin note", cx);
        drive(&rt, cx, |cx| !pinned_in_files(cx));
    }

    /// Set the app-wide pins the row menu reads to `pins`, as a stale read
    /// would leave them.
    fn force_pinned_paths(pins: &[&str], cx: &mut TestAppContext) {
        let pins = pins.iter().map(|p| path(p).absolute()).collect();
        cx.update(|cx| cx.set_global(PinnedPaths(pins)));
    }

    /// A stale `Pin note` on a note that is already pinned keeps the pin: a
    /// pin never unpins.
    #[gpui_kit::test]
    fn a_stale_pin_on_a_pinned_note_leaves_it_pinned(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s) = start_pinned(cx, &rt, &["beta.md"]);
        drive(&rt, cx, |cx| !pins_in_files(&s, cx).is_empty());
        force_pinned_paths(&[], cx);
        right_click(&s, files_row(&s, "beta.md", cx), cx);
        pick(&s, "Pin note", cx);
        drive(&rt, cx, |cx| s.flash(cx).as_deref() == Some(ALREADY_PINNED));
        let on_disk = rt.block_on(v.vault.list_pinned_notes()).unwrap();
        assert_eq!(on_disk.len(), 1, "{on_disk:?}");
        assert!(on_disk[0].is_like(&path("beta.md")));
        // The menu's pins are read again: the row offers Unpin.
        drive(&rt, cx, |cx| {
            cx.read(|cx| cx.global::<PinnedPaths>().0.len() == 1)
        });
    }

    /// A stale `Unpin note` on a note that is not pinned pins nothing.
    #[gpui_kit::test]
    fn a_stale_unpin_on_a_note_that_is_not_pinned_leaves_it_unpinned(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s) = start(cx, &rt);
        force_pinned_paths(&["beta.md"], cx);
        right_click(&s, files_row(&s, "beta.md", cx), cx);
        pick(&s, "Unpin note", cx);
        drive(&rt, cx, |cx| s.flash(cx).as_deref() == Some(NOT_PINNED));
        let on_disk = rt.block_on(v.vault.list_pinned_notes()).unwrap();
        assert!(on_disk.is_empty(), "{on_disk:?}");
        drive(&rt, cx, |cx| {
            cx.read(|cx| cx.global::<PinnedPaths>().0.is_empty())
        });
    }

    /// The explicit actions, run straight: a second Pin and a second Unpin
    /// change nothing.
    #[gpui_kit::test]
    fn pin_and_unpin_are_idempotent(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s) = start(cx, &rt);
        let run = |action, cx: &mut TestAppContext| {
            let workspace = s.workspace.clone();
            cx.update_window(s.handle, |_, window, cx| {
                workspace.update(cx, |w, cx| {
                    w.run_row_pick(RowPick::Act(action), path("beta.md"), window, cx)
                })
            })
            .unwrap();
        };
        let pinned = || {
            rt.block_on(v.vault.list_pinned_notes())
                .unwrap()
                .iter()
                .any(|p| p.is_like(&path("beta.md")))
        };
        run(RowAction::Pin, cx);
        drive(&rt, cx, |cx| {
            s.flash(cx).is_some_and(|f| f.starts_with("pinned as"))
        });
        assert!(pinned());
        run(RowAction::Pin, cx);
        drive(&rt, cx, |cx| s.flash(cx).as_deref() == Some(ALREADY_PINNED));
        assert!(pinned());
        run(RowAction::Unpin, cx);
        drive(&rt, cx, |cx| s.flash(cx).as_deref() == Some("unpinned"));
        assert!(!pinned());
        run(RowAction::Unpin, cx);
        drive(&rt, cx, |cx| s.flash(cx).as_deref() == Some(NOT_PINNED));
        assert!(!pinned());
    }

    #[gpui_kit::test]
    fn picking_reveal_shows_the_note_in_the_file_browser(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let (row, _) = find_row(&s, &rt, "sub/gamma.md", cx);
        assert!(cx.read(|cx| !s.workspace.read(cx).is_shown(DrawerView::Files)));
        right_click(&s, row, cx);
        pick(&s, "Reveal in file browser", cx);
        let selected = |cx: &TestAppContext| {
            cx.read(|cx| {
                let files = s.workspace.read(cx).files.clone().unwrap();
                let pane = files.read(cx);
                pane.model()
                    .core()
                    .and_then(|core| core.selected_row())
                    .map(|r| r.path().clone())
            })
        };
        drive(&rt, cx, |cx| {
            selected(cx).is_some_and(|p| p.is_like(&path("sub/gamma.md")))
        });
        assert!(cx.read(|cx| s.workspace.read(cx).is_shown(DrawerView::Files)));
        let dir = cx.read(|cx| {
            let files = s.workspace.read(cx).files.clone().unwrap();
            files.read(cx).current_dir().clone()
        });
        assert!(dir.is_like(&path("sub")), "{dir}");
        assert!(on(&s, "a.md", cx), "nothing opens");
    }

    #[gpui_kit::test]
    fn picking_rename_from_a_row_still_opens_the_rename_dialog(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let (row, _) = find_row(&s, &rt, "beta.md", cx);
        right_click(&s, row, cx);
        pick(&s, "Rename…", cx);
        s.settle_dialog(cx);
        assert_eq!(s.shown_modal(cx), Some(ModalKind::Rename));
        let target = cx.read(|cx| match s.workspace.read(cx).file_dialog.clone() {
            Some(super::super::file_ops::FileDialogView::Rename(view)) => {
                view.upgrade().unwrap().read(cx).target().clone()
            }
            _ => panic!("no rename dialog"),
        });
        assert!(target.is_like(&path("beta.md")));
    }

    #[gpui_kit::test]
    fn a_right_click_still_opens_no_row(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (_v, s) = start(cx, &rt);
        let row = files_row(&s, "beta.md", cx);
        right_click(&s, row, cx);
        assert!(menu_open(&s, cx));
        press(&s, "escape", cx);
        let (row, _) = find_row(&s, &rt, "beta.md", cx);
        right_click(&s, row, cx);
        assert!(menu_open(&s, cx));
        press(&s, "escape", cx);
        drive(&rt, cx, |_| true);
        assert!(on(&s, "a.md", cx), "a stays open");
        assert_eq!(tabs(&s, cx), 1);
        assert_eq!(groups(&s, cx), 1);
    }

    #[gpui_kit::test]
    fn a_pick_on_a_deleted_note_flashes_an_error_and_opens_nothing(cx: &mut TestAppContext) {
        let rt = runtime();
        let _guard = rt.enter();
        let (v, s) = start(cx, &rt);
        // The row stays drawn: nothing tells FIND the note went.
        let (row, named) = find_row(&s, &rt, "beta.md", cx);
        for label in ["Open", "Open in new tab", "Open in split down"] {
            right_click(&s, row.clone(), cx);
            // Gone after the menu opened, before the pick.
            if rt.block_on(v.vault.exists(&named)) {
                rt.block_on(v.vault.delete_note(&named)).unwrap();
            }
            pick(&s, label, cx);
            drive(&rt, cx, |cx| {
                cx.read(|cx| {
                    s.workspace
                        .read(cx)
                        .last_flash()
                        .is_some_and(|(_, error)| error)
                })
            });
            let flash = s.flash(cx).unwrap();
            assert!(flash.contains("beta"), "{label}: {flash}");
            assert!(on(&s, "a.md", cx), "{label}: a stays open");
            assert_eq!(tabs(&s, cx), 1, "{label}");
            assert_eq!(groups(&s, cx), 1, "{label}");
            assert_eq!(s.shown_modal(cx), None, "{label}: no create dialog");
            // The next round starts with no flash standing.
            s.workspace.update(cx, |w, _| w.flashes.clear());
        }
    }
}
