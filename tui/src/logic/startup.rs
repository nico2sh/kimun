//! Startup rules shared by every frontend: opening the vault the settings
//! name, and choosing which note a session lands on.

use std::sync::Arc;
use std::time::Duration;

use kimun_core::error::VaultError;
use kimun_core::nfs::VaultPath;
use kimun_core::{IndexReport, NoteVault, VaultConfig};

use crate::settings::SharedSettings;

/// A fresh `NoteVault` for whatever workspace the settings currently
/// resolve to, wired to the configured index and the workspace's inbox.
/// `None` if no workspace is configured or the vault fails to open.
///
/// Used at startup and every time the workspace changes (preferences
/// saved, onboarding finished, workspace switched).
pub async fn open_vault(settings: &SharedSettings) -> Option<Arc<NoteVault>> {
    match try_open_vault(settings).await {
        VaultOpen::Opened(vault) => Some(vault),
        VaultOpen::NoWorkspace | VaultOpen::Failed(_) => None,
    }
}

/// What [`try_open_vault`] found.
#[derive(Debug)]
pub enum VaultOpen {
    /// The settings name no workspace (onboarding has not run).
    NoWorkspace,
    /// The workspace is configured but would not open; the text names its
    /// path and says why (e.g. the index is locked by another kimün process).
    Failed(String),
    Opened(Arc<NoteVault>),
}

/// [`open_vault`], keeping the reason a configured workspace failed to open,
/// for a frontend that shows it rather than only logging it (the GUI's
/// empty state). Logs the failure the same way.
pub async fn try_open_vault(settings: &SharedSettings) -> VaultOpen {
    let (workspace_path, cache_path, inbox_path) = {
        let s = settings.read().expect("settings lock poisoned");
        let wp = s.resolve_workspace_path();
        let name = s.current_workspace_name();
        let cache = name.as_ref().map(|n| s.index_for(n));
        let ip = s
            .workspace_config
            .as_ref()
            .and_then(|wc| wc.get_current_workspace())
            .map(|e| e.effective_inbox_path());
        (wp, cache, ip)
    };
    let Some(workspace) = workspace_path else {
        return VaultOpen::NoWorkspace;
    };
    let mut config = VaultConfig::new(workspace.clone());
    if let Some(cp) = cache_path {
        config = config.with_index(cp);
    }
    match NoteVault::new(config).await {
        Ok(mut v) => {
            if let Some(ref ip) = inbox_path {
                v.set_inbox_path(VaultPath::new(ip));
            }
            VaultOpen::Opened(Arc::new(v))
        }
        // Don't swallow the cause: since the index self-heal, opening the
        // vault can fail on a cache probe error (e.g. the cache is locked
        // by another kimun process). The app falls back to the no-vault
        // start screen either way, but the reason must reach the log
        // instead of looking like an unconfigured workspace.
        Err(e) => {
            tracing::error!("could not open vault at {}: {e}", workspace);
            VaultOpen::Failed(format!("{workspace}: {e}"))
        }
    }
}

/// Where a restored session lands, and what to tell the user about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreTarget {
    pub path: VaultPath,
    /// Set when the note the session would have landed on is gone: the
    /// frontend shows it (the TUI parks it as a flash).
    pub gone: Option<String>,
}

/// Where the restored session lands: the newest history entry still on
/// disk, or the vault root when none is. Notes moved or deleted outside
/// kimün are skipped, not offered for re-creation — the user asked for
/// their last note, not for a new one — and dropped from the history so
/// they stop turning up in recents.
///
/// When the note it would have landed on is the one gone, `gone` says so:
/// the editor opening somewhere else unannounced reads as kimün losing the
/// note.
///
/// Only trusted after a successful index (`vault_reachable`): when the
/// vault itself can't be read every note looks gone, and pruning then
/// would wipe the history for good. It lands on the newest entry
/// unchecked instead.
pub async fn restore_target(
    settings: &SharedSettings,
    vault: Option<&Arc<NoteVault>>,
    vault_reachable: bool,
) -> RestoreTarget {
    let history = settings
        .read()
        .expect("settings lock poisoned")
        .current_last_paths();
    // No vault (no workspace — the open is routed to onboarding) or one
    // that failed to index: nothing trustworthy to check the history
    // against.
    let Some(vault) = vault.filter(|_| vault_reachable) else {
        return RestoreTarget {
            path: history.first().cloned().unwrap_or_else(VaultPath::root),
            gone: None,
        };
    };
    let mut live = Vec::with_capacity(history.len());
    let mut gone = Vec::new();
    for path in &history {
        // Gone means the filesystem said not found. Any other failure
        // (permissions, I/O) keeps the entry: it may well still be there.
        if vault
            .entry_kind(path)
            .await
            .is_err_and(|e| e.is_not_found())
        {
            gone.push(path.clone());
        } else {
            live.push(path.clone());
        }
    }
    if !gone.is_empty() {
        settings
            .read()
            .expect("settings lock poisoned")
            .edit_path_history(|paths| {
                paths.retain(|p| !gone.iter().any(|g| g.is_like(p)));
                true
            });
    }
    let gone_message = history
        .first()
        .filter(|last| live.first() != Some(*last))
        .map(|last| format!("{last} is gone (moved or deleted outside kimün)"));
    RestoreTarget {
        path: live.first().cloned().unwrap_or_else(VaultPath::root),
        gone: gone_message,
    }
}

