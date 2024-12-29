mod content_data;
mod db;
pub mod error;
pub mod nfs;
pub mod utilities;

use std::{
    fmt::Display,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    thread::sleep,
    time::Duration,
};

use content_data::NoteContentData;
use db::VaultDB;
// use db::async_sqlite::AsyncConnection;
// use db::async_db::AsyncConnection;
use error::{DBError, VaultError};
use log::{debug, info};
use nfs::{
    load_note, save_note, visitor::NoteListVisitorBuilder, EntryData, NoteEntryData, NotePath,
    VaultEntry,
};
use utilities::path_to_string;

#[derive(Debug, Clone, PartialEq)]
pub struct NoteVault {
    workspace_path: PathBuf,
    vault_db: VaultDB,
}

impl NoteVault {
    pub fn new<P: AsRef<Path>>(workspace_path: P) -> Result<Self, VaultError> {
        let workspace_path = workspace_path.as_ref();
        let workspace = workspace_path.to_path_buf();

        let path = workspace.clone();
        if !path.exists() {
            return Err(VaultError::PathNotFound {
                path: path_to_string(path),
            })?;
        }
        if !path.is_dir() {
            return Err(VaultError::PathIsNotDirectory {
                path: path_to_string(path),
            })?;
        };
        let vault_db = VaultDB::new(workspace_path);
        Ok(Self {
            workspace_path: workspace,
            vault_db,
        })
    }
    pub fn init(&self) -> Result<(), VaultError> {
        self.create_tables()?;
        self.create_index()?;
        Ok(())
    }

    fn create_tables(&self) -> Result<(), VaultError> {
        self.vault_db.call(db::init_db)?;
        Ok(())
    }

    fn create_index(&self) -> Result<(), VaultError> {
        info!("Start indexing files");
        let start = std::time::SystemTime::now();
        let workspace_path = self.workspace_path.clone();
        self.vault_db
            .call(move |conn| create_index_for(&workspace_path, conn, &NotePath::root()))?;

        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        info!(
            "Files indexed in the DB in {} milliseconds",
            time.as_millis()
        );
        Ok(())
    }

    pub fn load_note(&self, path: &NotePath) -> Result<String, VaultError> {
        let text = load_note(&self.workspace_path, path)?;
        Ok(text)
    }

    // Search notes using terms
    pub fn search_notes<S: AsRef<str>>(
        &self,
        terms: S,
        wildcard: bool,
    ) -> Result<Vec<NoteDetails>, VaultError> {
        // let mut connection = ConnectionBuilder::new(&self.workspace_path)
        //     .build()
        //     .unwrap();
        let terms = terms.as_ref().to_owned();

        let a = self.vault_db.call(move |conn| {
            db::search_terms(conn, terms, wildcard).map(|vec| {
                vec.into_iter()
                    .map(|(_data, details)| details)
                    .collect::<Vec<NoteDetails>>()
            })
        })?;

        Ok(a)
    }

    pub fn get_notes_channel<P: Into<NotePath>>(
        &self,
        path: P,
        options: NotesGetterOptions,
    ) -> Result<(), VaultError> {
        let start = std::time::SystemTime::now();
        debug!("> Start fetching files with Options:\n{}", options);
        let workspace_path = self.workspace_path.clone();
        let note_path = path.into();

        // TODO: See if we can put everything inside the closure
        let query_path = note_path.clone();
        let (cached_notes, cached_directories) = self.vault_db.call(move |conn| {
            let notes = db::get_notes(conn, &query_path, options.recursive)?;
            let dirs = db::get_directories(conn, &workspace_path, &query_path)?;
            Ok((notes, dirs))
        })?;

        let mut builder = NoteListVisitorBuilder::new(
            &self.workspace_path,
            options.validation,
            cached_notes,
            cached_directories,
            Some(options.sender.clone()),
        );
        // We traverse the directory
        let walker =
            nfs::get_file_walker(self.workspace_path.clone(), &note_path, options.recursive);
        walker.visit(&mut builder);

        let notes_to_add = builder.get_notes_to_add();
        let notes_to_delete = builder.get_notes_to_delete();
        let notes_to_modify = builder.get_notes_to_modify();

        let workspace_path = self.workspace_path.clone();
        self.vault_db.call(move |conn| {
            let tx = conn.transaction()?;
            db::insert_notes(&tx, &workspace_path, &notes_to_add)?;
            db::delete_notes(&tx, &notes_to_delete)?;
            db::update_notes(&tx, &workspace_path, &notes_to_modify)?;
            tx.commit()?;
            Ok(())
        })?;

        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!("> Files fetched in {} milliseconds", time.as_millis());

        Ok(())
    }

    pub fn get_notes<P: Into<NotePath>>(
        &self,
        path: P,
        recursive: bool,
    ) -> Result<Vec<SearchResult>, VaultError> {
        let start = std::time::SystemTime::now();
        debug!("> Start fetching files from cache");
        let workspace_path = self.workspace_path.clone();
        let note_path = path.into();

        let (cached_notes, cached_directories) = self.vault_db.call(move |conn| {
            let notes = db::get_notes(conn, &note_path, recursive)?;
            let dirs = db::get_directories(conn, &workspace_path, &note_path)?;
            Ok((notes, dirs))
        })?;

        let result = collect_from_cache(&cached_notes, &cached_directories);
        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!("> Files fetched in {} milliseconds", time.as_millis());
        result
    }

