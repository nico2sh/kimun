// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// lib.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and these TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::nfs::pinned_notes;
use crate::*;
use chrono::{NaiveDate, Utc};
use std::time::Duration;
use tempfile::TempDir;

// Helper: build a NoteVault pointing at a temp directory (no DB needed for pure-text tests).
async fn make_vault(dir: &std::path::Path) -> NoteVault {
    NoteVault::new(VaultConfig::new(crate::system::sys(dir)))
        .await
        .unwrap()
}

// ---- index observer ----

/// Test observer that records every change it is handed.
#[derive(Debug, Default)]
struct RecordingObserver {
    changes: std::sync::Mutex<Vec<NoteChange>>,
}

impl IndexObserver for RecordingObserver {
    fn on_change(&self, change: &NoteChange) {
        self.changes.lock().unwrap().push(change.clone());
    }
}

impl RecordingObserver {
    fn recorded(&self) -> Vec<NoteChange> {
        self.changes.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn create_note_emits_upsert_with_content_hash() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let obs = std::sync::Arc::new(RecordingObserver::default());
    vault.set_index_observer(obs.clone());

    let (_, content) = vault
        .create_note(&VaultPath::new("note.md"), "hello")
        .await
        .unwrap();

    assert_eq!(
        obs.recorded(),
        vec![NoteChange::Upsert {
            // Canonical (vault-absolute) index identity, regardless of the
            // relative form passed to create_note.
            path: VaultPath::new("/note.md"),
            hash: content.hash,
        }]
    );
}

#[tokio::test]
async fn delete_note_emits_delete() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("note.md"), "hello")
        .await
        .unwrap();

    let obs = std::sync::Arc::new(RecordingObserver::default());
    vault.set_index_observer(obs.clone());

    vault.delete_note(&VaultPath::new("note.md")).await.unwrap();

    assert_eq!(
        obs.recorded(),
        vec![NoteChange::Delete {
            path: VaultPath::new("/note.md"),
        }]
    );
}

#[tokio::test]
async fn rename_note_emits_delete_and_upsert() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let (_, content) = vault
        .create_note(&VaultPath::new("old.md"), "hello")
        .await
        .unwrap();

    let obs = std::sync::Arc::new(RecordingObserver::default());
    vault.set_index_observer(obs.clone());

    vault
        .rename_note(&VaultPath::new("old.md"), &VaultPath::new("new.md"))
        .await
        .unwrap();

    let changes = obs.recorded();
    assert!(changes.contains(&NoteChange::Delete {
        path: VaultPath::new("/old.md"),
    }));
    // A rename leaves content untouched, so the upsert carries the same hash.
    assert!(changes.contains(&NoteChange::Upsert {
        path: VaultPath::new("/new.md"),
        hash: content.hash,
    }));
}

#[tokio::test]
async fn rename_directory_emits_moves_for_contained_notes() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let (_, direct) = vault
        .create_note(&VaultPath::new("old_dir/direct.md"), "one")
        .await
        .unwrap();
    let (_, nested) = vault
        .create_note(&VaultPath::new("old_dir/sub/nested.md"), "two")
        .await
        .unwrap();

    let obs = std::sync::Arc::new(RecordingObserver::default());
    vault.set_index_observer(obs.clone());

    vault
        .rename_directory(&VaultPath::new("old_dir"), &VaultPath::new("new_dir"))
        .await
        .unwrap();

    let changes = obs.recorded();
    for (old, new, hash) in [
        ("/old_dir/direct.md", "/new_dir/direct.md", direct.hash),
        (
            "/old_dir/sub/nested.md",
            "/new_dir/sub/nested.md",
            nested.hash,
        ),
    ] {
        assert!(
            changes.contains(&NoteChange::Delete {
                path: VaultPath::new(old),
            }),
            "missing delete for {old}: {changes:?}"
        );
        assert!(
            changes.contains(&NoteChange::Upsert {
                path: VaultPath::new(new),
                hash,
            }),
            "missing upsert for {new}: {changes:?}"
        );
    }
}

#[tokio::test]
async fn delete_directory_emits_delete_per_contained_note() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("gone/a.md"), "a")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::new("gone/sub/b.md"), "b")
        .await
        .unwrap();

    let obs = std::sync::Arc::new(RecordingObserver::default());
    vault.set_index_observer(obs.clone());

    vault
        .delete_directory(&VaultPath::new("gone"))
        .await
        .unwrap();

    let changes = obs.recorded();
    for path in ["/gone/a.md", "/gone/sub/b.md"] {
        assert!(
            changes.contains(&NoteChange::Delete {
                path: VaultPath::new(path),
            }),
            "missing delete for {path}: {changes:?}"
        );
    }
}

#[tokio::test]
async fn clear_index_observer_if_only_clears_the_matching_observer() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;

    let a: std::sync::Arc<dyn IndexObserver> = std::sync::Arc::new(RecordingObserver::default());
    let b_inner = std::sync::Arc::new(RecordingObserver::default());
    let b: std::sync::Arc<dyn IndexObserver> = b_inner.clone();

    // B replaces A as the (zero-or-one) observer.
    vault.set_index_observer(a.clone());
    vault.set_index_observer(b.clone());

    // A superseded handle deregistering itself must NOT wipe B.
    vault.clear_index_observer_if(&a);

    vault
        .create_note(&VaultPath::new("note.md"), "hello")
        .await
        .unwrap();

    assert_eq!(
        b_inner.recorded().len(),
        1,
        "the surviving observer B must still receive events"
    );

    // Now B deregisters itself → no observer remains.
    vault.clear_index_observer_if(&b);
    vault
        .create_note(&VaultPath::new("note2.md"), "world")
        .await
        .unwrap();
    assert_eq!(
        b_inner.recorded().len(),
        1,
        "after B clears itself it must receive no further events"
    );
}

#[tokio::test]
async fn create_then_sync_keeps_one_row_in_canonical_form() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("sub/note.md"), "hi")
        .await
        .unwrap();

    // A full sync (as on vault open) must recognise the just-created note as
    // the same note — not delete the cached row and re-add it under a
    // different path form.
    vault.index_notes(NotesValidation::Full).await.unwrap();

    let all = vault.get_all_notes().await.unwrap();
    assert_eq!(all.len(), 1, "create + sync must leave exactly one row");
    assert_eq!(
        all[0].0.path,
        VaultPath::new("/sub/note.md"),
        "stored path must be the single canonical (vault-absolute) form"
    );
}

#[tokio::test]
async fn lookup_finds_note_regardless_of_path_form() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("sub/note.md"), "# Title\n\nbody")
        .await
        .unwrap();

    // The note is stored under its canonical key; a lookup by any form of
    // the same path must resolve to it.
    for form in ["sub/note.md", "/sub/note.md"] {
        let chunks = vault.get_note_chunks(&VaultPath::new(form)).await.unwrap();
        assert!(
            !chunks.is_empty(),
            "get_note_chunks({form}) should find the note"
        );
    }
}

#[tokio::test]
async fn vault_id_is_stable_across_reopen_and_persisted() {
    let dir = TempDir::new().unwrap();
    let id1 = make_vault(dir.path()).await.vault_id().await.unwrap();
    let id2 = make_vault(dir.path()).await.vault_id().await.unwrap();
    assert_eq!(id1, id2, "reopening the vault must keep the same id");
    assert!(
        dir.path().join(".kimun").join("vault-id").exists(),
        "vault id must be persisted under .kimun/"
    );
}

#[tokio::test]
async fn writes_succeed_without_observer() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    // No observer registered: every mutation path must remain a no-op emit.
    vault
        .create_note(&VaultPath::new("n.md"), "x")
        .await
        .unwrap();
    vault.save_note(&VaultPath::new("n.md"), "y").await.unwrap();
    vault.delete_note(&VaultPath::new("n.md")).await.unwrap();
}

#[tokio::test]
async fn bulk_reindex_emits_upsert_per_note() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("a.md"), "aaa")
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::new("b.md"), "bbb")
        .await
        .unwrap();

    let obs = std::sync::Arc::new(RecordingObserver::default());
    vault.set_index_observer(obs.clone());

    // Full rebuild goes through the bulk apply path, not per-note save.
    vault.recreate_index().await.unwrap();

    let mut got: Vec<(String, bool)> = obs
        .recorded()
        .into_iter()
        .map(|c| match c {
            NoteChange::Upsert { path, .. } => (path.to_string(), true),
            NoteChange::Delete { path } => (path.to_string(), false),
        })
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![("/a.md".to_string(), true), ("/b.md".to_string(), true)]
    );
}

