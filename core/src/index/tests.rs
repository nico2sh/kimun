// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// mod.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and this file's TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use super::*;

use crate::nfs::{NoteEntryData, VaultPath};

/// A fresh index in a per-test temp dir. The `TempDir` is returned so the
/// caller keeps it alive for the index's lifetime.
async fn open_temp() -> (tempfile::TempDir, NoteIndex) {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("kimun.sqlite");
    let db = NoteIndex::open(&file::IndexFile::at(crate::system::sys(&db_path)))
        .await
        .unwrap();
    (tmp, db)
}

/// One note as `apply` takes it: the entry (size from the body, mtime 0)
/// paired with its text.
fn note(path: &str, body: &str) -> (NoteEntryData, String) {
    (
        NoteEntryData {
            path: VaultPath::note_path_from(path),
            size: body.len() as u64,
            modified_secs: 0,
        },
        body.to_string(),
    )
}

/// An `IndexDiff` that only adds.
fn added(entries: Vec<(NoteEntryData, String)>) -> IndexDiff {
    IndexDiff {
        to_add: entries,
        ..Default::default()
    }
}

/// An `IndexDiff` that only modifies (re-indexes existing notes).
fn modified(entries: Vec<(NoteEntryData, String)>) -> IndexDiff {
    IndexDiff {
        to_modify: entries,
        ..Default::default()
    }
}

/// The paths of query rows, sorted — the interface promises no row order.
fn paths<T>(rows: &[(NoteEntryData, T)]) -> Vec<String> {
    let mut v: Vec<String> = rows.iter().map(|(e, _)| e.path.to_string()).collect();
    v.sort();
    v
}

/// `VaultPath`s as sorted strings — same reason as [`paths`].
fn sorted_paths(paths: Vec<VaultPath>) -> Vec<String> {
    let mut v: Vec<String> = paths.into_iter().map(|p| p.to_string()).collect();
    v.sort();
    v
}

#[tokio::test]
async fn open_creates_parent_dir_for_db_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let nested = tmp.path().join("nested/dir/cache.kimuncache");
    // Parent dir does not exist yet.
    assert!(!nested.parent().unwrap().exists());

    let db = super::NoteIndex::open(&file::IndexFile::at(crate::system::sys(&nested)))
        .await
        .unwrap();
    assert!(nested.parent().unwrap().exists());
    assert!(nested.exists());
    // A fresh file has no schema — open must have healed it.
    assert!(!db.ready());
    db.close().await;
}

/// A db path is a path, not a URL. Splicing one into `sqlite:{}?mode=rwc`
/// means the first `?` *in the path* starts the query string, so SQLite is
/// handed a truncated filename and a garbage parameter. `?` is legal in a
/// directory name on Unix, so this reproduces it directly; the Windows
/// manifestation of the same bug is
/// [`open_accepts_a_canonicalized_db_path`].
#[cfg(unix)]
#[tokio::test]
async fn open_accepts_a_db_path_containing_a_question_mark() {
    let tmp = tempfile::TempDir::new().unwrap();
    let awkward = tmp.path().join("why not?").join("cache.kimuncache");

    let db = super::NoteIndex::open(&file::IndexFile::at(crate::system::sys(&awkward)))
        .await
        .unwrap();

    assert!(awkward.exists(), "db must be created at {awkward:?}");
    assert!(
        !db.ready(),
        "a fresh file has no schema — open must heal it"
    );
    db.close().await;
}

/// Every db path Kimun computes comes from a canonicalized directory
/// (`AppSettings::expand_path`, `ensure_dir_exists`). On Windows that means
/// the verbatim prefix `\\?\`, whose `?` broke the old URL-formatted
/// connection string on *every* open — the platform-specific face of
/// [`open_accepts_a_db_path_containing_a_question_mark`]. A no-op on Unix,
/// where canonicalize only resolves symlinks.
#[tokio::test]
async fn open_accepts_a_canonicalized_db_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let canonical = tmp.path().canonicalize().unwrap().join("cache.kimuncache");

    let db = super::NoteIndex::open(&file::IndexFile::at(crate::system::sys(&canonical)))
        .await
        .unwrap();

    assert!(canonical.exists(), "db must be created at {canonical:?}");
    db.close().await;
}

#[test]
fn test_search_terms_query_empty() {
    let (sql, params) = build_search_sql_query("");
    assert_eq!(sql, "");
    assert_eq!(params.len(), 0);
}

#[test]
fn test_search_terms_query_simple_terms() {
    let (sql, params) = build_search_sql_query("foo bar");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"foo\" \"bar\"");
}

#[test]
fn test_search_terms_query_single_term() {
    let (sql, params) = build_search_sql_query("keyword");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"keyword\"");
}

#[test]
fn test_search_terms_query_breadcrumb_only() {
    let (sql, params) = build_search_sql_query("@heading");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"heading\"");
}

#[test]
fn test_search_terms_query_breadcrumb_with_in() {
    let (sql, params) = build_search_sql_query("in:section");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"section\"");
}

#[test]
fn test_search_terms_query_multiple_breadcrumbs() {
    let (sql, params) = build_search_sql_query("@heading1 in:heading2");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"heading1\" \"heading2\"");
}

#[test]
fn test_search_terms_query_path_only() {
    let (sql, params) = build_search_sql_query("=filename");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?1 ESCAPE '\\'"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "%filename%");
}

#[test]
fn test_search_terms_query_path_with_at() {
    let (sql, params) = build_search_sql_query("name:directory");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?1 ESCAPE '\\'"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "%directory%");
}

#[test]
fn test_search_terms_query_multiple_paths() {
    let (sql, params) = build_search_sql_query("=file1 name:file2");
    // Same-type operators AND together (consistent with #, <, >, and the
    // documented "all terms are ANDed" precedence).
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?1 ESCAPE '\\' AND notes.noteName LIKE ?2 ESCAPE '\\'"
        );
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], "%file1%");
    assert_eq!(params[1], "%file2%");
}

