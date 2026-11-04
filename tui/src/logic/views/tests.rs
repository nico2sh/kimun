use std::num::NonZeroU64;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use super::*;
use crate::logic::list::{Emit, ListRow, Loaded, RowSource};
use crate::logic::nav::LinksTab;
use crate::test_support::temp_vault;

fn note(name: &str) -> VaultPath {
    VaultPath::note_path_from(name)
}

/// Run a source's `load` and return the rows of its single `Replace`.
async fn load<R: ListRow>(source: &impl RowSource<R>) -> Vec<R> {
    let (emit, rx) = Emit::capture();
    source.load("ignored", emit).await;
    match rx.try_recv().expect("one event") {
        (_, Loaded::Replace(rows)) => rows,
        _ => panic!("expected a Replace"),
    }
}

// ---- TAGS -------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn tags_are_most_used_first_ties_alphabetical() {
    let vault = temp_vault("views-tags").await;
    vault.validate_and_init().await.unwrap();
    vault
        .save_note(&note("a"), "x #alpha #beta #zeta")
        .await
        .unwrap();
    vault
        .save_note(&note("b"), "y #alpha #gamma")
        .await
        .unwrap();

    let rows = tag_entries(&vault).await;
    let labels: Vec<(&str, usize)> = rows.iter().map(|r| (r.label.as_str(), r.count)).collect();
    assert_eq!(
        labels,
        vec![("alpha", 2), ("beta", 1), ("gamma", 1), ("zeta", 1)]
    );

    // The source emits the same rows once, and never reloads on a query.
    let source = TagSource::new(vault);
    assert!(!source.reload_on_query());
    let loaded: Vec<String> = load(&source).await.into_iter().map(|r| r.label).collect();
    assert_eq!(loaded, ["alpha", "beta", "gamma", "zeta"]);
}

#[test]
fn tag_yank_adds_the_sigil_only_for_a_hashtag_label() {
    let tag = |label: &str| TagEntry {
        label: label.to_string(),
        count: 1,
    };
    assert_eq!(tag("alpha").yank_target().unwrap().text, "#alpha");
    assert_eq!(tag("two words").yank_target().unwrap().text, "two words");
    assert_eq!(tag("alpha").yank_target().unwrap().noun, "tag");
}

// ---- LINKS ------------------------------------------------------------------

/// projectx is linked from linker (twice), mentioned without a link in
/// mentions, and mentions itself.
async fn links_vault() -> std::sync::Arc<NoteVault> {
    let vault = temp_vault("views-links").await;
    vault.validate_and_init().await.unwrap();
    vault
        .save_note(&note("projectx"), "projectx: the note body")
        .await
        .unwrap();
    vault
        .save_note(
            &note("linker"),
            "links to [[projectx]] here and [[projectx]] again, about projectx",
        )
        .await
        .unwrap();
    vault
        .save_note(&note("mentions"), "talks about projectx without linking")
        .await
        .unwrap();
    vault
}