// ---- attachments ----

#[tokio::test]
async fn entry_kind_classifies_note_directory_and_attachment() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;

    vault
        .create_note(&VaultPath::new("note.md"), "hi")
        .await
        .unwrap();
    vault
        .create_directory(&VaultPath::new("sub"))
        .await
        .unwrap();
    vault
        .save_attachment(&VaultPath::new("assets/diagram.png"), &[1, 2, 3])
        .await
        .unwrap();

    assert_eq!(
        vault.entry_kind(&VaultPath::new("note.md")).await.unwrap(),
        EntryKind::Note
    );
    assert_eq!(
        vault.entry_kind(&VaultPath::new("sub")).await.unwrap(),
        EntryKind::Directory
    );
    assert_eq!(
        vault
            .entry_kind(&VaultPath::new("assets/diagram.png"))
            .await
            .unwrap(),
        EntryKind::Attachment
    );
}

#[tokio::test]
async fn list_attachments_returns_attachments_and_no_notes() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("sub/note.md"), "hi")
        .await
        .unwrap();
    vault
        .save_attachment(&VaultPath::new("assets/Diagram.png"), &[1])
        .await
        .unwrap();
    vault
        .save_attachment(&VaultPath::new("sub/doc.pdf"), &[2])
        .await
        .unwrap();

    // A vault without a configured index keeps it inside the vault
    // (`IndexFile::legacy_in_workspace`), and a browse reports it as an
    // attachment too, so it is not asserted away here.
    let found = vault.list_attachments();
    assert!(found.contains(&VaultPath::new("/assets/diagram.png")));
    assert!(found.contains(&VaultPath::new("/sub/doc.pdf")));
    assert!(!found.contains(&VaultPath::new("/sub/note.md")));
    assert!(found.iter().all(|path| !path.is_note()));
}

#[tokio::test]
async fn entry_kind_errors_on_missing_path() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    assert!(vault.entry_kind(&VaultPath::new("nope.png")).await.is_err());
}

#[tokio::test]
async fn attachment_details_reads_text_with_metadata() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let body = "first line\nsecond line\n";
    vault
        .save_attachment(&VaultPath::new("notes.txt"), body.as_bytes())
        .await
        .unwrap();

    let d = vault
        .get_attachment_details(&VaultPath::new("notes.txt"))
        .await
        .unwrap();
    assert_eq!(d.size, body.len() as u64);
    assert_eq!(d.extension.as_deref(), Some("txt"));
    assert_eq!(
        d.content,
        AttachmentContent::Text {
            text: body.to_string(),
            truncated: false,
        }
    );
}

#[tokio::test]
async fn attachment_details_reads_text_larger_than_sniff_window() {
    // Exercises the two-phase read: a text file bigger than the 64 KiB
    // sniff window must be read in full (phase 1 sniff + phase 2 remainder),
    // not truncated at the sniff boundary.
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let body = "abcd\n".repeat(40_000); // ~200 KB, 40k lines
    vault
        .save_attachment(&VaultPath::new("big.txt"), body.as_bytes())
        .await
        .unwrap();

    let d = vault
        .get_attachment_details(&VaultPath::new("big.txt"))
        .await
        .unwrap();
    assert_eq!(
        d.content,
        AttachmentContent::Text {
            text: body.clone(),
            truncated: false,
        }
    );
}

#[tokio::test]
async fn attachment_details_detects_nul_past_sniff_window() {
    // A file that is text through the 64 KiB sniff window but holds a NUL
    // later must still classify as binary (full-buffer decode).
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let mut bytes = vec![b'a'; 100_000];
    bytes.push(0); // NUL well past the 64 KiB sniff window
    vault
        .save_attachment(&VaultPath::new("late.bin"), &bytes)
        .await
        .unwrap();

    let d = vault
        .get_attachment_details(&VaultPath::new("late.bin"))
        .await
        .unwrap();
    assert_eq!(d.content, AttachmentContent::Binary);
}

#[tokio::test]
async fn attachment_details_flags_binary_content() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    // A NUL byte makes it binary regardless of surrounding ASCII.
    vault
        .save_attachment(&VaultPath::new("blob.bin"), &[0x50, 0x00, 0x4b])
        .await
        .unwrap();

    let d = vault
        .get_attachment_details(&VaultPath::new("blob.bin"))
        .await
        .unwrap();
    assert_eq!(d.content, AttachmentContent::Binary);
}

#[tokio::test]
async fn attachment_details_extension_none_for_extensionless_file() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .save_attachment(&VaultPath::new("LICENSE"), b"MIT")
        .await
        .unwrap();
    let d = vault
        .get_attachment_details(&VaultPath::new("LICENSE"))
        .await
        .unwrap();
    assert_eq!(d.extension, None);
}

#[tokio::test]
async fn rename_attachment_moves_file_preserving_content() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .save_attachment(&VaultPath::new("assets/old.png"), &[9, 8, 7])
        .await
        .unwrap();

    vault
        .rename_attachment(
            &VaultPath::new("assets/old.png"),
            &VaultPath::new("pics/new.png"),
        )
        .await
        .unwrap();

    assert!(vault
        .entry_kind(&VaultPath::new("assets/old.png"))
        .await
        .is_err());
    let d = vault
        .get_attachment_details(&VaultPath::new("pics/new.png"))
        .await
        .unwrap();
    assert_eq!(d.size, 3);
}

#[tokio::test]
async fn rename_attachment_fails_when_destination_exists() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .save_attachment(&VaultPath::new("a.png"), &[1])
        .await
        .unwrap();
    vault
        .save_attachment(&VaultPath::new("b.png"), &[2])
        .await
        .unwrap();

    assert!(vault
        .rename_attachment(&VaultPath::new("a.png"), &VaultPath::new("b.png"))
        .await
        .is_err());
}

#[tokio::test]
async fn delete_attachment_removes_file() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .save_attachment(&VaultPath::new("gone.png"), &[1, 2])
        .await
        .unwrap();

    vault
        .delete_attachment(&VaultPath::new("gone.png"))
        .await
        .unwrap();
    assert!(vault.entry_kind(&VaultPath::new("gone.png")).await.is_err());
}

#[tokio::test]
async fn delete_entry_dispatches_by_kind() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("note.md"), "hi")
        .await
        .unwrap();
    vault
        .create_directory(&VaultPath::new("sub"))
        .await
        .unwrap();
    vault
        .save_attachment(&VaultPath::new("pic.png"), &[1])
        .await
        .unwrap();

    for p in ["note.md", "sub", "pic.png"] {
        vault.delete_entry(&VaultPath::new(p)).await.unwrap();
        assert!(
            vault.entry_kind(&VaultPath::new(p)).await.is_err(),
            "{p} should be gone"
        );
    }
    // Missing entry surfaces an error rather than silently succeeding.
    assert!(vault.delete_entry(&VaultPath::new("nope")).await.is_err());
}

#[tokio::test]
async fn rename_entry_dispatches_by_kind() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("note.md"), "hi")
        .await
        .unwrap();
    vault
        .save_attachment(&VaultPath::new("pic.png"), &[1, 2])
        .await
        .unwrap();

    vault
        .rename_entry(&VaultPath::new("note.md"), &VaultPath::new("renamed.md"))
        .await
        .unwrap();
    vault
        .rename_entry(&VaultPath::new("pic.png"), &VaultPath::new("img/photo.png"))
        .await
        .unwrap();

    assert_eq!(
        vault
            .entry_kind(&VaultPath::new("renamed.md"))
            .await
            .unwrap(),
        EntryKind::Note
    );
    assert_eq!(
        vault
            .entry_kind(&VaultPath::new("img/photo.png"))
            .await
            .unwrap(),
        EntryKind::Attachment
    );
    assert!(vault.entry_kind(&VaultPath::new("pic.png")).await.is_err());
}

