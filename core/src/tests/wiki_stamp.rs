// Gated with the inner attribute: TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::error::VaultError;
use crate::nfs::wiki_declaration::write_wiki_declaration;
use crate::nfs::VaultPath;
use crate::note::PropertyValue;
use crate::wiki::body_hash;
use crate::{NoteVault, VaultConfig, WikiDeclaration};
use tempfile::TempDir;

/// A vault with backups on, a declaration (unless `declared` is false) and
/// `/sources/a.md` holding `text`.
async fn vault_with(declared: bool, protect: bool, text: &str) -> (TempDir, NoteVault, VaultPath) {
    let dir = TempDir::new().unwrap();
    let ws = crate::system::sys(dir.path());
    if declared {
        let declaration =
            WikiDeclaration::with_folders(&VaultPath::new("/sources"), &VaultPath::new("/wiki"));
        write_wiki_declaration(&ws, &declaration).await.unwrap();
    }
    let vault = NoteVault::new(
        VaultConfig::new(ws)
            .with_backup(true)
            .with_protected_sources(protect),
    )
    .await
    .unwrap();
    vault.validate_and_init().await.unwrap();
    let source = VaultPath::new("/sources/a.md");
    vault.create_note(&source, text).await.unwrap();
    (dir, vault, source)
}

fn backup_files(dir: &TempDir) -> usize {
    let root = dir.path().join(".kimun").join("backups");
    let mut count = 0;
    let mut stack = vec![root];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in read.flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                count += 1;
            }
        }
    }
    count
}

#[tokio::test]
async fn stamp_writes_both_properties() {
    let (_d, vault, source) = vault_with(true, false, "body text\n").await;
    let report = vault.wiki_stamp().await.unwrap();
    assert_eq!(report.stamped, vec![source.clone()]);
    let hash = vault.get_property(&source, "content_hash").await.unwrap();
    assert_eq!(
        hash,
        Some(Some(PropertyValue::Text(body_hash("body text\n"))))
    );
    let ingested = vault.get_property(&source, "ingested_at").await.unwrap();
    assert!(matches!(ingested, Some(Some(PropertyValue::Date(_)))));
}

