use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;

use super::*;
use crate::logic::files::FileListEntry;
use crate::logic::list::{Emit, Loaded, RowSource};

fn p(s: &str) -> VaultPath {
    VaultPath::new(s)
}

fn record(path: &str, title: &str, modified_secs: u64) -> NoteRecord {
    (
        NoteEntryData {
            path: p(path),
            size: 0,
            modified_secs,
        },
        NoteContentData::new(title.to_string(), 0),
    )
}

fn paths(rows: &[FileListEntry]) -> Vec<String> {
    rows.iter().map(|r| r.path().to_string()).collect()
}

fn assert_like(rows: &[FileListEntry], expected: &[&str]) {
    assert_eq!(rows.len(), expected.len(), "{:?}", paths(rows));
    for (row, want) in rows.iter().zip(expected) {
        assert!(row.path().is_like(&p(want)), "{} vs {want}", row.path());
    }
}

fn names(notes: &[NoteRecord], idx: &[usize]) -> Vec<String> {
    idx.iter()
        .map(|&i| notes[i].0.path.get_parent_path().1)
        .collect()
}

/// Run a source's `load` and return the rows of its single `Replace`.
async fn load(source: &impl RowSource<FileListEntry>, query: &str) -> Vec<FileListEntry> {
    let (emit, rx) = Emit::capture();
    source.load(query, emit).await;
    match rx.try_recv().expect("one event") {
        (_, Loaded::Replace(rows)) => rows,
        _ => panic!("expected a Replace"),
    }
}

// ---- finder: order and filter ---------------------------------------------

#[test]
fn recency_order_is_newest_first_with_ties_in_list_order() {
    let notes = vec![
        record("old.md", "", 10),
        record("new.md", "", 30),
        record("tie_a.md", "", 20),
        record("tie_b.md", "", 20),
    ];
    assert_eq!(
        names(&notes, &recency_order(&notes)),
        ["new.md", "tie_a.md", "tie_b.md", "old.md"]
    );
    assert_eq!(finder_order(&notes, ""), recency_order(&notes));
}

#[test]
fn fuzzy_match_ranks_filename_and_title_and_drops_misses() {
    let notes = vec![
        record("pxlxaxn.md", "scattered", 1),
        record("banana.md", "Yellow fruit", 2),
        record("plan.md", "contiguous", 3),
    ];
    // Contiguous beats scattered; the miss is gone.
    assert_eq!(
        names(&notes, &fuzzy_match(&notes, "plan")),
        ["plan.md", "pxlxaxn.md"]
    );
    // The title is part of the haystack, case ignored.
    assert_eq!(names(&notes, &fuzzy_match(&notes, "yellow")), ["banana.md"]);
    assert!(fuzzy_match(&notes, "zzzz").is_empty());
    assert_eq!(finder_order(&notes, "plan"), fuzzy_match(&notes, "plan"));
}

// ---- finder: the Create row -----------------------------------------------

#[test]
fn create_row_only_for_a_non_blank_query() {
    assert!(create_row(&VaultPath::root(), "").is_none());
    let row = create_row(&p("projects"), "Idea").unwrap();
    let expected = p("projects").append(&VaultPath::note_path_from("Idea"));
    assert_eq!(row.path(), &expected);
    assert!(matches!(
        &row,
        FileListEntry::CreateNote { filename, .. } if *filename == expected.to_string()
    ));
    // Relative segments resolve against the current directory.
    let up = create_row(&p("projects"), "../top").unwrap();
    assert!(up.path().is_like(&VaultPath::note_path_from("top")));
}

