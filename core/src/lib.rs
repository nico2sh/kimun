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

#[derive(Debug, Clone, PartialEq)]
pub struct NoteVault {
    pub workspace_path: PathBuf,
    journal_path: VaultPath,
    vault_db: VaultDB,
}

impl Default for NoteVault {
    fn default() -> Self {
        let workspace_path = PathBuf::default();
        let vault_db = VaultDB::new(workspace_path.clone());
        Self {
            workspace_path,
            journal_path: VaultPath::new(DEFAULT_JOURNAL_PATH),
            vault_db,
        }
    }
}

impl NoteVault {
    /// Creates a new instance of the Note Vault.
    /// Make sure you call `NoteVault::init_and_validate(&self)` to initialize the DB index if
    /// needed
    pub fn new<P: AsRef<Path>>(workspace_path: P) -> Result<Self, VaultError> {
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

        let vault_db = VaultDB::new(&workspace_path);
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
    pub fn init_and_validate(&self) -> Result<IndexReport, VaultError> {
        debug!("Initializing DB and validating it");
        let db_result = self.vault_db.check_db();
        match db_result {
            Ok(check_res) => {
                match check_res {
                    db::DBStatus::Ready => {
                        // We only check if there are new notes
                        self.index_notes(NotesValidation::None)
                    }
                    db::DBStatus::Outdated => self.recreate_index(),
                    db::DBStatus::NotValid => self.force_rebuild(),
                    db::DBStatus::FileNotFound => {
                        // No need to validate, no data there
                        self.recreate_index()
                    }
                }
            }
            Err(e) => {
                debug!("Error validating the DB, rebuilding it: {}", e);
                self.force_rebuild()
            }
        }
    }

    /// Deletes the db file and recreates the index
    pub fn force_rebuild(&self) -> Result<IndexReport, VaultError> {
        let db_path = self.vault_db.get_db_path();
        let md = std::fs::metadata(&db_path).map_err(FSError::ReadFileError)?;
        // We delete the db file
        if md.is_dir() {
            std::fs::remove_dir_all(db_path).map_err(FSError::ReadFileError)?;
        } else {
            std::fs::remove_file(db_path).map_err(FSError::ReadFileError)?;
        }
        self.recreate_index()
    }

    /// Deletes all the cached data from the DB by destroying the tables
    /// and recreates the index
    /// This is similar to a force rebuild but instead of deleting the db file
    /// it only deletes the tables.
    pub fn recreate_index(&self) -> Result<IndexReport, VaultError> {
        let index_report = IndexReport::new();
        debug!("Initializing DB from Vault request");
        self.vault_db.call(db::init_db)?;
        debug!("Tables created, creating index");
        self.int_index_notes(index_report, NotesValidation::Full)
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
    pub fn index_notes(&self, validation_mode: NotesValidation) -> Result<IndexReport, VaultError> {
        let index_report = IndexReport::new();
        self.int_index_notes(index_report, validation_mode)
    }

    fn int_index_notes(
        &self,
        mut index_report: IndexReport,
        validation_mode: NotesValidation,
    ) -> Result<IndexReport, VaultError> {
        let workspace_path = self.workspace_path.clone();
        self.vault_db.call(move |conn| {
            create_index_for(&workspace_path, conn, &VaultPath::root(), validation_mode)
        })?;
        index_report.finish();
        debug!("TIME: {}", index_report.duration.as_secs());
        Ok(index_report)
    }

    pub fn exists(&self, path: &VaultPath) -> Option<VaultEntry> {
        VaultEntry::new(&self.workspace_path, path.to_owned()).ok()
    }

    pub fn journal_entry(&self) -> Result<(NoteDetails, String), VaultError> {
        let (title, note_path) = self.get_todays_journal();
        let content = self.load_or_create_note(&note_path, Some(format!("# {}\n\n", title)))?;
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
    pub fn load_or_create_note(
        &self,
        path: &VaultPath,
        default_text: Option<String>,
    ) -> Result<String, VaultError> {
        match nfs::load_note(&self.workspace_path, path) {
            Ok(text) => Ok(text),
            Err(e) => {
                if let FSError::VaultPathNotFound { path: _ } = e {
                    let text = default_text.unwrap_or_default();
                    self.create_note(path, &text)?;
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
    pub fn get_note_text(&self, path: &VaultPath) -> Result<String, VaultError> {
        let text = nfs::load_note(&self.workspace_path, path)?;
        Ok(text)
    }

    // Loads a note, returning its details that contain path, raw text and more
    // If the file doesn't exist you will get a VaultError::FSError with a
    // FSError::NotePathNotFound as the source, you can use that to
    // lazy create a note, or use the load_or_create_note function instead
    pub fn load_note(&self, path: &VaultPath) -> Result<NoteDetails, VaultError> {
        let text = self.get_note_text(path)?;
        Ok(NoteDetails::new(path, text))
    }

    pub fn get_note_chunks(
        &self,
        path: &VaultPath,
    ) -> Result<HashMap<VaultPath, Vec<ContentChunk>>, VaultError> {
        let path = path.to_owned();
        let a = self
            .vault_db
            .call(move |conn| db::get_notes_sections(conn, &path, false))?;

        Ok(a)
    }

    // Search notes using terms
    pub fn search_notes<S: AsRef<str>>(
        &self,
        terms: S,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, VaultError> {
        // let mut connection = ConnectionBuilder::new(&self.workspace_path)
        //     .build()
        //     .unwrap();
        let terms = terms.as_ref().to_owned();

        let a = self
            .vault_db
            .call(move |conn| db::search_terms(conn, terms))?;

        Ok(a)
    }

    pub fn path_to_pathbuf(&self, path: &VaultPath) -> PathBuf {
        path.to_pathbuf(&self.workspace_path)
    }

    pub fn browse_vault(&self, options: VaultBrowseOptions) -> Result<(), VaultError> {
        let start = std::time::SystemTime::now();
        debug!("> Start fetching files with Options:\n{}", options);

        // TODO: See if we can put everything inside the closure
        let query_path = options.path.clone();
        let cached_notes = self.vault_db.call(move |conn| {
            let notes = db::get_notes(conn, &query_path, options.recursive)?;
            Ok(notes)
        })?;

        let mut builder = NoteListVisitorBuilder::new(
            &self.workspace_path,
            options.validation,
            cached_notes,
            Some(options.sender.clone()),
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

        self.vault_db.call(move |conn| {
            let tx = conn.transaction()?;
            db::insert_notes(&tx, &notes_to_add)?;
            db::delete_notes(&tx, &notes_to_delete)?;
            db::update_notes(&tx, &notes_to_modify)?;
            tx.commit()?;
            Ok(())
        })?;

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

    /// Convenience method to get the directories from the filesystem
    pub fn get_directories(
        &self,
        path: &VaultPath,
        recursive: bool,
    ) -> Result<Vec<DirectoryDetails>, VaultError> {
        let result = vec![];

        Ok(result)
    }

    pub fn create_note<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        if self.exists(path).is_none() {
            self.save_note(path, text)
        } else {
            Err(VaultError::NoteExists { path: path.clone() })
        }
    }

    pub fn create_directory(&self, path: &VaultPath) -> Result<DirectoryEntryData, VaultError> {
        if self.exists(path).is_none() {
            let ded = nfs::create_directory(&self.workspace_path, path)?;
            Ok(ded)
        } else {
            Err(VaultError::DirectoryExists { path: path.clone() })
        }
    }

    pub fn save_note<S: AsRef<str>>(
        &self,
        path: &VaultPath,
        text: S,
    ) -> Result<(NoteEntryData, NoteContentData), VaultError> {
        // Save to disk
        let entry_data = nfs::save_note(&self.workspace_path, path, &text)?;
        // TODO: Check if we actually need to create details twice
        let details = entry_data.load_details(&self.workspace_path, path)?;
        let result = (entry_data.clone(), details.get_content_data());
        let text = text.as_ref().to_owned();

        // Save to DB
        self.vault_db
            .call(move |conn| db::save_note(conn, &entry_data, text))?;

        Ok(result)
    }

    /// If the string is a path, it looks for a specific note, if it's just a note name
    /// it looks for that note in any path in the vault, so it may return many results
    pub fn open_or_search(
        &self,
        path: &VaultPath,
    ) -> Result<Vec<(NoteEntryData, NoteContentData)>, DBError> {
        // We make sure the path is a note path, so we append the extension if doesn't exist
        // let path = VaultPath::note_path_from(&path_or_note);
        debug!("PATH: {}", path);
        let (_parent, name) = path.get_parent_path();

        // If it starts with the root trailing slash, we assume is looking for a path
        // let is_note_name = !path_or_note.as_ref().starts_with(nfs::PATH_SEPARATOR)
        //     && parent.eq(&VaultPath::root());

        if path.is_note_file() {
            debug!("We search by name {}", name);
            // It's a note name, we look for the note name, not the path
            self.vault_db
                .call(|conn| db::search_note_by_name(conn, name))
        } else {
            debug!("We search by path {}", path);
            let path = path.clone();
            self.vault_db
                .call(move |conn| db::search_note_by_path(conn, &path))
        }
    }

    pub fn delete_note(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        if !path.is_note() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: path.to_string(),
                message: "The path is not a note".to_string(),
            }));
        }

        // We delete in DB first
        let path_to_delete = path.clone();
        self.vault_db.call(move |conn| {
            let tx = conn.transaction()?;
            db::delete_notes(&tx, &vec![path_to_delete])?;
            tx.commit()?;
            Ok(())
        })?;

        nfs::delete_note(&self.workspace_path, &path)?;

        Ok(())
    }

    pub fn delete_directory(&self, path: &VaultPath) -> Result<(), VaultError> {
        let path = path.flatten();
        if path.is_note() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: path.to_string(),
                message: "The path is not a directory".to_string(),
            }));
        }

        // We delete in DB first
        let path_to_delete = path.clone();
        self.vault_db.call(move |conn| {
            let tx = conn.transaction()?;
            db::delete_directories(&tx, &vec![path_to_delete])?;
            tx.commit()?;
            Ok(())
        })?;

        nfs::delete_directory(&self.workspace_path, &path)?;

        Ok(())
    }

    pub fn rename_note(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();

        if self.exists(&to).is_some() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: to.to_string(),
                message: "Destination path already exists".to_string(),
            }));
        }
        nfs::rename_note(&self.workspace_path, &from, &to)?;

        self.vault_db.call(move |conn| {
            let tx = conn.transaction()?;
            db::rename_note(&tx, &from, &to)?;
            tx.commit()?;
            Ok(())
        })?;

        Ok(())
    }

    pub fn rename_directory(&self, from: &VaultPath, to: &VaultPath) -> Result<(), VaultError> {
        let from = from.flatten();
        let to = to.flatten();

        if self.exists(&to).is_some() {
            return Err(VaultError::FSError(FSError::InvalidPath {
                path: to.to_string(),
                message: "Destination path already exists".to_string(),
            }));
        }
        nfs::rename_directory(&self.workspace_path, &from, &to)?;

        self.vault_db.call(move |conn| {
            let tx = conn.transaction()?;
            db::rename_directory(&tx, &from, &to)?;
            tx.commit()?;
            Ok(())
        })?;

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

#[derive(Debug, Clone, Copy)]
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

fn create_index_for<P: AsRef<Path>>(
    workspace_path: P,
    connection: &mut rusqlite::Connection,
    path: &VaultPath,
    validation_mode: NotesValidation,
) -> Result<(), DBError> {
    debug!("Start fetching files at {}", path);
    let workspace_path = workspace_path.as_ref();
    let walker = nfs::get_file_walker(workspace_path, path, false);

    let cached_notes = db::get_notes(connection, path, false)?;
    let mut builder =
        NoteListVisitorBuilder::new(workspace_path, validation_mode, cached_notes, None);
    walker.visit(&mut builder);
    let notes_to_add = builder.get_notes_to_add();
    let notes_to_delete = builder.get_notes_to_delete();
    let notes_to_modify = builder.get_notes_to_modify();

    let tx = connection.transaction()?;
    db::delete_notes(&tx, &notes_to_delete)?;
    db::insert_notes(&tx, &notes_to_add)?;
    db::update_notes(&tx, &notes_to_modify)?;
    tx.commit()?;

    let directories_to_insert = builder.get_directories_found();
    for directory in directories_to_insert.iter().filter(|p| !p.eq(&path)) {
        create_index_for(workspace_path, connection, directory, validation_mode)?;
    }

    Ok(())
}
