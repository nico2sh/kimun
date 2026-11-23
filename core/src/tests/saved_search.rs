// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// lib.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and these TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::*;
use tempfile::TempDir;

async fn make_vault(dir: &std::path::Path) -> NoteVault {
    NoteVault::new(VaultConfig::new(crate::system::sys(dir)))
        .await
        .unwrap()
}

#[tokio::test]
async fn saved_search_crud() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;

    assert!(vault.list_saved_searches().await.unwrap().is_empty());

    vault.save_search("todo", "#todo").await.unwrap();
    vault.save_search("links", ">{note}").await.unwrap();
    let all = vault.list_saved_searches().await.unwrap();
    assert_eq!(all.len(), 2);

    // Upsert by case-insensitive name: overwrites, no duplicate.
    vault.save_search("Todo", "#todo #urgent").await.unwrap();
    let all = vault.list_saved_searches().await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        all.iter()
            .find(|s| s.name.eq_ignore_ascii_case("todo"))
            .unwrap()
            .query,
        "#todo #urgent"
    );

    vault
        .rename_saved_search("links", "backlinks")
        .await
        .unwrap();
    assert!(vault
        .list_saved_searches()
        .await
        .unwrap()
        .iter()
        .any(|s| s.name == "backlinks"));

    vault.delete_saved_search("todo").await.unwrap();
    let all = vault.list_saved_searches().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "backlinks");
}

#[tokio::test]
async fn suggest_by_prefix_filters_case_insensitively_and_caps() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.save_search("Today", "#today").await.unwrap();
    vault.save_search("todo-week", "#todo").await.unwrap();
    vault.save_search("journal", "in:journal").await.unwrap();

    // Case-insensitive prefix match; "journal" excluded.
    let hits = vault
        .suggest_saved_searches_by_prefix("TO", 9)
        .await
        .unwrap();
    let names: Vec<&str> = hits.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["Today", "todo-week"]);

    // Empty prefix lists all (in stored order).
    assert_eq!(
        vault
            .suggest_saved_searches_by_prefix("", 9)
            .await
            .unwrap()
            .len(),
        3
    );

    // Limit caps the result.
    assert_eq!(
        vault
            .suggest_saved_searches_by_prefix("", 2)
            .await
            .unwrap()
            .len(),
        2
    );

    // No match → empty.
    assert!(vault
        .suggest_saved_searches_by_prefix("zzz", 9)
        .await
        .unwrap()
        .is_empty());
}
