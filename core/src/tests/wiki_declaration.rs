// Gated with the inner attribute: TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::nfs::wiki_declaration::{read_wiki_declaration, write_wiki_declaration};
use crate::*;
use tempfile::TempDir;

async fn make_vault(dir: &std::path::Path) -> NoteVault {
    NoteVault::new(VaultConfig::new(crate::system::sys(dir)))
        .await
        .unwrap()
}

#[tokio::test]
async fn declaration_round_trips() {
    let dir = TempDir::new().unwrap();
    let ws = crate::system::sys(dir.path());
    let d = WikiDeclaration::defaults();
    write_wiki_declaration(&ws, &d).await.unwrap();
    assert_eq!(read_wiki_declaration(&ws).await.unwrap(), Some(d));
}

#[tokio::test]
async fn missing_file_reads_as_none() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    assert_eq!(vault.wiki_declaration().await.unwrap(), None);
}

#[tokio::test]
async fn unknown_key_is_an_error() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let ws = crate::system::sys(dir.path());
    write_wiki_declaration(&ws, &WikiDeclaration::defaults())
        .await
        .unwrap();
    let file = dir.path().join(".kimun").join("wiki.toml");
    let mut body = std::fs::read_to_string(&file).unwrap();
    body.push_str("colour = \"red\"\n");
    std::fs::write(&file, body).unwrap();
    match vault.wiki_declaration().await {
        Err(VaultError::InvalidWikiDeclaration { message }) => {
            assert!(message.contains("wiki.toml"), "{message}")
        }
        other => panic!("expected InvalidWikiDeclaration, got {other:?}"),
    }
}

#[test]
fn validate_rejects_equal_folders() {
    let mut d = WikiDeclaration::defaults();
    d.wiki = "/Sources/".to_string();
    assert!(d.validate().is_err());
}

#[test]
fn validate_rejects_wiki_inside_sources() {
    let mut d = WikiDeclaration::defaults();
    d.wiki = "/sources/wiki".to_string();
    assert!(d.validate().is_err());
    let mut d = WikiDeclaration::defaults();
    d.sources = "/wiki/sources".to_string();
    assert!(d.validate().is_err());
}

#[test]
fn validate_rejects_a_log_that_is_not_a_note_path() {
    let mut d = WikiDeclaration::defaults();
    d.log = "/wiki/log".to_string();
    assert!(d.validate().is_err());
    assert!(WikiDeclaration::defaults().validate().is_ok());
}

#[test]
fn folder_comparison_ignores_case_and_trailing_slash() {
    let d = WikiDeclaration::with_folders(&VaultPath::new("/Sources/"), &VaultPath::new("/Wiki"));
    assert_eq!(d.sources_path(), VaultPath::new("/sources"));
    assert_eq!(d.wiki_path(), VaultPath::new("/wiki"));
    assert_eq!(d.log_path(), VaultPath::note_path_from("/wiki/log"));
    assert!(d.validate().is_ok());
}

/// Writes a declaration with `sources` as written, and the default wiki,
/// log and schema. Returns a vault with protection on and `/sources/a.md`.
async fn vault_declaring_sources(sources: &str) -> (TempDir, NoteVault) {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join(".kimun").join("wiki.toml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let body = format!(
        "sources = '{sources}'\nwiki = \"/wiki\"\nlog = \"/wiki/log.md\"\nschema = \"/wiki/schema.md\"\nrequired_properties = []\n"
    );
    std::fs::write(&file, body).unwrap();
    let source_dir = dir.path().join("sources");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(source_dir.join("a.md"), "source text").unwrap();
    let vault = NoteVault::new(
        VaultConfig::new(crate::system::sys(dir.path())).with_protected_sources(true),
    )
    .await
    .unwrap();
    (dir, vault)
}

#[tokio::test]
async fn a_declared_sources_path_is_normalized_so_protection_still_applies() {
    for sources in [
        "./sources",
        "/x/../sources",
        "sources/",
        "/SOURCES",
        "sources",
    ] {
        let (_dir, vault) = vault_declaring_sources(sources).await;
        let d = vault.wiki_declaration().await.unwrap().unwrap();
        assert_eq!(d.sources_path(), VaultPath::new("/sources"), "{sources}");
        let source = VaultPath::new("/sources/a.md");
        match vault.save_note(&source, "changed").await {
            Err(VaultError::SourceProtected { .. }) => {}
            other => panic!("{sources}: expected SourceProtected, got {other:?}"),
        }
        assert_eq!(
            vault.get_note_text(&source).await.unwrap(),
            "source text",
            "{sources}"
        );
    }
}

#[tokio::test]
async fn a_declared_sources_path_that_is_not_a_usable_folder_is_rejected() {
    for sources in ["/sources.md", "C:/sources", "/..", "/", "\\sources", "/..x"] {
        let (_dir, vault) = vault_declaring_sources(sources).await;
        match vault.wiki_declaration().await {
            Err(VaultError::InvalidWikiDeclaration { message }) => {
                assert!(message.contains(".kimun/wiki.toml"), "{sources}: {message}")
            }
            other => panic!("{sources}: expected InvalidWikiDeclaration, got {other:?}"),
        }
        // The guards fail closed on a rejected declaration.
        let source = VaultPath::new("/sources/a.md");
        assert!(
            vault.save_note(&source, "changed").await.is_err(),
            "{sources}"
        );
        assert_eq!(
            vault.get_note_text(&source).await.unwrap(),
            "source text",
            "{sources}"
        );
    }
}

#[test]
fn validate_rejects_a_log_or_schema_inside_the_sources_folder() {
    let mut d = WikiDeclaration::defaults();
    d.log = "/sources/log.md".to_string();
    assert!(d.validate().is_err());
    let mut d = WikiDeclaration::defaults();
    d.schema = "/sources/deep/schema.md".to_string();
    assert!(d.validate().is_err());
}

#[test]
fn validate_rejects_a_log_with_a_character_that_is_not_allowed() {
    let mut d = WikiDeclaration::defaults();
    d.log = "/wiki/lo?g.md".to_string();
    assert!(d.validate().is_err());
}

#[test]
fn with_folders_stores_flat_absolute_paths() {
    let d = WikiDeclaration::with_folders(&VaultPath::new("./raw"), &VaultPath::new("kb/x/.."));
    assert_eq!(d.sources, "/raw");
    assert_eq!(d.wiki, "/kb");
    assert_eq!(
        d.log_path(),
        VaultPath::new("/kb").append(&VaultPath::note_path_from("log"))
    );
    assert!(d.validate().is_ok());
}
