//! Index-backed lookups on [`NoteVault`]: search, labels and their
//! autocomplete, and the note lists by folder. None of these touch the disk
//! except through the index.

use std::collections::HashMap;

use chrono::{Days, NaiveDate};

use crate::error::VaultError;
use crate::nfs::{NoteEntryData, VaultPath};
use crate::note::NoteContentData;
use crate::{NoteSuggestion, NoteVault, TagSuggestion};

/// Which date a dated search compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateSource {
    /// The date in a journal note's file name.
    Journal,
    /// A date or date-time frontmatter property, by key.
    Property(String),
}

/// A date range for [`NoteVault::search_notes_dated`]. Both ends are
/// inclusive and required: a caller fills an open end itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateFilter {
    /// The date to compare.
    pub source: DateSource,
    /// First day in range.
    pub from: NaiveDate,
    /// Last day in range.
    pub to: NaiveDate,
}

/// The first day the index reads as a date.
fn min_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1, 1, 1).expect("valid date")
}

/// The last day the index reads as a date.
fn max_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(9999, 12, 31).expect("valid date")
}

impl DateFilter {
    fn source_name(&self) -> String {
        match &self.source {
            DateSource::Journal => crate::JOURNAL_DATE_SOURCE.to_string(),
            DateSource::Property(key) => key.to_lowercase(),
        }
    }
}

impl NoteVault {
    /// Searches notes whose chosen date falls in the filter's range. An empty
    /// `query` lists the notes in range, newest note date first. A non-empty
    /// `query` keeps the order of [`search_notes`](Self::search_notes). Notes
    /// without the chosen date never match. The journal source reads only the
    /// date in a journal note's name, and a property source reads only that
    /// property, so a property named `journal` never mixes with the journal
    /// date. A note in the index whose file is gone is left out. Fails with a
    /// user error when `from` is after `to` or the source is not a date source
    /// of the vault (see [`check_date_source`](Self::check_date_source)).
    pub async fn search_notes_dated(
        &self,
        query: &str,
        filter: &DateFilter,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        if filter.from > filter.to {
            return Err(VaultError::InvalidDateFilter {
                message: format!(
                    "Invalid date range: from {} is after to {}",
                    filter.from, filter.to
                ),
            });
        }
        let query = query.trim();
        let source = filter.source_name();
        self.check_date_source(&filter.source).await?;
        let candidates = match &filter.source {
            DateSource::Journal => {
                if query.is_empty() {
                    self.get_notes(self.journal_path(), true).await?
                } else {
                    self.search_notes(query).await?
                }
            }
            DateSource::Property(_) => {
                // Loose by a day on each side so a date-time near the edge is
                // not lost to the index comparison. The exact test is below.
                let key = if source.contains(char::is_whitespace) {
                    format!("\"{source}\"")
                } else {
                    source.clone()
                };
                // The padding never leaves 0001-01-01..=9999-12-31: the index
                // reads a date only as YYYY-MM-DD, so a year 10000 bound
                // would turn the test into a text comparison that never holds.
                let lo = filter
                    .from
                    .checked_sub_days(Days::new(1))
                    .filter(|d| *d >= min_date())
                    .unwrap_or(filter.from);
                let hi = filter
                    .to
                    .checked_add_days(Days::new(1))
                    .filter(|d| *d <= max_date())
                    .unwrap_or(filter.to);
                self.search_notes(format!("{query} %{key}>={lo} %{key}<={hi}"))
                    .await?
            }
        };
        let mut dated = Vec::new();
        for item in candidates {
            // Each source reads only its own dates, so a property named
            // `journal` never mixes with the journal date.
            let days: Vec<NaiveDate> = match &filter.source {
                DateSource::Journal => self.journal_date(&item.0.path).into_iter().collect(),
                DateSource::Property(_) => match self.property_dates(&item.0.path).await {
                    Ok(dates) => dates
                        .into_iter()
                        .filter(|d| d.source == source)
                        .map(|d| d.day)
                        .collect(),
                    // A stale index entry: the file was deleted outside Kimun.
                    Err(e) if e.is_not_found() => continue,
                    Err(e) => return Err(e),
                },
            };
            let in_range = days
                .into_iter()
                .filter(|d| *d >= filter.from && *d <= filter.to);
            if let Some(day) = in_range.max() {
                dated.push((day, item));
            }
        }
        if query.is_empty() {
            dated.sort_by_key(|(day, _)| std::cmp::Reverse(*day));
        }
        Ok(dated.into_iter().map(|(_, item)| item).collect())
    }