/// How an indexing pass ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexOutcome {
    /// Finished in this long.
    Done(Duration),
    /// The vault is structurally unusable (a case conflict): the message
    /// to show.
    Conflict(String),
    /// Anything else went wrong: the message to show.
    Failed(String),
}

impl IndexOutcome {
    /// Deliver this outcome as `AppMessage::VaultIndexed`. A closed channel
    /// (the app is shutting down) is ignored.
    pub fn send(self, tx: &crate::logic::message::AppTx) {
        tx.send(crate::logic::message::AppMessage::VaultIndexed(self))
            .ok();
    }
}

impl From<Result<IndexReport, VaultError>> for IndexOutcome {
    fn from(result: Result<IndexReport, VaultError>) -> Self {
        match result {
            Ok(report) => Self::Done(report.duration),
            Err(e @ VaultError::CaseConflict { .. }) => Self::Conflict(e.to_string()),
            Err(e) => Self::Failed(e.to_string()),
        }
    }
}

impl From<Result<Duration, String>> for IndexOutcome {
    fn from(result: Result<Duration, String>) -> Self {
        match result {
            Ok(duration) => Self::Done(duration),
            Err(message) => Self::Failed(message),
        }
    }
}

/// Whether the open vault is usable. A conflict stays until an indexing
/// pass finishes cleanly; a failed pass says nothing about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum VaultStatus {
    #[default]
    Ready,
    /// The vault has a case conflict: the message to show.
    Conflicted(String),
}

impl VaultStatus {
    /// Fold the end of an indexing pass into the status.
    pub fn apply(&mut self, outcome: &IndexOutcome) {
        match outcome {
            IndexOutcome::Done(_) => *self = Self::Ready,
            IndexOutcome::Conflict(msg) => *self = Self::Conflicted(msg.clone()),
            IndexOutcome::Failed(_) => {}
        }
    }

    pub fn is_conflicted(&self) -> bool {
        matches!(self, Self::Conflicted(_))
    }

    /// The conflict message, if conflicted.
    pub fn conflict(&self) -> Option<&str> {
        match self {
            Self::Conflicted(msg) => Some(msg),
            Self::Ready => None,
        }
    }
}

