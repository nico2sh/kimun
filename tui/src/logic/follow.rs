//! **Follow** — what a link target under the cursor points at.
//!
//! [`resolve`] turns the raw target text of a follow (a wikilink, a markdown
//! link, a URL, an image reference) and the link's kind into a
//! [`FollowOutcome`]: the frontend then acts on it — hands a URL, an image or
//! an attachment to the OS, opens a note through
//! its session, offers to create a missing note, or lets the user choose
//! between several matches. Nothing here opens anything: the OS handoff
//! (`open::that_detached`) stays at the host edge so these rules are testable
//! without an OS.
//!
//! Labels (`#tag`) are not resolved here: a label follow opens a query, which
//! is the frontend's business.

use std::path::PathBuf;

use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::scan::{LinkSpanKind, is_remote_url, target_looks_like_image};
use kimun_core::note::{NoteContentData, NoteTarget, markdown_attachment};
use kimun_core::{LinkResolution, NoteVault};

/// Where a followed link leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowOutcome {
    /// A remote URL, for the OS browser/handler.
    External(String),
    /// An image or another file that is no note (a markdown link to an
    /// attachment), resolved against the following note's directory and
    /// turned into an OS path for the OS default handler. No note lookup ran.
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

/// Resolves `target`, a link of `kind` followed from the note at
/// `from_note`.
///
/// - a remote URL → [`FollowOutcome::External`];
/// - a wikilink to an image (`![[pic.png]]`) → [`FollowOutcome::OpenFile`],
///   relative to `from_note`'s directory;
/// - a wikilink names a note (`NoteTarget::from_wikilink`, its `#section` /
///   `^block` dropped): a name (`anton`, `work/anton`) matches notes in any
///   folder whose path ends with it, a `/`, `./` or `../` path is exact;
/// - a markdown link (or image) is an exact path, relative to `from_note`'s
///   folder (`NoteTarget::from_markdown`); one that is no note (`doc.pdf`,
///   `img/pic.png`) → [`FollowOutcome::OpenFile`];
/// - core resolves the note link (`NoteVault::resolve_note_target`) and the
///   hits pick `Note` / `Missing` / `Ambiguous`;
/// - anything else (`[[doc.pdf]]`, an invalid path) →
///   [`FollowOutcome::Error`].
pub async fn resolve(
    vault: &NoteVault,
    from_note: &VaultPath,
    target: &str,
    kind: LinkSpanKind,
) -> FollowOutcome {
    if is_remote_url(target) {
        return FollowOutcome::External(target.to_string());
    }

    let note_target = match kind {
        LinkSpanKind::WikiLink => {
            if target_looks_like_image(target) {
                let parent = from_note.get_parent_path().0;
                let resolved = parent.append(&VaultPath::new(target.trim()));
                return FollowOutcome::OpenFile(vault.path_to_pathbuf(&resolved));
            }
            NoteTarget::from_wikilink(target, from_note)
        }
        LinkSpanKind::Markdown | LinkSpanKind::Image => {
            if let Some(file) = markdown_attachment(target, from_note) {
                return FollowOutcome::OpenFile(vault.path_to_pathbuf(&file));
            }
            NoteTarget::from_markdown(target, from_note)
        }
    };
    let Some(note_target) = note_target else {
        return FollowOutcome::Error(format!("not a note link: {target}"));
    };
    match vault.resolve_note_target(&note_target).await {
        Ok(LinkResolution::Note(entry, _)) => FollowOutcome::Note(entry.path),
        Ok(LinkResolution::Missing(path)) => FollowOutcome::Missing(path),
        Ok(LinkResolution::Ambiguous(hits)) => FollowOutcome::Ambiguous(hits),
        Err(e) => FollowOutcome::Error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LinkSpanKind::{Image, Markdown, WikiLink};

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
            resolve(&v, &today(), "https://example.com/a", Markdown).await,
            FollowOutcome::External("https://example.com/a".into())
        );
    }

    #[tokio::test]
    async fn an_image_opens_relative_to_the_note_dir() {
        let v = vault().await;
        let expected = v.path_to_pathbuf(&VaultPath::new("journal/img/pic.png"));
        assert_eq!(
            resolve(&v, &today(), "img/pic.png", Image).await,
            FollowOutcome::OpenFile(expected)
        );
    }

    #[tokio::test]
    async fn a_fragment_is_dropped_before_lookup() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "/notes/design.md#goals", WikiLink).await,
            FollowOutcome::Note(abs("notes/design"))
        );
    }

    #[tokio::test]
    async fn a_relative_path_resolves_against_the_note_dir() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "../work/anton", WikiLink).await,
            FollowOutcome::Note(abs("work/anton"))
        );
    }

    #[tokio::test]
    async fn a_bare_name_with_one_hit_is_that_note() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "anton", WikiLink).await,
            FollowOutcome::Note(abs("work/anton"))
        );
    }

    #[tokio::test]
    async fn a_bare_name_with_several_hits_is_ambiguous() {
        let v = vault().await;
        let FollowOutcome::Ambiguous(hits) = resolve(&v, &today(), "dup", WikiLink).await else {
            panic!("expected Ambiguous");
        };
        let mut paths: Vec<VaultPath> = hits.into_iter().map(|(e, _)| e.path).collect();
        paths.sort_by_key(|p| p.to_string());
        assert_eq!(paths, vec![abs("dup"), abs("work/dup")]);
    }

    /// A missing name is created at the vault root (plus its written
    /// folders), canonical.
    #[tokio::test]
    async fn a_bare_name_with_no_hit_is_missing() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "nobody", WikiLink).await,
            FollowOutcome::Missing(abs("nobody"))
        );
        assert_eq!(
            resolve(&v, &today(), "work/nobody", WikiLink).await,
            FollowOutcome::Missing(abs("work/nobody"))
        );
    }

    /// A name with a folder part is a suffix matched in any folder, not a
    /// path under the following note's folder.
    #[tokio::test]
    async fn a_name_with_a_folder_part_is_a_vault_wide_suffix() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "work/dup", WikiLink).await,
            FollowOutcome::Note(abs("work/dup"))
        );
    }

    #[tokio::test]
    async fn a_non_note_file_is_no_note_link() {
        let v = vault().await;
        assert!(matches!(
            resolve(&v, &today(), "doc.pdf", WikiLink).await,
            FollowOutcome::Error(_)
        ));
    }

    /// A markdown link is an exact path, relative to the following note's
    /// folder: a bare `dup.md` is the same-folder note, never a name lookup,
    /// and a path with folders never matches a note in another folder.
    #[tokio::test]
    async fn a_markdown_link_is_an_exact_path() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "dup.md", Markdown).await,
            FollowOutcome::Missing(abs("journal/dup")),
            "a bare markdown name is no name lookup"
        );
        assert_eq!(
            resolve(&v, &today(), "../dup.md", Markdown).await,
            FollowOutcome::Note(abs("dup"))
        );
        assert_eq!(
            resolve(&v, &today(), "../work/dup.md#s", Markdown).await,
            FollowOutcome::Note(abs("work/dup"))
        );
        assert_eq!(
            resolve(&v, &today(), "work/dup.md", Markdown).await,
            FollowOutcome::Missing(abs("journal/work/dup")),
            "never a suffix of another folder"
        );
        assert_eq!(
            resolve(&v, &today(), "</work/dup.md>", Markdown).await,
            FollowOutcome::Note(abs("work/dup"))
        );
    }

    /// With a same-folder `dup` among three, a bare markdown `dup.md` opens
    /// that one; the wikilink `[[dup]]` stays ambiguous.
    #[tokio::test]
    async fn a_bare_markdown_name_opens_the_same_folder_note() {
        let v = vault().await;
        v.create_note(&VaultPath::note_path_from("journal/dup"), "# here")
            .await
            .unwrap();
        assert_eq!(
            resolve(&v, &today(), "dup.md", Markdown).await,
            FollowOutcome::Note(abs("journal/dup"))
        );
        let FollowOutcome::Ambiguous(hits) = resolve(&v, &today(), "dup", WikiLink).await else {
            panic!("expected Ambiguous");
        };
        assert_eq!(hits.len(), 3);
    }

    /// A markdown link to a file that is no note is handed to the OS,
    /// resolved against the following note's folder.
    #[tokio::test]
    async fn a_markdown_link_to_an_attachment_opens_the_file() {
        let v = vault().await;
        let expected = v.path_to_pathbuf(&VaultPath::new("/files/doc.pdf"));
        assert_eq!(
            resolve(&v, &today(), "../files/doc.pdf#page=2", Markdown).await,
            FollowOutcome::OpenFile(expected)
        );
    }

    #[tokio::test]
    async fn a_relative_path_with_no_hit_is_missing_at_the_resolved_path() {
        let v = vault().await;
        assert_eq!(
            resolve(&v, &today(), "../work/nobody", WikiLink).await,
            FollowOutcome::Missing(abs("work/nobody"))
        );
    }
}