    /// Checks that `source` is a date source of the vault. The journal source
    /// needs a journal note with a date name. A property needs a property of
    /// that key (case-insensitive) whose dominant kind is date or date-time,
    /// so `Property("journal")` needs a real property named journal. Fails
    /// with a user error that lists the vault's date sources.
    pub async fn check_date_source(&self, source: &DateSource) -> Result<(), VaultError> {
        let what = match source {
            DateSource::Journal => {
                if self.has_journal_dates().await? {
                    return Ok(());
                }
                "No journal notes with a date name".to_string()
            }
            DateSource::Property(key) => {
                let key = key.to_lowercase();
                if self.date_property_keys().await?.contains(&key) {
                    return Ok(());
                }
                format!("Unknown date property '{key}'")
            }
        };
        let keys = self.date_keys().await?;
        let known = if keys.is_empty() {
            "none".to_string()
        } else {
            keys.join(", ")
        };
        Err(VaultError::InvalidDateFilter {
            message: format!("{what}. Date sources in this vault: {known}"),
        })
    }

    /// Searches notes using the vault's query syntax (see [`SearchTerms`](crate::SearchTerms)).
    /// Returns each matching note's entry and content data.
    pub async fn search_notes<S: AsRef<str>>(
        &self,
        search_query: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let search_query = search_query.as_ref();
        let a = self.index.search(search_query).await?;
        Ok(a)
    }

    /// Returns every distinct label persisted in the vault, lowercased.
    pub async fn list_labels(&self) -> Result<Vec<String>, VaultError> {
        Ok(self.index.list_labels().await?)
    }

    /// Returns notes whose name (filename without extension) starts with
    /// `prefix`, case-insensitive, capped at `limit`. Used to feed the
    /// wikilink autocomplete popup — note that the inserted wikilink target
    /// is the `name` field, not the `path`.
    pub async fn suggest_notes_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<NoteSuggestion>, VaultError> {
        Ok(self.index.suggest_notes_by_prefix(prefix, limit).await?)
    }

    /// Returns tag labels matching `prefix` (case-insensitive) paired with
    /// usage counts, capped at `limit`. Used to feed the hashtag autocomplete
    /// popup in both the editor and the search box.
    pub async fn suggest_tags_by_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<TagSuggestion>, VaultError> {
        Ok(self.index.suggest_tags_by_prefix(prefix, limit).await?)
    }

    /// Returns every distinct label in the vault paired with the number of
    /// notes carrying it. Labels are returned sorted alphabetically.
    pub async fn label_counts(&self) -> Result<Vec<(String, usize)>, VaultError> {
        let rows = self.index.label_counts().await?;
        Ok(rows.into_iter().map(|(n, c)| (n, c as usize)).collect())
    }

    /// Returns every note path that carries the given label. The label
    /// argument is lowercased before lookup, matching how labels are stored.
    pub async fn notes_with_label<S: AsRef<str>>(
        &self,
        name: S,
    ) -> Result<Vec<VaultPath>, VaultError> {
        Ok(self.index.notes_with_label(name.as_ref()).await?)
    }

    /// Get notes under the given path. When `recursive` is false, only direct
    /// children are returned.
    pub async fn get_notes(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let notes = self.index.get_notes(path, recursive).await?;
        Ok(notes)
    }

    /// Counts the notes under each immediate child directory of `parent`,
    /// at any depth. Keys are canonical child directory paths; a child with
    /// no notes is absent, and notes directly in `parent` are not counted.
    pub async fn child_note_counts(
        &self,
        parent: &VaultPath,
    ) -> Result<HashMap<VaultPath, usize>, VaultError> {
        let depth = parent.canonical().get_slices().len();
        let mut counts: HashMap<VaultPath, usize> = HashMap::new();
        for (base, n) in self.index.note_counts_by_base(parent).await? {
            let mut child = VaultPath::new(base).canonical();
            let mut len = child.get_slices().len();
            if len <= depth {
                continue; // a note directly in `parent`
            }
            while len > depth + 1 {
                child = child.get_parent_path().0;
                len -= 1;
            }
            *counts.entry(child).or_insert(0) += n as usize;
        }
        Ok(counts)
    }

    /// Returns every note in the vault with its entry and content data.
    pub async fn get_all_notes(&self) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let a = self.index.get_all_notes().await?;
        Ok(a)
    }

    /// The note's labels: inline `#hashtags` and frontmatter `tags:` together,
    /// lowercased, sorted, distinct. Index-backed: a missing or not-yet-indexed
    /// note returns an empty list, not `NotFound`.
    pub async fn get_tags(&self, path: &VaultPath) -> Result<Vec<String>, VaultError> {
        Ok(self.index.labels_of(path).await?)
    }
}
