//! File operations on vault entries, toolkit-free: the target paths the
//! rename / move dialogs build, the availability check they show while the
//! user types, and the core calls behind delete / rename / move / create /
//! quick note / journal, each mapped to the [`FileOp`] (or path) the frontend
//! reports. The TUI dialogs and the GUI's file-op dialogs both call these, so
//! the two cannot drift.
//!
//! Every mutation goes through `NoteVault` (core); nothing here touches the
//! filesystem. Paths are built with core's `VaultPath` helpers, never by
//! joining strings.
//!
//! Two flavours of target builder:
//! - `*_candidate` is the raw construction the TUI has always used (no
//!   validation; an odd input sanitizes to some path and the availability
//!   check or core decides). The TUI keeps it so its behaviour is unchanged.
//! - `*_target` validates first and returns a user-facing reason when the
//!   input cannot name a sibling (rename) or the destination is the entry
//!   itself or inside it (move). New frontends use these.

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use crate::logic::list::{ListRow, fuzzy_rank};
use crate::logic::message::{AppTx, AppTxExt, FileOp};

/// Tracks the current state of an async name / destination availability
/// check, as the rename and move dialogs show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationState {
    /// No check has been triggered yet (initial state).
    Idle,
    /// A check is in progress.
    Pending,
    /// The name / destination is available (does not already exist).
    Available,
    /// The name / destination is already taken.
    Taken,
}

/// The result of [`availability`]: whether an entry can be created at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Nothing exists there.
    Free,
    /// An entry already exists there.
    Taken,
    /// The path names the vault root (or nothing): no entry can go there.
    Invalid,
}

impl Availability {
    /// `true` only for [`Availability::Free`].
    pub fn is_free(self) -> bool {
        self == Availability::Free
    }
}

impl From<Availability> for ValidationState {
    /// `Free` → `Available`; `Taken` and `Invalid` → `Taken` (the dialogs
    /// have no third state: either the confirm key works or it does not).
    fn from(a: Availability) -> Self {
        if a.is_free() {
            ValidationState::Available
        } else {
            ValidationState::Taken
        }
    }
}

/// `name` as the last component of an entry like `from`: a note keeps (or
/// gains) the note extension, a directory or attachment takes it as is.
fn entry_name_like(from: &VaultPath, name: &str) -> VaultPath {
    if from.is_note() {
        VaultPath::note_path_from(name)
    } else {
        VaultPath::new(name)
    }
}

/// The path `from` would have after renaming it to `input` in its own
/// directory. No validation (see the module docs); the TUI's rename dialog.
pub fn rename_candidate(from: &VaultPath, input: &str) -> VaultPath {
    let parent = from.get_parent_path().0;
    parent.append(&entry_name_like(from, input))
}

/// [`rename_candidate`], refused when `input` does not name exactly one entry
/// in `from`'s directory (empty, `.`/`..`, or a path with several components).
pub fn rename_target(from: &VaultPath, input: &str) -> Result<VaultPath, String> {
    if input.trim().is_empty() {
        return Err("Name cannot be empty".to_string());
    }
    let parent = from.get_parent_path().0;
    let candidate = rename_candidate(from, input);
    // Flattening drops `.` / `..` components: a plain name survives it intact.
    let flat = candidate.flatten();
    let one_plain_name = flat.is_like(&candidate)
        && flat.get_slices().len() == parent.get_slices().len() + 1
        && parent.is_ancestor_of(&flat);
    if !one_plain_name {
        return Err("Enter a name, not a path".to_string());
    }
    Ok(candidate)
}

/// The path `from` would have after moving it into `dest_dir` (same name).
/// No validation; the TUI's move dialog.
pub fn move_candidate(from: &VaultPath, dest_dir: &VaultPath) -> VaultPath {
    let filename = from.get_parent_path().1;
    dest_dir.append(&entry_name_like(from, &filename))
}

/// [`move_candidate`], refused when `dest_dir` is `from` itself or lies inside
/// it (a directory cannot move into itself).
pub fn move_target(from: &VaultPath, dest_dir: &VaultPath) -> Result<VaultPath, String> {
    if dest_dir.is_like(from) || from.is_ancestor_of(dest_dir) {
        return Err("Cannot move a directory into itself".to_string());
    }
    Ok(move_candidate(from, dest_dir))
}

/// Where `path` lives after `from` was renamed or moved to `to`: `to` itself
/// when `path` is `from`, the same place under `to` when `from` is a
/// directory holding `path`, `None` when the operation did not touch it.
/// Slice-based (core's `is_ancestor_of` / `get_relative_to` / `append`), so
/// case and absolute / relative spellings do not matter.
pub fn relocated(path: &VaultPath, from: &VaultPath, to: &VaultPath) -> Option<VaultPath> {
    if path.is_like(from) {
        return Some(to.clone());
    }
    if !from.is_ancestor_of(path) {
        return None;
    }
    // `from` is a proper prefix of `path`: what is left is plain components.
    let rest = path.get_relative_to(from);
    Some(to.append(&rest))
}

