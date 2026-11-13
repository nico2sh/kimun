//! Startup, toolkit-free: what the window works on (the configured vault,
//! or an empty state saying why there is none), which note it lands on
//! first, and the background indexing that follows. `main` decides; the
//! [`Workspace`](crate::workspace::Workspace) acts on the decision and on
//! the messages [`index_and_restore`] sends.
//!
//! **The first note.** A note named on the command line opens at once (it
//! needs no index). Otherwise, once indexing has finished, the window
//! returns to where the workspace's session record left it (its tabs, their
//! groups and notes); with no record, or one with no note left to open, the
//! session is restored from the history, as the TUI does. Only a vault that
//! indexed is trusted to say a note is gone (see
//! `logic::startup::restore_target`).

use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::error::VaultError;
use kimun_core::nfs::VaultPath;
use kimun_core::system::SystemPath;
use kimun_notes::logic::editor_region::EditorRegion;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::message::{AppMessage, AppTx};
use kimun_notes::logic::startup::{VaultOpen, restore_target};
use kimun_notes::settings::SharedSettings;
use kimun_notes::settings::session::{GroupTreeRecord, SessionFile, SessionRecord};

/// The application's name, as the window title shows it.
pub(crate) const APP_NAME: &str = "kimün";

/// What the window starts on.
pub(crate) enum Start {
    /// No workspace is configured: the setup assistant opens over the
    /// window (`workspace::onboarding`) to make one.
    NoWorkspace,
    /// The configured workspace would not open; the text says why.
    OpenFailed(String),
    /// The configured vault, open.
    Vault(VaultStart),
}

/// The configured vault and the note the window lands on.
pub(crate) struct VaultStart {
    pub(crate) vault: Arc<NoteVault>,
    /// The workspace's name, for the window title.
    pub(crate) name: String,
    pub(crate) first: FirstNote,
}

/// Where the session starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FirstNote {
    /// From the history, once indexing is done.
    Restore,
    /// The note named on the command line.
    Note(VaultPath),
    /// The command line named something that is not a note of this vault:
    /// the text says what, and the editor starts empty.
    Rejected(String),
}

/// The decision table: the vault open's outcome, the workspace's name and
/// the optional command-line argument give what the window starts on. `cwd`
/// is where kimün was started from, for a relative argument.
pub(crate) fn decide(
    open: VaultOpen,
    name: Option<String>,
    arg: Option<&OsStr>,
    cwd: Option<&SystemPath>,
) -> Start {
    match open {
        // A note on the command line means nothing without a vault: the
        // empty state already says what is missing.
        VaultOpen::NoWorkspace => Start::NoWorkspace,
        VaultOpen::Failed(reason) => Start::OpenFailed(reason),
        VaultOpen::Opened(vault) => {
            let first = match arg {
                None => FirstNote::Restore,
                Some(arg) => match note_from_arg(arg, vault.workspace_path(), cwd) {
                    Ok(path) => FirstNote::Note(path),
                    Err(reason) => FirstNote::Rejected(reason),
                },
            };
            Start::Vault(VaultStart {
                vault,
                name: name.unwrap_or_default(),
                first,
            })
        }
    }
}

/// What the editor area says when there is no vault to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmptyState {
    pub(crate) title: String,
    pub(crate) detail: String,
    /// Something went wrong (as opposed to nothing set up yet).
    pub(crate) failed: bool,
}

/// The editor area's empty state for `start`, `None` when there is a vault.
pub(crate) fn empty_state(start: &Start) -> Option<EmptyState> {
    match start {
        Start::NoWorkspace => Some(EmptyState {
            title: "No workspace configured".into(),
            detail: "The setup assistant makes one.".into(),
            failed: false,
        }),
        Start::OpenFailed(reason) => Some(EmptyState {
            title: "Couldn't open the workspace".into(),
            detail: reason.clone(),
            failed: true,
        }),
        Start::Vault(_) => None,
    }
}

/// The window title: the workspace's name, when there is one.
pub(crate) fn window_title(start: &Start) -> String {
    match start {
        Start::Vault(VaultStart { name, .. }) => title_for_workspace(name),
        _ => APP_NAME.to_string(),
    }
}

/// The window title for a workspace named `name` (just the app's name when
/// the name is empty).
pub(crate) fn title_for_workspace(name: &str) -> String {
    if name.is_empty() {
        APP_NAME.to_string()
    } else {
        format!("{name} - {APP_NAME}")
    }
}