#[test]
fn test_search_terms_query_terms_and_breadcrumb() {
    let (sql, params) = build_search_sql_query("keyword @section");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2"
        );
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], "\"keyword\"");
    assert_eq!(params[1], "\"section\"");
}

#[test]
fn test_search_terms_query_terms_and_path() {
    let (sql, params) = build_search_sql_query("keyword =file");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?2 ESCAPE '\\'"
        );
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], "\"keyword\"");
    assert_eq!(params[1], "%file%");
}

#[test]
fn test_search_terms_query_breadcrumb_and_path() {
    let (sql, params) = build_search_sql_query("@heading =file");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?2 ESCAPE '\\'"
        );
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], "\"heading\"");
    assert_eq!(params[1], "%file%");
}

#[test]
fn test_search_terms_query_all_combined() {
    let (sql, params) = build_search_sql_query("keyword @heading =file");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?3 ESCAPE '\\'"
        );
    assert_eq!(params.len(), 3);
    assert_eq!(params[0], "\"keyword\"");
    assert_eq!(params[1], "\"heading\"");
    assert_eq!(params[2], "%file%");
}

#[test]
fn test_search_terms_query_quoted_terms() {
    let (sql, params) = build_search_sql_query("\"exact phrase\" keyword");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"exact phrase\" \"keyword\"");
}

#[test]
fn test_search_terms_query_order_by_title_asc() {
    let (sql, params) = build_search_sql_query("keyword or:title");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"keyword\"");
}

#[test]
fn test_search_terms_query_order_by_title_desc() {
    let (sql, params) = build_search_sql_query("keyword -or:title");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"keyword\"");
}

#[test]
fn test_search_terms_query_order_by_filename_asc() {
    let (sql, params) = build_search_sql_query("keyword or:filename");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"keyword\"");
}

#[test]
fn test_search_terms_query_order_by_file_shorthand() {
    let (sql, params) = build_search_sql_query("keyword or:f");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"keyword\"");
}

#[test]
fn test_search_terms_query_order_by_title_shorthand() {
    let (sql, params) = build_search_sql_query("keyword or:t");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"keyword\"");
}

#[test]
fn test_search_terms_query_multiple_order_by() {
    let (sql, params) = build_search_sql_query("keyword ^title -^filename");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"keyword\"");
}

#[test]
fn test_search_terms_query_complex_with_order() {
    let (sql, params) = build_search_sql_query("keyword @section =file ^title");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notes.noteName LIKE ?3 ESCAPE '\\'"
        );
    assert_eq!(params.len(), 3);
    assert_eq!(params[0], "\"keyword\"");
    assert_eq!(params[1], "\"section\"");
    assert_eq!(params[2], "%file%");
}

#[test]
fn test_search_terms_query_only_order_by() {
    let (sql, params) = build_search_sql_query("^title");
    assert_eq!(sql, "");
    assert_eq!(params.len(), 0);
}

#[test]
fn test_search_terms_query_invalid_order_by_field() {
    let (sql, params) = build_search_sql_query("keyword ^invalid");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1"
        );
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"keyword\"");
}

#[test]
fn test_search_terms_query_whitespace_handling() {
    let (sql, params) = build_search_sql_query("  keyword   @section  ");
    assert_eq!(
            sql,
            "SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent MATCH ?1 INTERSECT SELECT DISTINCT notes.path as path, title, size, modified, hash, noteName FROM notesContent JOIN notes ON notesContent.path = notes.path WHERE notesContent.breadcrumb MATCH ?2"
        );
    assert_eq!(params.len(), 2);
    assert_eq!(params[0], "\"keyword\"");
    assert_eq!(params[1], "\"section\"");
}

#[test]
fn test_fts4_mixed_exclusion_sql_generation() {
    let (sql, params) = build_search_sql_query("meeting -cancelled");

    // Should use NOT IN subquery approach instead of FTS4 native exclusion
    assert!(sql.contains("notesContent MATCH"));
    assert!(sql.contains("NOT IN"));
    assert!(sql
        .contains("SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH"));
    // params: first is the excluded term (NOT IN subquery), second is the positive term
    assert_eq!(params.len(), 2);
    assert!(params.contains(&"\"cancelled\"".to_string()));
    assert!(params.contains(&"\"meeting\"".to_string()));

    assert!(sql.contains("SELECT DISTINCT"));
}

#[test]
fn test_exclusion_only_sql_generation() {
    // Critical test: exclusion-only queries MUST use NOT IN, not pure FTS4 MATCH
    let (sql, params) = build_search_sql_query("-cancelled");

    // Should NOT contain pure FTS4 exclusion (which is invalid)
    assert!(!sql.contains("MATCH \"-cancelled\""));
    // Should use NOT IN subquery approach
    assert!(sql.contains("NOT IN"));
    assert!(sql
        .contains("SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH"));
    assert_eq!(params.len(), 1);
    assert_eq!(params[0], "\"cancelled\"");
}

#[test]
fn test_breadcrumb_exclusion_sql_generation() {
    let (sql, params) = build_search_sql_query("@project -@draft");

    // Positive breadcrumb is a column-scoped MATCH; the exclusion is a
    // robust NOT IN subquery (not the old, broken inline `breadcrumb: -term`).
    assert!(sql.contains("notesContent.breadcrumb MATCH ?1"));
    assert!(sql.contains(
            "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent.breadcrumb MATCH ?2)"
        ));
    assert_eq!(
        params,
        vec!["\"project\"".to_string(), "\"draft\"".to_string()]
    );
}

#[test]
fn test_like_exclusion_sql_generation() {
    let (sql, params) = build_search_sql_query("=2024 -=draft");

    // Should generate filename query with positive and negative conditions
    assert!(sql.contains("notes.noteName LIKE"));
    assert!(sql.contains("notes.noteName NOT LIKE"));
    assert!(params.contains(&"%2024%".to_string()));
    assert!(params.contains(&"%draft%".to_string()));
}

