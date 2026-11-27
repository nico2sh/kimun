// Gated with the inner attribute: TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::nfs::wiki_declaration::write_wiki_declaration;
use crate::nfs::VaultPath;
use crate::note::{PropertyDateTime, PropertyValue};
use crate::wiki::is_stale;
use crate::{
    NoteVault, Severity, VaultConfig, WikiDeclaration, WikiFinding, WikiInitOptions, WikiReport,
    WikiRule,
};
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

/// A wiki page with every default required property set.
fn page(sources: &str, updated: &str, body: &str) -> String {
    format!("---\nsummary: A page.\nsources: {sources}\nupdated: {updated}\n---\n{body}\n")
}

fn of_rule(report: &WikiReport, rule: WikiRule) -> Vec<&WikiFinding> {
    report.findings.iter().filter(|f| f.rule == rule).collect()
}

fn paths(findings: &[&WikiFinding]) -> Vec<VaultPath> {
    findings.iter().map(|f| f.path.clone()).collect()
}

#[tokio::test]
async fn orphan_page_is_reported_and_linked_page_is_not() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[]", "2026-01-01", "See [[b]]."),
    )
    .await;
    add(&vault, "/wiki/b.md", &page("[]", "2026-01-01", "Plain.")).await;
    let report = vault.wiki_check().await.unwrap();
    let orphans = of_rule(&report, WikiRule::Orphan);
    assert_eq!(paths(&orphans), vec![VaultPath::new("/wiki/a.md")]);
    assert_eq!(orphans[0].severity, Severity::Problem);
}

#[tokio::test]
async fn self_link_does_not_save_an_orphan() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[]", "2026-01-01", "Me: [[a]]."),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let orphans = of_rule(&report, WikiRule::Orphan);
    assert_eq!(paths(&orphans), vec![VaultPath::new("/wiki/a.md")]);
}

#[tokio::test]
async fn unresolved_wikilink_is_reported_with_the_missing_name() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[]", "2026-01-01", "[[missing-page]] and [[missing-page]]."),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let unresolved = of_rule(&report, WikiRule::UnresolvedLink);
    assert_eq!(unresolved.len(), 1, "{:?}", report.findings);
    assert_eq!(unresolved[0].path, VaultPath::new("/wiki/a.md"));
    assert!(unresolved[0].message.contains("missing-page"));
}

#[tokio::test]
async fn ambiguous_wikilink_is_reported() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/wiki/x/dup.md", &page("[]", "2026-01-01", "X.")).await;
    add(&vault, "/wiki/y/dup.md", &page("[]", "2026-01-01", "Y.")).await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[]", "2026-01-01", "See [[dup]]."),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let ambiguous = of_rule(&report, WikiRule::AmbiguousLink);
    assert_eq!(paths(&ambiguous), vec![VaultPath::new("/wiki/a.md")]);
    assert!(ambiguous[0].message.contains("dup"));
}

#[tokio::test]
async fn missing_required_property_is_reported_once_for_each_name() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/a.md",
        "---\nsummary: Only this.\n---\nBody\n",
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let missing = of_rule(&report, WikiRule::MissingProperty);
    assert_eq!(missing.len(), 2, "{:?}", report.findings);
    assert!(missing.iter().any(|f| f.message.contains("sources")));
    assert!(missing.iter().any(|f| f.message.contains("updated")));
    assert!(missing.iter().all(|f| !f.message.contains("summary")));
}

#[tokio::test]
async fn source_that_does_not_exist_is_reported() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[\"[[nope]]\"]", "2026-01-01", "Body"),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let missing = of_rule(&report, WikiRule::MissingSource);
    assert_eq!(paths(&missing), vec![VaultPath::new("/wiki/a.md")]);
    assert!(missing[0].message.contains("nope"));
    // The frontmatter link is not also an unresolved body link.
    assert!(of_rule(&report, WikiRule::UnresolvedLink).is_empty());
}

