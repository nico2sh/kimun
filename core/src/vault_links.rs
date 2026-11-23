//! Links on [`NoteVault`]: where a note link leads, the shortest wikilink that
//! names a note, a note's rendered Markdown with its links, and its backlinks.

use crate::error::VaultError;
use crate::nfs::{NoteEntryData, VaultPath};
use crate::note::{self, NoteContentData};
use crate::{LinkResolution, NoteVault};

impl NoteVault {
    /// Where a note link leads: the notes `target` matches
    /// ([`note::NoteTarget::matches`]), looked up in the index.
    ///
    /// - [`note::NoteTarget::Name`] (a wikilink by name): every note named
    ///   like it whose path ends with the written components, in any folder,
    ///   case-insensitively; several are [`LinkResolution::Ambiguous`].
    /// - [`note::NoteTarget::Path`]: the note at exactly that path.
    ///
    /// No match is [`LinkResolution::Missing`] with the path a new note
    /// would get: a name at the vault root plus its written components
    /// (`[[work/new]]` → `/work/new.md`), a path as it is.
    pub async fn resolve_note_target(
        &self,
        target: &note::NoteTarget,
    ) -> Result<LinkResolution, VaultError> {
        let mut hits = match target {
            note::NoteTarget::Name(_) => {
                let mut hits = self.index.search_note_by_name(target.name()).await?;
                hits.retain(|(entry, _)| target.matches(&entry.path));
                hits
            }
            note::NoteTarget::Path(path) => self.index.search_note_by_path(path).await?,
        };
        Ok(match hits.len() {
            0 => LinkResolution::Missing(target.path().canonical()),
            1 => {
                let (entry, content) = hits.remove(0);
                LinkResolution::Note(entry, content)
            }
            _ => LinkResolution::Ambiguous(hits),
        })
    }

    /// The shortest wikilink target naming the note at `path` alone (no
    /// brackets, no note extension): its name when no other note shares it
    /// (`dup`), else the fewest trailing folders that tell it apart
    /// (`work/dup`), else its vault-absolute path (`/work/dup`) when even
    /// its whole path ends another note's.
    pub async fn wikilink_target_for(&self, path: &VaultPath) -> Result<String, VaultError> {
        let namesakes: Vec<VaultPath> = self
            .index
            .search_note_by_name(path.get_name())
            .await?
            .into_iter()
            .map(|(entry, _)| entry.path)
            .collect();
        Ok(note::unique_wikilink(path, 1, &namesakes))
    }

    /// Converts a note's raw Markdown into rendered Markdown and extracts all links.
    ///
    /// - WikiLinks (`[[note]]`) are converted to standard Markdown links.
    /// - Note links are resolved to vault-relative absolute paths.
    /// - Hashtags become Markdown links (`[#tag](#tag)`) and are added to the links list.
    /// - Image paths are resolved to absolute OS paths so renderers can load them directly.
    ///   Relative image paths are resolved against the note's location in the vault.
    ///   External image URLs are kept as-is.
    pub async fn get_markdown_and_links(
        &self,
        path: &VaultPath,
    ) -> Result<note::MarkdownNote, VaultError> {
        let note = self.load_note(path).await?;
        let note_parent = if note.path.is_note() {
            note.path.get_parent_path().0
        } else {
            note.path.clone()
        };
        let (md_text, mut links) = note.get_markdown_and_links();
        // Since this function is intended to return content ready to be rendered
        // We need the full path of the image links, so any markdown processor can find the image,
        // the full path can only be resolved from here as we have the vault path
        let (md_text, image_links) = note::process_image_links(&md_text, |alt_text, raw_path| {
            let resolved = if note::scan::is_remote_url(raw_path) {
                raw_path.to_string()
            } else {
                let image_vault_path = if raw_path.starts_with('/') {
                    VaultPath::new(raw_path)
                } else {
                    note_parent.append(&VaultPath::new(raw_path)).flatten()
                };
                image_vault_path
                    .to_pathbuf(self.workspace_path())
                    .display()
                    .to_string()
            };
            let link = note::NoteLink::image(&resolved, alt_text, raw_path);
            (resolved, link)
        });
        links.extend(image_links);
        Ok(note::MarkdownNote {
            text: md_text,
            links,
        })
    }

    /// Returns all notes that contain a link pointing to `path`: a path link
    /// (markdown, or a wikilink written from the root or the note's folder)
    /// to exactly this note, or a wikilink by name whose written components
    /// end this note's path (`[[dup]]`, `[[work/dup]]` for `/a/work/dup.md`)
    /// — see [`note::NoteTarget::matches`]. An ambiguous name link is a
    /// backlink of every note it can point at.
    pub async fn get_backlinks(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        Ok(self.index.get_backlinks(path).await?)
    }
}
