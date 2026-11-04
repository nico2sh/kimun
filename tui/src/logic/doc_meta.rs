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

use kimun_core::note::scan::LinkSpanKind;

use crate::logic::editor::claim::FollowTarget;
use crate::logic::follow::{self, FollowOutcome};
use crate::logic::message::{AppMessage, AppTx};

pub struct DocMeta {
    vault: Arc<NoteVault>,
    /// Backlink count of the open note (status line 2), async-loaded.
    backlink_count: Option<usize>,
    /// Frontmatter property count of the open note (status line 2).
    property_count: Option<usize>,
    /// Workspace git summary for the status bar, `None` when unknown/absent.
    git_status: Option<String>,
    /// When the last git fetch was spawned — throttles the per-event
    /// subprocess (rapid navigation must not fork one `git status` per note).
    last_git_fetch: Option<std::time::Instant>,
    /// When a fetch last reported "no repo / no git" — probing backs off to
    /// once a minute instead of once per open, but a `git init` mid-session
    /// still gets picked up.
    git_unavailable_since: Option<std::time::Instant>,
    /// Link-under-cursor affordance cache: `(target, kind, backlink count
    /// once loaded)`. Refreshed when the cursor enters a different link.
    link_meta: Option<(String, LinkSpanKind, Option<usize>)>,
}

impl DocMeta {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self {
            vault,
            backlink_count: None,
            property_count: None,
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

    pub fn properties(&self) -> Option<usize> {
        self.property_count
    }

    pub fn git(&self) -> Option<&String> {
        self.git_status.as_ref()
    }

    // ── Note lifecycle ──────────────────────────────────────────────────

    /// A note was (re)opened: reset and re-fetch its backlink count, and
    /// refresh the git summary.
    pub fn note_opened(&mut self, path: &VaultPath, tx: &AppTx) {
        self.load_note_counts(path, tx);
        self.refresh_git(tx);
    }

    /// A note was (re)opened: forget the last note's backlink and property
    /// counts and fetch this one's. No git: [`Self::note_opened`] adds it
    /// for a frontend that shows it.
    pub fn load_note_counts(&mut self, path: &VaultPath, tx: &AppTx) {
        self.backlink_count = None;
        self.refresh_backlinks(path, tx);
        self.property_count = None;
        self.refresh_properties(path, tx);
    }

