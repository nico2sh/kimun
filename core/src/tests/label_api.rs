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

#[tokio::test]
async fn list_labels_returns_distinct_lowercase_names() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/a.md"), "x #Foo and #bar")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::note_path_from("/b.md"), "y #foo only")
        .await
        .unwrap();

    let mut labels = vault.list_labels().await.unwrap();
    labels.sort();
    assert_eq!(labels, vec!["bar".to_string(), "foo".to_string()]);
}

#[tokio::test]
async fn notes_with_label_is_case_insensitive() {
    let (_tmp, vault) = new_vault().await;
    let a = VaultPath::note_path_from("/a.md");
    let b = VaultPath::note_path_from("/b.md");
    vault.create_note(&a, "x #Important").await.unwrap();
    vault.create_note(&b, "x #important #other").await.unwrap();

    let mut paths = vault.notes_with_label("IMPORTANT").await.unwrap();
    paths.sort_by_key(|p| p.to_string());
    assert_eq!(paths, vec![a, b]);
}

#[tokio::test]
async fn notes_with_unknown_label_returns_empty() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&VaultPath::note_path_from("/a.md"), "x")
        .await
        .unwrap();
    let paths = vault.notes_with_label("nosuch").await.unwrap();
    assert!(paths.is_empty());
}

#[tokio::test]
async fn label_counts_returns_count_per_label() {
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
        .create_note(&VaultPath::note_path_from("/c.md"), "z #baz")
        .await
        .unwrap();

    let counts = vault.label_counts().await.unwrap();
    assert_eq!(
        counts,
        vec![
            ("bar".to_string(), 1usize),
            ("baz".to_string(), 1usize),
            ("foo".to_string(), 2usize),
        ],
    );
}

#[tokio::test]
async fn label_counts_empty_vault_returns_empty() {
    let (_tmp, vault) = new_vault().await;
    let counts = vault.label_counts().await.unwrap();
    assert!(counts.is_empty());
}