fn filenames(rows: &[LinkEntry]) -> Vec<&str> {
    rows.iter().map(|r| r.filename.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn links_tabs_on_a_temp_vault() {
    let vault = links_vault().await;

    let back = link_entries(&vault, &note("projectx"), LinksTab::Backlinks).await;
    assert_eq!(filenames(&back), ["linker.md"], "backlinks");

    // Each outgoing target once, titled by its clean name.
    let out = link_entries(&vault, &note("linker"), LinksTab::Outgoing).await;
    assert_eq!(filenames(&out), ["projectx.md"], "outgoing");
    // A resolved row is titled like a backlink row: by the note's own title.
    assert_eq!(out[0].title, "projectx: the note body");

    // Unlinked: the mention, not the linker, not the note itself.
    let unlinked = link_entries(&vault, &note("projectx"), LinksTab::Unlinked).await;
    assert_eq!(filenames(&unlinked), ["mentions.md"], "unlinked");

    // The source wraps the same rows.
    let source = LinksSource::new(vault, note("projectx"), LinksTab::Unlinked);
    assert!(!source.reload_on_query());
    assert_eq!(filenames(&load(&source).await), ["mentions.md"]);
}

/// Outgoing links resolve the way a followed link does: a bare wikilink is a
/// vault-wide name lookup (the note may live in any folder), a relative path
/// link is already absolute, and a link to nothing keeps the path a follow
/// would offer to create.
#[tokio::test(flavor = "multi_thread")]
async fn outgoing_links_resolve_like_a_follow() {
    let vault = temp_vault("views-outgoing").await;
    vault.validate_and_init().await.unwrap();
    vault
        .save_note(&note("work/anton"), "anton body")
        .await
        .unwrap();
    vault
        .save_note(&note("notes/design"), "design body")
        .await
        .unwrap();
    vault
        .save_note(
            &note("notes/index"),
            "see [[anton]], [[design]], [the same](../work/anton.md) and [[ghost]]",
        )
        .await
        .unwrap();

    let out = link_entries(&vault, &note("notes/index"), LinksTab::Outgoing).await;
    let paths: Vec<String> = out
        .iter()
        .map(|r| r.path.to_string().trim_start_matches('/').to_string())
        .collect();
    // `anton` and `../work/anton` are one note; `design` is a sibling;
    // `ghost` is missing and keeps the target as written (a follow's Missing).
    assert_eq!(
        paths,
        ["work/anton.md", "notes/design.md", "ghost.md"],
        "outgoing rows: {paths:?}"
    );
    assert_eq!(out[0].title, "anton body", "titled like a backlink row");
}

/// Outgoing rows follow the link forms: a wikilink folder part is a suffix
/// matched in any folder (ambiguous: one row per candidate), a bare markdown
/// destination is a path in the linking note's folder.
#[tokio::test(flavor = "multi_thread")]
async fn outgoing_links_follow_the_link_forms() {
    let vault = temp_vault("views-outgoing-forms").await;
    vault.validate_and_init().await.unwrap();
    for path in ["dup", "work/dup", "a/work/dup"] {
        vault.save_note(&note(path), "body").await.unwrap();
    }
    vault
        .save_note(
            &note("journal/today"),
            "[[work/dup]] [[a/work/dup]] [x](dup.md) [y](../dup.md)",
        )
        .await
        .unwrap();

    let out = link_entries(&vault, &note("journal/today"), LinksTab::Outgoing).await;
    let mut paths: Vec<String> = out.iter().map(|r| r.path.to_string()).collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            "/a/work/dup.md",
            "/dup.md",
            "/journal/dup.md",
            "/work/dup.md"
        ],
        "outgoing rows: {paths:?}"
    );
}