#[test]
fn test_exclusion_only_like_query() {
    let (sql, params) = build_search_sql_query("-=draft -=temp");

    // Exclusion-only should still generate valid WHERE clause
    assert!(sql.contains("notes.noteName NOT LIKE"));
    // The new format embeds % in the param, not in the SQL template
    assert!(!sql.contains("NOT LIKE ('%'"));
    assert_eq!(params.len(), 2);
}

#[test]
fn test_path_exclusion_sql_generation() {
    let (sql, params) = build_search_sql_query("/projects -/archive");

    assert!(sql.contains("notes.basePath LIKE"));
    assert!(sql.contains("notes.basePath NOT LIKE"));
    assert!(params.contains(&"projects".to_string()));
    assert!(params.contains(&"archive".to_string()));
}

#[test]
fn test_exclusion_only_path_query() {
    let (sql, params) = build_search_sql_query("-/draft -/temp");

    assert!(sql.contains("notes.basePath NOT LIKE"));
    assert!(!sql.contains("notes.basePath LIKE ('/'"));
    assert_eq!(params.len(), 2);
}

#[tokio::test]
async fn labels_table_exists_after_create_tables() {
    let (_tmp, db) = open_temp().await;

    let row: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sqlite_master \
             WHERE type='table' AND name='labels'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(row.0, 1, "labels table should exist");

    // labels_by_name was removed in 0.7; the PK autoindex covers it.
    let idx_name: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sqlite_master \
             WHERE type='index' AND name='labels_by_name'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(
        idx_name.0, 0,
        "labels_by_name index must not exist (dropped in 0.7)"
    );

    let idx_path: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM sqlite_master \
             WHERE type='index' AND name='labels_by_path'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(idx_path.0, 1, "labels_by_path index should exist");

    db.close().await;
}

#[tokio::test]
async fn labels_are_persisted_on_note_insert() {
    let (_tmp, db) = open_temp().await;

    let path = VaultPath::note_path_from("/n.md");
    db.apply(added(vec![note(
        "/n.md",
        "Title\n\nbody with #foo and #Foo and #bar",
    )]))
    .await
    .unwrap();

    assert_eq!(
        db.label_counts().await.unwrap(),
        vec![("bar".to_string(), 1), ("foo".to_string(), 1)],
        "labels stored deduped + lowercased"
    );
    assert_eq!(
        sorted_paths(db.notes_with_label("foo").await.unwrap()),
        vec![path.to_string()]
    );
    assert_eq!(
        sorted_paths(db.notes_with_label("bar").await.unwrap()),
        vec![path.to_string()]
    );

    db.close().await;
}

#[tokio::test]
async fn reindexing_a_note_drops_removed_labels() {
    let (_tmp, db) = open_temp().await;

    db.apply(added(vec![note("/n.md", "before #draft #keep")]))
        .await
        .unwrap();
    db.apply(modified(vec![note("/n.md", "after #keep only")]))
        .await
        .unwrap();

    let mut labels = db.list_labels().await.unwrap();
    labels.sort();
    assert_eq!(
        labels,
        vec!["keep"],
        "reindex must drop labels no longer present"
    );
    assert!(db.notes_with_label("draft").await.unwrap().is_empty());

    db.close().await;
}

#[tokio::test]
async fn labels_are_removed_on_note_delete() {
    let (_tmp, db) = open_temp().await;

    let path = VaultPath::note_path_from("/n.md");
    db.apply(added(vec![note("/n.md", "x #drop")]))
        .await
        .unwrap();
    db.delete_notes(std::slice::from_ref(&path)).await.unwrap();

    assert!(db.label_counts().await.unwrap().is_empty());

    db.close().await;
}

#[test]
fn test_search_terms_query_label_only() {
    let (sql, params) = build_search_sql_query("#important");
    assert_eq!(params, vec!["important".to_string()]);
    assert!(
        sql.contains("FROM notes") && sql.contains("labels"),
        "query should join notes with labels: {}",
        sql
    );
}

#[test]
fn test_search_terms_query_two_labels_intersect() {
    let (sql, params) = build_search_sql_query("#a #b");
    assert_eq!(params.len(), 2);
    assert!(
        sql.contains("INTERSECT"),
        "two labels should INTERSECT: {}",
        sql
    );
}

#[test]
fn test_search_terms_query_links_only() {
    let (sql, params) = build_search_sql_query("<projects");
    assert_eq!(params, vec!["projects.md".to_string()]);
    assert!(
        sql.contains("FROM notes")
            && sql.contains("SELECT source FROM links")
            && sql.contains("notes.path IN"),
        "backlinks query should select sources from links: {}",
        sql
    );
    // Bare name (no wildcard) matches the indexed dest_name column with
    // plain equality — no leading-`%` scan.
    assert!(
        sql.contains("dest_name = ?1"),
        "expected indexed dest_name equality: {}",
        sql
    );
}

#[test]
fn test_search_terms_query_links_long_form() {
    let (_sql, params) = build_search_sql_query("lk:projects");
    assert_eq!(params, vec!["projects.md".to_string()]);
}

#[test]
fn test_search_terms_query_links_path_qualified() {
    let (sql, params) = build_search_sql_query("<work/projects");
    assert_eq!(params, vec!["work/projects.md".to_string()]);
    // Path-qualified anchors to the full path (relative or absolute) via
    // indexed equality on `destination`, not the bare-name column.
    assert!(
        sql.contains("destination = ?1 OR destination = ('/' || ?1)"),
        "expected path-anchored equality: {}",
        sql
    );
    assert!(!sql.contains("dest_name"));
}

#[test]
fn test_search_terms_query_links_wildcard() {
    let (sql, params) = build_search_sql_query("<proj*");
    assert_eq!(params, vec!["proj%.md".to_string()]);
    // Wildcard bare name uses a prefix LIKE on the indexed dest_name column.
    assert!(
        sql.contains("dest_name LIKE ?1 ESCAPE '\\'"),
        "expected dest_name LIKE for wildcard: {}",
        sql
    );
}