#[tokio::test]
async fn get_markdown_and_links_resolves_relative_image() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;

    std::fs::create_dir_all(dir.path().join("directory")).unwrap();
    std::fs::write(dir.path().join("directory/note.md"), "![alt](../photo.png)").unwrap();

    let md_note = vault
        .get_markdown_and_links(&VaultPath::new("/directory/note.md"))
        .await
        .unwrap();

    let expected_os_path = dir.path().join("photo.png").display().to_string();
    assert_eq!(md_note.text, format!("![alt]({})", expected_os_path));
    assert_eq!(1, md_note.links.len());
    let link = &md_note.links[0];
    assert_eq!(link.ltype, note::LinkType::Image(expected_os_path));
    assert_eq!(link.text, "alt");
    assert_eq!(link.raw_link, "../photo.png");
}

#[tokio::test]
async fn get_markdown_and_links_resolves_absolute_vault_image() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;

    std::fs::create_dir_all(dir.path().join("notes")).unwrap();
    std::fs::write(
        dir.path().join("notes/note.md"),
        "![banner](/assets/banner.png)",
    )
    .unwrap();

    let md_note = vault
        .get_markdown_and_links(&VaultPath::new("/notes/note.md"))
        .await
        .unwrap();

    let expected_os_path = dir
        .path()
        .join("assets")
        .join("banner.png")
        .display()
        .to_string();
    assert_eq!(md_note.text, format!("![banner]({})", expected_os_path));
    assert!(matches!(
        &md_note.links[0].ltype,
        note::LinkType::Image(p) if *p == expected_os_path
    ));
}

#[tokio::test]
async fn get_markdown_and_links_keeps_external_image_url() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;

    let url = "https://example.com/img.png";
    std::fs::write(dir.path().join("note.md"), format!("![remote]({})", url)).unwrap();

    let md_note = vault
        .get_markdown_and_links(&VaultPath::new("/note.md"))
        .await
        .unwrap();

    assert_eq!(md_note.text, format!("![remote]({})", url));
    assert!(matches!(
        &md_note.links[0].ltype,
        note::LinkType::Image(p) if p == url
    ));
    assert_eq!(md_note.links[0].raw_link, url);
}

#[tokio::test]
async fn get_markdown_and_links_mixed_content() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;

    std::fs::write(
        dir.path().join("note.md"),
        "[[Other Note]] [link](other.md) ![img](photo.png) #tag",
    )
    .unwrap();

    let md_note = vault
        .get_markdown_and_links(&VaultPath::new("/note.md"))
        .await
        .unwrap();

    assert_eq!(
        1,
        md_note
            .links
            .iter()
            .filter(|l| matches!(l.ltype, note::LinkType::Image(_)))
            .count()
    );
    assert_eq!(
        2,
        md_note
            .links
            .iter()
            .filter(|l| matches!(l.ltype, note::LinkType::Note(_)))
            .count()
    );
    assert_eq!(
        1,
        md_note
            .links
            .iter()
            .filter(|l| matches!(l.ltype, note::LinkType::Hashtag))
            .count()
    );
}

// ---- rename_note: backlink rewriting integration tests ----

/// Create a small vault with a DB, write two notes, index them, then rename one
/// and assert that the other note's content and DB links are updated.
async fn setup_vault_with_notes(dir: &std::path::Path) -> NoteVault {
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir)))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    vault
}

#[tokio::test]
async fn rename_note_updates_wikilink_in_backlink() {
    let dir = TempDir::new().unwrap();
    let vault = setup_vault_with_notes(dir.path()).await;

    // Create the note that will be renamed
    vault
        .save_note(&VaultPath::new("/target.md"), "# Target note")
        .await
        .unwrap();
    // Create a note that links to it via wikilink
    vault
        .save_note(
            &VaultPath::new("/referrer.md"),
            "# Referrer\nSee [[target]].",
        )
        .await
        .unwrap();

    vault
        .rename_note(
            &VaultPath::new("/target.md"),
            &VaultPath::new("/renamed.md"),
        )
        .await
        .unwrap();

    // The referrer file on disk must now use [[renamed]]
    let updated = nfs::load_note(
        &crate::system::sys(dir.path()),
        &VaultPath::new("/referrer.md"),
    )
    .await
    .unwrap();
    assert!(
        updated.contains("[[renamed]]"),
        "expected [[renamed]] in: {updated}"
    );
    assert!(
        !updated.contains("[[target]]"),
        "old wikilink still present in: {updated}"
    );
}

#[tokio::test]
async fn rename_note_updates_markdown_link_in_backlink() {
    let dir = TempDir::new().unwrap();
    let vault = setup_vault_with_notes(dir.path()).await;

    vault
        .save_note(&VaultPath::new("/target.md"), "# Target note")
        .await
        .unwrap();
    vault
        .save_note(
            &VaultPath::new("/referrer.md"),
            "# Referrer\n[link](/target.md) end.",
        )
        .await
        .unwrap();

    vault
        .rename_note(
            &VaultPath::new("/target.md"),
            &VaultPath::new("/renamed.md"),
        )
        .await
        .unwrap();

    let updated = nfs::load_note(
        &crate::system::sys(dir.path()),
        &VaultPath::new("/referrer.md"),
    )
    .await
    .unwrap();
    assert!(
        updated.contains("[link](/renamed.md)"),
        "expected updated link in: {updated}"
    );
    assert!(
        !updated.contains("/target.md"),
        "old path still present in: {updated}"
    );
}

#[tokio::test]
async fn rename_note_does_not_touch_unrelated_notes() {
    let dir = TempDir::new().unwrap();
    let vault = setup_vault_with_notes(dir.path()).await;

    vault
        .save_note(&VaultPath::new("/target.md"), "# Target")
        .await
        .unwrap();
    vault
        .save_note(
            &VaultPath::new("/unrelated.md"),
            "# Unrelated\nNo links here.",
        )
        .await
        .unwrap();

    vault
        .rename_note(
            &VaultPath::new("/target.md"),
            &VaultPath::new("/renamed.md"),
        )
        .await
        .unwrap();

    let unrelated = nfs::load_note(
        &crate::system::sys(dir.path()),
        &VaultPath::new("/unrelated.md"),
    )
    .await
    .unwrap();
    assert_eq!(unrelated, "# Unrelated\nNo links here.");
}

#[tokio::test]
async fn rename_note_handles_self_link() {
    let dir = TempDir::new().unwrap();
    let vault = setup_vault_with_notes(dir.path()).await;

    vault
        .save_note(
            &VaultPath::new("/target.md"),
            "# Target\nSee [[target]] here.",
        )
        .await
        .unwrap();

    vault
        .rename_note(
            &VaultPath::new("/target.md"),
            &VaultPath::new("/renamed.md"),
        )
        .await
        .unwrap();

    // Source no longer exists at the old path.
    assert!(
        !dir.path().join("target.md").exists(),
        "old file should be gone"
    );
    // New file exists with the self-link rewritten.
    let body = nfs::load_note(
        &crate::system::sys(dir.path()),
        &VaultPath::new("/renamed.md"),
    )
    .await
    .unwrap();
    assert!(
        body.contains("[[renamed]]"),
        "expected self-link rewritten in: {body}"
    );
    assert!(
        !body.contains("[[target]]"),
        "old self-link still present in: {body}"
    );

    // DB should have exactly one row for the renamed note.
    let all = vault.get_all_notes().await.unwrap();
    assert_eq!(all.len(), 1, "expected single DB row, got: {:?}", all);
}

#[test]
fn test_index_report_finish() {
    let mut report = IndexReport::new();

    // Sleep for a small amount to ensure duration is non-zero
    std::thread::sleep(Duration::from_millis(10));

    report.finish();

    // Check that duration is now set and non-zero
    assert!(report.duration > Duration::default());
    assert!(report.duration.as_millis() >= 10);
}

#[tokio::test]
async fn test_note_vault_new_with_nonexistent_path() {
    // Host-absolute: `/this/…` carries no drive prefix, so on Windows it is
    // not absolute, `sys` panics on it, and the test dies before it can
    // assert anything about the vault.
    let nonexistent_path = if cfg!(windows) {
        r"C:\this\path\does\not\exist"
    } else {
        "/this/path/does/not/exist"
    };
    let result = NoteVault::new(VaultConfig::new(crate::system::sys(nonexistent_path))).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        VaultError::VaultPathNotFound { path } => {
            assert_eq!(path, nonexistent_path);
        }
        _ => panic!("Expected VaultPathNotFound error"),
    }
}

