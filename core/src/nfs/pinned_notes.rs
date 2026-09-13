//! Pinned notes: an ordered, capped list of notes the user keeps within
//! reach, persisted in the vault under `.kimun/pinned-notes.toml` so it
//! travels with the notes (same rationale as saved searches). All
//! filesystem access for it lives here per the project rule that fs ops
//! belong in `nfs`. The list edits are pure functions over a `Vec` so the
//! rules (cap, dense order, rename rewrites) are tested without a disk.
//!
//! Every entry is stored and compared in `VaultPath::canonical` form —
//! flattened and vault-*absolute* (`notes = ["/dir/a.md", "/b.md"]`) — the
//! same identity rule the note index uses, so a pin has exactly one form
//! whether the caller reached it as `a.md` or `/a.md`. Every function here
//! that takes a `VaultPath` argument canonicalizes it on entry, so callers
//! never need to normalize first.
//!
//! Every writer goes through [`edit`]: one in-process lock per pin file
//! around the read-modify-write, and an atomic replace for the write
//! itself, so neither two overlapping edits (a rename hook racing a
//! toggle) nor a crash mid-write can lose or corrupt the list.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::error::FSError;
use crate::nfs::{VaultPath, PATH_SEPARATOR};
use crate::system::{self, SystemPath};

/// The most notes a vault can pin. The cap is the feature: every pinned
/// note has a one-digit shortcut, so a tenth is refused, never unreachable.
pub const PINNED_NOTES_CAP: usize = 9;

/// What [`toggle`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinToggle {
    /// Appended; `position` is 0-based.
    Pinned {
        position: usize,
    },
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

fn pinned_notes_path(workspace_path: &SystemPath) -> PathBuf {
    workspace_path
        .as_path()
        .join(".kimun")
        .join("pinned-notes.toml")
}

/// The in-process lock for one pin file, keyed by its path so every writer
/// in this process — each `NoteVault` clone's toggle/unpin/move and the
/// rename/delete hooks — serializes on the same lock. Cross-process writers
/// are not covered, the same stance `NoteLocks` takes for note content.
fn file_lock(path: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let map = LOCKS.get_or_init(Default::default);
    map.lock()
        .unwrap()
        .entry(path.to_path_buf())
        .or_default()
        .clone()
}

/// Read the pinned list, in order, exactly as stored. A vault with no file
/// has none. Entries are canonicalized on read, so a hand-edited relative
/// entry still matches.
///
/// Not truncated to `PINNED_NOTES_CAP`: only `toggle` guards the cap, and
/// the file lives inside the vault by design, so a hand-edit or a three-way
/// text merge of `notes = [ … ]` across two synced machines (a union, not a
/// cap) can leave more on disk. Every writer writes back what it read, so
/// truncating here would make the next unrelated edit silently delete the
/// entries past the cap. `NoteVault::list_pinned_notes` truncates for
/// display instead.
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
/// regardless of how the caller built the list. The write is an atomic
/// replace (`system::replace_atomically`): the file is rewritten by every
/// rename and delete hook, and a truncating write interrupted mid-way would
/// leave unparsable TOML that fails every pin operation until hand-edited.
pub async fn write_pinned_notes(
    workspace_path: &SystemPath,
    notes: &[VaultPath],
) -> Result<(), FSError> {
    let path = pinned_notes_path(workspace_path);
    let file = PinnedNotesFile {
        notes: notes.iter().map(|n| n.canonical()).collect(),
    };
    let body =
        toml::to_string_pretty(&file).map_err(|e| FSError::SerializationError(e.to_string()))?;
    tokio::task::spawn_blocking(move || system::replace_atomically(&path, body.as_bytes()))
        .await
        .map_err(|e| FSError::ReadFileError(std::io::Error::other(e)))??;
    Ok(())
}

