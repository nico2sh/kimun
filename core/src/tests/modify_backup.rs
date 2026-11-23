// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// lib.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and these TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::error::VaultError;
use crate::nfs::VaultPath;
use crate::{NoteVault, VaultConfig};
use std::path::PathBuf;

async fn backup_vault() -> (tempfile::TempDir, NoteVault) {
    let temp = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp.path())).with_backup(true))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    (temp, vault)
}

fn backups_dir_today(workspace: &crate::system::SystemPath) -> PathBuf {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    workspace
        .as_path()
        .join(".kimun")
        .join("backups")
        .join(date)
}

// ---- toggle_task ----

#[tokio::test]
async fn toggle_task_flips_the_checkbox_and_saves() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "intro\n- [ ] one\n").await.unwrap();

    vault.toggle_task(&p, 1, "- [ ] one").await.unwrap();

    assert_eq!(vault.get_note_text(&p).await.unwrap(), "intro\n- [x] one\n");
}

#[tokio::test]
async fn toggle_task_keeps_the_endings_of_a_crlf_note() {
    let temp = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    let file = temp.path().join("note.md");
    tokio::fs::write(&file, "intro\r\n- [ ] one\r\n")
        .await
        .unwrap();
    let p = VaultPath::new("note.md");

    vault.toggle_task(&p, 1, "- [ ] one").await.unwrap();

    let after = tokio::fs::read_to_string(&file).await.unwrap();
    assert_eq!(after, "intro\r\n- [x] one\r\n");
}

#[tokio::test]
async fn toggle_task_refuses_a_note_that_changed() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "- [ ] new text\n").await.unwrap();

    let err = vault
        .toggle_task(&p, 0, "- [ ] old text")
        .await
        .unwrap_err();

    assert!(matches!(err, VaultError::TaskChanged { .. }));
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "- [ ] new text\n");
}

// ---- replace_in_note ----

#[tokio::test]
async fn replace_swaps_unique_substring() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "hello world").await.unwrap();

    let n = vault
        .replace_in_note(&p, "world", "there", false, false)
        .await
        .unwrap();

    assert_eq!(n, 1);
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "hello there");
}

#[tokio::test]
async fn replace_errors_when_absent() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "hello").await.unwrap();

    let e = vault
        .replace_in_note(&p, "nope", "x", false, false)
        .await
        .unwrap_err();

    assert!(matches!(e, VaultError::ReplaceTextNotFound { .. }));
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "hello");
}

#[tokio::test]
async fn replace_errors_when_not_unique() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "a a a").await.unwrap();

    let e = vault
        .replace_in_note(&p, "a", "b", false, false)
        .await
        .unwrap_err();

    assert!(matches!(e, VaultError::ReplaceTextNotUnique { .. }));
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "a a a");
}

#[tokio::test]
async fn replace_all_replaces_every_occurrence() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "a a a").await.unwrap();

    let n = vault
        .replace_in_note(&p, "a", "b", true, false)
        .await
        .unwrap();

    assert_eq!(n, 3);
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "b b b");
}

#[tokio::test]
async fn replace_regex_unique_swaps_match() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "version 1.2.3 here").await.unwrap();

    let n = vault
        .replace_in_note(&p, r"\d+\.\d+\.\d+", "9.9.9", false, true)
        .await
        .unwrap();

    assert_eq!(n, 1);
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "version 9.9.9 here");
}

#[tokio::test]
async fn replace_regex_all_with_capture_groups() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "a1 b2 c3").await.unwrap();

    let n = vault
        .replace_in_note(&p, r"([a-z])(\d)", "$2$1", true, true)
        .await
        .unwrap();

    assert_eq!(n, 3);
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "1a 2b 3c");
}

#[tokio::test]
async fn replace_regex_not_unique_errors() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "cat cot cut").await.unwrap();

    let e = vault
        .replace_in_note(&p, r"c.t", "X", false, true)
        .await
        .unwrap_err();

    assert!(matches!(e, VaultError::ReplaceTextNotUnique { .. }));
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "cat cot cut");
}

#[tokio::test]
async fn replace_regex_invalid_pattern_errors_and_leaves_note() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "untouched").await.unwrap();

    let e = vault
        .replace_in_note(&p, "(unclosed", "y", false, true)
        .await
        .unwrap_err();

    assert!(matches!(e, VaultError::InvalidRegex { .. }));
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "untouched");
}

#[tokio::test]
async fn replace_literal_treats_metachars_literally() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "a.b and axb").await.unwrap();

    // Literal mode: "a.b" matches only the literal, not the regex "a<any>b".
    let n = vault
        .replace_in_note(&p, "a.b", "Z", false, false)
        .await
        .unwrap();

    assert_eq!(n, 1);
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "Z and axb");
}

#[tokio::test]
async fn preview_replace_reports_result_without_writing() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "hello world").await.unwrap();

    let pv = vault
        .preview_replace(&p, "world", "there", false, false)
        .await
        .unwrap();
    assert_eq!(pv.count, 1);
    assert_eq!(pv.content, "hello there");

    // The note on disk is untouched.
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "hello world");
}

#[tokio::test]
async fn preview_replace_surfaces_same_errors() {
    let (_t, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "a a").await.unwrap();

    let e = vault
        .preview_replace(&p, "a", "b", false, false)
        .await
        .unwrap_err();
    assert!(matches!(e, VaultError::ReplaceTextNotUnique { .. }));
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "a a");
}

// ---- backups ----

