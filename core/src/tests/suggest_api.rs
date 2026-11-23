// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// lib.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and these TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::nfs::VaultPath;
use crate::*;

async fn new_vault() -> (tempfile::TempDir, NoteVault) {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = VaultConfig::new(crate::system::sys(tmp.path()));
    let vault = NoteVault::new(cfg).await.unwrap();
    vault.validate_and_init().await.unwrap();
    (tmp, vault)
}

// Note: vault paths are stored lowercased (see VaultPathSlice::new), so
// these tests assert on the lowercase form. The `name` field strips the
// note extension via VaultPath::get_clean_name (no `.md` suffix).

#[tokio::test]
async fn suggest_notes_empty_prefix_returns_top_n() {
    let (_tmp, vault) = new_vault().await;
    for name in ["Alpha", "Beta", "Gamma"] {
        vault
            .create_note(&VaultPath::note_path_from(format!("/{name}.md")), "body")
            .await
            .unwrap();
    }

    let mut got = vault.suggest_notes_by_prefix("", 50).await.unwrap();
    got.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<String> = got.into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
}

#[tokio::test]
async fn suggest_notes_prefix_is_case_insensitive() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/Meeting.md"), "x")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/melon.md"), "x")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/zebra.md"), "x")
        .await
        .unwrap();

    let got = vault.suggest_notes_by_prefix("ME", 50).await.unwrap();
    let names: std::collections::HashSet<String> = got.into_iter().map(|s| s.name).collect();
    assert!(names.contains("meeting"));
    assert!(names.contains("melon"));
    assert!(!names.contains("zebra"));
}

#[tokio::test]
async fn suggest_notes_respects_limit() {
    let (_tmp, vault) = new_vault().await;
    for i in 0..10 {
        vault
            .create_note(&VaultPath::note_path_from(format!("/note{i}.md")), "x")
            .await
            .unwrap();
    }
    let got = vault.suggest_notes_by_prefix("note", 3).await.unwrap();
    assert_eq!(got.len(), 3);
}

#[tokio::test]
async fn suggest_notes_keeps_same_name_at_different_paths_separate() {
    let (_tmp, vault) = new_vault().await;
    vault.create_directory(&VaultPath::new("/a")).await.unwrap();
    vault.create_directory(&VaultPath::new("/b")).await.unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/a/Shared.md"), "x")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/b/Shared.md"), "y")
        .await
        .unwrap();

    let got = vault.suggest_notes_by_prefix("Shared", 50).await.unwrap();
    assert_eq!(got.len(), 2, "duplicates by name must not be deduped");
    let mut paths: Vec<String> = got.iter().map(|s| s.path.to_string()).collect();
    paths.sort();
    assert!(paths[0].contains("/a/"));
    assert!(paths[1].contains("/b/"));
    assert!(got.iter().all(|s| s.name == "shared"));
}

#[tokio::test]
async fn suggest_notes_empty_vault_returns_empty() {
    let (_tmp, vault) = new_vault().await;
    let got = vault.suggest_notes_by_prefix("anything", 50).await.unwrap();
    assert!(got.is_empty());
}

#[tokio::test]
async fn suggest_notes_unicode_and_long_prefix_do_not_panic() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/over.md"), "x")
        .await
        .unwrap();
    let long = "a".repeat(4096);
    let _ = vault.suggest_notes_by_prefix(&long, 50).await.unwrap();
    let _ = vault.suggest_notes_by_prefix("Über", 50).await.unwrap();
}

#[tokio::test]
async fn suggest_notes_special_like_chars_in_prefix_are_escaped() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/normal.md"), "x")
        .await
        .unwrap();
    // `%` and `_` are LIKE wildcards — escaping must prevent them matching
    // unrelated notes.
    let got = vault.suggest_notes_by_prefix("%", 50).await.unwrap();
    assert!(got.is_empty());
    let got = vault.suggest_notes_by_prefix("_", 50).await.unwrap();
    assert!(got.is_empty());
}

#[tokio::test]
async fn suggest_tags_ranks_by_usage_count_then_name() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/a.md"), "x #foo #bar")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/b.md"), "y #foo")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/c.md"), "z #foo #baz")
        .await
        .unwrap();

    // Prefix "" returns everything, ordered by usage_count desc, name asc.
    let got = vault.suggest_tags_by_prefix("", 50).await.unwrap();
    assert_eq!(got[0].label, "foo");
    assert_eq!(got[0].usage_count, 3);
    // bar and baz both have count 1; alphabetical tie-break puts bar first.
    let labels: Vec<&str> = got.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, vec!["foo", "bar", "baz"]);
}

#[tokio::test]
async fn suggest_tags_prefix_is_case_insensitive() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/a.md"), "x #Projects")
        .await
        .unwrap();
    let got = vault.suggest_tags_by_prefix("PRO", 50).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].label, "projects");
}

#[tokio::test]
async fn suggest_tags_respects_limit() {
    let (_tmp, vault) = new_vault().await;
    for i in 0..5 {
        vault
            .create_note(
                &VaultPath::note_path_from(format!("/n{i}.md")),
                format!("x #tag{i}"),
            )
            .await
            .unwrap();
    }
    let got = vault.suggest_tags_by_prefix("tag", 2).await.unwrap();
    assert_eq!(got.len(), 2);
}

#[tokio::test]
async fn suggest_tags_empty_vault_returns_empty() {
    let (_tmp, vault) = new_vault().await;
    let got = vault.suggest_tags_by_prefix("", 50).await.unwrap();
    assert!(got.is_empty());
}
