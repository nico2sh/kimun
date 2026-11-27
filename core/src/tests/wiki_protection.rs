// Gated with the inner attribute: TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::error::VaultError;
use crate::nfs::wiki_declaration::write_wiki_declaration;
use crate::nfs::VaultPath;
use crate::note::{FrontmatterFormat, PropertyValue};
use crate::{NoteVault, VaultConfig, WikiDeclaration};
use tempfile::TempDir;

/// A vault with the given protection flag and, when `sources` is set, a
/// declaration for that sources folder and `/wiki`. Holds `<sources>/a.md`.
async fn vault_with(protect: bool, sources: Option<&str>) -> (TempDir, NoteVault, VaultPath) {
    let dir = TempDir::new().unwrap();
    let ws = crate::system::sys(dir.path());
    let source_dir = VaultPath::new(sources.unwrap_or("/sources"));
    if sources.is_some() {
        let declaration = WikiDeclaration::with_folders(&source_dir, &VaultPath::new("/wiki"));
        write_wiki_declaration(&ws, &declaration).await.unwrap();
    }
    let vault = NoteVault::new(VaultConfig::new(ws).with_protected_sources(protect))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    let source = source_dir.append(&VaultPath::note_path_from("a"));
    vault.create_note(&source, "source text").await.unwrap();
    (dir, vault, source)
}

/// The default test vault: protection on, sources at `/sources`.
async fn protected() -> (TempDir, NoteVault, VaultPath) {
    vault_with(true, Some("/sources")).await
}

fn assert_protected<T: std::fmt::Debug>(result: Result<T, VaultError>, expected: &VaultPath) {
    match result {
        Err(VaultError::SourceProtected { path }) => assert_eq!(&path, expected),
        other => panic!("expected SourceProtected for {expected}, got {other:?}"),
    }
}

#[tokio::test]
async fn save_note_on_an_existing_source_is_refused() {
    let (_d, vault, source) = protected().await;
    assert_protected(vault.save_note(&source, "changed").await, &source);
    assert_eq!(vault.get_note_text(&source).await.unwrap(), "source text");
}

#[tokio::test]
async fn refusal_message_names_the_rule_and_the_file() {
    let (_d, vault, source) = protected().await;
    let err = vault.save_note(&source, "changed").await.unwrap_err();
    assert_eq!(
        err.user_message().unwrap(),
        "/sources/a.md is in the sources folder that .kimun/wiki.toml protects. \
         Automated edits cannot change it."
    );
}

#[tokio::test]
async fn create_note_in_sources_is_allowed() {
    let (_d, vault, _source) = protected().await;
    let path = VaultPath::new("/sources/b.md");
    vault.create_note(&path, "new").await.unwrap();
    assert_eq!(vault.get_note_text(&path).await.unwrap(), "new");
}

#[tokio::test]
async fn append_to_note_on_an_existing_source_is_refused() {
    let (_d, vault, source) = protected().await;
    assert_protected(vault.append_to_note(&source, "more", None).await, &source);
    assert_eq!(vault.get_note_text(&source).await.unwrap(), "source text");
}

#[tokio::test]
async fn append_to_note_creates_a_new_source_when_absent() {
    let (_d, vault, _source) = protected().await;
    let path = VaultPath::new("/sources/new.md");
    vault.append_to_note(&path, "first", None).await.unwrap();
    assert_eq!(vault.get_note_text(&path).await.unwrap(), "first");
}

#[tokio::test]
async fn replace_in_note_on_a_source_is_refused() {
    let (_d, vault, source) = protected().await;
    assert_protected(
        vault
            .replace_in_note(&source, "source", "other", false, false)
            .await,
        &source,
    );
    assert_eq!(vault.get_note_text(&source).await.unwrap(), "source text");
}

#[tokio::test]
async fn set_property_on_a_source_is_refused() {
    let (_d, vault, source) = protected().await;
    assert_protected(
        vault
            .set_property(
                &source,
                "status",
                PropertyValue::Text("x".into()),
                FrontmatterFormat::default(),
            )
            .await,
        &source,
    );
    assert_eq!(vault.get_note_text(&source).await.unwrap(), "source text");
}

#[tokio::test]
async fn delete_note_on_a_source_is_refused() {
    let (_d, vault, source) = protected().await;
    assert_protected(vault.delete_note(&source).await, &source);
    assert!(vault.exists(&source).await);
}

#[tokio::test]
async fn rename_note_on_a_source_is_refused() {
    let (_d, vault, source) = protected().await;
    let to = VaultPath::new("/elsewhere.md");
    assert_protected(vault.rename_note(&source, &to).await, &source);
    assert!(vault.exists(&source).await);
    assert!(!vault.exists(&to).await);
}

#[tokio::test]
async fn delete_directory_on_the_sources_folder_is_refused() {
    let (_d, vault, source) = protected().await;
    let dir = VaultPath::new("/sources");
    assert_protected(vault.delete_directory(&dir).await, &dir);
    assert!(vault.exists(&source).await);
}

#[tokio::test]
async fn rename_directory_on_an_ancestor_of_sources_is_refused() {
    let (_d, vault, source) = vault_with(true, Some("/lib/sources")).await;
    let from = VaultPath::new("/lib");
    let to = VaultPath::new("/library");
    assert_protected(vault.rename_directory(&from, &to).await, &from);
    assert!(vault.exists(&source).await);
    assert!(!vault.exists(&to).await);
}