#[tokio::test]
async fn test_note_vault_new_with_file_instead_of_directory() {
    // Create a temporary file
    let temp_file = tempfile::NamedTempFile::new().unwrap();
    let file_path = temp_file.path();

    let result = NoteVault::new(VaultConfig::new(crate::system::sys(file_path))).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        VaultError::FSError(FSError::InvalidPath { message, .. }) => {
            assert_eq!(message, "Path provided is not a directory");
        }
        _ => panic!("Expected FSError::InvalidPath"),
    }
}

#[tokio::test]
async fn test_note_vault_new_with_valid_directory() {
    let temp_dir = TempDir::new().unwrap();
    let dir_path = temp_dir.path();

    let result = NoteVault::new(VaultConfig::new(crate::system::sys(dir_path))).await;

    assert!(result.is_ok());
    let vault = result.unwrap();
    assert_eq!(vault.workspace_path().as_path(), dir_path);
    assert_eq!(vault.journal_path, VaultPath::new(DEFAULT_JOURNAL_PATH));
}

#[tokio::test]
async fn test_get_todays_journal() {
    let temp_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp_dir.path())))
        .await
        .unwrap();

    let (title, note_path) = vault.get_todays_journal();

    // Check that title matches today's date format
    let today = Utc::now();
    let expected_title = today.format("%Y-%m-%d").to_string();
    assert_eq!(title, expected_title);

    // Check that the path is correct
    let expected_path = vault
        .journal_path
        .append(&VaultPath::note_path_from(&expected_title))
        .absolute();
    assert_eq!(note_path, expected_path);
}

#[tokio::test]
async fn journal_entry_reports_creation_then_reuse() {
    let temp_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp_dir.path())))
        .await
        .unwrap();

    let (_, _, created_first) = vault.journal_entry().await.unwrap();
    assert!(created_first, "first call must create today's entry");

    let (_, _, created_second) = vault.journal_entry().await.unwrap();
    assert!(!created_second, "second call must reuse the existing entry");
}

#[tokio::test]
async fn load_or_create_note_reports_creation_then_reuse() {
    let temp_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp_dir.path())))
        .await
        .unwrap();
    let path = VaultPath::note_path_from("notes/fresh.md");

    let (_, created_first) = vault.load_or_create_note(&path, None).await.unwrap();
    assert!(created_first, "missing note must be created");

    let (_, created_second) = vault.load_or_create_note(&path, None).await.unwrap();
    assert!(
        !created_second,
        "existing note must be loaded, not recreated"
    );
}

#[tokio::test]
async fn test_journal_date_with_valid_journal_note() {
    let temp_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp_dir.path())))
        .await
        .unwrap();

    // Create a journal note path
    let journal_note_path = vault
        .journal_path
        .append(&VaultPath::note_path_from("2023-12-25"))
        .absolute();

    let result = vault.journal_date(&journal_note_path);

    assert!(result.is_some());
    let date = result.unwrap();
    assert_eq!(date, NaiveDate::from_ymd_opt(2023, 12, 25).unwrap());
}

#[tokio::test]
async fn test_journal_date_with_invalid_date_format() {
    let temp_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp_dir.path())))
        .await
        .unwrap();

    // Create a note path with invalid date format
    let invalid_journal_path = vault
        .journal_path
        .append(&VaultPath::note_path_from("invalid-date"))
        .absolute();

    let result = vault.journal_date(&invalid_journal_path);
    assert!(result.is_none());
}

#[tokio::test]
async fn test_journal_date_with_non_journal_path() {
    let temp_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp_dir.path())))
        .await
        .unwrap();

    // Create a note path outside of journal directory
    let non_journal_path = VaultPath::new("/other/2023-12-25.md");

    let result = vault.journal_date(&non_journal_path);
    assert!(result.is_none());
}

#[tokio::test]
async fn test_journal_date_with_non_note_path() {
    let temp_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp_dir.path())))
        .await
        .unwrap();

    // Create a directory path (not a note)
    let directory_path = vault.journal_path.append(&VaultPath::new("2023-12-25"));

    let result = vault.journal_date(&directory_path);
    assert!(result.is_none());
}

#[tokio::test]
async fn test_path_to_pathbuf() {
    let temp_dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(temp_dir.path())))
        .await
        .unwrap();

    let vault_path = VaultPath::new("/test/note.md");
    let result = vault.path_to_pathbuf(&vault_path);

    let expected = vault_path.to_pathbuf(vault.workspace_path());
    assert_eq!(result, expected);
}

#[test]
fn test_directory_details() {
    let path = VaultPath::new("/test/directory");
    let details = DirectoryDetails { path: path.clone() };

    assert_eq!(details.path, path);
}

#[test]
fn test_search_result_note() {
    let path = VaultPath::new("/test/note.md");
    let content_data = NoteContentData::new("Test Note".to_string(), 12345);
    let result = SearchResult::note(&path, &content_data);

    assert_eq!(result.path, path);
    match result.rtype {
        ResultType::Note(data) => assert_eq!(data, content_data),
        _ => panic!("Expected Note result type"),
    }
}

#[test]
fn test_search_result_directory() {
    let path = VaultPath::new("/test/directory");
    let result = SearchResult::directory(&path);

    assert_eq!(result.path, path);
    match result.rtype {
        ResultType::Directory => (),
        _ => panic!("Expected Directory result type"),
    }
}

#[test]
fn test_search_result_attachment() {
    let path = VaultPath::new("/test/image.png");
    let result = SearchResult::attachment(&path);

    assert_eq!(result.path, path);
    match result.rtype {
        ResultType::Attachment => (),
        _ => panic!("Expected Attachment result type"),
    }
}

#[test]
fn test_result_type_equality() {
    let content_data = NoteContentData::new("Test Note".to_string(), 12345);
    let note_type1 = ResultType::Note(content_data.clone());
    let note_type2 = ResultType::Note(content_data);
    let directory_type = ResultType::Directory;
    let attachment_type = ResultType::Attachment;

    assert_eq!(note_type1, note_type2);
    assert_eq!(directory_type, ResultType::Directory);
    assert_eq!(attachment_type, ResultType::Attachment);
    assert_ne!(directory_type, attachment_type);
}

#[test]
fn test_vault_browse_options_builder_default() {
    let builder = VaultBrowseOptionsBuilder::default();

    let options = builder.build();

    assert_eq!(options.path, VaultPath::root());
    assert_eq!(options.validation, NotesValidation::None);
    assert!(!options.recursive);
}

#[test]
fn test_vault_browse_options_builder_new() {
    let test_path = VaultPath::new("/test/path");
    let builder = VaultBrowseOptionsBuilder::new(&test_path);

    let options = builder.build();

    assert_eq!(options.path, test_path);
    assert_eq!(options.validation, NotesValidation::None);
    assert!(!options.recursive);
}

#[test]
fn test_vault_browse_options_builder_path() {
    let initial_path = VaultPath::new("/initial");
    let new_path = VaultPath::new("/new/path");

    let builder = VaultBrowseOptionsBuilder::new(&initial_path).path(new_path.clone());

    let options = builder.build();

    assert_eq!(options.path, new_path);
}

#[test]
fn test_vault_browse_options_builder_recursive() {
    let path = VaultPath::new("/test");

    let builder = VaultBrowseOptionsBuilder::new(&path).recursive(true);
    let options = builder.build();
    assert!(options.recursive);

    let builder = VaultBrowseOptionsBuilder::new(&path).recursive(false);
    let options = builder.build();
    assert!(!options.recursive);
}

#[test]
fn test_vault_browse_options_builder_validation_modes() {
    let path = VaultPath::new("/test");

    for v in [
        NotesValidation::Full,
        NotesValidation::Fast,
        NotesValidation::None,
    ] {
        let builder = VaultBrowseOptionsBuilder::new(&path).validation(v);
        let options = builder.build();
        assert_eq!(options.validation, v);
    }
}