#[tokio::test]
async fn overwrite_backs_up_previous_content_when_enabled() {
    let (temp, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "original").await.unwrap();

    vault.save_note(&p, "updated").await.unwrap();

    let backup = backups_dir_today(&crate::system::sys(temp.path())).join("note.md");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "original");
    assert_eq!(vault.get_note_text(&p).await.unwrap(), "updated");
}

#[tokio::test]
async fn overwrite_does_not_back_up_when_disabled() {
    let temp = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "original").await.unwrap();

    vault.save_note(&p, "updated").await.unwrap();

    assert!(!temp.path().join(".kimun").join("backups").exists());
}

#[tokio::test]
async fn delete_backs_up_when_enabled() {
    let (temp, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "content").await.unwrap();

    vault.delete_note(&p).await.unwrap();

    let backup = backups_dir_today(&crate::system::sys(temp.path())).join("note.md");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "content");
}

#[tokio::test]
async fn repeat_same_day_edit_keeps_every_backup() {
    let (temp, vault) = backup_vault().await;
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "v0").await.unwrap();
    vault.save_note(&p, "v1").await.unwrap();
    vault.save_note(&p, "v2").await.unwrap();

    let dir = backups_dir_today(&crate::system::sys(temp.path()));
    let count = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(count, 2, "both pre-images should be retained");
}

#[tokio::test]
async fn purge_removes_backups_older_than_retention() {
    let (temp, vault) = backup_vault().await;
    let old = temp
        .path()
        .join(".kimun")
        .join("backups")
        .join("2000-01-01");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("ancient.md"), "x").unwrap();

    // Any backup write triggers the lazy purge sweep.
    let p = VaultPath::new("note.md");
    vault.create_note(&p, "a").await.unwrap();
    vault.save_note(&p, "b").await.unwrap();

    assert!(!old.exists(), "stale date-dir should be purged");
    assert!(
        backups_dir_today(&crate::system::sys(temp.path())).exists(),
        "today's backup is kept"
    );
}

#[tokio::test]
async fn backups_are_not_indexed() {
    let (_temp, vault) = backup_vault().await;
    let p = VaultPath::note_path_from("/note.md");
    vault.create_note(&p, "live").await.unwrap();
    vault.save_note(&p, "changed").await.unwrap();

    // Re-scan the filesystem; the walker must skip the hidden .kimun dir.
    vault.validate_and_init().await.unwrap();
    let notes = vault.get_all_notes().await.unwrap();

    let paths: Vec<String> = notes.iter().map(|(e, _)| e.path.to_string()).collect();
    // The walker must skip the hidden `.kimun` backups dir: no indexed note
    // may point into it.
    assert!(
        paths
            .iter()
            .all(|p| !p.contains(".kimun") && !p.contains("backups")),
        "backup files must not be indexed: {paths:?}"
    );
    // The live note is still indexed.
    assert!(
        paths.iter().any(|p| p.contains("note")),
        "live note should be indexed: {paths:?}"
    );
}

#[tokio::test]
async fn rename_backs_up_backlink_victims() {
    let (temp, vault) = backup_vault().await;
    let b = VaultPath::note_path_from("/b.md");
    let a = VaultPath::note_path_from("/a.md");
    vault.create_note(&b, "I am b").await.unwrap();
    vault
        .create_note(&a, "see [[b]] for details")
        .await
        .unwrap();

    vault
        .rename_note(&b, &VaultPath::note_path_from("/c.md"))
        .await
        .unwrap();

    // The rename rewrites a's link b -> c. a's pre-rewrite content (still
    // pointing at b) must be backed up, since the rewrite goes through nfs
    // directly rather than NoteVault::save_note.
    let backup = backups_dir_today(&crate::system::sys(temp.path())).join("a.md");
    let content = std::fs::read_to_string(&backup)
        .unwrap_or_else(|e| panic!("victim backup a.md should exist: {e}"));
    assert!(
        content.contains("[[b]]"),
        "backup should hold the pre-rewrite content: {content:?}"
    );
    // And the live note was actually rewritten to point at c.
    assert!(vault.get_note_text(&a).await.unwrap().contains("[[c]]"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_replaces_do_not_lose_updates() {
    let (_temp, vault) = backup_vault().await;
    let path = VaultPath::note_path_from("/note.md");
    let tokens = ["a", "b", "c", "d", "e", "f", "g", "h"];
    vault.create_note(&path, tokens.join(" ")).await.unwrap();

    // Fire every replace concurrently against the same note. Each does a
    // read-modify-write; without the per-note lock, racing reads would drop
    // most updates (last-writer-wins). Clones share the lock map.
    let mut handles = Vec::new();
    for t in tokens {
        let v = vault.clone();
        let p = path.clone();
        handles.push(tokio::spawn(async move {
            v.replace_in_note(&p, t, &t.to_uppercase(), false, false)
                .await
                .unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    // All eight replacements survived; none lost to a racing writer.
    assert_eq!(vault.get_note_text(&path).await.unwrap(), "A B C D E F G H");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_appends_do_not_lose_updates() {
    let (_temp, vault) = backup_vault().await;
    let path = VaultPath::note_path_from("/log.md");
    let lines = ["a", "b", "c", "d", "e", "f", "g", "h"];

    // append_to_note does its read-or-create + write under the per-note lock,
    // so concurrent appends to one note all land (none lost to a racing read).
    let mut handles = Vec::new();
    for l in lines {
        let v = vault.clone();
        let p = path.clone();
        handles.push(tokio::spawn(async move {
            v.append_to_note(&p, l, None).await.unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let text = vault.get_note_text(&path).await.unwrap();
    assert_eq!(text.lines().count(), lines.len(), "lines: {text:?}");
    for l in lines {
        assert!(text.contains(l), "missing {l} in {text:?}");
    }
}
