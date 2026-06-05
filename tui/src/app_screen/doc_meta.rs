//! **DocMeta** — the editor screen's async document/status state behind one
//! interface: backlink count of the open note, throttled workspace git
//! summary, and the link-under-cursor affordance cache.
//!
//! Everything here shares one shape: a spawn site, a staleness guard, and a
//! small cache — the bug class the revamp reviews kept finding. Concentrating
//! them makes each rule unit-testable by feeding events and asserting
//! segments, with no screen, vault contents, or terminal involved.

use std::sync::Arc;
use std::time::Duration;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use crate::components::events::{AppEvent, AppTx};
use crate::components::text_editor::LinkTarget;

pub struct DocMeta {
    vault: Arc<NoteVault>,
    /// Backlink count of the open note (status line 2), async-loaded.
    backlink_count: Option<usize>,
    /// Workspace git summary for the status bar, `None` when unknown/absent.
    git_status: Option<String>,
    /// When the last git fetch was spawned — throttles the per-event
    /// subprocess (rapid navigation must not fork one `git status` per note).
    last_git_fetch: Option<std::time::Instant>,
    /// When a fetch last reported "no repo / no git" — probing backs off to
    /// once a minute instead of once per open, but a `git init` mid-session
    /// still gets picked up.
    git_unavailable_since: Option<std::time::Instant>,
    /// Link-under-cursor affordance cache: `(target, backlink count once
    /// loaded)`. Refreshed when the cursor enters a different link.
    link_meta: Option<(String, Option<usize>)>,
}

