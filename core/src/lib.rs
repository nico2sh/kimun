mod db;
pub mod error;
pub mod nfs;
pub mod note;
pub mod utilities;

use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender},
    time::{Duration, SystemTime},
};

use chrono::{NaiveDate, Utc};
use db::VaultDB;
use error::{DBError, FSError, VaultError};
use log::debug;
use nfs::{visitor::NoteListVisitorBuilder, NoteEntryData, VaultEntry, VaultPath};
use note::{ContentChunk, NoteContentData, NoteDetails};
use utilities::path_to_string;

use crate::nfs::DirectoryEntryData;

pub const DEFAULT_JOURNAL_PATH: &str = "/journal";

pub struct IndexReport {
    pub start: SystemTime,
    pub duration: Duration,
}

impl IndexReport {
    fn new() -> Self {
        let start = SystemTime::now();
        Self {
            start,
            duration: Duration::default(),
        }
    }

    fn finish(&mut self) {
        let time = SystemTime::now();
        let duration = time.duration_since(self.start).unwrap_or_default();
        self.duration = duration;
    }
}

#[derive(Debug, Clone)]
pub struct NoteVault {
    pub workspace_path: PathBuf,
    journal_path: VaultPath,
    vault_db: VaultDB,
}

// Manual PartialEq implementation comparing only workspace_path
// (SqlitePool doesn't implement PartialEq, but vaults with same workspace are equivalent)
impl PartialEq for NoteVault {
    fn eq(&self, other: &Self) -> bool {
        self.workspace_path == other.workspace_path
    }
}

