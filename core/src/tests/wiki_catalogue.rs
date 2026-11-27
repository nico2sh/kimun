// Gated with the inner attribute: TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::error::VaultError;
use crate::nfs::wiki_declaration::write_wiki_declaration;
use crate::nfs::VaultPath;
use crate::{NoteVault, VaultConfig, WikiDeclaration};
use tempfile::TempDir;

async fn vault_with(declared: bool) -> (TempDir, NoteVault) {
    let dir = TempDir::new().unwrap();
    let ws = crate::system::sys(dir.path());
    if declared {
        let declaration =
            WikiDeclaration::with_folders(&VaultPath::new("/sources"), &VaultPath::new("/wiki"));
        write_wiki_declaration(&ws, &declaration).await.unwrap();
    }
    let vault = NoteVault::new(VaultConfig::new(ws)).await.unwrap();
    vault.validate_and_init().await.unwrap();
    (dir, vault)
}

async fn add(vault: &NoteVault, path: &str, text: &str) {
    vault
        .create_note(&VaultPath::new(path), text)
        .await
        .unwrap();
}

#[tokio::test]
async fn catalogue_lists_pages_sorted_with_summary_and_type() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/zeta.md",
        "---\nsummary: Last one\ntype: topic\n---\n# Zeta Page\n",
    )
    .await;
    add(
        &vault,
        "/wiki/alpha.md",
        "---\nsummary: First one\ntype: concept\n---\n# Alpha Page\n",
    )
    .await;
    let c = vault.wiki_catalogue(10).await.unwrap();
    assert_eq!(c.total, 2);
    assert_eq!(c.entries.len(), 2);
    assert_eq!(c.entries[0].path, VaultPath::new("/wiki/alpha.md"));
    assert_eq!(c.entries[0].title, "Alpha Page");
    assert_eq!(c.entries[0].summary.as_deref(), Some("First one"));
    assert_eq!(c.entries[0].page_type.as_deref(), Some("concept"));
    assert_eq!(c.entries[1].path, VaultPath::new("/wiki/zeta.md"));
    assert_eq!(c.entries[1].page_type.as_deref(), Some("topic"));
}

#[tokio::test]
async fn catalogue_leaves_out_log_and_schema() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/wiki/log.md", "# Log\n").await;
    add(&vault, "/wiki/schema.md", "# Schema\n").await;
    add(&vault, "/wiki/page.md", "# Page\n").await;
    let c = vault.wiki_catalogue(10).await.unwrap();
    assert_eq!(c.total, 1);
    assert_eq!(c.entries[0].path, VaultPath::new("/wiki/page.md"));
}

#[tokio::test]
async fn catalogue_lists_nested_pages_and_leaves_out_nested_log_and_schema() {
    let dir = TempDir::new().unwrap();
    let ws = crate::system::sys(dir.path());
    let mut declaration =
        WikiDeclaration::with_folders(&VaultPath::new("/sources"), &VaultPath::new("/wiki"));
    declaration.log = "/wiki/meta/log.md".to_string();
    declaration.schema = "/wiki/meta/schema.md".to_string();
    write_wiki_declaration(&ws, &declaration).await.unwrap();
    let vault = NoteVault::new(VaultConfig::new(ws)).await.unwrap();
    vault.validate_and_init().await.unwrap();
    let d = vault.wiki_declaration().await.unwrap().unwrap();
    add(&vault, &d.log_path().to_string(), "# Log\n").await;
    add(&vault, &d.schema_path().to_string(), "# Schema\n").await;
    add(&vault, "/wiki/top.md", "# Top\n").await;
    add(&vault, "/wiki/deep/er/leaf.md", "# Leaf\n").await;
    let c = vault.wiki_catalogue(10).await.unwrap();
    let paths: Vec<_> = c.entries.iter().map(|e| e.path.clone()).collect();
    assert_eq!(c.total, 2);
    assert!(paths.contains(&VaultPath::new("/wiki/top.md")));
    assert!(paths.contains(&VaultPath::new("/wiki/deep/er/leaf.md")));
}

