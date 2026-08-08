// tui/tests/labels_command_test.rs
//
// Integration tests that exercise the labels command end-to-end against a
// temporary vault. Mirrors the style of other tui/tests/ files.

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

mod common;
use common::open_test_vault;

/// Returns the `TempDir` alongside the vault so it outlives it; every test
/// closes the vault before both drop (see `common::open_test_vault`).
async fn setup_vault() -> (tempfile::TempDir, NoteVault) {
    let tmp = tempfile::TempDir::new().unwrap();
    let vault = open_test_vault(tmp.path()).await;
    (tmp, vault)
}

#[tokio::test]
async fn labels_command_lists_each_label_with_count() {
    let (_tmp, vault) = setup_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/a.md"), "#foo and #bar")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/b.md"), "#foo only")
        .await
        .unwrap();

    let counts = vault.label_counts().await.unwrap();
    let names: Vec<&str> = counts.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["bar", "foo"]);
    let foo_count = counts.iter().find(|(n, _)| n == "foo").unwrap().1;
    assert_eq!(foo_count, 2);
    vault.close().await;
}

#[tokio::test]
async fn labels_command_empty_vault_returns_empty() {
    let (_tmp, vault) = setup_vault().await;
    let counts = vault.label_counts().await.unwrap();
    assert!(counts.is_empty());
    vault.close().await;
}

#[tokio::test]
async fn labels_command_single_label_shows_correct_count() {
    let (_tmp, vault) = setup_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/a.md"), "#rust intro")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/b.md"), "#rust advanced")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/c.md"), "#rust systems")
        .await
        .unwrap();

    let counts = vault.label_counts().await.unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0].0, "rust");
    assert_eq!(counts[0].1, 3);
    vault.close().await;
}

#[tokio::test]
async fn labels_command_sorted_alphabetically() {
    let (_tmp, vault) = setup_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/a.md"), "#zebra #apple #mango")
        .await
        .unwrap();

    let counts = vault.label_counts().await.unwrap();
    let names: Vec<&str> = counts.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["apple", "mango", "zebra"]);
    vault.close().await;
}