#[tokio::test]
async fn source_item_that_is_not_a_note_link_is_reported() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[\"https://example.com\"]", "2026-01-01", "Body"),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let missing = of_rule(&report, WikiRule::MissingSource);
    assert_eq!(paths(&missing), vec![VaultPath::new("/wiki/a.md")]);
    assert!(missing[0].message.contains("https://example.com"));
}

#[tokio::test]
async fn stale_page_is_reported_when_a_cited_source_is_newer() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/sources/s.md",
        "---\ningested_at: 2026-02-01\n---\nText\n",
    )
    .await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[\"[[s]]\"]", "2026-01-15", "Body"),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    assert!(of_rule(&report, WikiRule::MissingSource).is_empty());
    let stale = of_rule(&report, WikiRule::StalePage);
    assert_eq!(paths(&stale), vec![VaultPath::new("/wiki/a.md")]);
    assert!(stale[0].message.contains("/sources/s.md"));
}

#[tokio::test]
async fn same_day_is_not_stale() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/sources/s.md",
        "---\ningested_at: 2026-02-01T23:00:00\n---\nText\n",
    )
    .await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[\"[[s]]\"]", "2026-02-01", "Body"),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    assert!(of_rule(&report, WikiRule::StalePage).is_empty());
}

fn date(y: i32, m: u32, d: u32) -> PropertyValue {
    PropertyValue::Date(chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap())
}

fn date_time(y: i32, m: u32, d: u32, h: u32) -> PropertyValue {
    let naive = chrono::NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(h, 0, 0)
        .unwrap();
    PropertyValue::DateTime(PropertyDateTime::local(naive))
}

#[test]
fn is_stale_compares_dates() {
    assert!(is_stale(&date(2026, 1, 1), &date(2026, 1, 2)));
    assert!(!is_stale(&date(2026, 1, 2), &date(2026, 1, 2)));
    assert!(!is_stale(&date(2026, 1, 3), &date(2026, 1, 2)));
}

#[test]
fn is_stale_compares_the_date_part_of_date_times() {
    assert!(!is_stale(
        &date_time(2026, 1, 2, 1),
        &date_time(2026, 1, 2, 23)
    ));
    assert!(is_stale(&date_time(2026, 1, 1, 23), &date(2026, 1, 2)));
    assert!(!is_stale(&date(2026, 1, 2), &date_time(2026, 1, 2, 23)));
}

#[test]
fn is_stale_is_false_for_other_value_types() {
    let text = PropertyValue::Text("2026-01-01".to_string());
    assert!(!is_stale(&text, &date(2027, 1, 1)));
    assert!(!is_stale(&date(2020, 1, 1), &PropertyValue::Number(1.0)));
}

#[tokio::test]
async fn edited_source_is_reported_as_modified() {
    let (dir, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "Original text\n").await;
    vault.wiki_stamp().await.unwrap();
    let file = dir.path().join("sources").join("s.md");
    let stamped = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, stamped.replace("Original", "Edited")).unwrap();
    let report = vault.wiki_check().await.unwrap();
    let modified = of_rule(&report, WikiRule::SourceModified);
    assert_eq!(paths(&modified), vec![VaultPath::new("/sources/s.md")]);
    assert_eq!(modified[0].severity, Severity::Problem);
}

#[tokio::test]
async fn unstamped_source_is_info() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "Text\n").await;
    let report = vault.wiki_check().await.unwrap();
    let unstamped = of_rule(&report, WikiRule::Unstamped);
    assert_eq!(paths(&unstamped), vec![VaultPath::new("/sources/s.md")]);
    assert_eq!(unstamped[0].severity, Severity::Info);
    assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
    assert_eq!(report.problems(), 0);
}

#[tokio::test]
async fn crlf_source_is_not_modified_after_stamp() {
    let (dir, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "line one\nline two\n").await;
    vault.wiki_stamp().await.unwrap();
    let file = dir.path().join("sources").join("s.md");
    let stamped = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, stamped.replace('\n', "\r\n")).unwrap();
    let report = vault.wiki_check().await.unwrap();
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

