//! Text edits to one note on [`NoteVault`]: replace (and its dry run) and the
//! task checkbox toggle. Each edit holds the per-note lock across its read and
//! its write.

use crate::error::VaultError;
use crate::nfs::VaultPath;
use crate::note;
use crate::{NoteVault, ReplacePreview};

impl NoteVault {
    /// Computes the result of replacing `pattern` with `replacement` in `text`.
    /// Pure — no I/O, no locking.
    ///
    /// `pattern` is a literal substring by default. When `regex` is `true` it is
    /// a regular expression ([`regex`] crate syntax) and `replacement` may
    /// reference capture groups (`$1`, `${name}`; use `$$` for a literal `$`).
    /// Use inline flags for line/case behaviour, e.g. `(?m)`, `(?s)`, `(?i)`.
    ///
    /// When `all` is `false` the match must be unique: returns
    /// [`VaultError::ReplaceTextNotFound`] when there is no match and
    /// [`VaultError::ReplaceTextNotUnique`] when there is more than one. When
    /// `all` is `true` every match is replaced. An invalid regex yields
    /// [`VaultError::InvalidRegex`]. `path` is only used to build the errors.
    /// Returns `(match count, new content)`.
    fn compute_replacement(
        text: &str,
        pattern: &str,
        replacement: &str,
        all: bool,
        regex: bool,
        path: &VaultPath,
    ) -> Result<(usize, String), VaultError> {
        let count;
        let updated = if regex {
            let re = regex::Regex::new(pattern).map_err(|e| VaultError::InvalidRegex {
                pattern: pattern.to_string(),
                message: e.to_string(),
            })?;
            count = re.find_iter(text).count();
            if count == 0 {
                return Err(VaultError::ReplaceTextNotFound {
                    path: path.flatten(),
                });
            }
            if !all && count > 1 {
                return Err(VaultError::ReplaceTextNotUnique {
                    path: path.flatten(),
                });
            }
            // regex `replacen` treats a limit of 0 as "replace all".
            let limit = if all { 0 } else { 1 };
            re.replacen(text, limit, replacement).into_owned()
        } else {
            count = if pattern.is_empty() {
                0
            } else {
                text.matches(pattern).count()
            };
            if count == 0 {
                return Err(VaultError::ReplaceTextNotFound {
                    path: path.flatten(),
                });
            }
            if !all && count > 1 {
                return Err(VaultError::ReplaceTextNotUnique {
                    path: path.flatten(),
                });
            }
            if all {
                text.replace(pattern, replacement)
            } else {
                text.replacen(pattern, replacement, 1)
            }
        };
        Ok((count, updated))
    }

    /// Replaces occurrences of `pattern` with `replacement` in the note at
    /// `path`, writing the result. See `compute_replacement` for the matching
    /// rules (literal vs `regex`, unique-unless-`all`, capture references).
    /// Returns the number of replacements made.
    pub async fn replace_in_note(
        &self,
        path: &VaultPath,
        pattern: &str,
        replacement: &str,
        all: bool,
        regex: bool,
    ) -> Result<usize, VaultError> {
        // Hold the per-note lock across the read and the write so a concurrent
        // in-process writer can't change the note between them (lost update).
        let _guard = self.lock_note(path).await;
        let text = self.get_note_text(path).await?;
        let (count, updated) =
            Self::compute_replacement(&text, pattern, replacement, all, regex, path)?;
        // Already holding the per-note lock; use the unlocked save so we don't
        // re-acquire it (the tokio Mutex is not reentrant).
        self.save_note_unlocked(path, updated).await?;
        Ok(count)
    }

    /// Flips the checkbox of the task on source line `line` (0-based) of the
    /// note at `path`, writing the result. `seen` is the line as the caller
    /// read it; the note is left alone and [`VaultError::TaskChanged`] is
    /// returned when that line is no longer that task (see [`note::toggle_task`]).
    pub async fn toggle_task(
        &self,
        path: &VaultPath,
        line: usize,
        seen: &str,
    ) -> Result<(), VaultError> {
        // The per-note lock spans the read and the write, as in
        // `replace_in_note`.
        let _guard = self.lock_note(path).await;
        let text = self.get_note_text(path).await?;
        let updated = note::toggle_task(&text, line, seen)
            .ok_or_else(|| VaultError::TaskChanged { path: path.clone() })?;
        self.save_note_unlocked(path, updated).await?;
        Ok(())
    }

    /// Dry-run of [`Self::replace_in_note`]: computes what the note would contain after
    /// the replacement **without writing** (and without taking the write lock —
    /// the result is advisory). Returns the same errors the real replace would
    /// (not found / not unique / invalid regex).
    pub async fn preview_replace(
        &self,
        path: &VaultPath,
        pattern: &str,
        replacement: &str,
        all: bool,
        regex: bool,
    ) -> Result<ReplacePreview, VaultError> {
        let text = self.get_note_text(path).await?;
        let (count, content) =
            Self::compute_replacement(&text, pattern, replacement, all, regex, path)?;
        Ok(ReplacePreview { count, content })
    }
}
