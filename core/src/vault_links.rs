//! Link resolution on [`NoteVault`]: where a note link leads, and the
//! shortest wikilink that names a note.

use crate::error::VaultError;
use crate::nfs::VaultPath;
use crate::note;
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
}