#[tokio::test]
async fn rename_directory_inside_sources_is_refused() {
    let (_d, vault, _source) = protected().await;
    let from = VaultPath::new("/sources/sub");
    vault.create_directory(&from).await.unwrap();
    let to = VaultPath::new("/sub");
    assert_protected(vault.rename_directory(&from, &to).await, &from);
    assert!(vault.exists(&from).await);
}

#[tokio::test]
async fn attachment_edits_in_sources_are_refused() {
    let (_d, vault, _source) = protected().await;
    let file = VaultPath::new("/sources/img.png");
    vault.save_attachment(&file, b"one").await.unwrap();

    assert_protected(vault.save_attachment(&file, b"two").await, &file);
    assert_protected(vault.delete_attachment(&file).await, &file);
    let to = VaultPath::new("/img.png");
    assert_protected(vault.rename_attachment(&file, &to).await, &file);

    assert!(vault.exists(&file).await);
    assert!(!vault.exists(&to).await);
}

#[tokio::test]
async fn entry_dispatchers_reach_the_guards() {
    let (_d, vault, source) = protected().await;
    let dir = VaultPath::new("/sources");
    assert_protected(vault.delete_entry(&source).await, &source);
    assert_protected(vault.delete_entry(&dir).await, &dir);
    assert_protected(
        vault
            .rename_entry(&source, &VaultPath::new("/moved.md"))
            .await,
        &source,
    );
    assert_protected(
        vault.rename_entry(&dir, &VaultPath::new("/moved")).await,
        &dir,
    );
    assert!(vault.exists(&source).await);
}

#[tokio::test]
async fn rename_note_is_refused_when_a_source_links_to_it() {
    let (_d, vault, _source) = protected().await;
    let page = VaultPath::new("/wiki/page.md");
    vault.create_note(&page, "a page").await.unwrap();
    let linker = VaultPath::new("/sources/linker.md");
    vault.create_note(&linker, "see [[page]]").await.unwrap();

    let to = VaultPath::new("/wiki/renamed.md");
    assert_protected(vault.rename_note(&page, &to).await, &linker);
    assert!(vault.exists(&page).await);
    assert!(!vault.exists(&to).await);
    assert_eq!(vault.get_note_text(&linker).await.unwrap(), "see [[page]]");
}

/// Every automated edit the guards cover, run against `source`. Panics on
/// the first failure.
async fn run_every_edit(vault: &NoteVault, source: &VaultPath) {
    vault.save_note(source, "one").await.unwrap();
    vault.append_to_note(source, "two", None).await.unwrap();
    vault
        .replace_in_note(source, "one", "three", false, false)
        .await
        .unwrap();
    vault
        .set_property(
            source,
            "status",
            PropertyValue::Text("x".into()),
            FrontmatterFormat::default(),
        )
        .await
        .unwrap();
    let moved = VaultPath::new("/sources/moved.md");
    vault.rename_note(source, &moved).await.unwrap();
    vault.delete_note(&moved).await.unwrap();
    let file = VaultPath::new("/sources/img.png");
    vault.save_attachment(&file, b"one").await.unwrap();
    vault.save_attachment(&file, b"two").await.unwrap();
    vault.delete_attachment(&file).await.unwrap();
    vault
        .rename_directory(&VaultPath::new("/sources"), &VaultPath::new("/raw"))
        .await
        .unwrap();
    vault
        .delete_directory(&VaultPath::new("/raw"))
        .await
        .unwrap();
}

#[tokio::test]
async fn protection_is_off_by_default() {
    let dir = TempDir::new().unwrap();
    let ws = crate::system::sys(dir.path());
    write_wiki_declaration(&ws, &WikiDeclaration::defaults())
        .await
        .unwrap();
    let config = VaultConfig::new(ws);
    assert!(!config.protect_sources);
    let vault = NoteVault::new(config).await.unwrap();
    vault.validate_and_init().await.unwrap();
    let source = VaultPath::new("/sources/a.md");
    vault.create_note(&source, "source text").await.unwrap();

    run_every_edit(&vault, &source).await;
}

#[tokio::test]
async fn no_declaration_means_no_protection() {
    let (_d, vault, source) = vault_with(true, None).await;
    run_every_edit(&vault, &source).await;
}

#[tokio::test]
async fn a_malformed_declaration_blocks_automated_writes() {
    let (dir, vault, source) = protected().await;
    let file = dir.path().join(".kimun").join("wiki.toml");
    let mut body = std::fs::read_to_string(&file).unwrap();
    body.push_str("colour = \"red\"\n");
    std::fs::write(&file, body).unwrap();

    match vault.save_note(&source, "changed").await {
        Err(VaultError::InvalidWikiDeclaration { .. }) => {}
        other => panic!("expected InvalidWikiDeclaration, got {other:?}"),
    }
    assert_eq!(vault.get_note_text(&source).await.unwrap(), "source text");
}

#[tokio::test]
async fn source_match_ignores_case() {
    let (_d, vault, _source) = protected().await;
    let upper = VaultPath::new("/Sources/A.md");
    match vault.save_note(&upper, "changed").await {
        Err(VaultError::SourceProtected { .. }) => {}
        other => panic!("expected SourceProtected, got {other:?}"),
    }
    assert_eq!(vault.get_note_text(&upper).await.unwrap(), "source text");
}