/// Whether an entry could be created at `candidate` — the check the rename
/// and move dialogs run as the input / selection changes.
pub async fn availability(vault: &NoteVault, candidate: &VaultPath) -> Availability {
    if candidate.is_root_or_empty() {
        return Availability::Invalid;
    }
    if vault.exists(candidate).await {
        Availability::Taken
    } else {
        Availability::Free
    }
}

/// Delete the note, directory or attachment at `path` (core classifies it).
pub async fn delete(vault: &NoteVault, path: &VaultPath) -> Result<FileOp, String> {
    vault
        .delete_entry(path)
        .await
        .map(|()| FileOp::Deleted(path.clone()))
        .map_err(|e| e.to_string())
}

/// Rename `from` to `to` (core classifies the entry and rewrites links for
/// notes).
pub async fn rename(vault: &NoteVault, from: &VaultPath, to: &VaultPath) -> Result<FileOp, String> {
    vault
        .rename_entry(from, to)
        .await
        .map(|()| FileOp::Renamed {
            from: from.clone(),
            to: to.clone(),
        })
        .map_err(|e| e.to_string())
}

/// Move `from` to `to` — a cross-directory rename, reported as
/// [`FileOp::Moved`].
pub async fn move_entry(
    vault: &NoteVault,
    from: &VaultPath,
    to: &VaultPath,
) -> Result<FileOp, String> {
    vault
        .rename_entry(from, to)
        .await
        .map(|()| FileOp::Moved {
            from: from.clone(),
            to: to.clone(),
        })
        .map_err(|e| e.to_string())
}

/// The note a create / journal resolved to, and whether it was made just now
/// (`false`: it already existed and was loaded instead).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteOutcome {
    pub path: VaultPath,
    pub created: bool,
}

impl NoteOutcome {
    /// Report it: [`FileOp::Created`] when fresh, then open it.
    pub fn announce_and_open(self, tx: &AppTx) {
        tx.announce_and_open(self.path, self.created);
    }
}

/// The create-note dialog's confirm. With `content` (pre-filled text, e.g. a
/// saved Ask answer) the note always lands in a fresh file — a taken `path`
/// gets a conflict suffix, never a silent drop into the existing note. With
/// no content an existing note at `path` is loaded instead of created.
pub async fn create_note(
    vault: &NoteVault,
    path: &VaultPath,
    content: Option<String>,
) -> Result<NoteOutcome, String> {
    let result = match content {
        Some(text) => vault
            .create_note_avoiding_conflicts(path, text)
            .await
            .map(|path| NoteOutcome {
                path,
                created: true,
            }),
        None => vault
            .load_or_create_note(path, None)
            .await
            .map(|(_, created)| NoteOutcome {
                path: path.clone(),
                created,
            }),
    };
    result.map_err(|e| e.to_string())
}

/// A timestamped note holding `text` in the inbox; returns its path. The
/// caller reports [`FileOp::Created`] and opens it or not.
pub async fn quick_note(vault: &NoteVault, text: &str) -> Result<VaultPath, String> {
    vault
        .quick_note(text)
        .await
        .map(|details| details.path)
        .map_err(|e| e.to_string())
}

/// Today's journal entry, created with its date heading when missing.
pub async fn journal(vault: &NoteVault) -> Result<NoteOutcome, String> {
    vault
        .journal_entry()
        .await
        .map(|(details, _, created)| NoteOutcome {
            path: details.path,
            created,
        })
        .map_err(|e| e.to_string())
}

/// `OpenJournal`: resolve today's entry (creating it if needed), announce it
/// when fresh and open it. A failure does nothing, as in the TUI.
pub async fn open_journal(vault: &NoteVault, tx: &AppTx) {
    if let Ok(outcome) = journal(vault).await {
        outcome.announce_and_open(tx);
    }
}

/// Every directory of the vault, the root first-sorted with the rest — the
/// move dialog's destinations. Walks the disk: run it off the UI thread.
pub fn list_directories(vault: &NoteVault) -> Result<Vec<VaultPath>, String> {
    let dirs = vault
        .get_directories(&VaultPath::root(), true)
        .map_err(|e| e.to_string())?;
    let mut paths: Vec<VaultPath> = std::iter::once(VaultPath::root())
        .chain(dirs.into_iter().map(|d| d.path))
        .collect();
    paths.sort();
    Ok(paths)
}

#[derive(Clone)]
struct DirRow(String);

impl ListRow for DirRow {
    fn match_text(&self) -> Option<&str> {
        Some(&self.0)
    }
}

/// `dirs` fuzzy-ranked against `query` (best first, ties in list order); all
/// of them, in order, for an empty query.
pub fn filter_directories(dirs: &[VaultPath], query: &str) -> Vec<VaultPath> {
    let rows: Vec<DirRow> = dirs.iter().map(|d| DirRow(d.to_string())).collect();
    fuzzy_rank(&rows, query)
        .into_iter()
        .map(|i| dirs[i].clone())
        .collect()
}

#[cfg(test)]
mod tests;