#[test]
fn test_search_terms_query_links_extension_optional() {
    let (_sql, params) = build_search_sql_query("<projects.md");
    assert_eq!(params, vec!["projects.md".to_string()]);
}

#[test]
fn test_search_terms_query_excluded_links() {
    let (sql, params) = build_search_sql_query("-<draft");
    assert_eq!(params, vec!["draft.md".to_string()]);
    assert!(
        sql.contains("notes.path NOT IN (SELECT source FROM links"),
        "excluded backlinks should use NOT IN: {}",
        sql
    );
}

#[test]
fn test_search_terms_query_two_links_intersect() {
    let (sql, params) = build_search_sql_query("<a <b");
    assert_eq!(params.len(), 2);
    assert!(
        sql.contains("INTERSECT"),
        "two backlinks should INTERSECT: {}",
        sql
    );
}

#[test]
fn test_search_terms_query_links_combined_with_operators() {
    // Free-text term + backlink + label all compose via INTERSECT.
    let (sql, params) = build_search_sql_query("meeting <spec #urgent");
    assert_eq!(sql.matches("INTERSECT").count(), 2);
    assert!(sql.contains("notesContent MATCH"));
    assert!(sql.contains("SELECT source FROM links"));
    assert!(sql.contains("FROM labels WHERE name"));
    // Params follow the fan-out order: content term, label, then backlink.
    assert_eq!(
        params,
        vec![
            "\"meeting\"".to_string(),
            "urgent".to_string(),
            "spec.md".to_string()
        ]
    );
}

#[tokio::test]
async fn search_combining_links_with_other_operators() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/work/a.md", "# Tasks\n[[spec]] meeting #urgent"),
        note("/b.md", "[[spec]] casual"),
        note("/c.md", "#urgent only, no link"),
    ]))
    .await
    .unwrap();

    // backlink + free-text term.
    let r = db.search("<spec meeting").await.unwrap();
    assert_eq!(paths(&r), vec!["/work/a.md".to_string()]);

    // backlink + label.
    let r = db.search("<spec #urgent").await.unwrap();
    assert_eq!(paths(&r), vec!["/work/a.md".to_string()]);

    // backlink + excluded label.
    let r = db.search("<spec -#urgent").await.unwrap();
    assert_eq!(paths(&r), vec!["/b.md".to_string()]);

    // backlink + path filter.
    let r = db.search("<spec /work").await.unwrap();
    assert_eq!(paths(&r), vec!["/work/a.md".to_string()]);

    // backlink + section (breadcrumb) filter.
    let r = db.search("<spec @tasks").await.unwrap();
    assert_eq!(paths(&r), vec!["/work/a.md".to_string()]);

    // backlink + filename filter.
    let r = db.search("<spec =b").await.unwrap();
    assert_eq!(paths(&r), vec!["/b.md".to_string()]);

    // label without link still matches the non-linking note.
    let r = db.search("#urgent -spec").await.unwrap();
    assert!(paths(&r).contains(&"/c.md".to_string()));

    db.close().await;
}

#[tokio::test]
async fn multiple_filename_terms_are_anded() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/report-2024.md", "x"),
        note("/report-2023.md", "y"),
    ]))
    .await
    .unwrap();

    // =report =2024 must match ONLY the file containing both, not either.
    let r = db.search("=report =2024").await.unwrap();
    assert_eq!(paths(&r), vec!["/report-2024.md".to_string()]);

    db.close().await;
}

#[tokio::test]
async fn link_search_follows_rename() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![note("/a.md", "see [[target]]")]))
        .await
        .unwrap();
    // Rename the linked-to note; links (destination + dest_name) must follow.
    db.rename_note(
        &VaultPath::note_path_from("/target.md"),
        &VaultPath::note_path_from("/renamed.md"),
        &[],
    )
    .await
    .unwrap();

    let r = db.search("<renamed").await.unwrap();
    assert_eq!(paths(&r), vec!["/a.md".to_string()]);

    // The old name no longer matches.
    let r = db.search("<target").await.unwrap();
    assert!(r.is_empty());

    db.close().await;
}

#[tokio::test]
async fn search_by_link_returns_linking_notes() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/index.md", "links [[projects]] and [[work/spec]]"),
        note("/b.md", "see [[projects]]"),
        note("/c.md", "no links here"),
    ]))
    .await
    .unwrap();

    // Notes that link to "projects" (backlinks).
    let r = db.search("<projects").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/b.md".to_string(), "/index.md".to_string()]
    );

    // Extension optional.
    let r = db.search("<projects.md").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/b.md".to_string(), "/index.md".to_string()]
    );

    // Bare name matches a note in a subfolder (name-anywhere).
    let r = db.search("<spec").await.unwrap();
    assert_eq!(paths(&r), vec!["/index.md".to_string()]);

    // Path-qualified match.
    let r = db.search("<work/spec").await.unwrap();
    assert_eq!(paths(&r), vec!["/index.md".to_string()]);

    // Wildcard.
    let r = db.search("<proj*").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/b.md".to_string(), "/index.md".to_string()]
    );

    // Exclusion: all notes that do NOT link to projects (index and b both link it).
    let r = db.search("-<projects").await.unwrap();
    assert_eq!(paths(&r), vec!["/c.md".to_string()]);

    // Unknown target → no results.
    let r = db.search("<nonexistent").await.unwrap();
    assert!(r.is_empty());

    db.close().await;
}

