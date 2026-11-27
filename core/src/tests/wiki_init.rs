// Gated with the inner attribute: TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::error::VaultError;
use crate::nfs::VaultPath;
use crate::{NoteVault, VaultConfig, WikiDeclaration, WikiInitOptions, WikiInitReport};
use tempfile::TempDir;

async fn empty_vault() -> (TempDir, NoteVault) {
    let dir = TempDir::new().unwrap();
    let ws = crate::system::sys(dir.path());
    let vault = NoteVault::new(VaultConfig::new(ws)).await.unwrap();
    vault.validate_and_init().await.unwrap();
    (dir, vault)
}

fn declaration_text(dir: &TempDir) -> String {
    std::fs::read_to_string(dir.path().join(".kimun").join("wiki.toml")).unwrap()
}

fn schema_path() -> VaultPath {
    VaultPath::new("/wiki").append(&VaultPath::note_path_from("schema"))
}

fn log_path() -> VaultPath {
    VaultPath::new("/wiki").append(&VaultPath::note_path_from("log"))
}

fn all_kept(report: &WikiInitReport) -> bool {
    !report.entries.is_empty() && report.entries.iter().all(|e| !e.created)
}

#[tokio::test]
async fn init_creates_the_structure() {
    let (dir, vault) = empty_vault().await;
    let report = vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    assert!(report.entries.iter().all(|e| e.created));
    assert!(vault.exists(&VaultPath::new("/sources")).await);
    assert!(vault.exists(&VaultPath::new("/wiki")).await);
    assert!(vault.exists(&schema_path()).await);
    assert!(vault.exists(&log_path()).await);
    assert!(dir.path().join(".kimun").join("wiki.toml").exists());
    let schema = vault.get_note_text(&schema_path()).await.unwrap();
    assert!(schema.contains("/sources"));
    assert!(schema.contains("/wiki"));
}