/// Every wikilink form, from a note in another folder: a bare shared name
/// lists each candidate, a suffix any folder's notes ending with it, a
/// `/` or `../` path the one note, case ignored; each note once.
#[tokio::test(flavor = "multi_thread")]
async fn outgoing_rows_for_the_wikilink_forms() {
    let vault = temp_vault("views-outgoing-wikilinks").await;
    vault.validate_and_init().await.unwrap();
    for path in ["dup", "work/dup", "a/work/dup", "notwork/dup", "solo"] {
        vault.save_note(&note(path), "body").await.unwrap();
    }
    let rows_of = async |text: &str| {
        vault.save_note(&note("journal/today"), text).await.unwrap();
        let out = link_entries(&vault, &note("journal/today"), LinksTab::Outgoing).await;
        let mut paths: Vec<String> = out.iter().map(|r| r.path.to_string()).collect();
        paths.sort();
        paths
    };
    assert_eq!(
        rows_of("[[dup]]").await,
        [
            "/a/work/dup.md",
            "/dup.md",
            "/notwork/dup.md",
            "/work/dup.md"
        ]
    );
    assert_eq!(
        rows_of("[[WORK/DUP#Goals]]").await,
        ["/a/work/dup.md", "/work/dup.md"]
    );
    assert_eq!(rows_of("[[/work/dup]]").await, ["/work/dup.md"]);
    assert_eq!(rows_of("[[../notwork/dup]]").await, ["/notwork/dup.md"]);
    assert_eq!(
        rows_of("[[solo]] [[/solo]] ![[solo^b]]").await,
        ["/solo.md"],
        "one note reached by several forms is listed once"
    );
    assert_eq!(rows_of("[[ghost/dup]]").await, ["/ghost/dup.md"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn links_without_a_note_are_empty() {
    let vault = links_vault().await;
    for tab in LinksTab::ORDER {
        assert!(
            link_entries(&vault, &VaultPath::empty(), tab)
                .await
                .is_empty()
        );
        assert!(
            link_entries(&vault, &VaultPath::root(), tab)
                .await
                .is_empty()
        );
    }
}

#[test]
fn link_entry_from_path_and_yank() {
    let entry = LinkEntry::from_path(note("some note"));
    assert_eq!(entry.title, "some note");
    assert_eq!(entry.filename, "some note.md");
    assert_eq!(entry.match_text(), Some("some note.md"));
    let yank = entry.yank_target().unwrap();
    assert_eq!(yank.noun, "path");
    assert_eq!(yank.text, entry.path.to_string());
}

// ---- OUTLINE ----------------------------------------------------------------

fn rev(n: u64) -> Option<NonZeroU64> {
    NonZeroU64::new(n)
}

fn sync(model: &mut OutlineModel, revision: Option<NonZeroU64>, text: &str) -> Changed {
    let text = text.to_string();
    model.sync(&note("doc"), revision, move || text)
}

fn rows(model: &OutlineModel) -> Vec<(&str, usize)> {
    model
        .entries()
        .iter()
        .map(|e| (e.heading(), e.depth))
        .collect()
}

#[test]
fn outline_lists_headings_in_order_with_nesting_depth() {
    let mut model = OutlineModel::new();
    let text = "# Top\nintro\n## Sub One\nbody\n## Sub Two\nmore\n# Second\nend\n";
    assert_eq!(sync(&mut model, rev(1), text), Changed::Rebuilt);
    assert_eq!(
        rows(&model),
        [("Top", 1), ("Sub One", 2), ("Sub Two", 2), ("Second", 1)]
    );
    assert!(model.note().is_like(&note("doc")));
    assert_eq!(model.revision(), rev(1));

    // Depth is nesting, not the markdown level.
    let mut model = OutlineModel::new();
    sync(&mut model, rev(1), "## A\n#### B\n# C\n");
    assert_eq!(rows(&model), [("A", 1), ("B", 2), ("C", 1)]);
}

#[test]
fn outline_numbers_same_named_headings() {
    let mut model = OutlineModel::new();
    sync(&mut model, rev(1), "# Notes\n## Other\n## Notes\n");
    let targets: Vec<(usize, usize, usize)> = model
        .entries()
        .iter()
        .map(|e| (e.target.occurrence, e.target.count, e.target.line))
        .collect();
    assert_eq!(targets, [(0, 2, 0), (0, 1, 1), (1, 2, 2)]);
}

#[test]
fn outline_sync_skips_the_parse_at_an_unchanged_revision() {
    let mut model = OutlineModel::new();
    let doc = note("doc");
    assert_eq!(
        model.sync(&doc, rev(7), || "# A\n".to_string()),
        Changed::Rebuilt
    );
    assert_eq!(
        model.sync(&doc, rev(7), || panic!("parsed at an unchanged revision")),
        Changed::Kept
    );
    // An unknown revision always re-reads (same headings: kept).
    let mut read = false;
    let changed = model.sync(&doc, None, || {
        read = true;
        "# A\n".to_string()
    });
    assert!(read);
    assert_eq!(changed, Changed::Kept);
}

#[test]
fn outline_sync_rebuilds_for_another_note_at_the_same_revision() {
    let mut model = OutlineModel::new();
    model.sync(&note("one"), rev(1), || "# A\n".to_string());
    let mut read = false;
    let changed = model.sync(&note("two"), rev(1), || {
        read = true;
        "# A\n".to_string()
    });
    assert!(read);
    assert_eq!(changed, Changed::Rebuilt);
    assert!(model.note().is_like(&note("two")));
}

#[test]
fn outline_first_sync_rebuilds_even_with_no_headings() {
    // The empty note path and no headings equal the fresh model's state; the
    // first sync still hands out a (empty) list.
    let mut model = OutlineModel::new();
    assert_eq!(
        model.sync(&VaultPath::empty(), rev(1), String::new),
        Changed::Rebuilt
    );
    assert_eq!(
        model.sync(&VaultPath::empty(), rev(2), String::new),
        Changed::Kept
    );
}

#[test]
fn outline_moved_heading_keeps_the_rows_and_resolves_the_new_line() {
    let mut model = OutlineModel::new();
    sync(&mut model, rev(1), "# A\n## Beta\n");
    let beta = model.entries()[1].target.clone();
    assert_eq!(beta.line, 1);
    // Lines typed above: same headings, both moved down.
    assert_eq!(
        sync(&mut model, rev(2), "intro\n\n# A\n## Beta\n"),
        Changed::Kept
    );
    // The entries are current even though the rows were kept.
    assert_eq!(model.entries()[1].target.line, 3);
    assert_eq!(model.resolve(&beta), Some(3));
    assert_eq!(model.revision(), rev(2));
}

#[test]
fn outline_heading_edit_rebuilds() {
    let mut model = OutlineModel::new();
    sync(&mut model, rev(1), "# A\n## Beta\n## Bet\n");
    assert_eq!(
        sync(&mut model, rev(2), "# A\n## Beta\n## Bet\n## Gamma\n"),
        Changed::Rebuilt
    );
    assert_eq!(model.entries().len(), 4);
    // A depth change alone rebuilds too.
    assert_eq!(
        sync(&mut model, rev(3), "# A\n### Beta\n## Bet\n## Gamma\n"),
        Changed::Kept,
        "### under # is still depth 2"
    );
    assert_eq!(
        sync(&mut model, rev(4), "# A\n# Beta\n## Bet\n## Gamma\n"),
        Changed::Rebuilt
    );
}

#[test]
fn resolve_follows_same_named_headings_through_moved_lines() {
    let mut model = OutlineModel::new();
    sync(&mut model, rev(1), "# Notes\n## Notes\n");
    let second = model.entries()[1].target.clone();
    sync(&mut model, rev(2), "new\nnew\n# Notes\n## Notes\n");
    assert_eq!(model.resolve(&second), Some(3));
}

#[test]
fn resolve_takes_the_nearest_when_a_same_named_heading_was_added() {
    let mut model = OutlineModel::new();
    let mut text = String::from("# Notes\n");
    text.push_str(&"x\n".repeat(9));
    text.push_str("## Notes\n");
    sync(&mut model, rev(1), &text);
    let second = model.entries()[1].target.clone();
    assert_eq!(second.line, 10);
    let mut edited = String::from("# Notes\n");
    edited.push_str(&"x\n".repeat(4));
    edited.push_str("## Notes\n");
    edited.push_str(&"x\n".repeat(5));
    edited.push_str("## Notes\n");
    sync(&mut model, rev(2), &edited);
    assert_eq!(model.resolve(&second), Some(11));
}

#[test]
fn resolve_finds_nothing_for_a_renamed_heading() {
    let mut model = OutlineModel::new();
    sync(&mut model, rev(1), "# A\n## Foo\n");
    let foo = model.entries()[1].target.clone();
    sync(&mut model, rev(2), "# A\n## Bar\n");
    assert_eq!(model.resolve(&foo), None);
}

#[test]
fn outline_yank_is_the_heading_text() {
    let entries = OutlineEntry::all_of("# Top\n## Sub\n");
    let yank = entries[1].yank_target().unwrap();
    assert_eq!((yank.text.as_str(), yank.noun), ("Sub", "heading"));
    assert_eq!(entries[1].match_text(), Some("Sub"));
}