#[tokio::test]
async fn finder_provider_lists_every_note_and_leads_with_create() {
    let vault = crate::test_support::temp_vault("pickers_finder").await;
    vault
        .create_note(&p("alpha.md"), "Alpha title")
        .await
        .unwrap();
    vault.create_note(&p("sub/beta.md"), "").await.unwrap();
    let finder = FileFinderProvider::new(vault.clone(), p("sub"));

    // Index paths come back vault-absolute: compare slice-wise (`is_like`).
    let mut all = load(&finder, "").await;
    all.sort_by_key(|r| r.path().to_string());
    assert_like(&all, &["alpha.md", "sub/beta.md"]);
    assert!(finder.leading_row("").is_none());

    let hits = load(&finder, "alpha").await;
    assert_like(&hits, &["alpha.md"]);
    assert!(matches!(&hits[0], FileListEntry::Note { title, .. } if title == "Alpha title"));
    // Present even though `alpha` exists: creating it loads the existing note.
    assert_eq!(
        finder.leading_row("alpha").unwrap().path(),
        &p("sub").append(&VaultPath::note_path_from("alpha"))
    );

    // A blank title shows the placeholder.
    let beta = load(&finder, "beta").await;
    assert!(matches!(&beta[0], FileListEntry::Note { title, .. } if title == "<no title>"));
}

#[tokio::test]
async fn note_row_carries_the_journal_date() {
    let vault = crate::test_support::temp_vault("pickers_journal").await;
    let path = vault
        .journal_path()
        .append(&VaultPath::note_path_from("2026-03-17"));
    let (entry, content) = record(&path.to_string(), "Day", 0);
    let FileListEntry::Note { journal_date, .. } = note_row(&vault, &entry, &content) else {
        panic!("a note row");
    };
    assert_eq!(journal_date.as_deref(), Some("Tuesday, March 17, 2026"));
    let (entry, content) = record("plain.md", "Plain", 0);
    let FileListEntry::Note { journal_date, .. } = note_row(&vault, &entry, &content) else {
        panic!("a note row");
    };
    assert_eq!(journal_date, None);
}

// ---- link results ----------------------------------------------------------

#[tokio::test]
async fn link_results_list_every_match_in_order() {
    let results = vec![
        record("a/dup.md", "First", 0),
        record("b/dup.md", "", 0),
        record("c/dup.md", "Third", 0),
    ];
    let rows = link_rows(results.clone());
    assert_eq!(
        paths(&rows),
        [
            p("a/dup.md").to_string(),
            p("b/dup.md").to_string(),
            p("c/dup.md").to_string()
        ]
    );
    assert!(
        matches!(&rows[1], FileListEntry::Note { title, journal_date: None, .. } if title == "<no title>")
    );

    let provider = LinkResultsProvider::from_results(results);
    assert_eq!(paths(&load(&provider, "").await), paths(&rows));
    assert!(provider.leading_row("dup").is_none());
    // Substring over title / filename / path, case ignored.
    assert_eq!(
        paths(&load(&provider, "THIRD").await),
        [p("c/dup.md").to_string()]
    );
    assert_eq!(
        paths(&load(&provider, "b").await),
        [p("b/dup.md").to_string()]
    );
    assert_eq!(filter_link_rows(&rows, "dup").len(), 3);
    assert!(filter_link_rows(&rows, "nope").is_empty());
}

// ---- open rule -------------------------------------------------------------

#[test]
fn picker_open_creates_only_the_create_row() {
    let note = FileListEntry::Note {
        path: p("n.md"),
        title: "N".into(),
        filename: "n.md".into(),
        journal_date: None,
        is_open: false,
    };
    assert_eq!(picker_open(&note), PickerOpen::Open(p("n.md")));
    let create = FileListEntry::CreateNote {
        filename: "new.md".into(),
        path: p("new.md"),
    };
    assert_eq!(picker_open(&create), PickerOpen::Create(p("new.md")));
    assert_eq!(picker_open(&create).path(), &p("new.md"));
    // Never navigates: a directory row is opened by path like a note.
    let dir = FileListEntry::Directory {
        path: p("d"),
        name: "d".into(),
    };
    assert_eq!(picker_open(&dir), PickerOpen::Open(p("d")));
}
