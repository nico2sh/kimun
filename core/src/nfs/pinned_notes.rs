//! Pinned notes: an ordered, capped list of notes the user keeps within
//! reach, persisted in the vault under `.kimun/pinned-notes.toml` so it
//! travels with the notes (same rationale as saved searches). All
//! filesystem access for it lives here per the project rule that fs ops
//! belong in `nfs`. The list edits are pure functions over a `Vec` so the
//! rules (cap, dense order, rename rewrites) are tested without a disk.
//!
//! Every entry is stored and compared in [`VaultPath::canonical`] form —
//! flattened and vault-*absolute* (`notes = ["/dir/a.md", "/b.md"]`) — the
//! same identity rule the note index uses, so a pin has exactly one form
//! whether the caller reached it as `a.md` or `/a.md`. Every function here
//! that takes a `VaultPath` argument canonicalizes it on entry, so callers
//! never need to normalize first.

use serde::{Deserialize, Serialize};

use crate::error::FSError;
use crate::nfs::{VaultPath, PATH_SEPARATOR};
use crate::system::SystemPath;

/// The most notes a vault can pin. The cap is the feature: every pinned
/// note has a one-digit shortcut, so a tenth is refused, never unreachable.
pub const PINNED_NOTES_CAP: usize = 9;

/// What [`toggle`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinToggle {
    /// Appended; `position` is 0-based.
    Pinned { position: usize },
    Unpinned,
    /// Not pinned and the list already holds [`PINNED_NOTES_CAP`] entries.
    Full,
}

/// On-disk wrapper: `notes = ["/a.md", "/dir/b.md"]` (canonical form).
#[derive(Debug, Default, Serialize, Deserialize)]
struct PinnedNotesFile {
    #[serde(default)]
    notes: Vec<VaultPath>,
}

fn pinned_notes_path(workspace_path: &SystemPath) -> std::path::PathBuf {
    workspace_path
        .as_path()
        .join(".kimun")
        .join("pinned-notes.toml")
}

