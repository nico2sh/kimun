//! Frontmatter properties on [`NoteVault`]: reading a note's properties,
//! the vault-wide key and sort-value lookups, and the edits (set, remove,
//! rename). Edits are serialized per note and keep the block's format.

use std::collections::HashMap;

use crate::error::{FSError, VaultError};
use crate::nfs::{self, VaultPath};
use crate::note::{self, properties, FrontmatterFormat, NoteDetails, PropertyInput, PropertyValue};
use crate::{NoteVault, PropertySortValue};

impl NoteVault {
    /// The note's frontmatter properties in file order, keys as written, a
    /// key with no usable value (YAML `due:`) included with `None`. Read from
    /// the note itself; a malformed block yields none. Missing-file behaviour
    /// matches [`Self::get_note_text`].
    pub async fn get_properties(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<note::PropertyEntry>, VaultError> {
        let text = self.get_note_text(path).await?;
        Ok(NoteDetails::properties_of(text))
    }

    /// One frontmatter property, `key` matched case-insensitively: `None`
    /// when the note lacks the key, `Some(None)` when it has the key with no
    /// readable value (YAML `due:`), as [`Self::get_properties`] lists it.
    pub async fn get_property(
        &self,
        path: &VaultPath,
        key: &str,
    ) -> Result<Option<Option<PropertyValue>>, VaultError> {
        let properties = self.get_properties(path).await?;
        // A key that can't be a property (empty, multi-line) is never found.
        let Some(key) = properties::clean_key(key) else {
            return Ok(None);
        };
        Ok(properties
            .into_iter()
            .find_map(|(k, v)| properties::keys_match(&k, &key).then_some(v)))
    }

    /// Every property key any note has, in search form (lowercased, accents
    /// stripped — the form `%key` filters and `^%key` sorts by), distinct
    /// and sorted. Index-backed.
    pub async fn property_keys(&self) -> Result<Vec<String>, VaultError> {
        Ok(self.index.property_keys().await?)
    }

    /// Each indexed note's sort value for property `key` (matched in search
    /// form, as `or:prop:key` matches it), keyed by note path. Ordering the
    /// values with [`PropertySortValue`]'s `Ord`, notes absent from the map
    /// last, gives the same order as the query sort. Notes without the key
    /// (or not indexed yet) are absent. Index-backed.
    pub async fn property_sort_values(
        &self,
        key: &str,
    ) -> Result<HashMap<VaultPath, PropertySortValue>, VaultError> {
        let Some(key) = properties::search_key(key) else {
            return Ok(HashMap::new());
        };
        Ok(self.index.property_sort_values(&key).await?)
    }

    /// Sets a frontmatter property, keeping the existing block's format,
    /// comments and key order. `key` matches an existing one case-insensitively
    /// (that entry keeps its spelling); a new key is written as given. A note with no block gets one in
    /// `new_block_format`. Fails with [`FSError::InvalidFrontmatter`] (file
    /// untouched) when the existing block doesn't parse. Serialized per note.
    pub async fn set_property(
        &self,
        path: &VaultPath,
        key: &str,
        value: PropertyValue,
        new_block_format: FrontmatterFormat,
    ) -> Result<(), VaultError> {
        let key = Self::property_key(key)?;
        if matches!(value, PropertyValue::Number(n) if !n.is_finite()) {
            return Err(VaultError::InvalidProperty {
                key: key.to_string(),
                message: "number must be finite".to_string(),
            });
        }
        let _guard = self.lock_note(path).await;
        let text = nfs::to_lf(&self.get_note_text(path).await?);
        let updated = properties::NoteProperties::new(&text, new_block_format)
            .set(&key, &value)
            .map_err(|e| Self::frontmatter_error(path, &key, e))?;
        if updated != text {
            self.save_note_unlocked(path, updated).await?;
        }
        Ok(())
    }

    /// Sets a property from user-typed input, as the CLI and MCP server take
    /// it; see [`PropertyInput::resolve`] for how its type is decided. Without
    /// a forced type the input must fit the type most other notes give `key`,
    /// and a mismatch is refused with [`VaultError::InvalidProperty`] instead
    /// of drifting the vault's types. Returns the value written; only this
    /// note changes.
    pub async fn set_property_from_input(
        &self,
        path: &VaultPath,
        key: &str,
        input: &PropertyInput,
        new_block_format: FrontmatterFormat,
    ) -> Result<PropertyValue, VaultError> {
        let key = Self::property_key(key)?;
        let vault_kind = match input.forced_kind() {
            Some(_) => None,
            None => {
                let indexed = properties::search_form(&key);
                self.index.dominant_property_kind(&indexed, path).await?
            }
        };
        let value =
            input
                .resolve(&key, vault_kind)
                .map_err(|message| VaultError::InvalidProperty {
                    key: key.to_string(),
                    message,
                })?;
        self.set_property(path, &key, value.clone(), new_block_format)
            .await?;
        Ok(value)
    }

    /// Removes a frontmatter property (every case variant of `key`). Returns
    /// whether anything was removed. Same locking and refusal rules as
    /// [`Self::set_property`].
    pub async fn remove_property(&self, path: &VaultPath, key: &str) -> Result<bool, VaultError> {
        let key = Self::property_key(key)?;
        let _guard = self.lock_note(path).await;
        let text = nfs::to_lf(&self.get_note_text(path).await?);
        match properties::NoteProperties::new(&text, FrontmatterFormat::default())
            .remove(&key)
            .map_err(|e| Self::frontmatter_error(path, &key, e))?
        {
            Some(updated) => {
                self.save_note_unlocked(path, updated).await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Renames frontmatter property `old` to `new` in place — same position,
    /// value and comments. A case-only change respells the key. Refused with
    /// [`VaultError::InvalidProperty`] when `old` is missing or `new` names
    /// another existing property; same locking and malformed-block rules as
    /// [`Self::set_property`].
    pub async fn rename_property(
        &self,
        path: &VaultPath,
        old: &str,
        new: &str,
    ) -> Result<(), VaultError> {
        let old = Self::property_key(old)?;
        let new = Self::property_key(new)?;
        let _guard = self.lock_note(path).await;
        let text = nfs::to_lf(&self.get_note_text(path).await?);
        match properties::NoteProperties::new(&text, FrontmatterFormat::default())
            .rename(&old, &new)
            .map_err(|e| Self::frontmatter_error(path, &new, e))?
        {
            Some(updated) => {
                if updated != text {
                    self.save_note_unlocked(path, updated).await?;
                }
                Ok(())
            }
            None => Err(VaultError::InvalidProperty {
                key: old,
                message: "no such property".to_string(),
            }),
        }
    }

    /// The key as the user spelled it (casing kept; matching is
    /// case-insensitive, see `properties::match_key`).
    fn property_key(key: &str) -> Result<String, VaultError> {
        properties::clean_key(key).ok_or_else(|| VaultError::InvalidProperty {
            key: key.to_string(),
            message: "key must be non-empty and on one line".to_string(),
        })
    }

    /// A malformed existing block is the note's problem
    /// ([`FSError::InvalidFrontmatter`]); a refused edit of a sound block is the
    /// request's ([`VaultError::InvalidProperty`]).
    pub(crate) fn frontmatter_error(
        path: &VaultPath,
        key: &str,
        e: properties::FrontmatterError,
    ) -> VaultError {
        match e {
            properties::FrontmatterError::Malformed(message) => {
                VaultError::FSError(FSError::InvalidFrontmatter {
                    path: path.clone(),
                    message,
                })
            }
            properties::FrontmatterError::Refused(message) => VaultError::InvalidProperty {
                key: key.to_string(),
                message,
            },
        }
    }
}