impl DocMeta {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self {
            vault,
            backlink_count: None,
            git_status: None,
            last_git_fetch: None,
            git_unavailable_since: None,
            link_meta: None,
        }
    }

    // ── Reads (status bar segments) ─────────────────────────────────────

    pub fn backlinks(&self) -> Option<usize> {
        self.backlink_count
    }

    pub fn git(&self) -> Option<&String> {
        self.git_status.as_ref()
    }

    // ── Note lifecycle ──────────────────────────────────────────────────

    /// A note was (re)opened: reset and re-fetch its backlink count, and
    /// refresh the git summary.
    pub fn note_opened(&mut self, path: &VaultPath, tx: &AppTx) {
        self.backlink_count = None;
        let vault = self.vault.clone();
        let path = path.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let count = vault
                .get_backlinks(&path)
                .await
                .map(|b| b.len())
                .unwrap_or_default();
            tx2.send(AppEvent::BacklinkCountLoaded { path, count }).ok();
        });
        self.refresh_git(tx);
    }

    /// Spawn the workspace git summary fetch, throttled: at most one
    /// subprocess per couple of seconds, since rapid navigation would
    /// otherwise fork a whole-tree `git status` per note open.
    pub fn refresh_git(&mut self, tx: &AppTx) {
        const GIT_FETCH_MIN_INTERVAL: Duration = Duration::from_secs(2);
        const GIT_UNAVAILABLE_BACKOFF: Duration = Duration::from_secs(60);
        let now = std::time::Instant::now();
        if self
            .git_unavailable_since
            .is_some_and(|t| now.duration_since(t) < GIT_UNAVAILABLE_BACKOFF)
        {
            return;
        }
        if self
            .last_git_fetch
            .is_some_and(|t| now.duration_since(t) < GIT_FETCH_MIN_INTERVAL)
        {
            return;
        }
        self.last_git_fetch = Some(now);
        let root = self.vault.workspace_path().to_path_buf();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let status = crate::util::git_status::fetch(root).await;
            tx2.send(AppEvent::GitStatusLoaded(status)).ok();
        });
    }

    // ── Link-under-cursor affordance (spec §5.2) ────────────────────────

    /// The `→ target · N backlinks` status segment for the link under the
    /// cursor, if any. Caches per target; the count loads async (one fetch
    /// per target change, resolved exactly like follow-link so the count
    /// keys the note that would actually open). `tx == None` (before the
    /// screen's first on_enter) renders the target without a count.
    pub fn link_segment(
        &mut self,
        link: Option<&LinkTarget>,
        current_note: &VaultPath,
        tx: Option<&AppTx>,
    ) -> Option<String> {
        match link {
            Some(LinkTarget::Note(target)) => {
                if self.link_meta.as_ref().map(|(t, _)| t.as_str()) != Some(target.as_str()) {
                    self.link_meta = Some((target.clone(), None));
                    if let Some(tx) = tx {
                        // Resolve like follow_link does: strip a `#fragment`,
                        // then resolve relative targets against this note.
                        let target_clean = target
                            .split('#')
                            .next()
                            .unwrap_or(target)
                            .trim_end()
                            .to_string();
                        let vault = self.vault.clone();
                        let t2 = target.clone();
                        let note_path = current_note.clone();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let path = VaultPath::note_path_from(&target_clean)
                                .resolve_link_in_note(&note_path);
                            let count = vault
                                .get_backlinks(&path)
                                .await
                                .map(|b| b.len())
                                .unwrap_or_default();
                            tx2.send(AppEvent::LinkTargetMeta { target: t2, count })
                                .ok();
                        });
                    }
                }
                match &self.link_meta {
                    Some((t, Some(n))) => Some(format!("→ {t} · {n} backlinks")),
                    Some((t, None)) => Some(format!("→ {t}")),
                    None => None,
                }
            }
            Some(LinkTarget::Label(name)) => {
                self.link_meta = None;
                Some(format!("→ #{name} · tag query"))
            }
            None => {
                self.link_meta = None;
                None
            }
        }
    }

    // ── Event intake ────────────────────────────────────────────────────

    /// Consume the async-result events this module owns; hand anything else
    /// back to the caller. `current_note` drives the staleness guards.
    pub fn handle(&mut self, event: AppEvent, current_note: &VaultPath) -> Option<AppEvent> {
        match event {
            AppEvent::BacklinkCountLoaded { path, count } => {
                // Ignore stale loads for notes already navigated away from.
                if path == *current_note {
                    self.backlink_count = Some(count);
                }
                None
            }
            AppEvent::LinkTargetMeta { target, count } => {
                // Only land the count if the cursor is still on that link.
                if let Some((cached, slot)) = &mut self.link_meta
                    && *cached == target
                {
                    *slot = Some(count);
                }
                None
            }
            AppEvent::GitStatusLoaded(status) => {
                match (&self.git_status, status) {
                    // No repo (or no git): back off to a slow probe.
                    (None, None) => {
                        self.git_unavailable_since = Some(std::time::Instant::now());
                    }
                    // Had a value, fetch failed (index.lock contention, …):
                    // keep showing the last known state.
                    (Some(_), None) => {}
                    (_, some) => {
                        self.git_status = some;
                        self.git_unavailable_since = None;
                    }
                }
                None
            }
            other => Some(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::NoteVault;
    use tokio::sync::mpsc::unbounded_channel;

    async fn meta() -> (DocMeta, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let vault = Arc::new(
            NoteVault::new(kimun_core::VaultConfig::new(dir.path()))
                .await
                .unwrap(),
        );
        (DocMeta::new(vault), dir)
    }

    fn note(p: &str) -> VaultPath {
        VaultPath::note_path_from(p)
    }

    #[tokio::test]
    async fn backlink_count_guards_against_stale_paths() {
        let (mut dm, _dir) = meta().await;
        let current = note("/a.md");
        dm.handle(
            AppEvent::BacklinkCountLoaded {
                path: note("/old.md"),
                count: 7,
            },
            &current,
        );
        assert_eq!(dm.backlinks(), None, "stale path must not land");
        dm.handle(
            AppEvent::BacklinkCountLoaded {
                path: current.clone(),
                count: 3,
            },
            &current,
        );
        assert_eq!(dm.backlinks(), Some(3));
    }

    #[tokio::test]
    async fn git_memoizes_unavailability_and_keeps_last_on_transient_failure() {
        let (mut dm, _dir) = meta().await;
        let current = note("/a.md");
        // First None → unavailable: probing backs off.
        dm.handle(AppEvent::GitStatusLoaded(None), &current);
        assert!(dm.git_unavailable_since.is_some());
        // A value followed by a failure keeps the last value.
        dm.git_unavailable_since = None;
        dm.handle(AppEvent::GitStatusLoaded(Some("git ✓".into())), &current);
        dm.handle(AppEvent::GitStatusLoaded(None), &current);
        assert_eq!(dm.git().map(String::as_str), Some("git ✓"));
    }

    #[tokio::test]
    async fn link_count_lands_only_on_matching_cached_target() {
        let (mut dm, _dir) = meta().await;
        let current = note("/a.md");
        let (tx, _rx) = unbounded_channel();
        let target = LinkTarget::Note("b".to_string());
        // Cursor enters the link: target cached, no count yet.
        let seg = dm.link_segment(Some(&target), &current, Some(&tx));
        assert_eq!(seg.as_deref(), Some("→ b"));
        // A count for a DIFFERENT target is ignored.
        dm.handle(
            AppEvent::LinkTargetMeta {
                target: "other".into(),
                count: 9,
            },
            &current,
        );
        assert_eq!(
            dm.link_segment(Some(&target), &current, Some(&tx))
                .as_deref(),
            Some("→ b")
        );
        // The matching one lands.
        dm.handle(
            AppEvent::LinkTargetMeta {
                target: "b".into(),
                count: 2,
            },
            &current,
        );
        assert_eq!(
            dm.link_segment(Some(&target), &current, Some(&tx))
                .as_deref(),
            Some("→ b · 2 backlinks")
        );
        // Cursor leaves: cache cleared.
        assert_eq!(dm.link_segment(None, &current, Some(&tx)), None);
    }

    #[tokio::test]
    async fn unowned_events_are_handed_back() {
        let (mut dm, _dir) = meta().await;
        let back = dm.handle(AppEvent::Redraw, &note("/a.md"));
        assert!(matches!(back, Some(AppEvent::Redraw)));
    }
}
