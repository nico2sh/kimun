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

const DEFAULT_SCHEMA: &str = r####"# Wiki schema

This note tells you how this wiki works. An AI agent and a human both follow it.
Edit it to fit your needs.

## The three layers

- Sources are in `/sources`. They are raw notes. They are the ground truth.
- The wiki is in `/wiki`. It holds pages that you write and keep up to date.
- This schema note tells you how to do the work.

Write only under `/wiki`. Never edit anything under `/sources`.

## Page format

Every wiki page starts with a frontmatter block. It has these properties:

```
---
summary: One sentence that says what the page is about.
sources: ["[[a-source]]", "[[another-source]]"]
updated: 2026-01-31
type: concept
---
```

- `summary` is one short sentence.
- `sources` lists the source notes the page draws on. Quote every wikilink in a property, as in the example.
- `updated` is the date of the last change.
- `type` says what kind of page it is, such as `concept`, `person` or `topic`.

## Naming

Use kebab-case for page names. Use lower case. Use hyphens between words.
Example: `spaced-repetition`.

## Linking

- Link each concept page from a parent page, so that no page is an orphan.
- Cite sources with wikilinks in the `sources` property.
- Link related pages to each other with wikilinks.

## Ingest

Follow these steps for each new source:

1. Read the source in full.
2. Find the wiki pages that the source touches.
3. Update those pages. Create a new page when a concept has none.
4. Add the source to the `sources` property of every page you touched.
5. Set `updated` to today on every page you touched.
6. Add an entry to the log at `/wiki/log.md`.

## Answer a question

1. Read the wiki pages that are about the question.
2. Answer from the wiki. Follow links when you need more.
3. Check the sources when the wiki is unclear or seems wrong.
4. Say which pages you used.
5. When you find a gap, fix it in the wiki.

## Lint

Check the wiki for these problems:

- Pages that miss a required property.
- Pages that no other page links to.
- Links that point to notes that do not exist.
- Claims that the cited sources do not support.
- Pages that have not been updated after a source changed.

Fix what you can. Report the rest.

## Log entries

Add one entry at the end of `/wiki/log.md` for each piece of work. Each entry has:

- The date.
- The source.
- The pages you touched.
- The main finding.
"####;

#[test]
fn schema_template_for_the_default_declaration_is_unchanged() {
    assert_eq!(
        crate::wiki::schema_template(&WikiDeclaration::defaults()),
        DEFAULT_SCHEMA
    );
}

fn page_format(text: &str) -> &str {
    let start = text.find("## Page format").unwrap();
    let end = text.find("## Naming").unwrap();
    &text[start..end]
}

#[test]
fn schema_template_lists_a_custom_property_list() {
    let mut d = WikiDeclaration::defaults();
    d.required_properties = vec!["summary".to_string(), "owner".to_string()];
    let text = crate::wiki::schema_template(&d);
    let block = page_format(&text);
    assert!(block.contains("owner: <value>"), "{block}");
    assert!(
        block.contains("- `owner` is required on every page."),
        "{block}"
    );
    assert!(block.contains("summary"), "{block}");
    assert!(!block.contains("`updated`"), "{block}");
    assert!(!block.contains("updated:"), "{block}");
    assert!(block.contains("`type`"), "{block}");
    assert!(block.contains("optional"), "{block}");
}

#[test]
fn schema_template_lists_type_once_when_it_is_required() {
    let mut d = WikiDeclaration::defaults();
    d.required_properties = vec!["type".to_string()];
    let text = crate::wiki::schema_template(&d);
    let block = page_format(&text);
    assert_eq!(block.matches("- `type`").count(), 1, "{block}");
    assert!(!block.contains("optional"), "{block}");
}

#[tokio::test]
async fn init_with_a_plain_file_where_a_folder_is_wanted_is_an_error() {
    let (dir, vault) = empty_vault().await;
    std::fs::write(dir.path().join("sources"), "a plain file").unwrap();
    let result = vault.wiki_init(WikiInitOptions::default()).await;
    assert!(result.is_err(), "{result:?}");
    // wiki_init writes the declaration first, so the file stays. A second
    // run reports the same error until the file in the way is moved.
    assert!(dir.path().join(".kimun").join("wiki.toml").exists());
}