#[tokio::test]
async fn links_inside_sources_are_not_checked_when_declared() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "See [[missing]].\n").await;
    let report = vault.wiki_check().await.unwrap();
    assert!(of_rule(&report, WikiRule::UnresolvedLink).is_empty());
    assert!(of_rule(&report, WikiRule::Orphan).is_empty());
    assert_eq!(report.notes_checked, 1);
}

#[tokio::test]
async fn log_and_schema_are_not_orphans() {
    let (_d, vault) = vault_with(true).await;
    vault.wiki_init(WikiInitOptions::default()).await.unwrap();
    let report = vault.wiki_check().await.unwrap();
    assert!(report.findings.is_empty(), "{:?}", report.findings);
    assert_eq!(report.notes_checked, 0);
}

#[tokio::test]
async fn without_a_declaration_only_the_generic_rules_run_and_journal_is_skipped() {
    let (_d, vault) = vault_with(false).await;
    add(&vault, "/a.md", "See [[missing]].\n").await;
    add(&vault, "/sources/s.md", "No hash.\n").await;
    let journal = vault
        .journal_path()
        .append(&VaultPath::new("2026-01-01.md"));
    let inbox = vault.inbox_path().append(&VaultPath::new("idea.md"));
    add(&vault, &journal.to_string(), "Also [[gone]].\n").await;
    add(&vault, &inbox.to_string(), "And [[lost]].\n").await;
    let report = vault.wiki_check().await.unwrap();
    assert!(!report.declared);
    assert_eq!(report.notes_checked, 2);
    let generic = [
        WikiRule::Orphan,
        WikiRule::UnresolvedLink,
        WikiRule::AmbiguousLink,
    ];
    assert!(
        report.findings.iter().all(|f| generic.contains(&f.rule)),
        "{:?}",
        report.findings
    );
    assert!(report
        .findings
        .iter()
        .all(|f| f.path != journal && f.path != inbox));
    let unresolved = of_rule(&report, WikiRule::UnresolvedLink);
    assert_eq!(paths(&unresolved), vec![VaultPath::new("/a.md")]);
    // Sorted by rule first: the orphans come before the unresolved link.
    assert_eq!(report.findings[0].rule, WikiRule::Orphan);
    assert_eq!(report.findings[0].path, VaultPath::new("/a.md"));
    assert_eq!(report.findings[1].path, VaultPath::new("/sources/s.md"));
}

#[tokio::test]
async fn empty_wiki_gives_no_findings() {
    let (_d, vault) = vault_with(true).await;
    let report = vault.wiki_check().await.unwrap();
    assert!(report.declared);
    assert_eq!(report.notes_checked, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn problems_ignores_info_findings() {
    let finding = |rule, severity| WikiFinding {
        rule,
        severity,
        path: VaultPath::new("/x.md"),
        message: String::new(),
    };
    let report = WikiReport {
        declared: true,
        notes_checked: 2,
        findings: vec![
            finding(WikiRule::UnresolvedLink, Severity::Problem),
            finding(WikiRule::Unstamped, Severity::Info),
        ],
    };
    assert_eq!(report.problems(), 1);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["findings"][0]["rule"], "unresolved_link");
    assert_eq!(json["findings"][0]["severity"], "problem");
    assert_eq!(json["findings"][1]["rule"], "unstamped");
    assert_eq!(json["findings"][0]["path"], "/x.md");
}

#[tokio::test]
async fn unquoted_flow_sources_list_is_reported() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "Text\n").await;
    add(&vault, "/wiki/a.md", &page("[[s]]", "2026-01-01", "Body")).await;
    let report = vault.wiki_check().await.unwrap();
    let missing = of_rule(&report, WikiRule::MissingSource);
    assert_eq!(paths(&missing), vec![VaultPath::new("/wiki/a.md")]);
    assert_eq!(missing[0].severity, Severity::Problem);
    assert!(missing[0].message.contains("not quoted"));
}

