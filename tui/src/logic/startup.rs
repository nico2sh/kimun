//! Startup rules shared by every frontend: opening the vault the settings
//! name, and choosing which note a session lands on.

use std::sync::Arc;

use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, VaultConfig};

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
        let s = settings.read().unwrap();
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
    let history = settings.read().unwrap().current_last_paths();
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
        settings.read().unwrap().edit_path_history(|paths| {
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
}