#[tokio::test]
async fn catalogue_cap_keeps_total() {
    let (_d, vault) = vault_with(true).await;
    for n in 1..=5 {
        add(&vault, &format!("/wiki/p{n}.md"), "# P\n").await;
    }
    let c = vault.wiki_catalogue(2).await.unwrap();
    assert_eq!(c.entries.len(), 2);
    assert_eq!(c.total, 5);
}

#[tokio::test]
async fn catalogue_counts_a_page_with_no_properties() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/wiki/bare.md", "# Bare\n").await;
    let c = vault.wiki_catalogue(10).await.unwrap();
    assert_eq!(c.total, 1);
    assert_eq!(c.entries[0].title, "Bare");
    assert_eq!(c.entries[0].summary, None);
    assert_eq!(c.entries[0].page_type, None);
}

#[tokio::test]
async fn catalogue_without_a_declaration_is_wiki_not_declared() {
    let (_d, vault) = vault_with(false).await;
    assert!(matches!(
        vault.wiki_catalogue(10).await,
        Err(VaultError::WikiNotDeclared)
    ));
}

#[tokio::test]
async fn catalogue_ignores_notes_outside_the_wiki_folder() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/wiki/in.md", "# In\n").await;
    add(&vault, "/sources/out.md", "# Out\n").await;
    add(&vault, "/other.md", "# Other\n").await;
    let c = vault.wiki_catalogue(10).await.unwrap();
    assert_eq!(c.total, 1);
}

#[tokio::test]
async fn catalogue_of_a_wiki_folder_that_does_not_exist_is_empty() {
    let (dir, vault) = vault_with(true).await;
    assert!(!dir.path().join("wiki").exists());
    let c = vault.wiki_catalogue(10).await.unwrap();
    assert!(c.entries.is_empty());
    assert_eq!(c.total, 0);
}

#[tokio::test]
async fn catalogue_limit_zero_lists_nothing_and_keeps_total() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/wiki/a.md", "# A\n").await;
    add(&vault, "/wiki/b.md", "# B\n").await;
    let c = vault.wiki_catalogue(0).await.unwrap();
    assert!(c.entries.is_empty());
    assert_eq!(c.total, 2);
}

#[tokio::test]
async fn catalogue_does_not_list_an_attachment_under_the_wiki_folder() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/wiki/page.md", "# Page\n").await;
    vault
        .save_attachment(&VaultPath::new("/wiki/diagram.png"), b"png")
        .await
        .unwrap();
    let c = vault.wiki_catalogue(10).await.unwrap();
    assert_eq!(c.total, 1);
    assert_eq!(c.entries[0].path, VaultPath::new("/wiki/page.md"));
}

#[tokio::test]
async fn catalogue_lists_a_page_with_malformed_frontmatter_without_properties() {
    let (dir, vault) = vault_with(true).await;
    add(&vault, "/wiki/ok.md", "# Ok\n").await;
    let wiki = dir.path().join("wiki");
    std::fs::write(
        wiki.join("broken.md"),
        "---\nsummary: [unclosed\ntype: topic\n---\n# Broken\n",
    )
    .unwrap();
    let c = vault.wiki_catalogue(10).await.unwrap();
    assert_eq!(c.total, 2);
    let broken = c
        .entries
        .iter()
        .find(|e| e.path == VaultPath::new("/wiki/broken.md"))
        .expect("the broken page is listed");
    assert_eq!(broken.summary, None);
    assert_eq!(broken.page_type, None);
}

#[tokio::test]
async fn catalogue_summary_that_is_a_list_is_none() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/listy.md",
        "---\nsummary:\n  - one\n  - two\n---\n# Listy\n",
    )
    .await;
    let c = vault.wiki_catalogue(10).await.unwrap();
    assert_eq!(c.entries[0].summary, None);
}
