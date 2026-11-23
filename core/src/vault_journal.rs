//! Journal and inbox on [`NoteVault`]: the directories they live in, quick
//! capture into the inbox, and today's journal entry.

use chrono::{NaiveDate, Utc};

use crate::error::{FSError, VaultError};
use crate::nfs::VaultPath;
use crate::note::NoteDetails;
use crate::NoteVault;

impl NoteVault {
    /// Directory under which journal entries are created.
    pub fn journal_path(&self) -> &VaultPath {
        &self.journal_path
    }

    /// Directory under which quick-capture notes are created.
    pub fn inbox_path(&self) -> &VaultPath {
        &self.inbox_path
    }

    /// Overrides the inbox directory used by [`Self::quick_note`].
    pub fn set_inbox_path(&mut self, path: VaultPath) {
        self.inbox_path = path;
    }

    /// Creates a timestamped note under the inbox directory. On name collision
    /// (including TOCTOU between the in-memory probe and the FS create), tries
    /// the next suffix up to `-99`. The retry loop calls `create_note`
    /// directly so each iteration's existence check is the atomic
    /// `O_EXCL` open inside `nfs::create_note_exclusive`.
    pub async fn quick_note(&self, text: &str) -> Result<NoteDetails, VaultError> {
        let base_name = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
        let candidate = |name: &str| {
            self.inbox_path
                .append(&VaultPath::note_path_from(name))
                .absolute()
        };

        for attempt in 0..=99 {
            let path = if attempt == 0 {
                candidate(&base_name)
            } else if attempt == 1 {
                continue; // attempts are labelled `name`, `name-2`, … `name-99`
            } else {
                candidate(&format!("{}-{}", base_name, attempt))
            };
            match self.create_note(&path, text).await {
                Ok(_) => return Ok(NoteDetails::new(&path, text)),
                Err(VaultError::NoteExists { .. }) => continue,
                Err(e) => return Err(e),
            }
        }

        let placeholder = candidate(&base_name);
        Err(VaultError::FSError(FSError::InvalidPath {
            path: placeholder.to_string(),
            message: "Could not find a free quick note name".to_string(),
        }))
    }

    /// Loads today's journal entry, creating it with a date heading if it does
    /// not exist yet. Returns the note details, its content, and `true` when the
    /// entry was freshly created.
    pub async fn journal_entry(&self) -> Result<(NoteDetails, String, bool), VaultError> {
        let (title, note_path) = self.get_todays_journal();
        let (content, created) = self
            .load_or_create_note(&note_path, Some(format!("# {}\n\n", title)))
            .await?;
        let details = NoteDetails::new(&note_path, &content);
        Ok((details, content, created))
    }

    pub(crate) fn get_todays_journal(&self) -> (String, VaultPath) {
        let today_string = crate::dates::format_iso_date(Utc::now().date_naive());

        (
            today_string.clone(),
            self.journal_path
                .append(&VaultPath::note_path_from(&today_string))
                .absolute(),
        )
    }

    /// Parses the date out of a journal note path, or `None` when `note_path`
    /// is not a `YYYY-MM-DD` note directly under the journal directory.
    pub fn journal_date(&self, note_path: &VaultPath) -> Option<NaiveDate> {
        if !note_path.is_note() {
            return None;
        }

        let (parent, _) = note_path.get_parent_path();
        if parent.eq(&self.journal_path) {
            let name = note_path.get_clean_name();
            crate::dates::parse_iso_date(&name)
        } else {
            None
        }
    }
}
