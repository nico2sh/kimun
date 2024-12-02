mod db;
pub mod error;
pub mod nfs;
mod parser;
pub mod utilities;

use std::{
    fmt::Display,
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};

use db::ConnectionBuilder;
use error::NoteInitError;
use log::{debug, warn};
use nfs::{visitors::list::NoteListVisitorBuilder, NoteEntry, NotePath};
use rusqlite::Connection;
use utilities::path_to_string;

#[derive(Debug, Clone, PartialEq)]
pub struct NoteVault {
    workspace_path: PathBuf,
}

impl NoteVault {
    pub fn new<P: AsRef<Path>>(workspace_path: P) -> anyhow::Result<Self> {
        let workspace_path = workspace_path.as_ref();
        let workspace = workspace_path.to_path_buf();

        let path = workspace.clone();
        if !path.exists() {
            return Err(NoteInitError::PathNotFound {
                path: path_to_string(path),
            })?;
        }
        if !path.is_dir() {
            return Err(NoteInitError::PathIsNotDirectory {
                path: path_to_string(path),
            })?;
        };

        let note = Self {
            workspace_path: workspace,
        };
        note.create_full_index()?;
        Ok(note)
    }

    pub fn create_full_index(&self) -> anyhow::Result<()> {
        let mut connection = ConnectionBuilder::new(self.workspace_path.clone()).build()?;
        db::init_db(&mut connection)?;
        connection.close().expect("Error closing the DB");
        self.create_index()?;

        Ok(())
    }

    fn create_index(&self) -> anyhow::Result<()> {
        debug!("Start indexing files");
        let start = std::time::SystemTime::now();
        let workspace_path = self.workspace_path.clone();
        let mut connection = ConnectionBuilder::new(workspace_path).build().unwrap();
        self.create_index_for(&mut connection, &NotePath::root())?;
        connection.close().unwrap();
        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!(
            "Files indexed in the DB in {} milliseconds",
            time.as_millis()
        );
        Ok(())
    }

    fn create_index_for(&self, connection: &mut Connection, path: &NotePath) -> anyhow::Result<()> {
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
        let tx = connection.transaction()?;
        db::insert_directory(&tx, path)?;

        db::delete_notes(&tx, &builder.get_notes_to_delete())?;
        db::insert_notes(&tx, &builder.get_notes_to_add())?;
        db::update_notes(&tx, &builder.get_notes_to_modify())?;
        let directories_to_delete = builder.get_directories_to_delete();
        db::delete_directories(&tx, &directories_to_delete)?;
        let directories_to_insert = builder.get_directories_to_add();
        tx.commit()?;

        for directory in directories_to_insert.into_iter().filter(|p| !p.eq(path)) {
            self.create_index_for(connection, &directory)?;
        }

        warn!("Initialized");

        Ok(())
    }

    pub fn load_note<P: Into<NotePath>>(&self, path: P) -> anyhow::Result<String> {
        let os_path = path.into().into_path(&self.workspace_path);
        let file = std::fs::read(&os_path)?;
        let content = String::from_utf8(file)?;
        Ok(content)
    }

    pub fn search_notes<S: AsRef<str>>(
        &self,
        terms: S,
        wildcard: bool,
    ) -> anyhow::Result<Vec<NotePath>> {
        let mut connection = ConnectionBuilder::new(&self.workspace_path)
            .build()
            .unwrap();
        db::search_terms(&mut connection, terms, wildcard)
    }

    pub fn get_notes<P: Into<NotePath>>(
        &self,
        path: P,
        options: NotesGetterOptions,
    ) -> anyhow::Result<Option<Vec<NoteEntry>>> {
        let start = std::time::SystemTime::now();
        debug!("Start fetching files with Options:\n{}", options);
        let workspace_path = self.workspace_path.clone();
        let note_path = path.into();
        let walker =
            nfs::get_file_walker(self.workspace_path.clone(), &note_path, options.recursive);

        let mut connection = ConnectionBuilder::new(workspace_path).build().unwrap();
        let cached_notes = db::get_notes(
            &mut connection,
            &self.workspace_path,
            &note_path,
            options.recursive,
        )?;
        let cached_directories =
            db::get_directories(&mut connection, &self.workspace_path, &note_path)?;

        if matches!(options.validation, NotesValidation::None) {
            let result = collect_from_cache(&cached_notes, &cached_directories, options.sender);
            let time = std::time::SystemTime::now()
                .duration_since(start)
                .expect("Something's wrong with the time");
            debug!("Files fetched in {} milliseconds", time.as_millis());
            return result;
        }

        let mut builder = NoteListVisitorBuilder::new(
            &self.workspace_path,
            options.validation,
            cached_notes,
            cached_directories,
            options.sender.clone(),
        );
        // We traverse the directory
        walker.visit(&mut builder);

        let tx = connection.transaction()?;
        db::delete_notes(&tx, &builder.get_notes_to_delete())?;
        db::insert_notes(&tx, &builder.get_notes_to_add())?;
        db::update_notes(&tx, &builder.get_notes_to_modify())?;
        tx.commit()?;
        connection.close().unwrap();

        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!("Files fetched in {} milliseconds", time.as_millis());

        if options.sender.is_none() {
            Ok(Some(builder.get_entries_found()))
        } else {
            Ok(None)
        }
    }
    fn parse_note_text<P: Into<NotePath>>(&self, path: P) -> anyhow::Result<()> {
        let text = self.load_note(path)?;
        Ok(())
    }
}

fn collect_from_cache(
    cached_notes: &[(nfs::NoteData, nfs::NoteDetails)],
    cached_directories: &[(nfs::DirectoryData, nfs::DirectoryDetails)],
    sender: Option<Sender<NoteEntry>>,
) -> Result<Option<Vec<NoteEntry>>, anyhow::Error> {
    let notes = cached_notes.iter().map(|(note_data, note_details)| {
        let path = note_details.note_path.clone();
        NoteEntry {
            path_string: path.to_string(),
            path,
            data: nfs::EntryData::Note(note_data.to_owned()),
        }
    });
    let directories = cached_directories
        .iter()
        .map(|(directory_data, directory_details)| {
            let path = directory_details.note_path.clone();
            NoteEntry {
                path_string: path.to_string(),
                path,
                data: nfs::EntryData::Directory(directory_data.to_owned()),
            }
        });
    let result = directories.chain(notes);
    if let Some(rx) = sender {
        for entry in result {
            let _ = rx.send(entry);
        }
        Ok(None)
    } else {
        Ok(Some(result.collect()))
    }
}

#[derive(Debug)]
/// Options to traverse the Notes
/// You can set an optional sync::mpsc::Sender to use a channel to receive the entries
/// If a Sender is set, then it returns `None`, if there's no Sender, it returns
/// the NoteEntry
pub struct NotesGetterOptions {
    sender: Option<Sender<NoteEntry>>,
    validation: NotesValidation,
    recursive: bool,
}

impl Display for NotesGetterOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Notes Getter Options - [Using Channel: {}|Validation Type: {}|Recursive: {}]",
            self.sender.is_some(),
            self.validation,
            self.recursive
        )
    }
}

impl NotesGetterOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_sender(mut self, sender: Sender<NoteEntry>) -> Self {
        self.sender = Some(sender);
        self
    }

    pub fn recursive(mut self) -> Self {
        self.recursive = true;
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

impl Default for NotesGetterOptions {
    fn default() -> Self {
        Self {
            sender: None,
            validation: NotesValidation::None,
            recursive: false,
        }
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