#[test]
fn test_vault_browse_options_builder_chaining() {
    let path = VaultPath::new("/test");
    let new_path = VaultPath::new("/new");

    let builder = VaultBrowseOptionsBuilder::new(&path)
        .path(new_path.clone())
        .recursive(true)
        .validation(NotesValidation::Full);

    let options = builder.build();

    assert_eq!(options.path, new_path);
    assert!(options.recursive);
    assert_eq!(options.validation, NotesValidation::Full);
}

#[test]
fn test_notes_validation_display() {
    assert_eq!(format!("{}", NotesValidation::Full), "Full");
    assert_eq!(format!("{}", NotesValidation::Fast), "Fast");
    assert_eq!(format!("{}", NotesValidation::None), "None");
}

#[test]
fn test_vault_browse_options_display() {
    let path = VaultPath::new("/test/path");
    let builder = VaultBrowseOptionsBuilder::new(&path)
        .recursive(true)
        .validation(NotesValidation::Full);

    let options = builder.build();
    let display_string = format!("{}", options);

    assert!(display_string.contains("Path: `/test/path`"));
    assert!(display_string.contains("Validation Type: `Full`"));
    assert!(display_string.contains("Recursive: `true`"));
}

#[test]
fn test_default_journal_path_constant() {
    assert_eq!(DEFAULT_JOURNAL_PATH, "/journal");
}

// Verifies that validate_and_init rejects a vault containing case-insensitive
// path conflicts (e.g. note.md vs Note.md, projects/ vs Projects/).
// Linux only: macOS and Windows filesystems are case-insensitive by default,
// so creating note.md + Note.md would silently overwrite rather than produce two files.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn rejects_vault_with_case_conflicts() {
    let tmp = TempDir::new().unwrap();
    // file conflict at root
    std::fs::write(tmp.path().join("note.md"), "lowercase").unwrap();
    std::fs::write(tmp.path().join("Note.md"), "uppercase").unwrap();
    // directory conflict at root
    std::fs::create_dir(tmp.path().join("projects")).unwrap();
    std::fs::create_dir(tmp.path().join("Projects")).unwrap();

    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(tmp.path())))
        .await
        .unwrap();
    let result = vault.validate_and_init().await;

    match result {
        Err(VaultError::CaseConflict { conflicts }) => {
            assert_eq!(
                conflicts.len(),
                2,
                "expected 2 conflicts, got: {:?}",
                conflicts
            );
            let joined = conflicts.join("\n");
            assert!(
                joined.contains("note.md") && joined.contains("Note.md"),
                "expected note.md conflict in list, got: {}",
                joined
            );
            assert!(
                joined.contains("projects") && joined.contains("Projects"),
                "expected projects conflict in list, got: {}",
                joined
            );
        }
        other => panic!(
            "expected CaseConflict, got: {}",
            match other {
                Ok(_) => "Ok(_)".to_string(),
                Err(e) => format!("Err({})", e),
            }
        ),
    }
}

#[tokio::test]
async fn quick_note_creates_timestamped_note_in_inbox() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let details = vault.quick_note("my quick thought").await.unwrap();
    let (parent, _) = details.path.get_parent_path();
    assert!(parent.to_string().contains("inbox"));

    let text = vault.get_note_text(&details.path).await.unwrap();
    assert_eq!(text, "my quick thought");
}

#[tokio::test]
async fn quick_note_resolves_conflicts() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let d1 = vault.quick_note("first").await.unwrap();
    let d2 = vault.quick_note("second").await.unwrap();

    assert_ne!(d1.path, d2.path);
    assert_eq!(vault.get_note_text(&d1.path).await.unwrap(), "first");
    assert_eq!(vault.get_note_text(&d2.path).await.unwrap(), "second");
}

#[tokio::test]
async fn quick_note_uses_custom_inbox_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    vault.set_inbox_path(VaultPath::new("/capture"));

    let details = vault.quick_note("test").await.unwrap();
    let (parent, _) = details.path.get_parent_path();
    assert!(parent.to_string().contains("capture"));
}

#[tokio::test]
async fn create_note_avoiding_conflicts_uses_requested_path_when_free() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let path = VaultPath::note_path_from("ask/my-question");
    let used = vault
        .create_note_avoiding_conflicts(&path, "the answer")
        .await
        .unwrap();

    assert_eq!(used, path);
    assert_eq!(vault.get_note_text(&used).await.unwrap(), "the answer");
}

#[tokio::test]
async fn free_note_path_returns_the_requested_path_when_free() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let path = VaultPath::note_path_from("ask/my-question");
    assert_eq!(vault.free_note_path(&path).await, path);
}

/// The path a caller shows ("create this note?") and the path the
/// conflict-avoiding create actually writes must be the same one —
/// otherwise the user confirms one note and gets another.
#[tokio::test]
async fn free_note_path_agrees_with_the_name_the_create_picks() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let path = VaultPath::note_path_from("ask/my-question");
    vault.create_note(&path, "taken").await.unwrap();

    let free = vault.free_note_path(&path).await;
    assert_ne!(free, path);

    let used = vault
        .create_note_avoiding_conflicts(&path, "second answer")
        .await
        .unwrap();
    assert_eq!(used, free);
}

#[tokio::test]
async fn create_note_avoiding_conflicts_retries_on_collision() {
    let dir = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let path = VaultPath::note_path_from("ask/my-question");
    let first = vault
        .create_note_avoiding_conflicts(&path, "first answer")
        .await
        .unwrap();
    let second = vault
        .create_note_avoiding_conflicts(&path, "second answer")
        .await
        .unwrap();

    assert_ne!(first, second);
    assert_eq!(first, path);
    assert_eq!(vault.get_note_text(&first).await.unwrap(), "first answer");
    assert_eq!(vault.get_note_text(&second).await.unwrap(), "second answer");
}

#[tokio::test]
async fn create_note_errors_when_file_exists() {
    let dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let path = VaultPath::new("/already.md");
    vault.create_note(&path, "first").await.unwrap();

    match vault.create_note(&path, "second").await {
        Err(VaultError::NoteExists { path: p }) => assert_eq!(p, path.flatten()),
        other => panic!("expected NoteExists, got {:?}", other.err()),
    }

    // The original content must be intact (no overwrite).
    let text = vault.get_note_text(&path).await.unwrap();
    assert_eq!(text, "first");
}

#[tokio::test]
async fn create_directory_errors_when_dir_exists() {
    let dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let path = VaultPath::new("/projects");
    vault.create_directory(&path).await.unwrap();

    match vault.create_directory(&path).await {
        Err(VaultError::DirectoryExists { path: p }) => assert_eq!(p, path),
        other => panic!("expected DirectoryExists, got {:?}", other.err()),
    }
}

#[tokio::test]
async fn rename_note_errors_when_dest_exists() {
    let dir = TempDir::new().unwrap();
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let from = VaultPath::new("/source.md");
    let to = VaultPath::new("/dest.md");
    vault.create_note(&from, "src").await.unwrap();
    vault.create_note(&to, "dst").await.unwrap();

    match vault.rename_note(&from, &to).await {
        Err(VaultError::FSError(FSError::InvalidPath { message, .. })) => {
            assert_eq!(message, "Destination path already exists");
        }
        other => panic!("expected destination-exists error, got {:?}", other.err()),
    }

    // Both files unchanged.
    assert_eq!(vault.get_note_text(&from).await.unwrap(), "src");
    assert_eq!(vault.get_note_text(&to).await.unwrap(), "dst");
}

/// Indexing a multi-level directory tree should pick up notes at every depth
/// in a single pass (recursive walk + single transaction).
#[tokio::test(flavor = "multi_thread")]
async fn validate_and_init_indexes_nested_tree() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    std::fs::create_dir_all(root.join("dir1/sub")).unwrap();
    std::fs::write(root.join("a.md"), "# A").unwrap();
    std::fs::write(root.join("dir1/b.md"), "# B").unwrap();
    std::fs::write(root.join("dir1/sub/c.md"), "# C").unwrap();

    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(root)))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();

    let all = vault.get_all_notes().await.unwrap();
    let names: Vec<String> = all.iter().map(|(e, _)| e.path.to_string()).collect();

    assert_eq!(all.len(), 3, "expected 3 notes, got: {:?}", names);
    assert!(names.iter().any(|p| p.ends_with("/a.md")), "{:?}", names);
    assert!(
        names.iter().any(|p| p.ends_with("/dir1/b.md")),
        "{:?}",
        names
    );
    assert!(
        names.iter().any(|p| p.ends_with("/dir1/sub/c.md")),
        "{:?}",
        names
    );
}

