// Gated with the inner attribute: TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::error::VaultError;
use crate::nfs::VaultPath;
use crate::{
    NoteVault, SaveSourceRequest, VaultConfig, WikiDeclaration, WikiInitOptions, WikiInitReport,
};
use tempfile::TempDir;

async fn empty_vault() -> (TempDir, NoteVault) {
    let dir = TempDir::new().unwrap();
    let ws = crate::system::sys(dir.path());
    let vault = NoteVault::new(VaultConfig::new(ws)).await.unwrap();
    vault.validate_and_init().await.unwrap();
    (dir, vault)
}

fn file_of(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join(".kimun").join("wiki.toml")
}

fn declaration_text(dir: &TempDir) -> String {
    std::fs::read_to_string(file_of(dir)).unwrap()
}

fn request() -> SaveSourceRequest {
    SaveSourceRequest {
        url: "https://example.com/a".to_string(),
        title: Some("A".to_string()),
        content_type: "text/markdown".to_string(),
        markdown: "text".to_string(),
    }
}

fn all_kept(report: &WikiInitReport) -> bool {
    !report.entries.is_empty() && report.entries.iter().all(|e| !e.created)
}

#[tokio::test]
async fn configure_on_a_vault_with_no_declaration_writes_it_and_the_layout() {
    let (dir, vault) = empty_vault().await;
    let d = WikiDeclaration::defaults();
    vault.wiki_configure(d.clone()).await.unwrap();
    assert_eq!(Some(d.clone()), vault.wiki_declaration().await.unwrap());
    assert!(dir.path().join(".kimun").join("wiki.toml").exists());
    assert!(vault.exists(&d.sources_path()).await);
    assert!(vault.exists(&d.wiki_path()).await);
    assert!(vault.exists(&d.schema_path()).await);
    assert!(vault.exists(&d.log_path()).await);
}