impl NoteVault {
    /// Creates a new instance of the Note Vault.
    /// Make sure you call `NoteVault::init_and_validate(&self)` to initialize the DB index if
    /// needed
    pub async fn new<P: AsRef<Path>>(workspace_path: P) -> Result<Self, VaultError> {
        debug!("Creating new vault Instance");
        let workspace_path = workspace_path.as_ref().to_path_buf();
        if !workspace_path.exists() {
            return Err(VaultError::VaultPathNotFound {
                path: path_to_string(workspace_path),
            })?;
        }
        if !workspace_path.is_dir() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: path_to_string(workspace_path),
                message: "Path provided is not a directory".to_string(),
            }))?;
        };

        let vault_db = VaultDB::new(&workspace_path).await?;
        let note_vault = Self {
            workspace_path,
            journal_path: VaultPath::new(DEFAULT_JOURNAL_PATH),
            vault_db,
        };
        Ok(note_vault)
    }

    /// On init and validate it verifies the DB index to make sure:
    ///
    /// 1. It exists
    /// 2. It is valid.
    /// 3. Its schema is updated
    ///
    /// Then does a quick scan of the workspace directory to update the index if there are new or
    /// missing notes.
    /// This can be slow on large vaults.
    pub async fn init_and_validate(&self) -> Result<IndexReport, VaultError> {
        debug!("Initializing DB and validating it");
        let db_result = self.vault_db.check_db().await;
        match db_result {
            Ok(check_res) => {
                match check_res {
                    db::DBStatus::Ready => {
                        // We only check if there are new notes
                        self.index_notes(NotesValidation::None).await
                    }
                    db::DBStatus::Outdated => self.recreate_index().await,
                    db::DBStatus::NotValid => self.recreate_index().await,
                    db::DBStatus::FileNotFound => {
                        // No need to validate, no data there
                        self.recreate_index().await
                    }
                }
            }
            Err(e) => {
                debug!("Error validating the DB, rebuilding it: {}", e);
                self.recreate_index().await
            }
        }
    }

    /// Deletes the db file and recreates the index.
    /// On Windows, the pool must be closed before the file can be deleted,
    /// so this method closes the pool first. After calling this method,
    /// the NoteVault instance should be discarded and a new one created.
    pub async fn force_rebuild(&self) -> Result<IndexReport, VaultError> {
        let db_path = self.vault_db.get_db_path();
        // Close the pool to release file handles before deleting.
        // This is required on Windows where open handles prevent file deletion.
        self.vault_db.close().await?;
        let md = std::fs::metadata(&db_path).map_err(FSError::ReadFileError)?;
        // We delete the db file
        if md.is_dir() {
            std::fs::remove_dir_all(db_path).map_err(FSError::ReadFileError)?;
        } else {
            std::fs::remove_file(db_path).map_err(FSError::ReadFileError)?;
        }
        // Note: the pool is closed at this point. The caller should create
        // a new NoteVault instance if further DB operations are needed.
        // recreate_index will reconnect via the pool's rwc mode which
        // recreates the file.
        self.recreate_index().await
    }

    /// Deletes all the cached data from the DB by destroying the tables
    /// and recreates the index
    /// This is similar to a force rebuild but instead of deleting the db file
    /// it only deletes the tables.
    pub async fn recreate_index(&self) -> Result<IndexReport, VaultError> {
        let index_report = IndexReport::new();
        debug!("Initializing DB from Vault request");
        db::init_db(self.vault_db.pool()).await?;
        debug!("Tables created, creating index");
        self.int_index_notes(index_report, NotesValidation::Full).await
    }

    /// Traverses the whole vault directory and verifies the notes to
    /// update the cached data in the DB. The validation is defined by
    /// the validation mode:
    ///
    /// NotesValidation::Full Checks the content of the note by comparing a hash based on the text
    /// conatined in the file.
    /// NotesValidation::Fast Checks the size of the file to identify if the note has changed and
    /// then update the DB entry.
    /// NotesValidation::None Checks if the note exists or not.
    pub async fn index_notes(&self, validation_mode: NotesValidation) -> Result<IndexReport, VaultError> {
        let index_report = IndexReport::new();
        self.int_index_notes(index_report, validation_mode).await
    }

    async fn int_index_notes(
        &self,
        mut index_report: IndexReport,
        validation_mode: NotesValidation,
    ) -> Result<IndexReport, VaultError> {
        let workspace_path = self.workspace_path.clone();
        create_index_for(&workspace_path, self.vault_db.pool(), &VaultPath::root(), validation_mode).await?;
        index_report.finish();
        debug!("TIME: {}", index_report.duration.as_secs());
        Ok(index_report)
    }

    pub async fn exists(&self, path: &VaultPath) -> Option<VaultEntry> {
        VaultEntry::new(&self.workspace_path, path.to_owned()).await.ok()
    }

    pub async fn journal_entry(&self) -> Result<(NoteDetails, String), VaultError> {
        let (title, note_path) = self.get_todays_journal();
        let content = self.load_or_create_note(&note_path, Some(format!("# {}\n\n", title))).await?;
        let details = NoteDetails::new(&note_path, &content);
        Ok((details, content))
    }

    fn get_todays_journal(&self) -> (String, VaultPath) {
        let today = Utc::now();
        let today_string = today.format("%Y-%m-%d").to_string();

        (
            today_string.clone(),
            self.journal_path
                .append(&VaultPath::note_path_from(&today_string))
                .absolute(),
        )
    }

    // Returns a NaiveDate if the note path is a valid journal entry
    pub fn journal_date(&self, note_path: &VaultPath) -> Option<NaiveDate> {
        if !note_path.is_note() {
            return None;
        }

        let (parent, _) = note_path.get_parent_path();
        if parent.eq(&self.journal_path) {
            let name = note_path.get_clean_name();
            NaiveDate::parse_from_str(&name, "%Y-%m-%d").ok()
        } else {
            None
        }
    }

    // create a new one, a text can be specified as the initial text for the
    // note when created
    pub async fn load_or_create_note(
        &self,
        path: &VaultPath,
        default_text: Option<String>,
    ) -> Result<String, VaultError> {
        match nfs::load_note(&self.workspace_path, path).await {
            Ok(text) => Ok(text),
            Err(e) => {
                if let FSError::VaultPathNotFound { path: _ } = e {
                    let text = default_text.unwrap_or_default();
                    self.create_note(path, &text).await?;
                    Ok(text)
                } else {
                    Err(e)?
                }
            }
        }
    }

    // Loads the note's content, returns the text
    // If the file doesn't exist you will get a VaultError::FSError with a
    // FSError::NotePathNotFound as the source, you can use that to
    // lazy create a note, or use the load_or_create_note function instead
    pub async fn get_note_text(&self, path: &VaultPath) -> Result<String, VaultError> {
        let text = nfs::load_note(&self.workspace_path, path).await?;
        Ok(text)
    }

    // Loads a note, returning its details that contain path, raw text and more
    // If the file doesn't exist you will get a VaultError::FSError with a
    // FSError::NotePathNotFound as the source, you can use that to
    // lazy create a note, or use the load_or_create_note function instead
    pub async fn load_note(&self, path: &VaultPath) -> Result<NoteDetails, VaultError> {
        let text = self.get_note_text(path).await?;
        Ok(NoteDetails::new(path, text))
    }

    pub async fn get_note_chunks(
        &self,
        path: &VaultPath,
    ) -> Result<HashMap<VaultPath, Vec<ContentChunk>>, VaultError> {
        let a = db::get_notes_sections(self.vault_db.pool(), path, false).await?;
        Ok(a)
    }

    // Search notes using a search syntax
    pub async fn search_notes<S: AsRef<str>>(
        &self,
        search_query: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let search_query = search_query.as_ref();
        let a = db::search_terms(self.vault_db.pool(), search_query).await?;
        Ok(a)
    }

    // Get all notes
    pub async fn get_all_notes(&self) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        let a = db::get_all_notes(self.vault_db.pool()).await?;
        Ok(a)
    }
    pub fn path_to_pathbuf(&self, path: &VaultPath) -> PathBuf {
        path.to_pathbuf(&self.workspace_path)
    }

    pub async fn browse_vault(&self, options: VaultBrowseOptions) -> Result<(), VaultError> {
        let start = std::time::SystemTime::now();
        debug!("> Start fetching files with Options:\n{}", options);

        let cached_notes = db::get_notes(self.vault_db.pool(), &options.path, options.recursive).await?;

        let mut builder = NoteListVisitorBuilder::new(
            &self.workspace_path,
            options.validation,
            cached_notes,
            Some(options.sender.clone()),
            tokio::runtime::Handle::current(),
        );
        // We traverse the directory
        let walker = nfs::get_file_walker(
            self.workspace_path.clone(),
            &options.path,
            options.recursive,
        );
        walker.visit(&mut builder);

        let notes_to_add = builder.get_notes_to_add();
        let notes_to_delete = builder.get_notes_to_delete();
        let notes_to_modify = builder.get_notes_to_modify();

        let mut tx = self.vault_db.pool().begin().await.map_err(DBError::from)?;
        db::insert_notes(&mut tx, &notes_to_add).await?;
        db::delete_notes(&mut tx, &notes_to_delete).await?;
        db::update_notes(&mut tx, &notes_to_modify).await?;
        tx.commit().await.map_err(DBError::from)?;

        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!("> Files fetched in {} milliseconds", time.as_millis());

        Ok(())
    }

    // pub fn get_notes(
    //     &self,
    //     path: &VaultPath,
    //     recursive: bool,
    // ) -> Result<Vec<NoteContentData>, VaultError> {
    //     let start = std::time::SystemTime::now();
    //     debug!("> Start fetching files from cache");
    //     let note_path = path.into();

    //     let cached_notes = self.vault_db.call(move |conn| {
    //         let notes = db::get_notes(conn, &note_path, recursive)?;
    //         Ok(notes)
    //     })?;

    //     let result = cached_notes
    //         .iter()
    //         .map(|(_data, details)| details.to_owned())
    //         .collect::<Vec<NoteContentData>>();
    //     let time = std::time::SystemTime::now()
    //         .duration_since(start)
    //         .expect("Something's wrong with the time");
    //     debug!("> Files fetched in {} milliseconds", time.as_millis());
    //     Ok(result)
    // }

    /// Returns all subdirectories under `path`.
    /// Non-recursive returns only the immediate children; recursive returns the full tree.
    pub fn get_directories(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<DirectoryDetails>, VaultError> {
        Ok(nfs::list_directories(&self.workspace_path, path, recursive)?)
    }

    /// Converts a note's raw Markdown into rendered Markdown and extracts all links.
    ///
    /// - WikiLinks (`[[note]]`) are converted to standard Markdown links.
    /// - Note links are resolved to vault-relative absolute paths.
    /// - Hashtags become Markdown links (`[#tag](#tag)`) and are added to the links list.
    /// - Image paths are resolved to absolute OS paths so renderers can load them directly.
    ///   Relative image paths are resolved against the note's location in the vault.
    ///   External image URLs are kept as-is.
    pub fn get_markdown_and_links(&self, note: &NoteDetails) -> note::MarkdownNote {
        // Step 1: convert wikilinks, extract note/URL/hashtag links.
        let note_parent = if note.path.is_note() {
            note.path.get_parent_path().0
        } else {
            note.path.clone()
        };
        let (md_text, mut links) =
            note::content_extractor::get_markdown_and_links(&note.path, &note.raw_text);

        // Step 2: resolve image paths to absolute OS paths.
        let (md_text, image_links) =
            note::content_extractor::process_image_links(&md_text, |alt_text, raw_path| {
                let resolved = if raw_path.starts_with("http://")
                    || raw_path.starts_with("https://")
                {
                    // External URL: keep as-is
                    raw_path.to_string()
                } else {
                    // Vault-relative or note-relative path → absolute OS path
                    let image_vault_path = if raw_path.starts_with('/') {
                        VaultPath::new(raw_path)
                    } else {
                        note_parent.append(&VaultPath::new(raw_path)).flatten()
                    };
                    image_vault_path
                        .to_pathbuf(&self.workspace_path)
                        .display()
                        .to_string()
                };
                let link = note::NoteLink::image(&resolved, alt_text, raw_path);
                (resolved, link)
            });

        links.extend(image_links);
        note::MarkdownNote { text: md_text, links }
    }

    /// Returns all notes that contain a link pointing to `path`.
    /// Matches both absolute vault paths and bare filename links (wikilinks).
    pub async fn get_backlinks(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        Ok(db::get_backlinks(self.vault_db.pool(), path).await?)
    }

    pub async fn create_note<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        if self.exists(path).await.is_none() {
            self.save_note(path, text).await
        } else {
            Err(VaultError::NoteExists { path: path.clone() })
        }
    }

    pub async fn create_directory(&self, path: &VaultPath) -> Result<DirectoryEntryData, VaultError> {
        if self.exists(path).await.is_none() {
            let ded = nfs::create_directory(&self.workspace_path, path).await?;
            Ok(ded)
        } else {
            Err(VaultError::DirectoryExists { path: path.clone() })
        }
    }

    pub async fn save_note<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        // Save to disk
        let entry_data = nfs::save_note(&self.workspace_path, path, &text).await?;

        // Build NoteDetails once from the text already in memory — no re-read from disk
        let note_details = NoteDetails::new(path, text);
        let content_data = note_details.get_content_data();

        // Save to DB (reuses the same NoteDetails)
        db::save_note(self.vault_db.pool(), &entry_data, &note_details).await?;

        Ok((entry_data, content_data))
    }

    /// If the string is a path, it looks for a specific note, if it's just a note name
    /// it looks for that note in any path in the vault, so it may return many results
    pub async fn open_or_search(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        // We make sure the path is a note path, so we append the extension if doesn't exist
        // let path = VaultPath::note_path_from(&path_or_note);
        debug!("PATH: {}", path);
        let (_parent, name) = path.get_parent_path();

        // If it starts with the root trailing slash, we assume is looking for a path
        // let is_note_name = !path_or_note.as_ref().starts_with(nfs::PATH_SEPARATOR)
        //     && parent.eq(&VaultPath::root());

        if path.is_note_file() {
            debug!("We search by name {}", name);
            Ok(db::search_note_by_name(self.vault_db.pool(), name).await?)
        } else {
            debug!("We search by path {}", path);
            Ok(db::search_note_by_path(self.vault_db.pool(), path).await?)
        }
    }

    pub async fn delete_note(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        if !path.is_note() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: path.to_string(),
                message: "The path is not a note".to_string(),
            }));
        }

        // We delete in DB first
        let mut tx = self.vault_db.pool().begin().await.map_err(DBError::from)?;
        db::delete_notes(&mut tx, &[path.clone()]).await?;
        tx.commit().await.map_err(DBError::from)?;

        nfs::delete_note(&self.workspace_path, &path).await?;

        Ok(())
    }

    pub async fn delete_directory(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        if path.is_note() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: path.to_string(),
                message: "The path is not a directory".to_string(),
            }));
        }

        // We delete in DB first
        let mut tx = self.vault_db.pool().begin().await.map_err(DBError::from)?;
        db::delete_directories(&mut tx, &[path.clone()]).await?;
        tx.commit().await.map_err(DBError::from)?;

        nfs::delete_directory(&self.workspace_path, &path).await?;

        Ok(())
    }

    pub async fn rename_note(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();

        if self.exists(&to).await.is_some() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: to.to_string(),
                message: "Destination path already exists".to_string(),
            }));
        }

        // Update every note that links to `from`: rewrite those links to `to` in both
        // the file on disk and the DB index.
        let backlinks = db::get_backlinks(self.vault_db.pool(), &from).await?;
        for (entry_data, _) in &backlinks {
            let text = nfs::load_note(&self.workspace_path, &entry_data.path).await?;
            let (updated_text, changed) =
                note::content_extractor::replace_note_links(&text, &from, &to);
            if changed {
                self.save_note(&entry_data.path, updated_text).await?;
            }
        }

        // Rename the file on disk, then update the DB entry for the renamed note.
        nfs::rename_note(&self.workspace_path, &from, &to).await?;

        let mut tx = self.vault_db.pool().begin().await.map_err(DBError::from)?;
        db::rename_note(&mut tx, &from, &to).await?;
        tx.commit().await.map_err(DBError::from)?;

        Ok(())
    }

    pub async fn rename_directory(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();

        if self.exists(&to).await.is_some() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: to.to_string(),
                message: "Destination path already exists".to_string(),
            }));
        }
        nfs::rename_directory(&self.workspace_path, &from, &to).await?;

        let mut tx = self.vault_db.pool().begin().await.map_err(DBError::from)?;
        db::rename_directory(&mut tx, &from, &to).await?;
        tx.commit().await.map_err(DBError::from)?;

        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirectoryDetails {
    pub path: VaultPath,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub path: VaultPath,
    pub rtype: ResultType,
}