#[tokio::test]
async fn search_by_forward_link_returns_targets() {
    let (_tmp, db) = open_temp().await;
    // A links to B and C; B and C link nowhere; D links to A.
    db.apply(added(vec![
        note("/a.md", "see [[b]] and [[c]]"),
        note("/b.md", "b body"),
        note("/c.md", "c body"),
        note("/d.md", "points to [[a]]"),
    ]))
    .await
    .unwrap();

    // Forward links of A: the notes A links *to* (B and C).
    let r = db.search(">a").await.unwrap();
    assert_eq!(paths(&r), vec!["/b.md".to_string(), "/c.md".to_string()]);

    // Long form.
    let r = db.search("fwd:a").await.unwrap();
    assert_eq!(paths(&r), vec!["/b.md".to_string(), "/c.md".to_string()]);

    // Backlinks of B: the notes that link *to* B (A).
    let r = db.search("<b").await.unwrap();
    assert_eq!(paths(&r), vec!["/a.md".to_string()]);

    // Forward links of D: A.
    let r = db.search(">d").await.unwrap();
    assert_eq!(paths(&r), vec!["/a.md".to_string()]);

    // Exclusion: notes that are NOT forward links of A (everything but B and C).
    let r = db.search("->a").await.unwrap();
    assert_eq!(paths(&r), vec!["/a.md".to_string(), "/d.md".to_string()]);

    // A note with no outgoing links has no forward links.
    let r = db.search(">b").await.unwrap();
    assert!(r.is_empty());

    db.close().await;
}

#[tokio::test]
async fn fts_content_and_breadcrumb_combinations() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        // "meeting" under a "Work" heading, also says "done".
        note("/a.md", "# Work\nmeeting notes, all done"),
        // "meeting" but under "Personal", not "Work".
        note("/b.md", "# Personal\nmeeting with a friend"),
        // "Work" heading but no "meeting".
        note("/c.md", "# Work\nbudget review"),
    ]))
    .await
    .unwrap();

    // content AND breadcrumb (both must hold).
    let r = db.search("meeting @work").await.unwrap();
    assert_eq!(paths(&r), vec!["/a.md".to_string()]);

    // two content terms AND (only /a.md has both "meeting" and "notes").
    let r = db.search("meeting notes").await.unwrap();
    assert_eq!(paths(&r), vec!["/a.md".to_string()]);

    // content positive + content exclusion.
    let r = db.search("meeting -done").await.unwrap();
    assert_eq!(paths(&r), vec!["/b.md".to_string()]);

    // breadcrumb positive + content exclusion.
    let r = db.search("@work -budget").await.unwrap();
    assert_eq!(paths(&r), vec!["/a.md".to_string()]);

    // breadcrumb positive + breadcrumb exclusion.
    let r = db.search("@work -@personal").await.unwrap();
    assert_eq!(paths(&r), vec!["/a.md".to_string(), "/c.md".to_string()]);

    // pure content exclusion (no positives anywhere).
    let r = db.search("-meeting").await.unwrap();
    assert_eq!(paths(&r), vec!["/c.md".to_string()]);

    // pure breadcrumb exclusion.
    let r = db.search("-@work").await.unwrap();
    assert_eq!(paths(&r), vec!["/b.md".to_string()]);

    db.close().await;
}

#[tokio::test]
async fn search_by_label_returns_matching_notes() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/a.md", "a #important #todo"),
        note("/b.md", "b #todo"),
        note("/c.md", "c plain"),
    ]))
    .await
    .unwrap();

    let results = db.search("#important").await.unwrap();
    assert_eq!(paths(&results), vec!["/a.md".to_string()]);

    let results = db.search("#important #todo").await.unwrap();
    assert_eq!(paths(&results), vec!["/a.md".to_string()]);

    let results = db.search("#nope").await.unwrap();
    assert!(results.is_empty());

    db.close().await;
}

/// The `or:`/`-or:` directive is applied in Rust after the SQL — the one
/// piece of query logic behind the door that no SQL-shape test exercises.
/// Paths and titles are chosen so title order ≠ path order, and `apple`
/// is lowercase so a case-sensitive compare (which would put `Mango`
/// first) fails the test.
#[tokio::test]
async fn search_orders_results_by_the_order_directive() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/a.md", "# Zebra\nmeeting"),
        note("/b.md", "# apple\nmeeting"),
        note("/c.md", "# Mango\nmeeting"),
    ]))
    .await
    .unwrap();

    // Result order is the assertion here, so no sorting helper.
    let in_order = |rows: &[(NoteEntryData, NoteContentData)]| -> Vec<String> {
        rows.iter().map(|(e, _)| e.path.to_string()).collect()
    };

    let r = db.search("meeting or:title").await.unwrap();
    assert_eq!(
        in_order(&r),
        vec!["/b.md", "/c.md", "/a.md"],
        "title ascending, case-insensitive"
    );

    let r = db.search("meeting -or:title").await.unwrap();
    assert_eq!(
        in_order(&r),
        vec!["/a.md", "/c.md", "/b.md"],
        "title descending"
    );

    let r = db.search("meeting or:file").await.unwrap();
    assert_eq!(
        in_order(&r),
        vec!["/a.md", "/b.md", "/c.md"],
        "file name ascending"
    );

    db.close().await;
}

#[tokio::test]
async fn label_search_uses_index() {
    // Confirms the PK autoindex (sqlite_autoindex_labels_1) is used for
    // label lookups after the explicit labels_by_name index was dropped in
    // 0.7. A hashtag filter must not degrade to a full table scan.
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![note("/a.md", "x #important")]))
        .await
        .unwrap();

    let (sql, _) = super::build_search_sql_query("#important");
    let plan_sql = format!("EXPLAIN QUERY PLAN {}", sql);
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(&plan_sql)
        .bind("important")
        .fetch_all(db.pool())
        .await
        .unwrap();
    let plan_text = rows
        .iter()
        .map(|(_, _, _, detail)| detail.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    // The PK autoindex covers WHERE name = ? lookups on (name, path).
    // No explicit labels_by_name index any more (removed in 0.7).
    // Accept any sqlite_autoindex_labels_ suffix to tolerate DROP+CREATE migration changes.
    assert!(
        plan_text.contains("sqlite_autoindex_labels_"),
        "expected PK autoindex on labels in plan: {}",
        plan_text
    );

    db.close().await;
}

