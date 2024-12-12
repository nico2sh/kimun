mod db;
pub mod error;
pub mod nfs;
mod parser;
pub mod utilities;

use std::{
    fmt::Display,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{mpsc::Sender, RwLock},
};

use db::ConnectionBuilder;
use error::VaultError;
use log::{debug, info};
use nfs::{visitors::list::NoteListVisitorBuilder, DirectoryDetails, NoteDetails, NotePath};
use rusqlite::Connection;
use utilities::path_to_string;

#[derive(Debug, Clone, PartialEq)]
pub struct NoteVault {
    workspace_path: PathBuf,
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

        let vault = Self {
            workspace_path: workspace,
        };
        vault.create_tables()?;
        vault.create_index()?;
        Ok(vault)
    }

    fn create_tables(&self) -> Result<(), VaultError> {
        let mut connection = ConnectionBuilder::new(self.workspace_path.clone()).build()?;
        db::init_db(&mut connection)?;
        connection.close().expect("Error closing the DB");
        Ok(())
    }

    fn create_index(&self) -> Result<(), VaultError> {
        info!("Start indexing files");
        let start = std::time::SystemTime::now();
        let workspace_path = self.workspace_path.clone();
        let mut connection = ConnectionBuilder::new(workspace_path).build().unwrap();
        self.create_index_for(&mut connection, &NotePath::root())?;
        connection.close().unwrap();
        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        info!(
            "Files indexed in the DB in {} milliseconds",
            time.as_millis()
        );
        Ok(())
    }

    fn create_index_for(
        &self,
        connection: &mut Connection,
        path: &NotePath,
    ) -> Result<(), VaultError> {
        debug!("Start fetching files at {}", path);
        let walker = nfs::get_file_walker(self.workspace_path.clone(), path, false);

        let cached_notes = db::get_notes(connection, &self.workspace_path, path, false)?;
        let cached_directories = db::get_directories(connection, &self.workspace_path, path)?;
        let mut builder = NoteListVisitorBuilder::new(
            &self.workspace_path,
            NotesValidation::Full,
            cached_notes,
            cached_directories,
            None,
        );
        walker.visit(&mut builder);
        let directories_to_insert = Rc::new(RwLock::new(Vec::new()));
        let notes_to_add = builder.get_notes_to_add();
        let notes_to_delete = builder.get_notes_to_delete();
        let notes_to_modify = builder.get_notes_to_modify();
        let directories_to_delete = builder.get_directories_to_delete();
        let dir_path = path.clone();
        let dti = directories_to_insert.clone();
        db::execute_in_transaction(
            connection,
            Box::new(move |tx| {
                db::insert_directory(tx, &dir_path)?;

                db::delete_notes(tx, &notes_to_delete)?;
                db::insert_notes(tx, &notes_to_add)?;
                db::update_notes(tx, &notes_to_modify)?;
                db::delete_directories(tx, &directories_to_delete)?;
                let mut ins = dti.write().unwrap();
                *ins = builder.get_directories_to_add();
                Ok(())
            }),
        )?;
        // let tx = connection.transaction()?;
        // db::insert_directory(&tx, path)?;
        //
        // db::delete_notes(&tx, &builder.get_notes_to_delete())?;
        // db::insert_notes(&tx, &builder.get_notes_to_add())?;
        // db::update_notes(&tx, &builder.get_notes_to_modify())?;
        // let directories_to_delete = builder.get_directories_to_delete();
        // db::delete_directories(&tx, &directories_to_delete)?;
        // let directories_to_insert = builder.get_directories_to_add();
        // tx.commit()?;

        for directory in directories_to_insert
            .read()
            .unwrap()
            .iter()
            .filter(|p| !p.eq(&path))
        {
            self.create_index_for(connection, directory)?;
        }

        info!("Initialized");

        Ok(())
    }

    pub fn load_note<P: Into<NotePath>>(&self, path: P) -> Result<String, VaultError> {
        let os_path = path.into().into_path(&self.workspace_path);
        let file = std::fs::read(&os_path)?;
        let content = String::from_utf8(file)?;
        Ok(content)
    }

    // Search notes using terms
    pub fn search_notes<S: AsRef<str>>(
        &self,
        terms: S,
        wildcard: bool,
    ) -> Result<Vec<NoteDetails>, VaultError> {
        let mut connection = ConnectionBuilder::new(&self.workspace_path)
            .build()
            .unwrap();

        let a = db::search_terms(&mut connection, &self.workspace_path, terms, wildcard).map(
            |vec| {
                vec.into_iter()
                    .map(|(_data, details)| details)
                    .collect::<Vec<NoteDetails>>()
            },
        )?;
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

        let mut connection = ConnectionBuilder::new(workspace_path).build().unwrap();
        let cached_notes = db::get_notes(
            &mut connection,
            &self.workspace_path,
            &note_path,
            options.recursive,
        )?;
        let cached_directories =
            db::get_directories(&mut connection, &self.workspace_path, &note_path)?;

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
        db::execute_in_transaction(
            &mut connection,
            Box::new(move |tx| {
                db::insert_notes(tx, &notes_to_add)?;
                db::delete_notes(tx, &notes_to_delete)?;
                db::update_notes(tx, &notes_to_modify)?;
                Ok(())
            }),
        )?;
        // let tx = connection.transaction()?;
        // db::delete_notes(&tx, &builder.get_notes_to_delete())?;
        // db::insert_notes(&tx, &builder.get_notes_to_add())?;
        // db::update_notes(&tx, &builder.get_notes_to_modify())?;
        // tx.commit()?;
        connection.close().unwrap();

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

        let mut connection = ConnectionBuilder::new(workspace_path).build().unwrap();
        let cached_notes =
            db::get_notes(&mut connection, &self.workspace_path, &note_path, recursive)?;
        let cached_directories =
            db::get_directories(&mut connection, &self.workspace_path, &note_path)?;

        let result = collect_from_cache(&cached_notes, &cached_directories);
        connection.close().unwrap();
        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!("> Files fetched in {} milliseconds", time.as_millis());
        result
    }
}

#[derive(Debug, Clone)]
pub enum SearchResult {
    Note(NoteDetails),
    Directory(DirectoryDetails),
    Attachment(NotePath),
}

fn collect_from_cache(
    cached_notes: &[(nfs::NoteData, nfs::NoteDetails)],
    cached_directories: &[(nfs::DirectoryData, nfs::DirectoryDetails)],
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
/// You can set an optional sync::mpsc::Sender to use a channel to receive the entries
/// If a Sender is set, then it returns `None`, if there's no Sender, it returns
/// the NoteEntry
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
