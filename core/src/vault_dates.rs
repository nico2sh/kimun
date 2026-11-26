//! Note dates on [`NoteVault`]: the days a note carries, from its journal
//! file name and from its date-typed frontmatter properties. One rule shared
//! by dated search and the server push.

use chrono::NaiveDate;

use crate::error::VaultError;
use crate::nfs::VaultPath;
use crate::note::{PropertyKind, PropertyValue};
use crate::NoteVault;

/// The source name of a journal note's date.
pub const JOURNAL_DATE_SOURCE: &str = "journal";

/// One day a note carries, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteDate {
    /// `"journal"`, or the lowercase property key.
    pub source: String,
    /// The day, as written (a date-time keeps its local day).
    pub day: NaiveDate,
}

impl NoteVault {
    /// Every date the note carries: its journal date (source `"journal"`)
    /// when it sits directly under the journal directory with a date as file
    /// name, then each date or date-time property in file order. Properties
    /// use their lowercase key as source. A note with no dates returns an
    /// empty list.
    pub async fn note_dates(&self, path: &VaultPath) -> Result<Vec<NoteDate>, VaultError> {
        let mut dates = Vec::new();
        if let Some(day) = self.journal_date(path) {
            dates.push(NoteDate {
                source: JOURNAL_DATE_SOURCE.to_string(),
                day,
            });
        }
        dates.extend(self.property_dates(path).await?);
        Ok(dates)
    }

    /// The note's date and date-time properties in file order, each with its
    /// lowercase key as source. Never includes the journal date, even for a
    /// property named `journal`. Reads the note from disk.
    pub(crate) async fn property_dates(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<NoteDate>, VaultError> {
        let mut dates = Vec::new();
        for (key, value) in self.get_properties(path).await? {
            // A date-time keeps the day as written, with no zone conversion.
            let day = match value {
                Some(PropertyValue::Date(d)) => d,
                Some(PropertyValue::DateTime(dt)) => dt.naive_local().date(),
                _ => continue,
            };
            dates.push(NoteDate {
                source: key.to_lowercase(),
                day,
            });
        }
        Ok(dates)
    }

    /// Whether any note directly under the journal directory has a date as
    /// file name, so that `"journal"` is a usable date source.
    pub async fn has_journal_dates(&self) -> Result<bool, VaultError> {
        Ok(self
            .get_notes(self.journal_path(), false)
            .await?
            .iter()
            .any(|(entry, _)| self.journal_date(&entry.path).is_some()))
    }

    /// The property keys whose dominant kind is date or date-time, sorted.
    /// A property literally named `journal` is listed like any other; it is
    /// not merged with the journal source (see [`NoteVault::date_keys`]).
    pub async fn date_property_keys(&self) -> Result<Vec<String>, VaultError> {
        let mut keys = Vec::new();
        let nothing_excluded = VaultPath::root();
        for key in self.property_keys().await? {
            let kind = self
                .index
                .dominant_property_kind(&key, &nothing_excluded)
                .await?;
            if matches!(kind, Some(PropertyKind::Date | PropertyKind::DateTime)) {
                keys.push(key);
            }
        }
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    /// The date sources the vault has, sorted: `"journal"` when any note
    /// directly under the journal directory has a date name, plus each
    /// property key whose dominant kind is date or date-time. A property
    /// named `journal` merges with the journal source here.
    pub async fn date_keys(&self) -> Result<Vec<String>, VaultError> {
        let mut keys = self.date_property_keys().await?;
        if self.has_journal_dates().await? {
            keys.push(JOURNAL_DATE_SOURCE.to_string());
        }
        keys.sort();
        keys.dedup();
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VaultConfig;
    use tempfile::TempDir;

    async fn vault_with_notes() -> (TempDir, NoteVault) {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::new("/journal/2026-11-20.md"), "# day\n")
            .await
            .unwrap();
        vault
            .create_note(
                &VaultPath::new("task.md"),
                "---\ndue: 2026-11-25\ntitle: Plain\n---\nbody\n",
            )
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::new("plain.md"), "just text\n")
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        (dir, vault)
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[tokio::test]
    async fn note_dates_reads_journal_name_and_date_properties() {
        let (_dir, vault) = vault_with_notes().await;
        let journal = vault
            .note_dates(&VaultPath::new("/journal/2026-11-20.md"))
            .await
            .unwrap();
        assert_eq!(
            journal,
            vec![NoteDate {
                source: "journal".into(),
                day: day(2026, 11, 20)
            }]
        );
        let task = vault.note_dates(&VaultPath::new("task.md")).await.unwrap();
        assert_eq!(
            task,
            vec![NoteDate {
                source: "due".into(),
                day: day(2026, 11, 25)
            }]
        );
    }

    #[tokio::test]
    async fn note_dates_empty_for_plain_note() {
        let (_dir, vault) = vault_with_notes().await;
        assert!(vault
            .note_dates(&VaultPath::new("plain.md"))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn date_keys_lists_journal_and_date_properties_only() {
        let (_dir, vault) = vault_with_notes().await;
        assert_eq!(vault.date_keys().await.unwrap(), vec!["due", "journal"]);
    }

    #[tokio::test]
    async fn date_keys_omits_journal_when_no_journal_notes() {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::new("task.md"), "---\ndue: 2026-11-25\n---\nx\n")
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        assert_eq!(vault.date_keys().await.unwrap(), vec!["due"]);
    }

    #[tokio::test]
    async fn date_property_keys_lists_properties_only_and_keeps_a_journal_property() {
        let (_dir, vault) = vault_with_notes().await;
        assert_eq!(vault.date_property_keys().await.unwrap(), vec!["due"]);
        assert!(vault.has_journal_dates().await.unwrap());
        vault
            .create_note(
                &VaultPath::new("odd.md"),
                "---\njournal: 2026-11-01\n---\nx\n",
            )
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        assert_eq!(
            vault.date_property_keys().await.unwrap(),
            vec!["due", "journal"]
        );
        // date_keys still merges the two.
        assert_eq!(vault.date_keys().await.unwrap(), vec!["due", "journal"]);
    }

    #[tokio::test]
    async fn has_journal_dates_is_false_without_journal_notes() {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir.path())))
            .await
            .unwrap();
        vault.validate_and_init().await.unwrap();
        assert!(!vault.has_journal_dates().await.unwrap());
    }
}