#[tokio::test]
async fn rename_note_updates_labels() {
    let (_tmp, db) = open_temp().await;

    let from = VaultPath::note_path_from("/old.md");
    let to = VaultPath::note_path_from("/new.md");
    db.apply(added(vec![note("/old.md", "x #foo")]))
        .await
        .unwrap();
    db.rename_note(&from, &to, &[]).await.unwrap();

    assert_eq!(
        sorted_paths(db.notes_with_label("foo").await.unwrap()),
        vec![to.to_string()],
        "label follows the rename and leaves nothing at the old path"
    );

    db.close().await;
}

#[tokio::test]
async fn rename_directory_renames_direct_children_note_rows() {
    let (_tmp, db) = open_temp().await;

    // One note directly in the renamed directory, one nested deeper.
    db.apply(added(vec![
        note("/old_dir/note.md", "content"),
        note("/old_dir/sub/deep.md", "content"),
    ]))
    .await
    .unwrap();
    db.rename_directory(&VaultPath::new("/old_dir"), &VaultPath::new("/new_dir"))
        .await
        .unwrap();

    assert_eq!(
        paths(&db.get_all_notes().await.unwrap()),
        vec!["/new_dir/note.md", "/new_dir/sub/deep.md"]
    );
    assert_eq!(
        paths(
            &db.get_notes(&VaultPath::new("/new_dir"), false)
                .await
                .unwrap()
        ),
        vec!["/new_dir/note.md"],
        "direct child re-based"
    );
    assert_eq!(
        paths(
            &db.get_notes(&VaultPath::new("/new_dir/sub"), false)
                .await
                .unwrap()
        ),
        vec!["/new_dir/sub/deep.md"],
        "nested child re-based"
    );

    db.close().await;
}

#[tokio::test]
async fn rename_directory_updates_labels() {
    let (_tmp, db) = open_temp().await;

    db.apply(added(vec![note("/old_dir/note.md", "x #moved")]))
        .await
        .unwrap();
    db.rename_directory(&VaultPath::new("/old_dir"), &VaultPath::new("/new_dir"))
        .await
        .unwrap();

    assert_eq!(
        sorted_paths(db.notes_with_label("moved").await.unwrap()),
        vec!["/new_dir/note.md"]
    );
    assert_eq!(
        db.label_counts().await.unwrap(),
        vec![("moved".to_string(), 1)]
    );

    db.close().await;
}

#[tokio::test]
async fn delete_directory_removes_labels() {
    let (_tmp, db) = open_temp().await;

    db.apply(added(vec![note("/sub/note.md", "x #gone")]))
        .await
        .unwrap();
    db.delete_directories(&[VaultPath::new("/sub")])
        .await
        .unwrap();

    assert!(db.label_counts().await.unwrap().is_empty());

    db.close().await;
}

#[tokio::test]
async fn delete_directory_with_underscore_does_not_touch_siblings() {
    let (_tmp, db) = open_temp().await;

    let sibling = VaultPath::note_path_from("/myXdir/b.md");
    db.apply(added(vec![
        note("/my_dir/a.md", "x #t"),
        note("/myXdir/b.md", "y #s"),
    ]))
    .await
    .unwrap();
    db.delete_directories(&[VaultPath::new("/my_dir")])
        .await
        .unwrap();

    assert_eq!(
        paths(&db.get_all_notes().await.unwrap()),
        vec![sibling.to_string()],
        "sibling /myXdir/b.md must be untouched"
    );
    assert_eq!(
        sorted_paths(db.notes_with_label("s").await.unwrap()),
        vec![sibling.to_string()],
        "sibling label preserved"
    );
    assert!(
        db.notes_with_label("t").await.unwrap().is_empty(),
        "the deleted directory's label is gone"
    );

    db.close().await;
}

#[test]
fn escape_like_pattern_escapes_metacharacters() {
    assert_eq!(super::escape_like_pattern("/my_dir/"), "/my\\_dir/");
    assert_eq!(super::escape_like_pattern("/a%b/"), "/a\\%b/");
    assert_eq!(super::escape_like_pattern("/a\\b/"), "/a\\\\b/");
    assert_eq!(super::escape_like_pattern("/normal/"), "/normal/");
}

/// Verify that `escape_like_pattern` leaves `*` and `.` untouched — a
/// prerequisite for the escape-then-replace order in the wildcard branch.
#[test]
fn escape_like_pattern_leaves_star_and_dot_untouched() {
    assert_eq!(super::escape_like_pattern("task*"), "task*");
    assert_eq!(super::escape_like_pattern("task*.md"), "task*.md");
    assert_eq!(super::escape_like_pattern("*report.md"), "*report.md");
}

/// SQL-shape unit test: confirm the bound parameter produced for `=task*`
/// is `task%.md` and for plain `=task` is `%task%`.
#[test]
fn filename_wildcard_produces_correct_pattern_param() {
    // Wildcard term: =task*  → param should be "task%.md"
    let (_, params) = build_search_sql_query("=task*");
    assert_eq!(
        params,
        vec!["task%.md".to_string()],
        "=task* must produce bound param 'task%.md'"
    );

    // Non-wildcard term: =task  → param should be "%task%"
    let (_, params) = build_search_sql_query("=task");
    assert_eq!(
        params,
        vec!["%task%".to_string()],
        "=task must produce bound param '%task%'"
    );

    // Suffix wildcard: =*report → param should be "%report.md"
    let (_, params) = build_search_sql_query("=*report");
    assert_eq!(
        params,
        vec!["%report.md".to_string()],
        "=*report must produce bound param '%report.md'"
    );

    // Mid wildcard: =ta*sk → param should be "ta%sk.md"
    let (_, params) = build_search_sql_query("=ta*sk");
    assert_eq!(
        params,
        vec!["ta%sk.md".to_string()],
        "=ta*sk must produce bound param 'ta%sk.md'"
    );
}