#[tokio::test]
async fn pin_toggle_pins_then_unpins_through_the_vault() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    let a = VaultPath::new("a.md");
    vault.create_note(&a, "hi").await.unwrap();
    assert_eq!(
        vault.toggle_pinned_note(&a).await.unwrap(),
        PinToggle::Pinned { position: 0 }
    );
    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![VaultPath::new("/a.md")]
    );
    assert_eq!(
        vault.toggle_pinned_note(&a).await.unwrap(),
        PinToggle::Unpinned
    );
    assert!(vault.list_pinned_notes().await.unwrap().is_empty());
}

#[tokio::test]
async fn pin_note_pins_once_and_never_unpins() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    let a = VaultPath::new("a.md");
    vault.create_note(&a, "hi").await.unwrap();
    assert_eq!(
        vault.pin_note(&a).await.unwrap(),
        PinOutcome::Pinned { position: 0 }
    );
    assert_eq!(vault.pin_note(&a).await.unwrap(), PinOutcome::AlreadyPinned);
    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![VaultPath::new("/a.md")]
    );
}

#[tokio::test]
async fn pin_note_refuses_a_directory_a_missing_note_and_a_full_list() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    assert!(vault.pin_note(&VaultPath::new("dir")).await.is_err());
    match vault.pin_note(&VaultPath::new("gone.md")).await {
        Err(VaultError::FSError(FSError::VaultPathNotFound { .. })) => {}
        other => panic!("expected VaultPathNotFound, got {:?}", other),
    }
    for i in 0..PINNED_NOTES_CAP {
        let n = VaultPath::new(format!("n{i}.md"));
        vault.create_note(&n, "hi").await.unwrap();
        vault.pin_note(&n).await.unwrap();
    }
    let extra = VaultPath::new("extra.md");
    vault.create_note(&extra, "hi").await.unwrap();
    assert_eq!(vault.pin_note(&extra).await.unwrap(), PinOutcome::Full);
    assert_eq!(
        vault.list_pinned_notes().await.unwrap().len(),
        PINNED_NOTES_CAP
    );
}

#[tokio::test]
async fn pin_note_keeps_a_pin_past_the_cap_that_a_hand_edit_left() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    let mut all = Vec::new();
    for i in 0..=PINNED_NOTES_CAP {
        let n = VaultPath::new(format!("n{i}.md"));
        vault.create_note(&n, "hi").await.unwrap();
        all.push(n);
    }
    pinned_notes::write_pinned_notes(vault.workspace_path(), &all)
        .await
        .unwrap();
    let tenth = all.last().unwrap();
    assert_eq!(
        vault.pin_note(tenth).await.unwrap(),
        PinOutcome::AlreadyPinned
    );
    // The display list stops at the cap; the full list does not.
    assert_eq!(
        vault.list_pinned_notes().await.unwrap().len(),
        PINNED_NOTES_CAP
    );
    let all = vault.list_all_pinned_notes().await.unwrap();
    assert_eq!(all.len(), PINNED_NOTES_CAP + 1);
    assert!(pinned_notes::position_of(&all, tenth).is_some());
    let on_disk = pinned_notes::read_pinned_notes(vault.workspace_path())
        .await
        .unwrap();
    assert_eq!(on_disk.len(), PINNED_NOTES_CAP + 1);
}

/// The doc promises "already pinned" wins over "not on disk": a pin
/// whose note is gone stays, and is not an error.
#[tokio::test]
async fn pin_note_on_a_pinned_note_that_is_gone_is_already_pinned() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    let gone = VaultPath::new("gone.md");
    pinned_notes::write_pinned_notes(vault.workspace_path(), std::slice::from_ref(&gone))
        .await
        .unwrap();
    assert_eq!(
        vault.pin_note(&gone).await.unwrap(),
        PinOutcome::AlreadyPinned
    );
    assert_eq!(vault.list_pinned_notes().await.unwrap().len(), 1);
}

/// Many `pin_note` calls on one note at once: one pins, the rest find
/// it pinned, and the list holds it once. The lock around the check and
/// the append is what makes that so.
#[tokio::test]
async fn concurrent_pin_note_calls_pin_a_note_exactly_once() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    let a = VaultPath::new("a.md");
    vault.create_note(&a, "hi").await.unwrap();
    let calls: Vec<_> = (0..12)
        .map(|_| {
            let (vault, a) = (vault.clone(), a.clone());
            tokio::spawn(async move { vault.pin_note(&a).await.unwrap() })
        })
        .collect();
    let mut outcomes = Vec::new();
    for call in calls {
        outcomes.push(call.await.unwrap());
    }
    let pinned = outcomes
        .iter()
        .filter(|o| matches!(o, PinOutcome::Pinned { .. }))
        .count();
    assert_eq!(pinned, 1, "{outcomes:?}");
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == PinOutcome::AlreadyPinned)
            .count(),
        11,
        "{outcomes:?}"
    );
    assert_eq!(vault.list_pinned_notes().await.unwrap().len(), 1);
}

#[tokio::test]
async fn pin_toggle_rejects_a_directory_path() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    match vault.toggle_pinned_note(&VaultPath::new("dir")).await {
        Err(VaultError::FSError(FSError::InvalidPath { message, .. })) => {
            assert_eq!(message, "The path is not a note");
        }
        other => panic!("expected InvalidPath, got {:?}", other),
    }
    assert!(vault.list_pinned_notes().await.unwrap().is_empty());
}

#[tokio::test]
async fn pin_toggle_refuses_past_the_cap() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    for i in 0..PINNED_NOTES_CAP {
        let n = VaultPath::new(format!("n{i}.md"));
        vault.create_note(&n, "hi").await.unwrap();
        vault.toggle_pinned_note(&n).await.unwrap();
    }
    let extra = VaultPath::new("extra.md");
    vault.create_note(&extra, "hi").await.unwrap();
    assert_eq!(
        vault.toggle_pinned_note(&extra).await.unwrap(),
        PinToggle::Full
    );
}

/// A pin is an explicit act on a real note: pinning a path with no note
/// behind it (the open buffer's file removed outside kimün, or a create
/// dialog dismissed with the path left set) is refused rather than
/// stored as a pin born "missing".
#[tokio::test]
async fn pin_toggle_refuses_a_note_missing_on_disk() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    match vault.toggle_pinned_note(&VaultPath::new("gone.md")).await {
        Err(VaultError::FSError(FSError::VaultPathNotFound { path })) => {
            assert_eq!(path, VaultPath::new("gone.md"));
        }
        other => panic!("expected VaultPathNotFound, got {:?}", other),
    }
    assert!(vault.list_pinned_notes().await.unwrap().is_empty());
    assert!(
        !dir.path().join(".kimun").join("pinned-notes.toml").exists(),
        "a refused pin must not create the pin file"
    );
}

/// The missing-note rule only guards *pinning*: a pin whose note vanished
/// afterwards is kept by design and must still be removable by toggle.
#[tokio::test]
async fn pin_toggle_unpins_a_note_that_vanished_after_pinning() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    let a = VaultPath::new("a.md");
    vault.create_note(&a, "hi").await.unwrap();
    vault.toggle_pinned_note(&a).await.unwrap();
    std::fs::remove_file(dir.path().join("a.md")).unwrap();

    assert_eq!(
        vault.toggle_pinned_note(&a).await.unwrap(),
        PinToggle::Unpinned
    );
    assert!(vault.list_pinned_notes().await.unwrap().is_empty());
}