    pub fn save_note<S: AsRef<str>>(&self, path: &NotePath, text: S) -> Result<(), VaultError> {
        // Save to disk
        let entry_data = save_note(&self.workspace_path, path, &text)?;

        let details = entry_data.load_details(&self.workspace_path, path)?;

        // Save to DB
        let text = text.as_ref().to_owned();
        self.vault_db
            .call(move |conn| db::save_note(conn, text, &entry_data, &details))?;

        Ok(())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoteDetails {
    pub path: NotePath,
    pub data: NoteContentData,
    // Content may be lazy fetched
    // if the details are taken from the DB, the content is
    // likely not going to be there, so the `get_content` function
    // will take it from disk, and store in the cache
    cached_text: Option<String>,
}

impl NoteDetails {
    pub fn new(note_path: NotePath, hash: u32, title: String, text: Option<String>) -> Self {
        let data = NoteContentData {
            hash,
            title: Some(title),
            content_chunks: vec![],
        };
        Self {
            path: note_path,
            data,
            cached_text: text,
        }
    }

    fn from_content<S: AsRef<str>>(text: S, note_path: &NotePath) -> Self {
        let data = content_data::extract_data(&text);
        Self {
            path: note_path.to_owned(),
            data,
            cached_text: Some(text.as_ref().to_owned()),
        }
    }

    fn from_path<P: AsRef<Path>>(base_path: P, note_path: &NotePath) -> Result<Self, VaultError> {
        let content = load_note(&base_path, note_path)?;
        Ok(Self::from_content(content, note_path))
    }

    pub fn get_text<P: AsRef<Path>>(&mut self, base_path: P) -> Result<String, VaultError> {
        let content = self.cached_text.clone();
        // Content may be lazy loaded from disk since it's
        // the only data that is not stored in the DB
        if let Some(content) = content {
            Ok(content)
        } else {
            let content = load_note(base_path, &self.path)?;
            self.cached_text = Some(content.clone());
            Ok(content)
        }
    }

    pub fn get_title(&self) -> String {
        self.data
            .title
            .clone()
            .unwrap_or_else(|| self.path.get_parent_path().1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirectoryDetails {
    pub base_path: PathBuf,
    pub path: NotePath,
}

#[derive(Debug, Clone)]
pub enum SearchResult {
    Note(NoteDetails),
    Directory(DirectoryDetails),
    Attachment(NotePath),
}

fn collect_from_cache(
    cached_notes: &[(nfs::NoteEntryData, NoteDetails)],
    cached_directories: &[(nfs::DirectoryEntryData, DirectoryDetails)],
) -> Result<Vec<SearchResult>, VaultError> {
    let notes = cached_notes
        .iter()
        .map(|(_note_data, note_details)| SearchResult::Note(note_details.clone()));
    let result = cached_directories
        .iter()
        .map(|(_directory_data, directory_details)| {
            SearchResult::Directory(directory_details.clone())
        })
        .chain(notes);
    Ok(result.collect())
}

#[derive(Debug)]
/// Options to traverse the Notes
/// You need a sync::mpsc::Sender to use a channel to receive the entries
pub struct NotesGetterOptions {
    validation: NotesValidation,
    recursive: bool,
    sender: Sender<SearchResult>,
}

impl Display for NotesGetterOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Notes Getter Options - [Validation Type: {}|Recursive: {}]",
            self.validation, self.recursive
        )
    }
}

impl NotesGetterOptions {
    pub fn new(sender: Sender<SearchResult>) -> Self {
        Self {
            validation: NotesValidation::None,
            recursive: false,
            sender,
        }
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

#[derive(Debug, Clone)]
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
    path: &NotePath,
) -> Result<(), DBError> {
    debug!("Start fetching files at {}", path);
    let workspace_path = workspace_path.as_ref();
    let walker = nfs::get_file_walker(workspace_path, path, false);

    let cached_notes = db::get_notes(connection, path, false)?;
    let cached_directories = db::get_directories(connection, workspace_path, path)?;
    let mut builder = NoteListVisitorBuilder::new(
        workspace_path,
        NotesValidation::Full,
        cached_notes,
        cached_directories,
        None,
    );
    walker.visit(&mut builder);
    let notes_to_add = builder.get_notes_to_add();
    let notes_to_delete = builder.get_notes_to_delete();
    let notes_to_modify = builder.get_notes_to_modify();
    let directories_to_delete = builder.get_directories_to_delete();

    let tx = connection.transaction()?;
    db::delete_notes(&tx, &notes_to_delete)?;
    db::insert_notes(&tx, workspace_path, &notes_to_add)?;
    db::update_notes(&tx, workspace_path, &notes_to_modify)?;
    db::delete_directories(&tx, &directories_to_delete)?;
    tx.commit()?;

    let directories_to_insert = builder.get_directories_to_add();
    for directory in directories_to_insert.iter().filter(|p| !p.eq(&path)) {
        create_index_for(workspace_path, connection, directory)?;
    }

    info!("Initialized");

    Ok(())
}