#[tokio::test]
async fn search_by_filename_wildcard() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/task.md", "x"),
        note("/tasks.md", "y"),
        note("/weekly-report.md", "z"),
        note("/other.md", "w"),
    ]))
    .await
    .unwrap();

    // Substring (non-wildcard): =task → task.md and tasks.md
    let r = db.search("=task").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/task.md".to_string(), "/tasks.md".to_string()],
        "=task must match task.md and tasks.md as substrings"
    );

    // Prefix wildcard: =task* → task.md and tasks.md, NOT weekly-report.md
    let r = db.search("=task*").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/task.md".to_string(), "/tasks.md".to_string()],
        "=task* must match task.md and tasks.md, not weekly-report.md"
    );

    // Suffix wildcard: =*report → weekly-report.md only
    let r = db.search("=*report").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/weekly-report.md".to_string()],
        "=*report must match only weekly-report.md"
    );

    // Exclusion with wildcard: -=task* → other.md and weekly-report.md
    let r = db.search("-=task*").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/other.md".to_string(), "/weekly-report.md".to_string()],
        "-=task* must exclude task.md and tasks.md"
    );

    db.close().await;
}

#[tokio::test]
async fn search_by_path_wildcard() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/work/a.md", "a"),
        note("/work/sub/b.md", "b"),
        note("/personal/c.md", "c"),
        note("/d.md", "d"),
    ]))
    .await
    .unwrap();

    // Prefix (non-wildcard) is unchanged: /work matches the folder + subfolders.
    let r = db.search("/work").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/work/a.md".to_string(), "/work/sub/b.md".to_string()],
    );

    // Wildcard prefix: /wo* behaves like the prefix form.
    let r = db.search("/wo*").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/work/a.md".to_string(), "/work/sub/b.md".to_string()],
    );

    // Suffix wildcard on the folder path: /*sub → only notes whose folder ends in "sub".
    let r = db.search("/*sub").await.unwrap();
    assert_eq!(paths(&r), vec!["/work/sub/b.md".to_string()]);

    // Subfolder wildcard: /work/* → only notes strictly under /work/.
    let r = db.search("/work/*").await.unwrap();
    assert_eq!(paths(&r), vec!["/work/sub/b.md".to_string()]);

    // Excluded wildcard: -/wo* drops everything under /work.
    let r = db.search("-/wo*").await.unwrap();
    assert_eq!(
        paths(&r),
        vec!["/d.md".to_string(), "/personal/c.md".to_string()],
    );

    db.close().await;
}

#[tokio::test]
async fn delete_directory_no_trailing_slash_does_not_match_sibling_prefix() {
    let (_tmp, db) = open_temp().await;

    let sibling = VaultPath::note_path_from("/notes_archive/b.md");
    db.apply(added(vec![
        note("/notes/a.md", "x"),
        note("/notes_archive/b.md", "y"),
    ]))
    .await
    .unwrap();
    db.delete_directories(&[VaultPath::new("/notes")])
        .await
        .unwrap();

    assert_eq!(
        paths(&db.get_all_notes().await.unwrap()),
        vec![sibling.to_string()],
        "sibling /notes_archive/ must not be deleted"
    );
    db.close().await;
}

#[tokio::test]
async fn recursive_listing_does_not_match_sibling_directory_prefixes() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/foo/a.md", "# A\nbody"),
        note("/foobar/b.md", "# B\nbody"),
    ]))
    .await
    .unwrap();

    let under_foo = db.get_notes(&VaultPath::new("/foo"), true).await.unwrap();
    assert_eq!(
        paths(&under_foo),
        vec!["/foo/a.md"],
        "/foobar is a sibling, not a child"
    );

    let mut keys: Vec<String> = db
        .get_notes_sections(&VaultPath::new("/foo"), true)
        .await
        .unwrap()
        .into_keys()
        .map(|p| p.to_string())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["/foo/a.md"]);

    let everything = db.get_notes(&VaultPath::root(), true).await.unwrap();
    assert_eq!(
        paths(&everything),
        vec!["/foo/a.md", "/foobar/b.md"],
        "the root still lists all"
    );

    db.close().await;
}

#[tokio::test]
async fn path_search_with_underscore_does_not_treat_as_wildcard() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/my_notes/a.md", "x"),
        note("/myXnotes/b.md", "y"),
    ]))
    .await
    .unwrap();

    // pt:my_notes search must only match /my_notes/, not /myXnotes/.
    let results = db.search("pt:my_notes").await.unwrap();
    assert_eq!(
        paths(&results),
        vec!["/my_notes/a.md".to_string()],
        "underscore must be literal in path search"
    );
    db.close().await;
}

#[tokio::test]
async fn filename_search_with_underscore_does_not_treat_as_wildcard() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![
        note("/my_note.md", "x"),
        note("/myXnote.md", "y"),
    ]))
    .await
    .unwrap();

    let results = db.search("=my_note").await.unwrap();
    assert_eq!(
        paths(&results),
        vec!["/my_note.md".to_string()],
        "underscore must be literal in filename search"
    );
    db.close().await;
}

#[tokio::test]
async fn fts_term_with_metachar_does_not_error() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![note("/a.md", "some meeting note")]))
        .await
        .unwrap();

    // Each of these would have produced an FTS4 syntax error before the fix.
    for q in &[
        "(meeting",
        "*",
        "meet*ing",
        "title:value",
        "a^b",
        "<",
        ">",
        "=",
        "@",
        "-",
        "-<",
        "->",
        "in:",
        "name:",
    ] {
        let res = db.search(q).await;
        assert!(
            res.is_ok(),
            "query {:?} must not error; got {:?}",
            q,
            res.err()
        );
    }

    db.close().await;
}

#[tokio::test]
async fn breadcrumb_term_with_metachar_does_not_error() {
    let (_tmp, db) = open_temp().await;
    db.apply(added(vec![note("/a.md", "# Heading\n\ntext")]))
        .await
        .unwrap();

    for q in &["@(heading", "@*", "in:title:", ">(heading", ">*"] {
        let res = db.search(q).await;
        assert!(
            res.is_ok(),
            "breadcrumb query {:?} must not error; got {:?}",
            q,
            res.err()
        );
    }

    db.close().await;
}