#[tokio::test]
async fn stamp_keeps_an_existing_ingested_at() {
    let (_d, vault, source) =
        vault_with(true, false, "---\ningested_at: 2020-01-02\n---\nbody\n").await;
    vault.wiki_stamp().await.unwrap();
    let ingested = vault.get_property(&source, "ingested_at").await.unwrap();
    let expected = chrono::NaiveDate::from_ymd_opt(2020, 1, 2).unwrap();
    assert_eq!(ingested, Some(Some(PropertyValue::Date(expected))));
    assert!(vault
        .get_property(&source, "content_hash")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn stamp_twice_stamps_nothing_the_second_time() {
    let (dir, vault, _source) = vault_with(true, false, "body\n").await;
    assert_eq!(vault.wiki_stamp().await.unwrap().stamped.len(), 1);
    let backups = backup_files(&dir);
    assert!(backups >= 1);
    assert!(vault.wiki_stamp().await.unwrap().stamped.is_empty());
    assert_eq!(backup_files(&dir), backups);
}

#[test]
fn body_hash_ignores_frontmatter_changes() {
    let plain = body_hash("body\n");
    assert_eq!(plain.len(), 16);
    assert_eq!(plain, body_hash("---\na: 1\n---\nbody\n"));
    assert_eq!(plain, body_hash("---\nb: other\n---\nbody\n"));
    assert_ne!(plain, body_hash("---\na: 1\n---\nbody changed\n"));
}

#[test]
fn body_hash_is_the_same_for_lf_and_crlf() {
    assert_eq!(
        body_hash("---\na: 1\n---\nline one\nline two\n"),
        body_hash("---\r\na: 1\r\n---\r\nline one\r\nline two\r\n")
    );
}

#[tokio::test]
async fn stamp_works_on_a_protected_vault() {
    let (_d, vault, source) = vault_with(true, true, "body\n").await;
    let report = vault.wiki_stamp().await.unwrap();
    assert_eq!(report.stamped, vec![source.clone()]);
    assert!(vault
        .get_property(&source, "content_hash")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn stamp_without_a_declaration_is_wiki_not_declared() {
    let (_d, vault, _source) = vault_with(false, false, "body\n").await;
    let err = vault.wiki_stamp().await.unwrap_err();
    assert!(matches!(err, VaultError::WikiNotDeclared));
    assert_eq!(
        err.user_message().unwrap(),
        "This vault has no wiki declaration. Run kimun wiki init."
    );
}

/// Writes `bytes` as `/sources/a.md` straight to disk, then stamps.
async fn stamp_raw(bytes: &str) -> (TempDir, NoteVault, VaultPath) {
    let (dir, vault, source) = vault_with(true, false, "x").await;
    std::fs::write(dir.path().join("sources").join("a.md"), bytes).unwrap();
    vault.wiki_stamp().await.unwrap();
    (dir, vault, source)
}

#[tokio::test]
async fn stamp_matches_body_hash_of_the_file_as_written() {
    let cases = [
        ("lf with frontmatter", "---\ntitle: t\n---\nbody\nmore\n"),
        ("no frontmatter", "body\nmore\n"),
        ("crlf on disk", "---\r\ntitle: t\r\n---\r\nbody\r\nmore\r\n"),
        ("bom no frontmatter", "\u{feff}body\nmore\n"),
        ("bom with frontmatter", "\u{feff}---\ntitle: t\n---\nbody\n"),
    ];
    for (name, bytes) in cases {
        let (_d, vault, source) = stamp_raw(bytes).await;
        let text = vault.get_note_text(&source).await.unwrap();
        let hash = vault.get_property(&source, "content_hash").await.unwrap();
        assert_eq!(
            hash,
            Some(Some(PropertyValue::Text(body_hash(&text)))),
            "{name}"
        );
    }
}

#[tokio::test]
async fn stamp_skips_a_source_with_malformed_frontmatter_and_leaves_the_file() {
    let bad = "---\nkey: [unclosed\n---\nbody\n";
    let (dir, vault, source) = vault_with(true, false, "x").await;
    let file = dir.path().join("sources").join("a.md");
    std::fs::write(&file, bad).unwrap();
    let report = vault.wiki_stamp().await.unwrap();
    assert!(report.stamped.is_empty());
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].path, source);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), bad);
}

#[tokio::test]
async fn stamp_goes_on_past_a_broken_source_and_lists_it() {
    let (dir, vault, first) = vault_with(true, false, "first body\n").await;
    let middle = VaultPath::new("/sources/b.md");
    let last = VaultPath::new("/sources/c.md");
    vault.create_note(&last, "last body\n").await.unwrap();
    let bad = "---\nkey: [unclosed\n---\nmiddle body\n";
    let middle_file = dir.path().join("sources").join("b.md");
    std::fs::write(&middle_file, bad).unwrap();

    let report = vault.wiki_stamp().await.unwrap();
    assert_eq!(report.stamped, vec![first.clone(), last.clone()]);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].path, middle);
    assert!(!report.skipped[0].reason.is_empty());
    assert_eq!(std::fs::read_to_string(&middle_file).unwrap(), bad);

    let again = vault.wiki_stamp().await.unwrap();
    assert!(again.stamped.is_empty());
    assert_eq!(again.skipped.len(), 1);
    assert_eq!(again.skipped[0].path, middle);
    assert_eq!(std::fs::read_to_string(&middle_file).unwrap(), bad);
}

#[tokio::test]
async fn stamp_writes_the_day_it_is_given() {
    let (_d, vault, source) = vault_with(true, false, "body\n").await;
    let day = chrono::NaiveDate::from_ymd_opt(2031, 7, 4).unwrap();
    vault.wiki_stamp_on(day).await.unwrap();
    let ingested = vault.get_property(&source, "ingested_at").await.unwrap();
    assert_eq!(ingested, Some(Some(PropertyValue::Date(day))));
}

#[tokio::test]
async fn stamp_uses_the_local_day() {
    let (_d, vault, source) = vault_with(true, false, "body\n").await;
    let before = chrono::Local::now().date_naive();
    vault.wiki_stamp().await.unwrap();
    let after = chrono::Local::now().date_naive();
    let Some(Some(PropertyValue::Date(day))) =
        vault.get_property(&source, "ingested_at").await.unwrap()
    else {
        panic!("no ingested_at date");
    };
    assert!(day == before || day == after, "{day} is not the local day");
}
