// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// lib.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and these TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::*;
use tempfile::TempDir;

/// Saving a CRLF note must index the hash a later scan will compute.
///
/// The scan hashes the file; `save_note` restores the endings the file had,
/// so the LF text the editor hands over is not what lands on disk. Hashing
/// the handed-in text stored a hash the scan disagreed with, and the note was
/// upserted (and re-embedded) a second time for content that never changed.
#[tokio::test]
async fn saving_a_crlf_note_indexes_the_hash_the_scan_will_compute() {
    let dir = TempDir::new().unwrap();
    let on_disk = dir.path().join("note.md");
    // Authored on Windows, before the vault ever sees it.
    tokio::fs::write(&on_disk, "alpha\r\nbeta\r\n")
        .await
        .unwrap();

    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
        .await
        .unwrap();
    let path = VaultPath::note_path_from("note.md");

    // Saved the way the editor saves: its text can hold no carriage return.
    let (_, content) = vault.save_note(&path, "alpha\nbeta\n").await.unwrap();

    let raw = tokio::fs::read_to_string(&on_disk).await.unwrap();
    assert!(
        raw.contains("\r\n"),
        "precondition: the note keeps the endings it had"
    );

    let scanned = NoteDetails::content_data_of(&raw).hash;
    assert_eq!(
        content.hash, scanned,
        "the indexed hash must be the one the scan computes from the file"
    );
}