/// The one read-modify-write seam. Takes the pin file's lock, reads the
/// list, applies `f`, and writes the list back when `f` reports a change.
/// `f` returns `(result, changed)`; `result` is handed back untouched.
pub async fn edit<R>(
    workspace_path: &SystemPath,
    f: impl FnOnce(&mut Vec<VaultPath>) -> (R, bool),
) -> Result<R, FSError> {
    let lock = file_lock(&pinned_notes_path(workspace_path));
    let _guard = lock.lock().await;
    let mut all = read_pinned_notes(workspace_path).await?;
    let (out, changed) = f(&mut all);
    if changed {
        write_pinned_notes(workspace_path, &all).await?;
    }
    Ok(out)
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

/// Move the pin for `path` by `delta` slots (negative moves it up).
/// Anchored on the path rather than an index so a caller working from a
/// stale copy of the list (a dialog whose rows predate an external edit)
/// moves the note it meant to, or nothing. `false` — and no change — when
/// `path` is not pinned, `delta` is zero, or the move would leave the list.
pub fn move_by(notes: &mut Vec<VaultPath>, path: &VaultPath, delta: isize) -> bool {
    let Some(from) = position_of(notes, path) else {
        return false;
    };
    let to = from as isize + delta;
    if to < 0 || to as usize >= notes.len() {
        return false;
    }
    move_entry(notes, from, to as usize)
}

/// Drop every later duplicate of an entry (canonical, component-wise
/// match), keeping the first — the one holding the older, lower slot. A
/// rename can land on a path that is already pinned: the destination's
/// note vanished outside kimün, its pin was kept as missing by design, and
/// `nfs::rename_path` only refuses a destination that exists on disk.
fn dedup(notes: &mut Vec<VaultPath>) -> bool {
    let before = notes.len();
    let mut seen: Vec<VaultPath> = Vec::with_capacity(before);
    notes.retain(|n| {
        let c = n.canonical();
        if seen.iter().any(|s| s.is_like(&c)) {
            false
        } else {
            seen.push(c);
            true
        }
    });
    notes.len() != before
}

/// A note was renamed/moved: point its pin at the new path. `to` is stored
/// canonical, so the entry stays in the on-disk form regardless of how the
/// caller built `to`. A stale pin already at `to` is dropped (see `dedup`).
pub fn rewrite_note_rename(notes: &mut Vec<VaultPath>, from: &VaultPath, to: &VaultPath) -> bool {
    let from = from.canonical();
    let to = to.canonical();
    match notes.iter_mut().find(|n| n.is_like(&from)) {
        Some(slot) => {
            *slot = to;
            dedup(notes);
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

/// A directory was renamed: rewrite every pin beneath it. Stale pins the
/// rewrite lands on are dropped (see `dedup`).
pub fn rewrite_directory_rename(
    notes: &mut Vec<VaultPath>,
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
    if changed {
        dedup(notes);
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

/// [`edit`] for the rename/delete hooks. Pins are non-critical bookkeeping
/// beside a note operation that already succeeded, so any failure is logged
/// and swallowed — the caller's operation stands.
async fn best_effort_edit(
    workspace_path: &SystemPath,
    what: &str,
    f: impl FnOnce(&mut Vec<VaultPath>) -> bool,
) {
    if let Err(e) = edit(workspace_path, |all| ((), f(all))).await {
        log::warn!("pinned notes: could not update list while {what}: {e}");
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
pub async fn on_directory_renamed(workspace_path: &SystemPath, from: &VaultPath, to: &VaultPath) {
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
    async fn read_keeps_a_file_holding_more_than_the_cap() {
        // Not producible through `toggle` (the only cap guard) — this is
        // what a hand-edit or a three-way merge of `notes = [ … ]` across
        // two machines looks like: more entries than the cap, on disk. Read
        // returns them all, so a write-back never drops the extras.
        let dir = tempfile::TempDir::new().unwrap();
        let ws = sys(dir.path());
        let kimun_dir = dir.path().join(".kimun");
        tokio::fs::create_dir_all(&kimun_dir).await.unwrap();
        let extra = PINNED_NOTES_CAP + 3;
        let entries: Vec<String> = (0..extra).map(|i| format!("\"/n{i}.md\"")).collect();
        let body = format!("notes = [{}]\n", entries.join(", "));
        tokio::fs::write(kimun_dir.join("pinned-notes.toml"), body)
            .await
            .unwrap();

        let got = read_pinned_notes(&ws).await.unwrap();
        assert_eq!(got.len(), extra, "every entry kept, in file order");

        // An unrelated edit writes back the whole list, extras included.
        edit(&ws, |all| ((), remove(all, &p("/n0.md"))))
            .await
            .unwrap();
        let got = read_pinned_notes(&ws).await.unwrap();
        assert_eq!(got.len(), extra - 1);
        assert_eq!(got.last(), Some(&p(&format!("/n{}.md", extra - 1))));
    }

    #[tokio::test]
    async fn edit_writes_only_when_the_closure_reports_a_change() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = sys(dir.path());
        let out = edit(&ws, |all| (all.len(), false)).await.unwrap();
        assert_eq!(out, 0);
        assert!(
            !dir.path().join(".kimun").join("pinned-notes.toml").exists(),
            "an unchanged list must not create the file"
        );
        let out = edit(&ws, |all| (toggle(all, &p("a.md")), true))
            .await
            .unwrap();
        assert_eq!(out, PinToggle::Pinned { position: 0 });
        assert_eq!(read_pinned_notes(&ws).await.unwrap(), vec![p("/a.md")]);
    }

    /// Two edits started together must both land: the second reads what the
    /// first wrote instead of a stale copy. Without the per-file lock the
    /// last write would win and one edit would silently vanish.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_edits_serialize_instead_of_losing_one() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = sys(dir.path());
        let mut tasks = Vec::new();
        for i in 0..PINNED_NOTES_CAP {
            let ws = ws.clone();
            tasks.push(tokio::spawn(async move {
                edit(&ws, |all| {
                    ((), toggle(all, &p(&format!("n{i}.md"))) != PinToggle::Full)
                })
                .await
                .unwrap();
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(
            read_pinned_notes(&ws).await.unwrap().len(),
            PINNED_NOTES_CAP,
            "every toggle must have landed"
        );
    }

    /// A crash mid-write must leave the previous list intact, never a
    /// half-written file: the write goes through `replace_atomically`, so
    /// the target is only ever a complete file (proved by the temp-then-
    /// rename leaving no sibling temp file behind on the happy path, and by
    /// the body being the exact serialization).
    #[tokio::test]
    async fn write_replaces_the_file_whole() {
        let dir = tempfile::TempDir::new().unwrap();
        let ws = sys(dir.path());
        write_pinned_notes(&ws, &[p("a.md")]).await.unwrap();
        write_pinned_notes(&ws, &[p("b.md"), p("c.md")])
            .await
            .unwrap();
        let kimun_dir = dir.path().join(".kimun");
        let mut names: Vec<String> = std::fs::read_dir(&kimun_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["pinned-notes.toml".to_string()],
            "no temp file left behind"
        );
        let body = tokio::fs::read_to_string(kimun_dir.join("pinned-notes.toml"))
            .await
            .unwrap();
        assert_eq!(body, "notes = [\n    \"/b.md\",\n    \"/c.md\",\n]\n");
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
        assert_eq!(
            toggle(&mut notes, &p("b.md")),
            PinToggle::Pinned { position: 1 }
        );
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
        assert_eq!(
            toggle(&mut relative_store, &p("/a.md")),
            PinToggle::Unpinned
        );
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
    fn move_by_is_anchored_on_the_path() {
        let mut notes = vec![p("a.md"), p("b.md"), p("c.md")];
        assert!(move_by(&mut notes, &p("/a.md"), 2));
        assert_eq!(notes, vec![p("b.md"), p("c.md"), p("a.md")]);
        assert!(move_by(&mut notes, &p("a.md"), -1));
        assert_eq!(notes, vec![p("b.md"), p("a.md"), p("c.md")]);
        // Not pinned, zero, and out-of-range moves leave the list alone.
        assert!(!move_by(&mut notes, &p("zzz.md"), 1));
        assert!(!move_by(&mut notes, &p("a.md"), 0));
        assert!(!move_by(&mut notes, &p("b.md"), -1));
        assert!(!move_by(&mut notes, &p("c.md"), 1));
        assert_eq!(notes, vec![p("b.md"), p("a.md"), p("c.md")]);
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
        assert!(rewrite_note_rename(
            &mut notes,
            &p("dir/b.md"),
            &p("other/c.md")
        ));
        // The rewritten slot is stored canonical even though `to` was relative;
        // the untouched entry keeps whatever form it already had.
        assert_eq!(notes, vec![p("a.md"), p("/other/c.md")]);
        assert!(!rewrite_note_rename(&mut notes, &p("nope.md"), &p("x.md")));
    }

    #[test]
    fn directory_rename_rewrites_entries_beneath_it() {
        let mut notes = vec![
            p("proj/a.md"),
            p("proj/sub/b.md"),
            p("projx/c.md"),
            p("d.md"),
        ];
        assert!(rewrite_directory_rename(&mut notes, &p("proj"), &p("work")));
        assert_eq!(
            notes,
            vec![
                p("/work/a.md"),
                p("/work/sub/b.md"),
                p("projx/c.md"),
                p("d.md")
            ]
        );
        assert!(!rewrite_directory_rename(
            &mut notes,
            &p("missing"),
            &p("x")
        ));
    }

    #[test]
    fn directory_rename_matches_regardless_of_absoluteness() {
        // Stored relative, renamed with an absolute `from`.
        let mut notes = vec![p("proj/a.md")];
        assert!(rewrite_directory_rename(
            &mut notes,
            &p("/proj"),
            &p("work")
        ));
        assert_eq!(notes, vec![p("/work/a.md")]);

        // Stored canonical (absolute), renamed with a relative `from`/`to`.
        let mut notes = vec![p("/proj/a.md")];
        assert!(rewrite_directory_rename(&mut notes, &p("proj"), &p("work")));
        assert_eq!(notes, vec![p("/work/a.md")]);
    }

    /// Renaming onto a path whose stale pin was kept as "missing" must not
    /// leave the note pinned twice: the renamed note keeps its slot and the
    /// stale pin goes.
    #[test]
    fn note_rename_onto_a_stale_pin_leaves_one_entry() {
        let mut notes = vec![p("/a.md"), p("/b.md"), p("/c.md")];
        assert!(rewrite_note_rename(&mut notes, &p("a.md"), &p("b.md")));
        assert_eq!(notes, vec![p("/b.md"), p("/c.md")]);

        // The stale pin sitting *before* the renamed one: the earlier slot
        // survives either way, so the list is still one entry per note.
        let mut notes = vec![p("/b.md"), p("/a.md"), p("/c.md")];
        assert!(rewrite_note_rename(&mut notes, &p("a.md"), &p("b.md")));
        assert_eq!(notes, vec![p("/b.md"), p("/c.md")]);
    }

    #[test]
    fn directory_rename_onto_stale_pins_leaves_one_entry_each() {
        let mut notes = vec![p("/proj/a.md"), p("/work/a.md"), p("/work/b.md")];
        assert!(rewrite_directory_rename(&mut notes, &p("proj"), &p("work")));
        assert_eq!(notes, vec![p("/work/a.md"), p("/work/b.md")]);
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