#[tokio::test]
async fn unquoted_block_sources_list_is_reported() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "Text\n").await;
    add(
        &vault,
        "/wiki/a.md",
        "---\nsummary: A page.\nsources:\n  - [[s]]\nupdated: 2026-01-01\n---\nBody\n",
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let missing = of_rule(&report, WikiRule::MissingSource);
    assert_eq!(paths(&missing), vec![VaultPath::new("/wiki/a.md")]);
    assert_eq!(missing[0].severity, Severity::Problem);
}

#[tokio::test]
async fn quoted_sources_list_that_resolves_is_not_reported() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "Text\n").await;
    add(
        &vault,
        "/wiki/a.md",
        "---\nsummary: A page.\nsources:\n  - \"[[s]]\"\nupdated: 2026-01-01\n---\nBody\n",
    )
    .await;
    add(
        &vault,
        "/wiki/b.md",
        &page("[\"[[s]]\"]", "2026-01-01", "[[a]]"),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    assert!(of_rule(&report, WikiRule::MissingSource).is_empty());
}

#[tokio::test]
async fn source_with_broken_frontmatter_is_a_problem() {
    let (dir, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "Text\n").await;
    vault.wiki_stamp().await.unwrap();
    let file = dir.path().join("sources").join("s.md");
    let stamped = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, stamped.replacen("---\n", "---\nkey: [unclosed\n", 1)).unwrap();
    let report = vault.wiki_check().await.unwrap();
    assert!(of_rule(&report, WikiRule::Unstamped).is_empty());
    let modified = of_rule(&report, WikiRule::SourceModified);
    assert_eq!(paths(&modified), vec![VaultPath::new("/sources/s.md")]);
    assert_eq!(modified[0].severity, Severity::Problem);
    assert!(modified[0].message.contains("frontmatter does not parse"));
    assert_eq!(report.problems(), 1);
}

#[tokio::test]
async fn source_item_with_an_alias_resolves() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/sources/s.md", "Text\n").await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[\"[[s|The source]]\"]", "2026-01-01", "Body"),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    assert!(of_rule(&report, WikiRule::MissingSource).is_empty());
}

#[tokio::test]
async fn bare_string_sources_is_one_item() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/a.md",
        &page("\"[[nope]]\"", "2026-01-01", "Body"),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let missing = of_rule(&report, WikiRule::MissingSource);
    assert_eq!(paths(&missing), vec![VaultPath::new("/wiki/a.md")]);
    assert!(missing[0].message.contains("nope"));
}

#[tokio::test]
async fn ambiguous_source_is_reported_as_ambiguous_link() {
    let (_d, vault) = vault_with(true).await;
    add(&vault, "/sources/x/dup.md", "X\n").await;
    add(&vault, "/sources/y/dup.md", "Y\n").await;
    add(
        &vault,
        "/wiki/a.md",
        &page("[\"[[dup]]\"]", "2026-01-01", "Body"),
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    assert!(of_rule(&report, WikiRule::MissingSource).is_empty());
    let ambiguous = of_rule(&report, WikiRule::AmbiguousLink);
    assert_eq!(paths(&ambiguous), vec![VaultPath::new("/wiki/a.md")]);
    assert!(ambiguous[0].message.contains("dup"));
}

#[tokio::test]
async fn empty_updated_counts_as_missing() {
    let (_d, vault) = vault_with(true).await;
    add(
        &vault,
        "/wiki/a.md",
        "---\nsummary: A page.\nsources: []\nupdated:\n---\nBody\n",
    )
    .await;
    let report = vault.wiki_check().await.unwrap();
    let missing = of_rule(&report, WikiRule::MissingProperty);
    assert_eq!(missing.len(), 1, "{:?}", report.findings);
    assert!(missing[0].message.contains("updated"));
}
