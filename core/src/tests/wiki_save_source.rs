// Gated with the inner attribute: TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::error::VaultError;
use crate::nfs::wiki_declaration::write_wiki_declaration;
use crate::nfs::VaultPath;
use crate::note::PropertyValue;
use crate::wiki::{body_hash, source_slug};
use crate::{NoteVault, SaveSourceRequest, VaultConfig, WikiDeclaration, WikiRule};
use chrono::NaiveDate;
use tempfile::TempDir;

async fn vault_with(declared: bool, protect: bool) -> (TempDir, NoteVault) {
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
    (dir, vault)
}

fn request(markdown: &str) -> SaveSourceRequest {
    SaveSourceRequest {
        url: "https://example.com/a?x=1&y=\"2\"".to_string(),
        title: Some("My Page".to_string()),
        content_type: "text/html".to_string(),
        markdown: markdown.to_string(),
    }
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()
}

async fn text(vault: &NoteVault, path: &VaultPath) -> String {
    vault.get_note_text(path).await.unwrap()
}

async fn assert_clean(vault: &NoteVault, path: &VaultPath) {
    let report = vault.wiki_check().await.unwrap();
    assert!(
        !report
            .findings
            .iter()
            .any(|f| matches!(f.rule, WikiRule::Unstamped | WikiRule::SourceModified)),
        "{:?}",
        report.findings
    );
    assert!(vault.wiki_stamp().await.unwrap().stamped.is_empty());
    let _ = path;
}

#[tokio::test]
async fn save_source_writes_the_four_properties_and_the_body() {
    let (_d, vault) = vault_with(true, false).await;
    let path = vault
        .save_source_on(day(), &request("hello\n"))
        .await
        .unwrap();
    assert_eq!(
        path,
        VaultPath::note_path_from("/sources/2026-03-04-my-page")
    );
    let get = |k: &'static str| {
        let (vault, path) = (&vault, &path);
        async move { vault.get_property(path, k).await.unwrap() }
    };
    assert_eq!(
        get("source_url").await,
        Some(Some(PropertyValue::Text(
            "https://example.com/a?x=1&y=\"2\"".to_string()
        )))
    );
    assert_eq!(
        get("ingested_at").await,
        Some(Some(PropertyValue::Date(day())))
    );
    assert_eq!(
        get("content_type").await,
        Some(Some(PropertyValue::Text("text/html".to_string())))
    );
    let t = text(&vault, &path).await;
    assert_eq!(
        get("content_hash").await,
        Some(Some(PropertyValue::Text(body_hash(&t))))
    );
    assert!(t.ends_with("# My Page\n\nhello\n"), "{t}");
}

#[tokio::test]
async fn saved_source_is_stamped_and_unmodified() {
    let (_d, vault) = vault_with(true, false).await;
    let path = vault.save_source(&request("hello\n")).await.unwrap();
    assert_clean(&vault, &path).await;
}

#[tokio::test]
async fn markdown_starting_with_a_rule_line_keeps_a_valid_frontmatter() {
    for fence in ["---", "+++"] {
        let (_d, vault) = vault_with(true, false).await;
        let md = format!("{fence}\nnot: frontmatter\n{fence}\nafter\n");
        let path = vault.save_source_on(day(), &request(&md)).await.unwrap();
        for key in ["source_url", "ingested_at", "content_type", "content_hash"] {
            let v = vault.get_property(&path, key).await.unwrap();
            assert!(matches!(v, Some(Some(_))), "{fence} {key}");
        }
        let t = text(&vault, &path).await;
        assert!(t.contains(&md), "{t}");
        assert_clean(&vault, &path).await;
    }
}

#[tokio::test]
async fn title_gets_an_h1_only_when_the_body_has_none() {
    let (_d, vault) = vault_with(true, false).await;
    let with = vault
        .save_source_on(day(), &request("# Own title\n\ntext\n"))
        .await
        .unwrap();
    assert!(!text(&vault, &with).await.contains("# My Page"));
    let without = vault
        .save_source_on(day(), &request("text\n"))
        .await
        .unwrap();
    assert!(text(&vault, &without).await.contains("# My Page\n\ntext"));
    let mut no_title = request("text\n");
    no_title.title = None;
    let host = vault.save_source_on(day(), &no_title).await.unwrap();
    assert!(text(&vault, &host).await.contains("# example.com\n\ntext"));
    assert!(host.to_string().contains("example-com"));
}

#[tokio::test]
async fn two_saves_of_the_same_url_on_one_day_give_two_notes() {
    let (_d, vault) = vault_with(true, false).await;
    let a = vault
        .save_source_on(day(), &request("one\n"))
        .await
        .unwrap();
    let b = vault
        .save_source_on(day(), &request("two\n"))
        .await
        .unwrap();
    assert_ne!(a, b);
    assert!(text(&vault, &a).await.contains("one"));
    assert!(text(&vault, &b).await.contains("two"));
}

#[tokio::test]
async fn save_source_works_on_a_protected_vault() {
    let (_d, vault) = vault_with(true, true).await;
    let path = vault.save_source(&request("hello\n")).await.unwrap();
    assert_clean(&vault, &path).await;
}

#[tokio::test]
async fn save_source_without_a_declaration_is_wiki_not_declared() {
    let (_d, vault) = vault_with(false, false).await;
    let err = vault.save_source(&request("hello\n")).await.unwrap_err();
    assert!(matches!(err, VaultError::WikiNotDeclared), "{err:?}");
}

#[tokio::test]
async fn save_source_on_uses_the_given_date_in_the_path_and_in_ingested_at() {
    let (_d, vault) = vault_with(true, false).await;
    let date = NaiveDate::from_ymd_opt(2001, 12, 31).unwrap();
    let path = vault.save_source_on(date, &request("x\n")).await.unwrap();
    assert!(path.to_string().contains("2001-12-31-"), "{path}");
    let v = vault.get_property(&path, "ingested_at").await.unwrap();
    assert_eq!(v, Some(Some(PropertyValue::Date(date))));
}

#[test]
fn slug_of_an_ascii_title() {
    assert_eq!(source_slug("Hello, World! 2", "x"), "hello-world-2");
}

#[test]
fn slug_of_emoji_only_falls_back() {
    assert_eq!(source_slug("🎉🎉", "Fall Back"), "fall-back");
}

#[test]
fn slug_keeps_non_latin_letters() {
    assert_eq!(source_slug("Привет Мир 日本語", "x"), "привет-мир-日本語");
}

#[test]
fn slug_is_cut_at_60_characters() {
    let slug = source_slug(&"a".repeat(500), "x");
    assert_eq!(slug.chars().count(), 60);
}

#[test]
fn slug_cut_on_a_multibyte_character_does_not_panic() {
    let title = format!("{}日日日", "a".repeat(58));
    let slug = source_slug(&title, "x");
    assert_eq!(slug.chars().count(), 60);
    let dash = format!("{}-bbb", "a".repeat(59));
    assert_eq!(source_slug(&dash, "x"), "a".repeat(59));
}

#[test]
fn slug_of_punctuation_falls_back_and_empty_fallback_gives_source() {
    assert_eq!(source_slug("?!?", "https://a.b"), "https-a-b");
    assert_eq!(source_slug("?!?", ""), "source");
    assert_eq!(source_slug("", "..."), "source");
    assert!(VaultPath::is_valid(source_slug("🎉", "")));
}