impl SearchResult {
    pub fn note(path: &VaultPath, content_data: &NoteContentData) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Note(content_data.to_owned()),
        }
    }
    pub fn directory(path: &VaultPath) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Directory,
        }
    }
    pub fn attachment(path: &VaultPath) -> Self {
        Self {
            path: path.to_owned(),
            rtype: ResultType::Attachment,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResultType {
    Note(NoteContentData),
    Directory,
    Attachment,
}

pub struct VaultBrowseOptionsBuilder {
    path: VaultPath,
    validation: NotesValidation,
    recursive: bool,
}

impl VaultBrowseOptionsBuilder {
    pub fn new(path: &VaultPath) -> Self {
        Self::default().path(path.clone())
    }

    pub fn build(self) -> (VaultBrowseOptions, Receiver<SearchResult>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            VaultBrowseOptions {
                path: self.path,
                validation: self.validation,
                recursive: self.recursive,
                sender,
            },
            receiver,
        )
    }

    pub fn path(mut self, path: VaultPath) -> Self {
        self.path = path;
        self
    }

    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    pub fn non_recursive(mut self) -> Self {
        self.recursive = false;
        self
    }

    pub fn full_validation(mut self) -> Self {
        self.validation = NotesValidation::Full;
        self
    }

    pub fn fast_validation(mut self) -> Self {
        self.validation = NotesValidation::Fast;
        self
    }

    pub fn no_validation(mut self) -> Self {
        self.validation = NotesValidation::None;
        self
    }
}

impl Default for VaultBrowseOptionsBuilder {
    fn default() -> Self {
        Self {
            path: VaultPath::root(),
            validation: NotesValidation::None,
            recursive: false,
        }
    }
}

#[derive(Debug, Clone)]
/// Options to traverse the Notes
/// You need a sync::mpsc::Sender to use a channel to receive the entries
pub struct VaultBrowseOptions {
    path: VaultPath,
    validation: NotesValidation,
    recursive: bool,
    sender: Sender<SearchResult>,
}

impl Display for VaultBrowseOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Vault Browse Options - [Path: `{}`|Validation Type: `{}`|Recursive: `{}`]",
            self.path, self.validation, self.recursive
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NotesValidation {
    Full,
    Fast,
    None,
}

impl Display for NotesValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                NotesValidation::Full => "Full",
                NotesValidation::Fast => "Fast",
                NotesValidation::None => "None",
            }
        )
    }
}