/// Validate and index `vault`, logging a conflict or a failure.
pub async fn index_vault(vault: &NoteVault) -> IndexOutcome {
    let outcome = IndexOutcome::from(vault.validate_and_init().await);
    match &outcome {
        IndexOutcome::Done(_) => {}
        IndexOutcome::Conflict(msg) => tracing::warn!("{msg}"),
        IndexOutcome::Failed(msg) => tracing::error!("indexing failed: {msg}"),
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use crate::test_support::{sys, temp_vault};
    use std::sync::RwLock;

    /// A vault holding `existing` notes, and settings whose history for it is
    /// `history` (newest first). Returns the history dir guard with the rest.
    async fn fixture(
        history: &[&str],
        existing: &[&str],
    ) -> (Arc<NoteVault>, SharedSettings, tempfile::TempDir) {
        let vault = temp_vault("startup").await;
        for note in existing {
            vault
                .create_note(&VaultPath::new(*note), "text")
                .await
                .unwrap();
        }
        let history_dir = tempfile::TempDir::new().unwrap();
        let settings =
            AppSettings::for_test_workspace("ws", vault.workspace_path(), sys(history_dir.path()));
        settings.seed_test_history(history);
        (vault, Arc::new(RwLock::new(settings)), history_dir)
    }

    fn history(settings: &SharedSettings) -> Vec<VaultPath> {
        settings.read().unwrap().current_last_paths()
    }

    #[tokio::test]
    async fn open_vault_is_none_without_a_workspace() {
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        assert!(open_vault(&settings).await.is_none());
    }

    #[tokio::test]
    async fn open_vault_opens_the_configured_workspace() {
        let dir = tempfile::TempDir::new().unwrap();
        let hist = tempfile::TempDir::new().unwrap();
        let settings = AppSettings::for_test_workspace("ws", &sys(dir.path()), sys(hist.path()));
        let settings: SharedSettings = Arc::new(RwLock::new(settings));
        let vault = open_vault(&settings).await.expect("vault opens");
        assert_eq!(vault.workspace_path(), &sys(dir.path()));
    }

    #[tokio::test]
    async fn try_open_vault_tells_no_workspace_from_a_failed_open() {
        let settings: SharedSettings = Arc::new(RwLock::new(AppSettings::default()));
        assert!(matches!(
            try_open_vault(&settings).await,
            VaultOpen::NoWorkspace
        ));

        let dir = tempfile::TempDir::new().unwrap();
        let hist = tempfile::TempDir::new().unwrap();
        let missing = sys(dir.path()).join("not-there");
        let settings = AppSettings::for_test_workspace("ws", &missing, sys(hist.path()));
        let settings: SharedSettings = Arc::new(RwLock::new(settings));
        match try_open_vault(&settings).await {
            VaultOpen::Failed(reason) => {
                assert!(reason.contains("not-there"), "names the path: {reason}")
            }
            other => panic!("expected a failed open, got {other:?}"),
        }
        // The `Option` form still folds both into `None`.
        assert!(open_vault(&settings).await.is_none());
    }

    #[tokio::test]
    async fn restore_skips_a_last_note_gone_from_disk_and_says_so() {
        let (vault, settings, _d) = fixture(&["gone.md", "still.md"], &["still.md"]).await;
        let t = restore_target(&settings, Some(&vault), true).await;
        assert_eq!(t.path, VaultPath::new("still.md"));
        let msg = t.gone.expect("gone message");
        assert!(msg.contains("gone.md"), "must name the note: {msg}");
    }

    /// Only the note the user expected to land on is worth a word; older
    /// entries vanishing from recents is not news.
    #[tokio::test]
    async fn restore_is_silent_when_only_older_entries_are_gone() {
        let (vault, settings, _d) = fixture(&["still.md", "gone.md"], &["still.md"]).await;
        let t = restore_target(&settings, Some(&vault), true).await;
        assert_eq!(t.path, VaultPath::new("still.md"));
        assert_eq!(t.gone, None);
    }

    #[tokio::test]
    async fn restore_lands_on_the_root_when_no_history_entry_survives() {
        let (vault, settings, _d) = fixture(&["a.md", "b.md"], &[]).await;
        let t = restore_target(&settings, Some(&vault), true).await;
        assert_eq!(t.path, VaultPath::root());
    }

    #[tokio::test]
    async fn restore_drops_notes_gone_from_disk_from_the_history() {
        let (vault, settings, _d) =
            fixture(&["gone.md", "still.md", "also_gone.md"], &["still.md"]).await;
        restore_target(&settings, Some(&vault), true).await;
        assert_eq!(history(&settings), vec![VaultPath::new("still.md")]);
    }

    /// A failed index usually means the vault itself is out of reach (an
    /// unmounted drive, a sync folder not there yet). Every note looks
    /// missing then, and pruning would wipe the history for good.
    #[tokio::test]
    async fn restore_after_failed_indexing_leaves_the_history_alone() {
        let (vault, settings, _d) = fixture(&["gone.md", "still.md"], &["still.md"]).await;
        let t = restore_target(&settings, Some(&vault), false).await;
        assert_eq!(
            history(&settings),
            vec![VaultPath::new("gone.md"), VaultPath::new("still.md")]
        );
        assert_eq!(t.path, VaultPath::new("gone.md"));
        assert_eq!(t.gone, None);
    }

    #[tokio::test]
    async fn restore_without_a_vault_takes_the_newest_entry_unchecked() {
        let (_vault, settings, _d) = fixture(&["a.md"], &[]).await;
        let t = restore_target(&settings, None, true).await;
        assert_eq!(t.path, VaultPath::new("a.md"));
        assert_eq!(history(&settings), vec![VaultPath::new("a.md")]);
    }

    /// Only a note the filesystem reports as not found is gone. One it can't
    /// look at right now (permissions, I/O) may well still be there.
    #[cfg(unix)]
    #[tokio::test]
    async fn restore_keeps_a_note_it_cannot_check() {
        use std::os::unix::fs::PermissionsExt;

        let (vault, settings, _d) = fixture(
            &["locked/note.md", "still.md"],
            &["locked/note.md", "still.md"],
        )
        .await;
        let locked = vault.workspace_path().join("locked").into_path_buf();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        restore_target(&settings, Some(&vault), true).await;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            history(&settings),
            vec![VaultPath::new("locked/note.md"), VaultPath::new("still.md")]
        );
    }

    #[test]
    fn send_delivers_vault_indexed() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        IndexOutcome::Conflict("x".into()).send(&tx);
        match rx.try_recv() {
            Ok(crate::logic::message::AppMessage::VaultIndexed(IndexOutcome::Conflict(m))) => {
                assert_eq!(m, "x")
            }
            other => panic!("unexpected: {other:?}"),
        }
        drop(rx);
        IndexOutcome::Failed("y".into()).send(&tx); // closed channel: no panic
    }

    #[test]
    fn from_a_case_conflict_error_is_conflict() {
        let e = kimun_core::error::VaultError::CaseConflict {
            conflicts: vec!["a.md vs A.md".to_string()],
        };
        let msg = e.to_string();
        assert_eq!(IndexOutcome::from(Err(e)), IndexOutcome::Conflict(msg));
    }

    #[test]
    fn from_another_error_is_failed() {
        let e = kimun_core::error::VaultError::TaskJoin("boom".to_string());
        let msg = e.to_string();
        assert_eq!(IndexOutcome::from(Err(e)), IndexOutcome::Failed(msg));
        assert_eq!(
            IndexOutcome::from(Err::<std::time::Duration, String>("x".into())),
            IndexOutcome::Failed("x".into())
        );
    }

    #[test]
    fn from_ok_report_is_done_with_its_duration() {
        let report = kimun_core::IndexReport {
            start: std::time::SystemTime::now(),
            duration: std::time::Duration::from_millis(42),
        };
        assert_eq!(
            IndexOutcome::from(Ok(report)),
            IndexOutcome::Done(std::time::Duration::from_millis(42))
        );
    }

    fn conflicted() -> VaultStatus {
        VaultStatus::Conflicted("old".into())
    }

    #[test]
    fn done_makes_the_status_ready() {
        let mut s = VaultStatus::default();
        s.apply(&IndexOutcome::Done(Duration::from_millis(1)));
        assert_eq!(s, VaultStatus::Ready);
        assert!(!s.is_conflicted());
        assert_eq!(s.conflict(), None);
    }

    #[test]
    fn conflict_stores_the_message() {
        let mut s = VaultStatus::default();
        s.apply(&IndexOutcome::Conflict("a vs A".into()));
        assert!(s.is_conflicted());
        assert_eq!(s.conflict(), Some("a vs A"));
    }

    #[test]
    fn failed_leaves_a_conflicted_status_unchanged() {
        let mut s = conflicted();
        s.apply(&IndexOutcome::Failed("boom".into()));
        assert_eq!(s, conflicted());
    }

    #[test]
    fn failed_leaves_a_ready_status_ready() {
        let mut s = VaultStatus::Ready;
        s.apply(&IndexOutcome::Failed("boom".into()));
        assert_eq!(s, VaultStatus::Ready);
    }

    #[test]
    fn a_later_done_clears_a_conflict() {
        let mut s = conflicted();
        s.apply(&IndexOutcome::Done(Duration::from_millis(1)));
        assert_eq!(s, VaultStatus::Ready);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn index_vault_on_a_case_clashing_vault_is_conflict() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("note.md"), "a").unwrap();
        std::fs::write(tmp.path().join("Note.md"), "b").unwrap();
        let vault = NoteVault::new(VaultConfig::new(sys(tmp.path())))
            .await
            .unwrap();
        match index_vault(&vault).await {
            IndexOutcome::Conflict(msg) => {
                assert!(msg.contains("note.md") && msg.contains("Note.md"), "{msg}")
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }
}