#[cfg(test)]
mod note_columns_consistency {
    #[test]
    fn note_columns_is_path_plus_rest() {
        assert_eq!(
            super::super::NOTE_COLUMNS,
            format!("path, {}", super::super::NOTE_COLUMNS_REST),
            "NOTE_COLUMNS must equal 'path, ' + NOTE_COLUMNS_REST"
        );
    }
}

/// On a stored DB version older than the current `VERSION`, reopening the
/// vault must self-heal the schema: the index comes back valid
/// but empty, `index_ready` reports `false`, and the next sync pass
/// (`validate_and_init`) refills it. After the heal, stale `>`-separated
/// breadcrumb rows are gone and the new `\x1f` separator is in place.
#[tokio::test(flavor = "multi_thread")]
async fn reopen_self_heals_outdated_schema() {
    use crate::{NoteVault, VaultConfig};
    use sqlx::Row;

    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("note.md"), "# Note\n## Sub\nbody text").unwrap();

    // Bring the index up at the current version with one indexed note.
    {
        let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        // A brand-new index is healed-on-open, hence not ready; reopening
        // it below (current version) must report ready.

        // Force the schema backwards: stamp version `0.4` and rewrite
        // stored breadcrumbs in the legacy `>`-joined form to simulate a
        // vault upgraded across the separator change.
        let pool = vault.index.pool();
        sqlx::query("UPDATE appData SET value = '0.4' WHERE name = 'version'")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE notesContent SET breadcrumb = REPLACE(breadcrumb, x'1f', '>')")
            .execute(pool)
            .await
            .unwrap();

        // Sanity: the stale row really does contain `>`.
        let stale: Vec<String> =
            sqlx::query("SELECT breadcrumb FROM notesContent WHERE breadcrumb != ''")
                .fetch_all(pool)
                .await
                .unwrap()
                .into_iter()
                .map(|r| r.try_get("breadcrumb").unwrap())
                .collect();
        assert!(
            stale.iter().any(|b| b.contains('>')),
            "expected legacy `>` separator in: {:?}",
            stale
        );
        vault.index.close().await;
    }

    // Reopen: the outdated schema is healed silently; the probe reports
    // not-ready until a sync pass fills the empty index.
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    assert!(!vault.index_ready(), "healed index must not report ready");
    vault.validate_and_init().await.unwrap();
    // The sync pass marks the index synced: the SAME instance now
    // reports ready (regression: the old write-once flag kept lying).
    assert!(
        vault.index_ready(),
        "synced index must report ready on the same instance"
    );

    // Post-heal: no row carries the legacy separator; non-empty
    // breadcrumbs use `\x1f`.
    let pool = vault.index.pool();
    let after: Vec<String> =
        sqlx::query("SELECT breadcrumb FROM notesContent WHERE breadcrumb != ''")
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.try_get("breadcrumb").unwrap())
            .collect();
    assert!(
        !after.is_empty(),
        "expected reindexed breadcrumb rows after heal"
    );
    assert!(
        after.iter().all(|b| !b.contains('>')),
        "stale `>` separator survived the heal: {:?}",
        after
    );

    // End-to-end: the public chunk accessor exposes sane breadcrumb
    // leaves after the heal (storage-level separator checks alone would
    // miss an accessor-level splitting bug).
    let chunks = vault
        .get_note_chunks(&crate::nfs::VaultPath::new("/note.md"))
        .await
        .unwrap();
    let leaves: Vec<&str> = chunks
        .values()
        .flatten()
        .filter_map(|c| c.breadcrumb_last())
        .collect();
    assert!(
        leaves.iter().any(|l| *l == "Note" || *l == "Sub"),
        "expected Note/Sub breadcrumb leaves, got: {:?}",
        leaves
    );

    // A second reopen with a current schema must report ready.
    vault.index.close().await;
    drop(vault);
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    assert!(vault.index_ready(), "current schema must report ready");

    // recreate_index drops the tables and runs a full sync; the probe
    // must still report ready on the same instance afterwards.
    vault.recreate_index().await.unwrap();
    assert!(
        vault.index_ready(),
        "recreated-and-synced index must report ready"
    );
}

/// `open` on a current-version schema must not heal: `ready` is `true`
/// and existing rows survive.
#[tokio::test]
async fn open_preserves_current_schema() {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("kimun.sqlite");

    // First open heals the fresh file into a current schema.
    let first = super::NoteIndex::open(&file::IndexFile::at(crate::system::sys(&db_path)))
        .await
        .unwrap();
    assert!(!first.ready());
    sqlx::query("INSERT INTO appData (name, value) VALUES ('marker', 'kept')")
        .execute(first.pool())
        .await
        .unwrap();
    first.close().await;

    // Second open sees a current schema: no heal, data intact.
    let second = super::NoteIndex::open(&file::IndexFile::at(crate::system::sys(&db_path)))
        .await
        .unwrap();
    assert!(second.ready());
    let marker: Option<String> =
        sqlx::query_scalar("SELECT value FROM appData WHERE name = 'marker'")
            .fetch_optional(second.pool())
            .await
            .unwrap();
    assert_eq!(marker.as_deref(), Some("kept"));
    second.close().await;
}

/// A recursive browse from the root is a whole-vault sync and must mark
/// the index synced — the readiness probe reports true afterwards even
/// though the schema was healed at open (regression for the
/// browse-only path that previously left the probe stuck on false).
#[tokio::test(flavor = "multi_thread")]
async fn whole_vault_browse_marks_index_ready() {
    use crate::{NoteVault, VaultBrowseOptionsBuilder, VaultConfig};

    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("note.md"), "# Note\nbody").unwrap();

    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    assert!(!vault.index_ready(), "fresh index is healed, not ready");

    let (options, rx) = VaultBrowseOptionsBuilder::new(&crate::nfs::VaultPath::root())
        .recursive(true)
        .build();
    vault.browse_vault(options).await.unwrap();
    drop(rx);

    assert!(
        vault.index_ready(),
        "recursive root browse is a whole-vault sync — probe must report ready"
    );
}
