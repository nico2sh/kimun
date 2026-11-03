//! **Follow** — what a link target under the cursor points at.
//!
//! [`resolve`] turns the raw target text of a follow (a wikilink, a markdown
//! link, a URL, an image reference) into a [`FollowOutcome`]: the frontend
//! then acts on it — hands a URL or an image to the OS, opens a note through
//! its session, offers to create a missing note, or lets the user choose
//! between several matches. Nothing here opens anything: the OS handoff
//! (`open::that_detached`) stays at the host edge so these rules are testable
//! without an OS.
//!
//! Labels (`#tag`) are not resolved here: a label follow opens a query, which
//! is the frontend's business.

use std::path::PathBuf;

use kimun_core::NoteVault;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;
use kimun_core::note::scan::{is_remote_url, split_link_fragment, target_looks_like_image};

/// Where a followed link leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowOutcome {
    /// A remote URL, for the OS browser/handler.
    External(String),
    /// An image, resolved against the following note's directory and turned
    /// into an OS path for the OS default handler. Images are not notes, so
    /// no note lookup ran.
    OpenFile(PathBuf),
    /// Exactly one note matches.
    Note(VaultPath),
    /// No note matches: the (resolved) path a new note would get.
    Missing(VaultPath),
    /// Several notes match a bare name; in index order.
    Ambiguous(Vec<(NoteEntryData, NoteContentData)>),
    /// The lookup failed; the error text.
    Error(String),
}

/// Resolves `target`, followed from the note at `from_note`.
///
/// - a remote URL → [`FollowOutcome::External`];
/// - an image reference → [`FollowOutcome::OpenFile`], relative to
///   `from_note`'s directory;
/// - anything else is a note reference: a `#section` / `^block` suffix is
///   dropped, a relative path (`../work/anton`) is resolved against
///   `from_note`'s directory, a bare name stays a vault-wide name lookup
///   (index-backed), and the hit count picks `Note` / `Missing` /
///   `Ambiguous`.
pub async fn resolve(vault: &NoteVault, from_note: &VaultPath, target: &str) -> FollowOutcome {
    if is_remote_url(target) {
        return FollowOutcome::External(target.to_string());
    }

    if target_looks_like_image(target) {
        let parent = from_note.get_parent_path().0;
        let resolved = parent.append(&VaultPath::new(target.trim()));
        return FollowOutcome::OpenFile(vault.path_to_pathbuf(&resolved));
    }

    // Strip any `#section` / `^block` suffix before resolving (e.g.
    // `notes/design.md#goals` resolves to `notes/design.md`, not
    // `notes/design.md#goals.md`) — the same split the index uses.
    let (target_clean, _) = split_link_fragment(target);
    // Resolve the (possibly relative) target against this note's directory so
    // the existence lookup uses the same absolute path the note is stored
    // under. Bare names stay name-lookups.
    let path = VaultPath::note_path_from(target_clean).resolve_link_in_note(from_note);
    match vault.open_or_search(&path).await {
        Ok(results) if results.is_empty() => FollowOutcome::Missing(path),
        Ok(mut results) if results.len() == 1 => FollowOutcome::Note(results.remove(0).0.path),
        Ok(results) => FollowOutcome::Ambiguous(results),
        Err(e) => FollowOutcome::Error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An indexed temp vault holding `work/anton`, `journal/today`,
    /// `notes/design`, and two notes named `dup` (at the root and in `work`).
    async fn vault() -> std::sync::Arc<NoteVault> {
        let vault = crate::test_support::temp_vault("follow").await;
        vault.validate_and_init().await.unwrap();
        for name in [
            "work/anton",
            "journal/today",
            "notes/design",
            "dup",
            "work/dup",
        ] {
            vault
                .create_note(&VaultPath::note_path_from(name), "# body")
                .await
                .unwrap();
        }
        vault
    }

    /// The absolute note path for `name` (what the index hands back).
    fn abs(name: &str) -> VaultPath {
        VaultPath::note_path_from(name).absolute()
    }

    fn today() -> VaultPath {
        VaultPath::note_path_from("journal/today")
    }

    #[tokio::test]
    async fn a_url_is_external() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "https://example.com/a").await,
            FollowOutcome::External("https://example.com/a".into())
        );
    }

    #[tokio::test]
    async fn an_image_opens_relative_to_the_note_dir() {
        let v = vault().await;
        let expected = v.path_to_pathbuf(&VaultPath::new("journal/img/pic.png"));
        assert_eq!(
            resolve(&v, &today(), "img/pic.png").await,
            FollowOutcome::OpenFile(expected)
        );
    }

    #[tokio::test]
    async fn a_fragment_is_dropped_before_lookup() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "/notes/design.md#goals").await,
            FollowOutcome::Note(abs("notes/design"))
        );
    }

    #[tokio::test]
    async fn a_relative_path_resolves_against_the_note_dir() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "../work/anton").await,
            FollowOutcome::Note(abs("work/anton"))
        );
    }

    #[tokio::test]
    async fn a_bare_name_with_one_hit_is_that_note() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "anton").await,
            FollowOutcome::Note(abs("work/anton"))
        );
    }

    #[tokio::test]
    async fn a_bare_name_with_several_hits_is_ambiguous() {
        let v = vault().await;
        let FollowOutcome::Ambiguous(hits) = resolve(&v, &today(), "dup").await else {
            panic!("expected Ambiguous");
        };
        let mut paths: Vec<VaultPath> = hits.into_iter().map(|(e, _)| e.path).collect();
        paths.sort_by_key(|p| p.to_string());
        assert_eq!(paths, vec![abs("dup"), abs("work/dup")]);
    }

    #[tokio::test]
    async fn a_bare_name_with_no_hit_is_missing() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "nobody").await,
            FollowOutcome::Missing(VaultPath::note_path_from("nobody"))
        );
    }

    #[tokio::test]
    async fn a_relative_path_with_no_hit_is_missing_at_the_resolved_path() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "../work/nobody").await,
            FollowOutcome::Missing(abs("work/nobody"))
        );
    }
}