#[tokio::test]
async fn init_twice_changes_nothing() {
    let (dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let toml = declaration_text(&dir);
    let schema = vault.get_note_text(&schema_path()).await.unwrap();
    let log = vault.get_note_text(&log_path()).await.unwrap();
    let report = vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    assert!(all_kept(&report));
    assert_eq!(toml, declaration_text(&dir));
    assert_eq!(schema, vault.get_note_text(&schema_path()).await.unwrap());
    assert_eq!(log, vault.get_note_text(&log_path()).await.unwrap());
}

#[tokio::test]
async fn init_keeps_an_edited_schema_note() {
    let (_dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    vault
        .save_note(&schema_path(), "my own rules")
        .await
        .unwrap();
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    assert_eq!(
        "my own rules",
        vault.get_note_text(&schema_path()).await.unwrap()
    );
}

#[tokio::test]
async fn init_in_a_vault_with_notes_leaves_them_alone() {
    let (_dir, vault) = empty_vault().await;
    let note = VaultPath::note_path_from("/inbox/idea");
    vault.create_note(&note, "an idea").await.unwrap();
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    assert_eq!("an idea", vault.get_note_text(&note).await.unwrap());
}

#[tokio::test]
async fn init_with_custom_folders_writes_them_to_the_declaration() {
    let (_dir, vault) = empty_vault().await;
    vault
        .wiki_init(WikiInitOptions {
            sources: Some(VaultPath::new("/raw")),
            wiki: Some(VaultPath::new("/kb")),
        })
        .await
        .unwrap();
    let d = vault.wiki_declaration().await.unwrap().unwrap();
    assert_eq!(VaultPath::new("/raw"), d.sources_path());
    assert_eq!(VaultPath::new("/kb"), d.wiki_path());
    assert!(vault.exists(&d.schema_path()).await);
    assert!(vault.exists(&d.log_path()).await);
    let schema = vault.get_note_text(&d.schema_path()).await.unwrap();
    assert!(schema.contains("/raw"));
    assert!(schema.contains("/kb/"));
    assert!(
        schema.contains(&d.log),
        "the schema names the declared log {}:\n{schema}",
        d.log
    );
}

#[tokio::test]
async fn init_refuses_overlapping_folders() {
    let (dir, vault) = empty_vault().await;
    let err = vault
        .wiki_init(WikiInitOptions {
            sources: Some(VaultPath::new("/a")),
            wiki: Some(VaultPath::new("/a/b")),
        })
        .await
        .unwrap_err();
    assert!(matches!(err, VaultError::InvalidWikiDeclaration { .. }));
    assert!(!dir.path().join(".kimun").join("wiki.toml").exists());
}

#[tokio::test]
async fn init_with_an_existing_declaration_ignores_the_options() {
    let (_dir, vault) = empty_vault().await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let before: WikiDeclaration = vault.wiki_declaration().await.unwrap().unwrap();
    let report = vault
        .wiki_init(WikiInitOptions {
            sources: Some(VaultPath::new("/raw")),
            wiki: Some(VaultPath::new("/kb")),
        })
        .await
        .unwrap();
    assert!(all_kept(&report));
    assert_eq!(before, vault.wiki_declaration().await.unwrap().unwrap());
    assert!(!vault.exists(&VaultPath::new("/raw")).await);
}

#[tokio::test]
async fn schema_example_sources_line_reads_as_a_list_of_two() {
    let template = crate::wiki::schema_template(&WikiDeclaration::defaults());
    let line = template
        .lines()
        .find(|l| l.starts_with("sources:"))
        .expect("the template has an example sources line");
    let note = format!("---\n{line}\n---\nbody\n");
    let properties = crate::note::NoteDetails::properties_of(note);
    let value = properties
        .into_iter()
        .find(|(k, _)| k == "sources")
        .and_then(|(_, v)| v)
        .expect("sources has a value");
    match value {
        crate::note::PropertyValue::List(items) => assert_eq!(2, items.len()),
        other => panic!("expected a list, got {other:?}"),
    }
}

#[tokio::test]
async fn init_with_a_malformed_declaration_changes_nothing() {
    let (dir, vault) = empty_vault().await;
    let file = dir.path().join(".kimun").join("wiki.toml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let bad = "sources = \"/sources\"\nwiki = \"/wiki\"\nlog = \"/wiki/log.md\"\nschema = \"/wiki/schema.md\"\nrequired_properties = []\nbogus = 1\n";
    std::fs::write(&file, bad).unwrap();
    let err = vault
        .wiki_init(WikiInitOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, VaultError::InvalidWikiDeclaration { .. }));
    assert_eq!(bad, std::fs::read_to_string(&file).unwrap());
    assert!(!vault.exists(&VaultPath::new("/sources")).await);
    assert!(!vault.exists(&VaultPath::new("/wiki")).await);
}

#[tokio::test]
async fn init_with_dot_relative_folders_protects_the_sources() {
    let (dir, vault) = empty_vault().await;
    vault
        .wiki_init(WikiInitOptions {
            sources: Some(VaultPath::new("./raw")),
            wiki: Some(VaultPath::new("./kb")),
        })
        .await
        .unwrap();
    let d = vault.wiki_declaration().await.unwrap().unwrap();
    assert_eq!(d.sources, "/raw");
    assert_eq!(d.wiki, "/kb");
    let source = VaultPath::new("/raw").append(&VaultPath::note_path_from("a"));
    vault.create_note(&source, "source text").await.unwrap();

    let protected = NoteVault::new(
        VaultConfig::new(crate::system::sys(dir.path())).with_protected_sources(true),
    )
    .await
    .unwrap();
    assert!(matches!(
        protected.save_note(&source, "changed").await,
        Err(VaultError::SourceProtected { .. })
    ));
    assert_eq!(vault.get_note_text(&source).await.unwrap(), "source text");

    let report = vault.wiki_check().await.unwrap();
    for f in &report.findings {
        assert!(
            !f.path.is_like(&d.log_path()) && !f.path.is_like(&d.schema_path()),
            "log or schema checked as a page: {f:?}"
        );
    }
}

#[tokio::test]
async fn init_with_a_note_path_folder_writes_nothing_and_does_not_get_stuck() {
    let (dir, vault) = empty_vault().await;
    for bad in ["a.md", "/.."] {
        let err = vault
            .wiki_init(WikiInitOptions {
                sources: Some(VaultPath::new(bad)),
                wiki: None,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(err, VaultError::InvalidWikiDeclaration { .. }),
            "{bad}: {err:?}"
        );
        assert!(
            !dir.path().join(".kimun").join("wiki.toml").exists(),
            "{bad}"
        );
    }
    // A later init with good folders works.
    let report = vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    assert!(report.entries.iter().all(|e| e.created));
}