/// Read the pinned list, in order. A vault with no file has none. Entries
/// are canonicalized on read, so a hand-edited relative entry still matches.
pub async fn read_pinned_notes(workspace_path: &SystemPath) -> Result<Vec<VaultPath>, FSError> {
    let path = pinned_notes_path(workspace_path);
    match tokio::fs::read_to_string(&path).await {
        Ok(body) => {
            let parsed: PinnedNotesFile =
                toml::from_str(&body).map_err(|e| FSError::SerializationError(e.to_string()))?;
            Ok(parsed.notes.into_iter().map(|n| n.canonical()).collect())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(FSError::ReadFileError(e)),
    }
}

/// Write the whole list, creating `.kimun/` if needed. Entries are
/// canonicalized before serializing, so the stored form is always canonical
/// regardless of how the caller built the list.
pub async fn write_pinned_notes(
    workspace_path: &SystemPath,
    notes: &[VaultPath],
) -> Result<(), FSError> {
    let path = pinned_notes_path(workspace_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file = PinnedNotesFile {
        notes: notes.iter().map(|n| n.canonical()).collect(),
    };
    let body =
        toml::to_string_pretty(&file).map_err(|e| FSError::SerializationError(e.to_string()))?;
    tokio::fs::write(&path, body).await?;
    Ok(())
}

/// 0-based position of `path` in the list. The one match rule for pins:
/// canonical, then component-wise (`VaultPath::is_like`), so an
/// absolute-vs-relative mismatch between caller and stored form never
/// matters.
pub fn position_of(notes: &[VaultPath], path: &VaultPath) -> Option<usize> {
    let path = path.canonical();
    notes.iter().position(|n| n.is_like(&path))
}

/// Pin `path` (appending) or unpin it if already pinned. Unpinning always
/// works; pinning is refused at the cap.
pub fn toggle(notes: &mut Vec<VaultPath>, path: &VaultPath) -> PinToggle {
    let path = path.canonical();
    if let Some(i) = position_of(notes, &path) {
        notes.remove(i);
        return PinToggle::Unpinned;
    }
    if notes.len() >= PINNED_NOTES_CAP {
        return PinToggle::Full;
    }
    notes.push(path);
    PinToggle::Pinned {
        position: notes.len() - 1,
    }
}

/// Remove `path` if pinned. Later entries move up (dense list).
pub fn remove(notes: &mut Vec<VaultPath>, path: &VaultPath) -> bool {
    let path = path.canonical();
    match position_of(notes, &path) {
        Some(i) => {
            notes.remove(i);
            true
        }
        None => false,
    }
}

/// Move the entry at `from` so it lands at index `to`. Out-of-range or
/// no-op moves return `false` and leave the list untouched.
pub fn move_entry(notes: &mut Vec<VaultPath>, from: usize, to: usize) -> bool {
    if from == to || from >= notes.len() || to >= notes.len() {
        return false;
    }
    let entry = notes.remove(from);
    notes.insert(to, entry);
    true
}

/// A note was renamed/moved: point its pin at the new path. `to` is stored
/// canonical, so the entry stays in the on-disk form regardless of how the
/// caller built `to`.
pub fn rewrite_note_rename(notes: &mut [VaultPath], from: &VaultPath, to: &VaultPath) -> bool {
    let from = from.canonical();
    let to = to.canonical();
    match notes.iter_mut().find(|n| n.is_like(&from)) {
        Some(slot) => {
            *slot = to;
            true
        }
        None => false,
    }
}

/// `"dir"` → `"/dir/"` so a prefix test cannot match `/dirx/`. Canonicalizes
/// `dir` itself, so every caller of this prefix — and every note string it
/// is compared against — shares one representation (absolute, flattened)
/// regardless of the form either side started in.
fn dir_prefix(dir: &VaultPath) -> String {
    let s = dir.canonical().to_string();
    if s.ends_with(PATH_SEPARATOR) {
        s
    } else {
        format!("{s}{PATH_SEPARATOR}")
    }
}

/// A directory was renamed: rewrite every pin beneath it.
pub fn rewrite_directory_rename(
    notes: &mut [VaultPath],
    from: &VaultPath,
    to: &VaultPath,
) -> bool {
    let from = from.canonical();
    let to = to.canonical();
    let from_prefix = dir_prefix(&from);
    let to_prefix = dir_prefix(&to);
    let mut changed = false;
    for slot in notes.iter_mut() {
        if let Some(rest) = slot.canonical().to_string().strip_prefix(&from_prefix) {
            *slot = VaultPath::new(format!("{to_prefix}{rest}"));
            changed = true;
        }
    }
    changed
}

/// A directory was deleted: drop every pin beneath it.
pub fn remove_under_directory(notes: &mut Vec<VaultPath>, dir: &VaultPath) -> bool {
    let dir = dir.canonical();
    let prefix = dir_prefix(&dir);
    let before = notes.len();
    notes.retain(|n| !n.canonical().to_string().starts_with(&prefix));
    notes.len() != before
}

/// Apply `edit` to the stored list and write it back if it changed. Pins are
/// non-critical bookkeeping beside a note operation that already succeeded,
/// so any failure is logged and swallowed — the caller's operation stands.
async fn best_effort_edit(
    workspace_path: &SystemPath,
    what: &str,
    edit: impl FnOnce(&mut Vec<VaultPath>) -> bool,
) {
    let mut all = match read_pinned_notes(workspace_path).await {
        Ok(all) => all,
        Err(e) => {
            log::warn!("pinned notes: could not read list while {what}: {e}");
            return;
        }
    };
    if !edit(&mut all) {
        return;
    }
    if let Err(e) = write_pinned_notes(workspace_path, &all).await {
        log::warn!("pinned notes: could not write list while {what}: {e}");
    }
}

/// A note was renamed or moved: keep its pin.
pub async fn on_note_renamed(workspace_path: &SystemPath, from: &VaultPath, to: &VaultPath) {
    best_effort_edit(workspace_path, "renaming a note", |all| {
        rewrite_note_rename(all, from, to)
    })
    .await;
}

/// A directory was renamed: keep every pin beneath it.
pub async fn on_directory_renamed(
    workspace_path: &SystemPath,
    from: &VaultPath,
    to: &VaultPath,
) {
    best_effort_edit(workspace_path, "renaming a directory", |all| {
        rewrite_directory_rename(all, from, to)
    })
    .await;
}

/// A note was deleted: drop its pin.
pub async fn on_note_deleted(workspace_path: &SystemPath, path: &VaultPath) {
    best_effort_edit(workspace_path, "deleting a note", |all| remove(all, path)).await;
}

/// A directory was deleted: drop every pin beneath it.
pub async fn on_directory_deleted(workspace_path: &SystemPath, dir: &VaultPath) {
    best_effort_edit(workspace_path, "deleting a directory", |all| {
        remove_under_directory(all, dir)
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::sys;

    fn p(s: &str) -> VaultPath {
        VaultPath::new(s)
    }

    #[tokio::test]
    async fn read_missing_file_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let got = read_pinned_notes(&sys(dir.path())).await.unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn write_then_read_round_trips_in_order() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = sys(dir.path());
        // Mixed relative/absolute input; the round trip normalizes both to
        // canonical (absolute) form.
        let notes = vec![p("b.md"), p("/dir/a.md"), p("c.md")];
        write_pinned_notes(&ws, &notes).await.unwrap();
        let got = read_pinned_notes(&ws).await.unwrap();
        let want: Vec<VaultPath> = notes.iter().map(|n| n.canonical()).collect();
        assert_eq!(got, want);
        assert!(dir.path().join(".kimun").join("pinned-notes.toml").exists());
    }

    #[tokio::test]
    async fn write_stores_canonical_absolute_paths_on_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = sys(dir.path());
        write_pinned_notes(&ws, &[p("dir/a.md"), p("b.md")])
            .await
            .unwrap();
        let body = tokio::fs::read_to_string(dir.path().join(".kimun").join("pinned-notes.toml"))
            .await
            .unwrap();
        assert_eq!(body, "notes = [\n    \"/dir/a.md\",\n    \"/b.md\",\n]\n");
    }

    #[test]
    fn toggle_appends_then_removes() {
        let mut notes = vec![p("/a.md")];
        assert_eq!(toggle(&mut notes, &p("b.md")), PinToggle::Pinned { position: 1 });
        assert_eq!(notes, vec![p("/a.md"), p("/b.md")]);
        assert_eq!(toggle(&mut notes, &p("a.md")), PinToggle::Unpinned);
        assert_eq!(notes, vec![p("/b.md")]);
    }

    #[test]
    fn toggle_matches_case_insensitively() {
        let mut notes = vec![p("Notes/Plan.md")];
        assert_eq!(toggle(&mut notes, &p("notes/plan.md")), PinToggle::Unpinned);
        assert!(notes.is_empty());
    }

    #[test]
    fn toggle_and_position_of_match_regardless_of_absoluteness() {
        // A relatively-stored pin (e.g. hand-edited) is found by an absolute
        // lookup, and a canonically-stored pin is found by a relative one.
        let mut relative_store = vec![p("a.md")];
        assert_eq!(position_of(&relative_store, &p("/a.md")), Some(0));
        assert_eq!(toggle(&mut relative_store, &p("/a.md")), PinToggle::Unpinned);
        assert!(relative_store.is_empty());

        let mut absolute_store = vec![p("/b.md")];
        assert_eq!(position_of(&absolute_store, &p("b.md")), Some(0));
        assert_eq!(toggle(&mut absolute_store, &p("b.md")), PinToggle::Unpinned);
        assert!(absolute_store.is_empty());
    }

    #[test]
    fn toggle_refuses_a_tenth_pin() {
        let mut notes: Vec<VaultPath> = (0..PINNED_NOTES_CAP)
            .map(|i| p(&format!("n{i}.md")))
            .collect();
        assert_eq!(toggle(&mut notes, &p("extra.md")), PinToggle::Full);
        assert_eq!(notes.len(), PINNED_NOTES_CAP);
        // A pinned entry can still be unpinned when full.
        assert_eq!(toggle(&mut notes, &p("n0.md")), PinToggle::Unpinned);
        assert_eq!(notes.len(), PINNED_NOTES_CAP - 1);
    }

    #[test]
    fn remove_is_dense() {
        let mut notes = vec![p("a.md"), p("b.md"), p("c.md")];
        assert!(remove(&mut notes, &p("b.md")));
        assert_eq!(notes, vec![p("a.md"), p("c.md")]);
        assert!(!remove(&mut notes, &p("zzz.md")));
    }

    #[test]
    fn move_entry_reorders_and_rejects_out_of_range() {
        let mut notes = vec![p("a.md"), p("b.md"), p("c.md")];
        assert!(move_entry(&mut notes, 2, 0));
        assert_eq!(notes, vec![p("c.md"), p("a.md"), p("b.md")]);
        assert!(move_entry(&mut notes, 0, 2));
        assert_eq!(notes, vec![p("a.md"), p("b.md"), p("c.md")]);
        assert!(!move_entry(&mut notes, 0, 3));
        assert!(!move_entry(&mut notes, 5, 0));
        assert!(!move_entry(&mut notes, 1, 1));
        assert_eq!(notes, vec![p("a.md"), p("b.md"), p("c.md")]);
    }

    #[test]
    fn position_of_is_zero_based() {
        let notes = vec![p("a.md"), p("b.md")];
        assert_eq!(position_of(&notes, &p("b.md")), Some(1));
        assert_eq!(position_of(&notes, &p("x.md")), None);
    }

    #[test]
    fn note_rename_rewrites_the_matching_entry_only() {
        let mut notes = vec![p("a.md"), p("dir/b.md")];
        assert!(rewrite_note_rename(&mut notes, &p("dir/b.md"), &p("other/c.md")));
        // The rewritten slot is stored canonical even though `to` was relative;
        // the untouched entry keeps whatever form it already had.
        assert_eq!(notes, vec![p("a.md"), p("/other/c.md")]);
        assert!(!rewrite_note_rename(&mut notes, &p("nope.md"), &p("x.md")));
    }

    #[test]
    fn directory_rename_rewrites_entries_beneath_it() {
        let mut notes = vec![p("proj/a.md"), p("proj/sub/b.md"), p("projx/c.md"), p("d.md")];
        assert!(rewrite_directory_rename(&mut notes, &p("proj"), &p("work")));
        assert_eq!(
            notes,
            vec![p("/work/a.md"), p("/work/sub/b.md"), p("projx/c.md"), p("d.md")]
        );
        assert!(!rewrite_directory_rename(&mut notes, &p("missing"), &p("x")));
    }

    #[test]
    fn directory_rename_matches_regardless_of_absoluteness() {
        // Stored relative, renamed with an absolute `from`.
        let mut notes = vec![p("proj/a.md")];
        assert!(rewrite_directory_rename(&mut notes, &p("/proj"), &p("work")));
        assert_eq!(notes, vec![p("/work/a.md")]);

        // Stored canonical (absolute), renamed with a relative `from`/`to`.
        let mut notes = vec![p("/proj/a.md")];
        assert!(rewrite_directory_rename(&mut notes, &p("proj"), &p("work")));
        assert_eq!(notes, vec![p("/work/a.md")]);
    }

    #[test]
    fn remove_under_directory_drops_descendants_only() {
        let mut notes = vec![p("proj/a.md"), p("proj/sub/b.md"), p("projx/c.md")];
        assert!(remove_under_directory(&mut notes, &p("proj")));
        assert_eq!(notes, vec![p("projx/c.md")]);
        assert!(!remove_under_directory(&mut notes, &p("proj")));
    }

    #[test]
    fn remove_under_directory_matches_regardless_of_absoluteness() {
        let mut notes = vec![p("proj/a.md"), p("other.md")];
        assert!(remove_under_directory(&mut notes, &p("/proj")));
        assert_eq!(notes, vec![p("other.md")]);
    }
}