#[async_recursion::async_recursion]
async fn create_index_for<P: AsRef<Path> + Send>(
    workspace_path: P,
    pool: &sqlx::SqlitePool,
    path: &VaultPath,
    validation_mode: NotesValidation,
) -> Result<(), DBError> {
    debug!("Start fetching files at {}", path);
    let workspace_path = workspace_path.as_ref();
    let walker = nfs::get_file_walker(workspace_path, path, false);

    let cached_notes = db::get_notes(pool, path, false).await?;
    let mut builder =
        NoteListVisitorBuilder::new(workspace_path, validation_mode, cached_notes, None, tokio::runtime::Handle::current());
    walker.visit(&mut builder);
    let notes_to_add = builder.get_notes_to_add();
    let notes_to_delete = builder.get_notes_to_delete();
    let notes_to_modify = builder.get_notes_to_modify();

    let mut tx = pool.begin().await?;
    db::delete_notes(&mut tx, &notes_to_delete).await?;
    db::insert_notes(&mut tx, &notes_to_add).await?;
    db::update_notes(&mut tx, &notes_to_modify).await?;
    tx.commit().await?;

    let directories_to_insert = builder.get_directories_found();
    for directory in directories_to_insert.iter().filter(|p| !p.eq(&path)) {
        create_index_for(workspace_path, pool, directory, validation_mode).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use std::time::Duration;
    use tempfile::TempDir;

    // Helper: build a NoteVault pointing at a temp directory (no DB needed for pure-text tests).
    async fn make_vault(dir: &std::path::Path) -> NoteVault {
        NoteVault::new(dir).await.unwrap()
    }

    #[tokio::test]
    async fn get_markdown_and_links_resolves_relative_image() {
        let dir = TempDir::new().unwrap();
        let vault = make_vault(dir.path()).await;

        // Note at /directory/note.md, image at ../photo.png  →  /photo.png in vault
        let note = note::NoteDetails::new(
            &VaultPath::new("/directory/note.md"),
            "![alt](../photo.png)",
        );
        let md_note = vault.get_markdown_and_links(&note);

        let expected_os_path = dir.path().join("photo.png").display().to_string();
        assert_eq!(md_note.text, format!("![alt]({})", expected_os_path));
        assert_eq!(1, md_note.links.len());
        let link = &md_note.links[0];
        assert_eq!(
            link.ltype,
            note::LinkType::Image(expected_os_path)
        );
        assert_eq!(link.text, "alt");
        assert_eq!(link.raw_link, "../photo.png");
    }

    #[tokio::test]
    async fn get_markdown_and_links_resolves_absolute_vault_image() {
        let dir = TempDir::new().unwrap();
        let vault = make_vault(dir.path()).await;

        // Note anywhere, image at /assets/banner.png (vault-absolute)
        let note = note::NoteDetails::new(
            &VaultPath::new("/notes/note.md"),
            "![banner](/assets/banner.png)",
        );
        let md_note = vault.get_markdown_and_links(&note);

        let expected_os_path = dir
            .path()
            .join("assets")
            .join("banner.png")
            .display()
            .to_string();
        assert_eq!(md_note.text, format!("![banner]({})", expected_os_path));
        assert!(matches!(
            &md_note.links[0].ltype,
            note::LinkType::Image(p) if *p == expected_os_path
        ));
    }

    #[tokio::test]
    async fn get_markdown_and_links_keeps_external_image_url() {
        let dir = TempDir::new().unwrap();
        let vault = make_vault(dir.path()).await;

        let url = "https://example.com/img.png";
        let note = note::NoteDetails::new(
            &VaultPath::new("/note.md"),
            &format!("![remote]({})", url),
        );
        let md_note = vault.get_markdown_and_links(&note);

        // URL must be kept verbatim in the output markdown
        assert_eq!(md_note.text, format!("![remote]({})", url));
        assert!(matches!(
            &md_note.links[0].ltype,
            note::LinkType::Image(p) if p == url
        ));
        assert_eq!(md_note.links[0].raw_link, url);
    }

    #[tokio::test]
    async fn get_markdown_and_links_mixed_content() {
        let dir = TempDir::new().unwrap();
        let vault = make_vault(dir.path()).await;

        // Mix: wikilink, note link, image, hashtag
        let note = note::NoteDetails::new(
            &VaultPath::new("/note.md"),
            "[[Other Note]] [link](other.md) ![img](photo.png) #tag",
        );
        let md_note = vault.get_markdown_and_links(&note);

        // Image link present
        assert_eq!(
            1,
            md_note
                .links
                .iter()
                .filter(|l| matches!(l.ltype, note::LinkType::Image(_)))
                .count()
        );
        // Note links: wikilink + markdown note link
        assert_eq!(
            2,
            md_note
                .links
                .iter()
                .filter(|l| matches!(l.ltype, note::LinkType::Note(_)))
                .count()
        );
        // Hashtag
        assert_eq!(
            1,
            md_note
                .links
                .iter()
                .filter(|l| matches!(l.ltype, note::LinkType::Hashtag))
                .count()
        );
    }

    // ---- rename_note: backlink rewriting integration tests ----

    /// Create a small vault with a DB, write two notes, index them, then rename one
    /// and assert that the other note's content and DB links are updated.
    async fn setup_vault_with_notes(
        dir: &std::path::Path,
    ) -> NoteVault {
        let vault = NoteVault::new(dir).await.unwrap();
        vault.init_and_validate().await.unwrap();
        vault
    }

    #[tokio::test]
    async fn rename_note_updates_wikilink_in_backlink() {
        let dir = TempDir::new().unwrap();
        let vault = setup_vault_with_notes(dir.path()).await;

        // Create the note that will be renamed
        vault
            .save_note(&VaultPath::new("/target.md"), "# Target note")
            .await
            .unwrap();
        // Create a note that links to it via wikilink
        vault
            .save_note(
                &VaultPath::new("/referrer.md"),
                "# Referrer\nSee [[target]].",
            )
            .await
            .unwrap();

        vault
            .rename_note(&VaultPath::new("/target.md"), &VaultPath::new("/renamed.md"))
            .await
            .unwrap();

        // The referrer file on disk must now use [[renamed]]
        let updated = nfs::load_note(dir.path(), &VaultPath::new("/referrer.md"))
            .await
            .unwrap();
        assert!(
            updated.contains("[[renamed]]"),
            "expected [[renamed]] in: {updated}"
        );
        assert!(
            !updated.contains("[[target]]"),
            "old wikilink still present in: {updated}"
        );
    }

    #[tokio::test]
    async fn rename_note_updates_markdown_link_in_backlink() {
        let dir = TempDir::new().unwrap();
        let vault = setup_vault_with_notes(dir.path()).await;

        vault
            .save_note(&VaultPath::new("/target.md"), "# Target note")
            .await
            .unwrap();
        vault
            .save_note(
                &VaultPath::new("/referrer.md"),
                "# Referrer\n[link](/target.md) end.",
            )
            .await
            .unwrap();

        vault
            .rename_note(&VaultPath::new("/target.md"), &VaultPath::new("/renamed.md"))
            .await
            .unwrap();

        let updated = nfs::load_note(dir.path(), &VaultPath::new("/referrer.md"))
            .await
            .unwrap();
        assert!(
            updated.contains("[link](/renamed.md)"),
            "expected updated link in: {updated}"
        );
        assert!(
            !updated.contains("/target.md"),
            "old path still present in: {updated}"
        );
    }

    #[tokio::test]
    async fn rename_note_does_not_touch_unrelated_notes() {
        let dir = TempDir::new().unwrap();
        let vault = setup_vault_with_notes(dir.path()).await;

        vault
            .save_note(&VaultPath::new("/target.md"), "# Target")
            .await
            .unwrap();
        vault
            .save_note(&VaultPath::new("/unrelated.md"), "# Unrelated\nNo links here.")
            .await
            .unwrap();

        vault
            .rename_note(&VaultPath::new("/target.md"), &VaultPath::new("/renamed.md"))
            .await
            .unwrap();

        let unrelated = nfs::load_note(dir.path(), &VaultPath::new("/unrelated.md"))
            .await
            .unwrap();
        assert_eq!(unrelated, "# Unrelated\nNo links here.");
    }

    #[test]
    fn test_index_report_finish() {
        let mut report = IndexReport::new();

        // Sleep for a small amount to ensure duration is non-zero
        std::thread::sleep(Duration::from_millis(10));

        report.finish();

        // Check that duration is now set and non-zero
        assert!(report.duration > Duration::default());
        assert!(report.duration.as_millis() >= 10);
    }

    #[tokio::test]
    async fn test_note_vault_new_with_nonexistent_path() {
        let nonexistent_path = "/this/path/does/not/exist";
        let result = NoteVault::new(nonexistent_path).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            VaultError::VaultPathNotFound { path } => {
                assert_eq!(path, nonexistent_path);
            }
            _ => panic!("Expected VaultPathNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_note_vault_new_with_file_instead_of_directory() {
        // Create a temporary file
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let file_path = temp_file.path();

        let result = NoteVault::new(file_path).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            VaultError::FSError(FSError::InvalidPath { message, .. }) => {
                assert_eq!(message, "Path provided is not a directory");
            }
            _ => panic!("Expected FSError::InvalidPath"),
        }
    }

    #[tokio::test]
    async fn test_note_vault_new_with_valid_directory() {
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path();

        let result = NoteVault::new(dir_path).await;

        assert!(result.is_ok());
        let vault = result.unwrap();
        assert_eq!(vault.workspace_path, dir_path);
        assert_eq!(vault.journal_path, VaultPath::new(DEFAULT_JOURNAL_PATH));
    }

    #[tokio::test]
    async fn test_get_todays_journal() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(temp_dir.path()).await.unwrap();

        let (title, note_path) = vault.get_todays_journal();

        // Check that title matches today's date format
        let today = Utc::now();
        let expected_title = today.format("%Y-%m-%d").to_string();
        assert_eq!(title, expected_title);

        // Check that the path is correct
        let expected_path = vault
            .journal_path
            .append(&VaultPath::note_path_from(&expected_title))
            .absolute();
        assert_eq!(note_path, expected_path);
    }

    #[tokio::test]
    async fn test_journal_date_with_valid_journal_note() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(temp_dir.path()).await.unwrap();

        // Create a journal note path
        let journal_note_path = vault
            .journal_path
            .append(&VaultPath::note_path_from("2023-12-25"))
            .absolute();

        let result = vault.journal_date(&journal_note_path);

        assert!(result.is_some());
        let date = result.unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2023, 12, 25).unwrap());
    }

    #[tokio::test]
    async fn test_journal_date_with_invalid_date_format() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(temp_dir.path()).await.unwrap();

        // Create a note path with invalid date format
        let invalid_journal_path = vault
            .journal_path
            .append(&VaultPath::note_path_from("invalid-date"))
            .absolute();

        let result = vault.journal_date(&invalid_journal_path);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_journal_date_with_non_journal_path() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(temp_dir.path()).await.unwrap();

        // Create a note path outside of journal directory
        let non_journal_path = VaultPath::new("/other/2023-12-25.md");

        let result = vault.journal_date(&non_journal_path);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_journal_date_with_non_note_path() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(temp_dir.path()).await.unwrap();

        // Create a directory path (not a note)
        let directory_path = vault.journal_path.append(&VaultPath::new("2023-12-25"));

        let result = vault.journal_date(&directory_path);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_path_to_pathbuf() {
        let temp_dir = TempDir::new().unwrap();
        let vault = NoteVault::new(temp_dir.path()).await.unwrap();

        let vault_path = VaultPath::new("/test/note.md");
        let result = vault.path_to_pathbuf(&vault_path);

        let expected = vault_path.to_pathbuf(&vault.workspace_path);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_directory_details() {
        let path = VaultPath::new("/test/directory");
        let details = DirectoryDetails { path: path.clone() };

        assert_eq!(details.path, path);
    }

    #[test]
    fn test_search_result_note() {
        let path = VaultPath::new("/test/note.md");
        let content_data = NoteContentData::new("Test Note".to_string(), 12345);
        let result = SearchResult::note(&path, &content_data);

        assert_eq!(result.path, path);
        match result.rtype {
            ResultType::Note(data) => assert_eq!(data, content_data),
            _ => panic!("Expected Note result type"),
        }
    }

    #[test]
    fn test_search_result_directory() {
        let path = VaultPath::new("/test/directory");
        let result = SearchResult::directory(&path);

        assert_eq!(result.path, path);
        match result.rtype {
            ResultType::Directory => (),
            _ => panic!("Expected Directory result type"),
        }
    }

    #[test]
    fn test_search_result_attachment() {
        let path = VaultPath::new("/test/image.png");
        let result = SearchResult::attachment(&path);

        assert_eq!(result.path, path);
        match result.rtype {
            ResultType::Attachment => (),
            _ => panic!("Expected Attachment result type"),
        }
    }

    #[test]
    fn test_result_type_equality() {
        let content_data = NoteContentData::new("Test Note".to_string(), 12345);
        let note_type1 = ResultType::Note(content_data.clone());
        let note_type2 = ResultType::Note(content_data);
        let directory_type = ResultType::Directory;
        let attachment_type = ResultType::Attachment;

        assert_eq!(note_type1, note_type2);
        assert_eq!(directory_type, ResultType::Directory);
        assert_eq!(attachment_type, ResultType::Attachment);
        assert_ne!(directory_type, attachment_type);
    }

    #[test]
    fn test_vault_browse_options_builder_default() {
        let builder = VaultBrowseOptionsBuilder::default();

        // We can't directly inspect private fields, but we can test the build result
        let (options, _receiver) = builder.build();

        assert_eq!(options.path, VaultPath::root());
        assert_eq!(options.validation, NotesValidation::None);
        assert!(!options.recursive);
    }

    #[test]
    fn test_vault_browse_options_builder_new() {
        let test_path = VaultPath::new("/test/path");
        let builder = VaultBrowseOptionsBuilder::new(&test_path);

        let (options, _receiver) = builder.build();

        assert_eq!(options.path, test_path);
        assert_eq!(options.validation, NotesValidation::None);
        assert!(!options.recursive);
    }

    #[test]
    fn test_vault_browse_options_builder_path() {
        let initial_path = VaultPath::new("/initial");
        let new_path = VaultPath::new("/new/path");

        let builder = VaultBrowseOptionsBuilder::new(&initial_path).path(new_path.clone());

        let (options, _receiver) = builder.build();

        assert_eq!(options.path, new_path);
    }

    #[test]
    fn test_vault_browse_options_builder_recursive() {
        let path = VaultPath::new("/test");

        let builder = VaultBrowseOptionsBuilder::new(&path).recursive();
        let (options, _receiver) = builder.build();
        assert!(options.recursive);

        let builder = VaultBrowseOptionsBuilder::new(&path).non_recursive();
        let (options, _receiver) = builder.build();
        assert!(!options.recursive);
    }

    #[test]
    fn test_vault_browse_options_builder_validation_modes() {
        let path = VaultPath::new("/test");

        // Test full validation
        let builder = VaultBrowseOptionsBuilder::new(&path).full_validation();
        let (options, _receiver) = builder.build();
        assert_eq!(options.validation, NotesValidation::Full);

        // Test fast validation
        let builder = VaultBrowseOptionsBuilder::new(&path).fast_validation();
        let (options, _receiver) = builder.build();
        assert_eq!(options.validation, NotesValidation::Fast);

        // Test no validation
        let builder = VaultBrowseOptionsBuilder::new(&path).no_validation();
        let (options, _receiver) = builder.build();
        assert_eq!(options.validation, NotesValidation::None);
    }

    #[test]
    fn test_vault_browse_options_builder_chaining() {
        let path = VaultPath::new("/test");
        let new_path = VaultPath::new("/new");

        let builder = VaultBrowseOptionsBuilder::new(&path)
            .path(new_path.clone())
            .recursive()
            .full_validation();

        let (options, _receiver) = builder.build();

        assert_eq!(options.path, new_path);
        assert!(options.recursive);
        assert_eq!(options.validation, NotesValidation::Full);
    }

    #[test]
    fn test_vault_browse_options_build_returns_channel() {
        let path = VaultPath::new("/test");
        let builder = VaultBrowseOptionsBuilder::new(&path);

        let (_options, receiver) = builder.build();

        // Test that the receiver is valid by checking if it's ready to receive
        // (it should be empty initially)
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_notes_validation_display() {
        assert_eq!(format!("{}", NotesValidation::Full), "Full");
        assert_eq!(format!("{}", NotesValidation::Fast), "Fast");
        assert_eq!(format!("{}", NotesValidation::None), "None");
    }

    #[test]
    fn test_vault_browse_options_display() {
        let path = VaultPath::new("/test/path");
        let builder = VaultBrowseOptionsBuilder::new(&path)
            .recursive()
            .full_validation();

        let (options, _receiver) = builder.build();
        let display_string = format!("{}", options);

        assert!(display_string.contains("Path: `/test/path`"));
        assert!(display_string.contains("Validation Type: `Full`"));
        assert!(display_string.contains("Recursive: `true`"));
    }

    #[test]
    fn test_default_journal_path_constant() {
        assert_eq!(DEFAULT_JOURNAL_PATH, "/journal");
    }
}
