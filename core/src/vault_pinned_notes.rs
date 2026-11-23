//! Pinned notes on [`NoteVault`]: the short list of notes a user keeps at hand,
//! stored with the vault and capped at [`PINNED_NOTES_CAP`].

use crate::error::{FSError, VaultError};
use crate::nfs::{self, pinned_notes, VaultPath};
use crate::{NoteVault, PinOutcome, PinToggle, PINNED_NOTES_CAP};

impl NoteVault {
    /// The vault's pinned notes, in the user's order, at most
    /// [`PINNED_NOTES_CAP`] of them. A vault with no pin file has none.
    ///
    /// Truncated here, at the display seam, and nowhere else: only
    /// `toggle_pinned_note` and `pin_note` refuse a pin past the cap, and a
    /// hand-edit or a sync merge of the file can hold more. A question of
    /// "is this note pinned?" must not use this list: ask
    /// [`Self::list_all_pinned_notes`], or a pin past the cap reads as not
    /// pinned. Those extras are kept on disk
    /// by every edit — truncating on read would let the next unrelated
    /// rename or unpin silently delete them.
    pub async fn list_pinned_notes(&self) -> Result<Vec<VaultPath>, VaultError> {
        let mut all = pinned_notes::read_pinned_notes(self.workspace_path()).await?;
        all.truncate(PINNED_NOTES_CAP);
        Ok(all)
    }

    /// Every pin on disk, in order, past [`PINNED_NOTES_CAP`] too. For
    /// deciding whether a note is pinned; use [`Self::list_pinned_notes`]
    /// to show the pins.
    pub async fn list_all_pinned_notes(&self) -> Result<Vec<VaultPath>, VaultError> {
        Ok(pinned_notes::read_pinned_notes(self.workspace_path()).await?)
    }

    /// Pin `path` (appended last) or unpin it if it is already pinned.
    /// Returns [`PinToggle::Full`] — and writes nothing — when the list is at
    /// the cap and `path` is not in it. Fails if `path` is not a note: a
    /// pinned directory would be doubly orphaned by its own deletion (the
    /// directory-delete rewrite only drops pins *beneath* the directory, not
    /// a pin *at* it), so a pin is a note or nothing. Pinning also fails
    /// with [`FSError::VaultPathNotFound`] when the note is not on disk — a
    /// pin is an explicit act on a real note, and a pin born missing would
    /// only ever spend a cap slot. Unpinning a missing note always works.
    pub async fn toggle_pinned_note(&self, path: &VaultPath) -> Result<PinToggle, VaultError> {
        path.ensure_note()?;
        let exists = nfs::path_exists(self.workspace_path(), path).await?;
        pinned_notes::edit(self.workspace_path(), |all| {
            if !exists && pinned_notes::position_of(all, path).is_none() {
                let err = FSError::VaultPathNotFound { path: path.clone() };
                return (Err(VaultError::FSError(err)), false);
            }
            let outcome = pinned_notes::toggle(all, path);
            (Ok(outcome), outcome != PinToggle::Full)
        })
        .await?
    }

    /// Pin `path` (appended last), never unpin it: a note that is already
    /// pinned stays so ([`PinOutcome::AlreadyPinned`], nothing written). The
    /// check and the append run under one lock, so a stale caller or a race
    /// cannot turn a pin into an unpin. Same rules as
    /// [`Self::toggle_pinned_note`]: `path` must be a note, a note not on
    /// disk cannot be newly pinned, and a full list gives
    /// [`PinOutcome::Full`].
    pub async fn pin_note(&self, path: &VaultPath) -> Result<PinOutcome, VaultError> {
        path.ensure_note()?;
        let exists = nfs::path_exists(self.workspace_path(), path).await?;
        pinned_notes::edit(self.workspace_path(), |all| {
            if pinned_notes::position_of(all, path).is_some() {
                return (Ok(PinOutcome::AlreadyPinned), false);
            }
            if !exists {
                let err = FSError::VaultPathNotFound { path: path.clone() };
                return (Err(VaultError::FSError(err)), false);
            }
            match pinned_notes::toggle(all, path) {
                PinToggle::Pinned { position } => (Ok(PinOutcome::Pinned { position }), true),
                PinToggle::Full => (Ok(PinOutcome::Full), false),
                // `position_of` said "not pinned" just above, so the toggle
                // cannot unpin. If the two rules ever disagree, nothing is
                // written (the edit is not saved) and the note stays pinned.
                PinToggle::Unpinned => {
                    debug_assert!(
                        false,
                        "pin_note: toggle unpinned a note it just found absent"
                    );
                    (Ok(PinOutcome::AlreadyPinned), false)
                }
            }
        })
        .await?
    }

    /// Unpin `path`. `Ok(false)` when it was not pinned (nothing written).
    pub async fn unpin_note(&self, path: &VaultPath) -> Result<bool, VaultError> {
        Ok(pinned_notes::edit(self.workspace_path(), |all| {
            let changed = pinned_notes::remove(all, path);
            (changed, changed)
        })
        .await?)
    }

    /// Move the pin for `path` by `delta` slots (negative moves it up).
    /// `Ok(false)` — nothing written — when `path` is not pinned or the
    /// move would leave the list; see `pinned_notes::move_by`.
    pub async fn move_pinned_note(
        &self,
        path: &VaultPath,
        delta: isize,
    ) -> Result<bool, VaultError> {
        Ok(pinned_notes::edit(self.workspace_path(), |all| {
            let changed = pinned_notes::move_by(all, path, delta);
            (changed, changed)
        })
        .await?)
    }
}