/// The note a command-line argument names. Two spellings are accepted:
///
/// - an OS path inside the workspace: absolute (or `~`-relative), or
///   relative to `cwd` when that names an existing file inside it;
/// - a vault-relative path (`projects/todo`, `projects/todo.md`).
///
/// Anything else is refused with the reason: a path outside the workspace,
/// one that is not a note (a directory, an attachment), or one with
/// characters a vault path cannot hold. The note need not exist; opening it
/// says so.
pub(crate) fn note_from_arg(
    arg: &OsStr,
    workspace: &SystemPath,
    cwd: Option<&SystemPath>,
) -> Result<VaultPath, String> {
    let shown = arg.to_string_lossy().into_owned();
    let path = Path::new(arg);
    let home_relative = shown.starts_with('~');
    if path.is_absolute() || home_relative {
        let os = SystemPath::resolve(path, workspace);
        let vault_path = VaultPath::from_path(workspace, &os)
            .map_err(|_| format!("{shown} is not in the workspace ({workspace})"))?;
        return note_only(vault_path, &shown);
    }
    // Relative: a file next to where kimün was started wins when it is
    // inside the workspace; otherwise the argument is read as a vault path.
    if let Some(cwd) = cwd {
        let os = SystemPath::resolve(path, cwd);
        if os.exists()
            && let Ok(vault_path) = VaultPath::from_path(workspace, &os)
        {
            return note_only(vault_path, &shown);
        }
    }
    if !VaultPath::is_valid(&shown) {
        return Err(format!("{shown} is not a valid note path"));
    }
    note_only(VaultPath::note_path_from(&shown), &shown)
}

fn note_only(path: VaultPath, shown: &str) -> Result<VaultPath, String> {
    if path.is_note() {
        Ok(path)
    } else {
        Err(format!("{shown} is not a note"))
    }
}

/// The arrangement a workspace's session record restores (ADR-0058): the
/// region with its tabs and groups, the note each group opens, and the
/// recorded notes that are gone (dropped from their groups).
pub(crate) struct SessionRestore {
    pub(crate) region: EditorRegion,
    pub(crate) notes: Vec<(GroupId, VaultPath)>,
    pub(crate) gone: Vec<VaultPath>,
}

/// Bring the index in step with the vault, then, when `restore`, land
/// where the last session left off. Reports on `tx` as the TUI's start
/// screen does: `IndexingDone` (or `VaultConflict` for names that differ
/// only by case). Then, when `session`'s record still has a note to open,
/// that record is the answer ([`SessionRestore`]) and nothing more is sent.
/// Otherwise the history's newest note: a `FlashMessage` when the last note
/// is gone, then an `OpenPath` for the note to land on (the vault root when
/// the history has none). Run it on the tokio runtime.
///
/// A recorded note is gone only when the vault indexed and the filesystem
/// says not found, as for the history ([`restore_target`]): a vault that
/// could not be read would make every note look gone.
pub(crate) async fn index_and_restore(
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    restore: bool,
    session: Option<SessionFile>,
    tx: AppTx,
) -> Option<SessionRestore> {
    let indexed = match vault.validate_and_init().await {
        Ok(report) => {
            tx.send(AppMessage::IndexingDone(Ok(report.duration))).ok();
            true
        }
        Err(e @ VaultError::CaseConflict { .. }) => {
            tracing::warn!("{e}");
            tx.send(AppMessage::VaultConflict(e.to_string())).ok();
            false
        }
        Err(e) => {
            tracing::error!("indexing failed: {e}");
            tx.send(AppMessage::IndexingDone(Err(e.to_string()))).ok();
            false
        }
    };
    if !restore {
        return None;
    }
    if let Some(record) = session.and_then(|file| file.load()) {
        let restored = session_restore(&vault, &record, indexed).await;
        if !restored.notes.is_empty() {
            return Some(restored);
        }
    }
    let target = restore_target(&settings, Some(&vault), indexed).await;
    if let Some(gone) = target.gone {
        tx.send(AppMessage::FlashMessage(gone)).ok();
    }
    tx.send(AppMessage::open(target.path)).ok();
    None
}

/// The region `record` restores, its notes checked against the vault when
/// it is trusted (`vault_reachable`).
async fn session_restore(
    vault: &NoteVault,
    record: &SessionRecord,
    vault_reachable: bool,
) -> SessionRestore {
    let mut gone: Vec<VaultPath> = Vec::new();
    if vault_reachable {
        for note in record_notes(record) {
            if note.is_note()
                && vault
                    .entry_kind(&note)
                    .await
                    .is_err_and(|e| e.is_not_found())
            {
                gone.push(note);
            }
        }
    }
    let (region, notes) =
        EditorRegion::from_record(record, |note| !gone.iter().any(|g| g.is_like(note)));
    SessionRestore {
        region,
        notes,
        gone,
    }
}

/// Every note `record` names, tab by tab, in reading order.
fn record_notes(record: &SessionRecord) -> Vec<VaultPath> {
    fn walk(tree: &GroupTreeRecord, out: &mut Vec<VaultPath>) {
        match tree {
            GroupTreeRecord::Leaf { note } => out.extend(note.iter().cloned()),
            GroupTreeRecord::Split { children, .. } => {
                children.iter().for_each(|child| walk(child, out))
            }
        }
    }
    let mut out = Vec::new();
    record.tabs.iter().for_each(|tab| walk(&tab.tree, &mut out));
    out
}

#[cfg(test)]
mod tests;