/// `list_pinned_notes` shows at most the cap, but an over-cap file (a
/// hand-edit or a sync merge) keeps its extra entries through every
/// edit — the cap is enforced on pinning, never by silently deleting.
#[tokio::test]
async fn pin_edits_keep_entries_past_the_cap_that_list_hides() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    let extra = PINNED_NOTES_CAP + 2;
    let entries: Vec<String> = (0..extra).map(|i| format!("\"/n{i}.md\"")).collect();
    let kimun_dir = dir.path().join(".kimun");
    std::fs::create_dir_all(&kimun_dir).unwrap();
    std::fs::write(
        kimun_dir.join("pinned-notes.toml"),
        format!("notes = [{}]\n", entries.join(", ")),
    )
    .unwrap();

    assert_eq!(
        vault.list_pinned_notes().await.unwrap().len(),
        PINNED_NOTES_CAP
    );
    assert!(vault.unpin_note(&VaultPath::new("n0.md")).await.unwrap());
    let on_disk = crate::nfs::pinned_notes::read_pinned_notes(vault.workspace_path())
        .await
        .unwrap();
    assert_eq!(on_disk.len(), extra - 1, "the extras survived the write");
    assert_eq!(
        on_disk.last(),
        Some(&VaultPath::new(format!("/n{}.md", extra - 1)))
    );
}

#[tokio::test]
async fn pin_unpin_and_move_persist() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    for n in ["a.md", "b.md", "c.md"] {
        let n = VaultPath::new(n);
        vault.create_note(&n, "hi").await.unwrap();
        vault.toggle_pinned_note(&n).await.unwrap();
    }
    assert!(vault
        .move_pinned_note(&VaultPath::new("c.md"), -2)
        .await
        .unwrap());
    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![
            VaultPath::new("/c.md"),
            VaultPath::new("/a.md"),
            VaultPath::new("/b.md"),
        ]
    );
    // No-op, out-of-range and not-pinned moves report `false` and leave
    // the order untouched.
    assert!(!vault
        .move_pinned_note(&VaultPath::new("c.md"), 0)
        .await
        .unwrap());
    assert!(!vault
        .move_pinned_note(&VaultPath::new("c.md"), -1)
        .await
        .unwrap());
    assert!(!vault
        .move_pinned_note(&VaultPath::new("never.md"), 1)
        .await
        .unwrap());
    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![
            VaultPath::new("/c.md"),
            VaultPath::new("/a.md"),
            VaultPath::new("/b.md"),
        ]
    );
    assert!(vault.unpin_note(&VaultPath::new("a.md")).await.unwrap());
    assert!(!vault.unpin_note(&VaultPath::new("a.md")).await.unwrap());
    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![VaultPath::new("/c.md"), VaultPath::new("/b.md")]
    );
}

/// A no-op write must not just report `false`: it must not touch the
/// filesystem at all. Proved by absence rather than an mtime comparison
/// (which filesystem timestamp granularity can make flaky) — on a vault
/// with no pin file yet, a no-op unpin must leave it absent.
#[tokio::test]
async fn pin_noop_unpin_does_not_create_the_pin_file() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault.validate_and_init().await.unwrap();

    assert!(!vault.unpin_note(&VaultPath::new("never.md")).await.unwrap());
    assert!(
        !dir.path().join(".kimun").join("pinned-notes.toml").exists(),
        "a no-op unpin must not create the pin file"
    );
}

#[tokio::test]
async fn deleting_a_note_unpins_it() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let a = VaultPath::new("a.md");
    vault.create_note(&a, "hi").await.unwrap();
    vault.toggle_pinned_note(&a).await.unwrap();
    let keep = VaultPath::new("keep.md");
    vault.create_note(&keep, "hi").await.unwrap();
    vault.toggle_pinned_note(&keep).await.unwrap();

    vault.delete_note(&a).await.unwrap();

    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![VaultPath::new("/keep.md")]
    );
}

#[tokio::test]
async fn renaming_a_directory_rewrites_pins_beneath_it() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let inside = VaultPath::new("proj/a.md");
    vault.create_note(&inside, "hi").await.unwrap();
    vault.toggle_pinned_note(&inside).await.unwrap();

    vault
        .rename_directory(&VaultPath::new("proj"), &VaultPath::new("work"))
        .await
        .unwrap();

    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![VaultPath::new("/work/a.md")]
    );
}

#[tokio::test]
async fn deleting_a_directory_unpins_everything_beneath_it() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    let inside = VaultPath::new("proj/a.md");
    vault.create_note(&inside, "hi").await.unwrap();
    vault.toggle_pinned_note(&inside).await.unwrap();
    let keep = VaultPath::new("keep.md");
    vault.create_note(&keep, "hi").await.unwrap();
    vault.toggle_pinned_note(&keep).await.unwrap();

    vault
        .delete_directory(&VaultPath::new("proj"))
        .await
        .unwrap();

    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![VaultPath::new("/keep.md")]
    );
}

// ---- child_note_counts ----

async fn note(vault: &NoteVault, path: &str) {
    vault
        .create_note(&VaultPath::new(path), "# t")
        .await
        .unwrap();
}

#[tokio::test]
async fn child_note_counts_sums_nested_notes() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    for p in ["a/x.md", "a/b/y.md", "a/b/c/z.md", "d/w.md", "r.md"] {
        note(&vault, p).await;
    }

    let counts = vault.child_note_counts(&VaultPath::root()).await.unwrap();
    assert_eq!(counts.get(&VaultPath::new("/a")), Some(&3));
    assert_eq!(counts.get(&VaultPath::new("/d")), Some(&1));
    assert_eq!(counts.len(), 2);

    let counts = vault
        .child_note_counts(&VaultPath::new("/a"))
        .await
        .unwrap();
    assert_eq!(counts.get(&VaultPath::new("/a/b")), Some(&2));
    assert_eq!(counts.len(), 1);
}

#[tokio::test]
async fn child_note_counts_ignores_attachments_and_empty_dirs() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .save_attachment(&VaultPath::new("pics/a.png"), &[1, 2, 3])
        .await
        .unwrap();
    vault
        .create_directory(&VaultPath::new("empty"))
        .await
        .unwrap();
    note(&vault, "full/n.md").await;

    let counts = vault.child_note_counts(&VaultPath::root()).await.unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts.get(&VaultPath::new("/full")), Some(&1));
}

#[tokio::test]
async fn child_note_counts_does_not_mix_like_wildcards() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    note(&vault, "100%_done/sub/a.md").await;
    // Match only if `%` / `_` in the parent act as LIKE wildcards.
    note(&vault, "100xydone/sub/n.md").await;
    note(&vault, "1000_done/sub/n.md").await;

    let counts = vault
        .child_note_counts(&VaultPath::new("/100%_done"))
        .await
        .unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts.get(&VaultPath::new("/100%_done/sub")), Some(&1));
}

#[tokio::test]
async fn child_note_counts_is_case_insensitive() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    note(&vault, "Work/n.md").await;
    note(&vault, "Work/Sub/n.md").await;

    let counts = vault.child_note_counts(&VaultPath::root()).await.unwrap();
    assert_eq!(counts.get(&VaultPath::new("/work")), Some(&2));
    let counts = vault
        .child_note_counts(&VaultPath::new("/Work"))
        .await
        .unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts.get(&VaultPath::new("/work/sub")), Some(&1));
}

// ---- dated search ----

async fn dated_vault() -> (TempDir, NoteVault) {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    for day in ["2026-11-18", "2026-11-19", "2026-11-20", "2026-11-21"] {
        vault
            .create_note(
                &VaultPath::new(format!("/journal/{day}.md")),
                &format!("# day\nmeeting on {day}\n"),
            )
            .await
            .unwrap();
    }
    vault
        .create_note(
            &VaultPath::new("a.md"),
            "---\ndue: 2026-11-10\n---\nreport alpha\n",
        )
        .await
        .unwrap();
    vault
        .create_note(
            &VaultPath::new("b.md"),
            "---\ndue: 2026-11-12\n---\nreport report report\n",
        )
        .await
        .unwrap();
    vault
        .create_note(
            &VaultPath::new("c.md"),
            "---\ndue: 2026-12-01\n---\nreport gamma\n",
        )
        .await
        .unwrap();
    vault
        .create_note(&VaultPath::new("none.md"), "report with no date\n")
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    (dir, vault)
}

fn nd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn names(r: &[(NoteEntryData, NoteContentData)]) -> Vec<String> {
    r.iter().map(|(e, _)| e.path.to_string()).collect()
}

#[tokio::test]
async fn dated_search_journal_range_is_inclusive() {
    let (_d, vault) = dated_vault().await;
    let f = DateFilter {
        source: DateSource::Journal,
        from: nd(2026, 11, 19),
        to: nd(2026, 11, 20),
    };
    let mut got = names(&vault.search_notes_dated("", &f).await.unwrap());
    got.sort();
    assert_eq!(
        got,
        vec!["/journal/2026-11-19.md", "/journal/2026-11-20.md"]
    );
}