#[tokio::test]
async fn configure_a_new_sources_folder_keeps_the_old_folder_and_its_notes() {
    let (dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let old_note = VaultPath::new("/sources").append(&VaultPath::note_path_from("old"));
    vault.create_note(&old_note, "old text").await.unwrap();
    let mut d = vault.wiki_declaration().await.unwrap().unwrap();
    d.sources = "/raw".to_string();
    vault.wiki_configure(d).await.unwrap();
    assert_eq!("old text", vault.get_note_text(&old_note).await.unwrap());
    assert!(vault.exists(&VaultPath::new("/raw")).await);
    assert!(declaration_text(&dir).contains("/raw"));
    let saved = vault.save_source(&request()).await.unwrap();
    assert!(saved.to_string().starts_with("/raw"), "saved into {saved}");
}

#[tokio::test]
async fn configure_moves_the_protection_to_the_new_sources_folder() {
    let (dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let old_note = VaultPath::new("/sources").append(&VaultPath::note_path_from("old"));
    let new_note = VaultPath::new("/raw").append(&VaultPath::note_path_from("new"));
    vault.create_note(&old_note, "old").await.unwrap();
    let mut d = vault.wiki_declaration().await.unwrap().unwrap();
    d.sources = "/raw".to_string();
    vault.wiki_configure(d).await.unwrap();
    vault.create_note(&new_note, "new").await.unwrap();
    let protected = NoteVault::new(
        VaultConfig::new(crate::system::sys(dir.path())).with_protected_sources(true),
    )
    .await
    .unwrap();
    protected.save_note(&old_note, "changed").await.unwrap();
    assert!(matches!(
        protected.save_note(&new_note, "changed").await,
        Err(VaultError::SourceProtected { .. })
    ));
}

#[tokio::test]
async fn configure_replaces_a_broken_declaration_file() {
    let (dir, vault) = empty_vault().await;
    std::fs::create_dir_all(file_of(&dir).parent().unwrap()).unwrap();
    std::fs::write(file_of(&dir), "this is [[ not toml").unwrap();
    vault
        .wiki_configure(WikiDeclaration::defaults())
        .await
        .unwrap();
    assert!(vault.wiki_declaration().await.unwrap().is_some());
}

#[tokio::test]
async fn configure_rejects_an_invalid_declaration_and_keeps_the_file() {
    let (dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let before = declaration_text(&dir);
    let d = WikiDeclaration::with_folders(&VaultPath::new("/a"), &VaultPath::new("/a/b"));
    let err = vault.wiki_configure(d).await.unwrap_err();
    assert!(matches!(err, VaultError::InvalidWikiDeclaration { .. }));
    assert_eq!(before, declaration_text(&dir));
}

#[tokio::test]
async fn configure_rejects_a_new_sources_path_that_holds_a_note() {
    let (dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let before = declaration_text(&dir);
    std::fs::write(dir.path().join("raw"), "a plain file").unwrap();
    let mut d = vault.wiki_declaration().await.unwrap().unwrap();
    d.sources = "/raw".to_string();
    assert!(vault.wiki_configure(d).await.is_err());
    assert_eq!(before.as_bytes(), declaration_text(&dir).as_bytes());
}

#[tokio::test]
async fn configure_makes_the_parent_of_a_log_outside_the_wiki_folder() {
    let (_dir, vault) = empty_vault().await;
    let mut d = WikiDeclaration::defaults();
    d.log = "/meta/log.md".to_string();
    let report = vault.wiki_configure(d.clone()).await.unwrap();
    assert!(vault.exists(&d.log_path()).await);
    assert_eq!(4, report.entries.len(), "{report:?}");
}

#[tokio::test]
async fn configure_cleans_the_property_list() {
    let (_dir, vault) = empty_vault().await;
    let mut d = WikiDeclaration::defaults();
    d.required_properties = ["summary", " summary", "", "owner"]
        .into_iter()
        .map(String::from)
        .collect();
    vault.wiki_configure(d).await.unwrap();
    let read = vault.wiki_declaration().await.unwrap().unwrap();
    assert_eq!(vec!["summary", "owner"], read.required_properties);
}

#[tokio::test]
async fn configure_accepts_an_empty_property_list() {
    let (_dir, vault) = empty_vault().await;
    let mut d = WikiDeclaration::defaults();
    d.required_properties = Vec::new();
    vault.wiki_configure(d).await.unwrap();
    let read = vault.wiki_declaration().await.unwrap().unwrap();
    assert!(read.required_properties.is_empty());
}

#[tokio::test]
async fn configure_rejects_an_invalid_property_key() {
    let (dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let before = declaration_text(&dir);
    let mut d = WikiDeclaration::defaults();
    d.required_properties = vec!["two\nlines".to_string()];
    let err = vault.wiki_configure(d).await.unwrap_err();
    assert!(matches!(err, VaultError::InvalidProperty { .. }), "{err:?}");
    assert_eq!(before, declaration_text(&dir));
}

#[tokio::test]
async fn configure_reports_created_and_kept() {
    let (_dir, vault) = empty_vault().await;
    let first = vault
        .wiki_configure(WikiDeclaration::defaults())
        .await
        .unwrap();
    assert!(first.entries.iter().any(|e| e.created));
    let second = vault
        .wiki_configure(WikiDeclaration::defaults())
        .await
        .unwrap();
    assert!(all_kept(&second), "{second:?}");
}

#[tokio::test]
async fn configure_never_changes_an_existing_schema_note() {
    let (_dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let schema = WikiDeclaration::defaults().schema_path();
    vault.save_note(&schema, "my own rules").await.unwrap();
    let mut d = WikiDeclaration::defaults();
    d.required_properties = vec!["owner".to_string()];
    vault.wiki_configure(d).await.unwrap();
    assert_eq!("my own rules", vault.get_note_text(&schema).await.unwrap());
}

#[test]
fn template_handles_an_empty_list_and_type_in_the_middle() {
    let mut d = WikiDeclaration::defaults();
    d.required_properties = Vec::new();
    assert!(crate::wiki::schema_template(&d).contains("# Wiki schema"));
    d.required_properties = ["summary", "type", "owner"]
        .into_iter()
        .map(String::from)
        .collect();
    let text = crate::wiki::schema_template(&d);
    for name in ["summary", "type", "owner"] {
        assert!(text.contains(&format!("{name}:")), "{name} in:\n{text}");
    }
}

#[tokio::test]
async fn configure_drops_a_property_name_that_differs_only_in_case() {
    let (_dir, vault) = empty_vault().await;
    let mut d = WikiDeclaration::defaults();
    d.required_properties = ["summary", "Summary", "owner"]
        .into_iter()
        .map(String::from)
        .collect();
    vault.wiki_configure(d).await.unwrap();
    let read = vault.wiki_declaration().await.unwrap().unwrap();
    assert_eq!(vec!["summary", "owner"], read.required_properties);
}