    /// Re-read the open note's backlink count, keeping the one shown until
    /// it lands (a rename or a delete elsewhere may have changed it).
    pub fn refresh_backlinks(&mut self, path: &VaultPath, tx: &AppTx) {
        let vault = self.vault.clone();
        let path = path.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let count = vault
                .get_backlinks(&path)
                .await
                .map(|b| b.len())
                .unwrap_or_default();
            tx2.send(AppMessage::BacklinkCountLoaded { path, count })
                .ok();
        });
    }

    /// Re-read the open note's property count (note open, save, dialog write).
    pub fn refresh_properties(&mut self, path: &VaultPath, tx: &AppTx) {
        let vault = self.vault.clone();
        let path = path.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let count = vault
                .get_properties(&path)
                .await
                .map(|p| p.len())
                .unwrap_or_default();
            tx2.send(AppMessage::PropertyCountLoaded { path, count })
                .ok();
        });
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
        let root = self.vault.workspace_path().as_path().to_path_buf();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let status = crate::util::git_status::fetch(root).await;
            tx2.send(AppMessage::GitStatusLoaded(status)).ok();
        });
    }

    // ── Link-under-cursor affordance ────────────────────────

    /// The `→ target · N backlinks` status segment for the link under the
    /// cursor, if any. Caches per target; the count loads async (one fetch
    /// per target change). The target is resolved exactly like a follow
    /// (`logic::follow::resolve`), so the count is the backlinks of the note
    /// that would actually open; a link several notes answer to, or none,
    /// shows no count. `tx == None` (before the screen's first on_enter)
    /// renders the target without a count.
    pub fn link_segment(
        &mut self,
        link: Option<&FollowTarget>,
        current_note: &VaultPath,
        tx: Option<&AppTx>,
    ) -> Option<String> {
        match link {
            Some(FollowTarget::Link { target, kind }) => {
                let cached = self
                    .link_meta
                    .as_ref()
                    .is_some_and(|(t, k, _)| t == target && k == kind);
                if !cached {
                    self.link_meta = Some((target.clone(), *kind, None));
                    if let Some(tx) = tx {
                        let vault = self.vault.clone();
                        let target = target.clone();
                        let kind = *kind;
                        let note_path = current_note.clone();
                        let tx2 = tx.clone();
                        tokio::spawn(async move {
                            let FollowOutcome::Note(path) =
                                follow::resolve(&vault, &note_path, &target, kind).await
                            else {
                                return;
                            };
                            let count = vault
                                .get_backlinks(&path)
                                .await
                                .map(|b| b.len())
                                .unwrap_or_default();
                            tx2.send(AppMessage::LinkTargetMeta {
                                target,
                                kind,
                                count,
                            })
                            .ok();
                        });
                    }
                }
                match &self.link_meta {
                    Some((t, _, Some(n))) => Some(format!("→ {t} · {n} backlinks")),
                    Some((t, _, None)) => Some(format!("→ {t}")),
                    None => None,
                }
            }
            Some(FollowTarget::Label(name)) => {
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
    pub fn handle(&mut self, event: AppMessage, current_note: &VaultPath) -> Option<AppMessage> {
        match event {
            AppMessage::BacklinkCountLoaded { path, count } => {
                // Ignore stale loads for notes already navigated away from.
                if path == *current_note {
                    self.backlink_count = Some(count);
                }
                None
            }
            AppMessage::PropertyCountLoaded { path, count } => {
                if path == *current_note {
                    self.property_count = Some(count);
                }
                None
            }
            AppMessage::LinkTargetMeta {
                target,
                kind,
                count,
            } => {
                // Only land the count if the cursor is still on that link.
                if let Some((cached, cached_kind, slot)) = &mut self.link_meta
                    && *cached == target
                    && *cached_kind == kind
                {
                    *slot = Some(count);
                }
                None
            }
            AppMessage::GitStatusLoaded(status) => {
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
            NoteVault::new(kimun_core::VaultConfig::new(crate::test_support::sys(
                dir.path(),
            )))
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
            AppMessage::BacklinkCountLoaded {
                path: note("/old.md"),
                count: 7,
            },
            &current,
        );
        assert_eq!(dm.backlinks(), None, "stale path must not land");
        dm.handle(
            AppMessage::BacklinkCountLoaded {
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
        dm.handle(AppMessage::GitStatusLoaded(None), &current);
        assert!(dm.git_unavailable_since.is_some());
        // A value followed by a failure keeps the last value.
        dm.git_unavailable_since = None;
        dm.handle(AppMessage::GitStatusLoaded(Some("git ✓".into())), &current);
        dm.handle(AppMessage::GitStatusLoaded(None), &current);
        assert_eq!(dm.git().map(String::as_str), Some("git ✓"));
    }

    #[tokio::test]
    async fn link_count_lands_only_on_matching_cached_target() {
        let (mut dm, _dir) = meta().await;
        let current = note("/a.md");
        let (tx, _rx) = unbounded_channel();
        let target = FollowTarget::Link {
            target: "b".to_string(),
            kind: LinkSpanKind::WikiLink,
        };
        // Cursor enters the link: target cached, no count yet.
        let seg = dm.link_segment(Some(&target), &current, Some(&tx));
        assert_eq!(seg.as_deref(), Some("→ b"));
        // A count for a DIFFERENT target is ignored.
        dm.handle(
            AppMessage::LinkTargetMeta {
                target: "other".into(),
                kind: LinkSpanKind::WikiLink,
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
            AppMessage::LinkTargetMeta {
                target: "b".into(),
                kind: LinkSpanKind::Markdown,
                count: 4,
            },
            &current,
        );
        assert_eq!(
            dm.link_segment(Some(&target), &current, Some(&tx))
                .as_deref(),
            Some("→ b"),
            "a count for another kind of link is ignored"
        );
        dm.handle(
            AppMessage::LinkTargetMeta {
                target: "b".into(),
                kind: LinkSpanKind::WikiLink,
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

    /// The count is the backlinks of the note the link resolves to, read
    /// by its form: `[[work/dup]]` from another folder is `/work/dup` (a
    /// suffix, not a path under the note's folder); a name several notes
    /// share, or a link to no note, loads no count.
    #[tokio::test]
    async fn link_count_is_the_resolved_notes_backlinks() {
        let vault = crate::test_support::temp_vault("doc-meta-link").await;
        vault.validate_and_init().await.unwrap();
        for (path, text) in [
            ("dup", ""),
            ("work/dup", ""),
            ("journal/today", "[[work/dup]] [[dup]]"),
            ("other", "[x](work/dup.md)"),
        ] {
            vault.save_note(&note(path), text).await.unwrap();
        }
        let mut dm = DocMeta::new(vault);
        let today = note("journal/today");
        let (tx, mut rx) = unbounded_channel();
        let link = |target: &str, kind| FollowTarget::Link {
            target: target.to_string(),
            kind,
        };

        dm.link_segment(
            Some(&link("work/dup", LinkSpanKind::WikiLink)),
            &today,
            Some(&tx),
        );
        let event = rx.recv().await.expect("a count");
        assert!(
            matches!(
                &event,
                AppMessage::LinkTargetMeta { target, kind: LinkSpanKind::WikiLink, count: 2 }
                    if target == "work/dup"
            ),
            "{event:?}"
        );

        for (target, kind) in [
            ("dup", LinkSpanKind::WikiLink),
            ("work/dup.md", LinkSpanKind::Markdown),
        ] {
            dm.link_segment(Some(&link(target, kind)), &today, Some(&tx));
        }
        // Neither the ambiguous name nor the missing path sends a count.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn unowned_events_are_handed_back() {
        let (mut dm, _dir) = meta().await;
        let back = dm.handle(AppMessage::Redraw, &note("/a.md"));
        assert!(matches!(back, Some(AppMessage::Redraw)));
    }

    /// Loading a note's counts forgets the last note's at once and fetches
    /// both for the new one; a refresh keeps the count on show until the
    /// new one lands.
    #[tokio::test]
    async fn loading_counts_resets_then_fetches_both() {
        let (mut dm, _dir) = meta().await;
        let current = note("/a.md");
        dm.handle(
            AppMessage::BacklinkCountLoaded {
                path: current.clone(),
                count: 5,
            },
            &current,
        );
        dm.handle(
            AppMessage::PropertyCountLoaded {
                path: current.clone(),
                count: 2,
            },
            &current,
        );
        let (tx, mut rx) = unbounded_channel();
        dm.refresh_backlinks(&current, &tx);
        assert_eq!(dm.backlinks(), Some(5), "a refresh keeps the shown count");
        let next = note("/b.md");
        dm.load_note_counts(&next, &tx);
        assert_eq!((dm.backlinks(), dm.properties()), (None, None));
        let mut backlinks_for = Vec::new();
        let mut props_for = Vec::new();
        while backlinks_for.len() < 2 || props_for.is_empty() {
            match rx.recv().await.expect("a count") {
                AppMessage::BacklinkCountLoaded { path, .. } => backlinks_for.push(path),
                AppMessage::PropertyCountLoaded { path, .. } => props_for.push(path),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(backlinks_for.contains(&next) && backlinks_for.contains(&current));
        assert_eq!(props_for, vec![next]);
    }

    #[tokio::test]
    async fn property_count_guards_against_stale_paths() {
        let (mut dm, _dir) = meta().await;
        let current = note("/a.md");
        dm.handle(
            AppMessage::PropertyCountLoaded {
                path: note("/old.md"),
                count: 4,
            },
            &current,
        );
        assert_eq!(dm.properties(), None);
        dm.handle(
            AppMessage::PropertyCountLoaded {
                path: current.clone(),
                count: 2,
            },
            &current,
        );
        assert_eq!(dm.properties(), Some(2));
    }
}