#[tokio::test]
async fn dated_search_property_range_excludes_notes_without_it() {
    let (_d, vault) = dated_vault().await;
    let f = DateFilter {
        source: DateSource::Property("due".into()),
        from: nd(2026, 11, 10),
        to: nd(2026, 11, 30),
    };
    let mut got = names(&vault.search_notes_dated("", &f).await.unwrap());
    got.sort();
    assert_eq!(got, vec!["/a.md", "/b.md"]);
}

#[tokio::test]
async fn dated_search_empty_query_orders_newest_first() {
    let (_d, vault) = dated_vault().await;
    let f = DateFilter {
        source: DateSource::Journal,
        from: nd(2026, 11, 1),
        to: nd(2026, 11, 30),
    };
    let got = names(&vault.search_notes_dated("", &f).await.unwrap());
    assert_eq!(
        got,
        vec![
            "/journal/2026-11-21.md",
            "/journal/2026-11-20.md",
            "/journal/2026-11-19.md",
            "/journal/2026-11-18.md"
        ]
    );
    let p = DateFilter {
        source: DateSource::Property("due".into()),
        from: nd(2026, 11, 1),
        to: nd(2026, 12, 31),
    };
    assert_eq!(
        names(&vault.search_notes_dated("", &p).await.unwrap()),
        vec!["/c.md", "/b.md", "/a.md"]
    );
}

#[tokio::test]
async fn dated_search_with_query_keeps_relevance_order() {
    let (_d, vault) = dated_vault().await;
    let f = DateFilter {
        source: DateSource::Property("due".into()),
        from: nd(2026, 11, 1),
        to: nd(2026, 12, 31),
    };
    let expected: Vec<String> = names(&vault.search_notes("report").await.unwrap())
        .into_iter()
        .filter(|p| p != "/none.md")
        .collect();
    let got = names(&vault.search_notes_dated("report", &f).await.unwrap());
    assert_eq!(got.len(), 3);
    assert_eq!(got, expected);
    let j = DateFilter {
        source: DateSource::Journal,
        from: nd(2026, 11, 19),
        to: nd(2026, 11, 20),
    };
    let mut got = names(&vault.search_notes_dated("meeting", &j).await.unwrap());
    got.sort();
    assert_eq!(
        got,
        vec!["/journal/2026-11-19.md", "/journal/2026-11-20.md"]
    );
}

#[tokio::test]
async fn dated_search_rejects_from_after_to() {
    let (_d, vault) = dated_vault().await;
    let f = DateFilter {
        source: DateSource::Journal,
        from: nd(2026, 11, 20),
        to: nd(2026, 11, 19),
    };
    let err = vault.search_notes_dated("", &f).await.unwrap_err();
    let msg = err.user_message().expect("user error");
    assert!(
        msg.contains("2026-11-20") && msg.contains("2026-11-19"),
        "{msg}"
    );
}

#[tokio::test]
async fn dated_search_unknown_property_key_errors_with_keys() {
    let (_d, vault) = dated_vault().await;
    let f = DateFilter {
        source: DateSource::Property("nope".into()),
        from: nd(2026, 11, 1),
        to: nd(2026, 11, 30),
    };
    let err = vault.search_notes_dated("", &f).await.unwrap_err();
    let msg = err.user_message().expect("user error");
    assert!(
        msg.contains("nope") && msg.contains("due") && msg.contains("journal"),
        "{msg}"
    );
}

#[tokio::test]
async fn dated_search_journal_errors_when_no_journal_notes_with_keys_listed() {
    let dir = TempDir::new().unwrap();
    let vault = make_vault(dir.path()).await;
    vault
        .create_note(&VaultPath::new("a.md"), "---\ndue: 2026-11-10\n---\nx\n")
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    let f = DateFilter {
        source: DateSource::Journal,
        from: nd(2026, 11, 1),
        to: nd(2026, 11, 30),
    };
    let err = vault.search_notes_dated("", &f).await.unwrap_err();
    let msg = err.user_message().expect("user error");
    assert!(msg.contains("journal") && msg.contains("due"), "{msg}");
}

#[tokio::test]
async fn dated_search_property_from_alone_matches_through_the_max_date() {
    let (_d, vault) = dated_vault().await;
    let f = DateFilter {
        source: DateSource::Property("due".into()),
        from: nd(2026, 11, 11),
        to: nd(9999, 12, 31),
    };
    let mut got = names(&vault.search_notes_dated("", &f).await.unwrap());
    got.sort();
    assert_eq!(got, vec!["/b.md", "/c.md"]);
    let mut got = names(&vault.search_notes_dated("report", &f).await.unwrap());
    got.sort();
    assert_eq!(got, vec!["/b.md", "/c.md"]);
}

#[tokio::test]
async fn dated_search_property_to_alone_matches_from_the_min_date() {
    let (_d, vault) = dated_vault().await;
    let f = DateFilter {
        source: DateSource::Property("due".into()),
        from: nd(1, 1, 1),
        to: nd(2026, 11, 11),
    };
    assert_eq!(
        names(&vault.search_notes_dated("", &f).await.unwrap()),
        vec!["/a.md"]
    );
}

#[tokio::test]
async fn dated_search_skips_a_note_deleted_outside_kimun() {
    let (dir, vault) = dated_vault().await;
    std::fs::remove_file(dir.path().join("b.md")).unwrap();
    std::fs::remove_file(dir.path().join("journal").join("2026-11-19.md")).unwrap();
    let p = DateFilter {
        source: DateSource::Property("due".into()),
        from: nd(2026, 11, 1),
        to: nd(2026, 12, 31),
    };
    for query in ["", "report"] {
        let mut got = names(&vault.search_notes_dated(query, &p).await.unwrap());
        got.sort();
        assert_eq!(got, vec!["/a.md", "/c.md"], "query {query:?}");
    }
    let j = DateFilter {
        source: DateSource::Journal,
        from: nd(2026, 11, 1),
        to: nd(2026, 11, 30),
    };
    for query in ["", "meeting"] {
        let got = names(&vault.search_notes_dated(query, &j).await.unwrap());
        assert_eq!(got.len(), 4, "index still lists the journal note: {got:?}");
    }
}

async fn vault_with_journal_property() -> (TempDir, NoteVault) {
    let (dir, vault) = dated_vault().await;
    vault
        .create_note(
            &VaultPath::new("odd.md"),
            "---\njournal: 2026-11-19\n---\nmeeting notes\n",
        )
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    (dir, vault)
}

#[tokio::test]
async fn dated_search_journal_ignores_a_property_named_journal() {
    let (_d, vault) = vault_with_journal_property().await;
    let j = DateFilter {
        source: DateSource::Journal,
        from: nd(2026, 11, 19),
        to: nd(2026, 11, 20),
    };
    for query in ["", "meeting"] {
        let mut got = names(&vault.search_notes_dated(query, &j).await.unwrap());
        got.sort();
        assert_eq!(
            got,
            vec!["/journal/2026-11-19.md", "/journal/2026-11-20.md"],
            "query {query:?}"
        );
    }
}

#[tokio::test]
async fn dated_search_property_journal_matches_only_the_property() {
    let (_d, vault) = vault_with_journal_property().await;
    let p = DateFilter {
        source: DateSource::Property("journal".into()),
        from: nd(2026, 11, 1),
        to: nd(2026, 11, 30),
    };
    for query in ["", "meeting"] {
        let got = names(&vault.search_notes_dated(query, &p).await.unwrap());
        assert_eq!(got, vec!["/odd.md"], "query {query:?}");
    }
}

#[tokio::test]
async fn dated_search_property_journal_without_such_property_errors_with_keys() {
    let (_d, vault) = dated_vault().await;
    let p = DateFilter {
        source: DateSource::Property("journal".into()),
        from: nd(2026, 11, 1),
        to: nd(2026, 11, 30),
    };
    let err = vault.search_notes_dated("", &p).await.unwrap_err();
    let msg = err.user_message().expect("user error");
    assert!(
        msg.contains("Unknown date property 'journal'") && msg.contains("due"),
        "{msg}"
    );
}
