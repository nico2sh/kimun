mod db;
pub mod error;
pub mod nfs;
pub mod utilities;

use std::{
    path::{Path, PathBuf},
    sync::mpsc::Sender,
};

use db::ConnectionBuilder;
use error::NoteInitError;
use log::debug;
use nfs::{
    visitors::{list::NoteListVisitorBuilder, sync::NoteSyncVisitorBuilder},
    NoteEntry, NotePath,
};
use rusqlite::Connection;
use utilities::path_to_string;

#[derive(Clone, PartialEq)]
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
        debug!("Files stored in DB in {} milliseconds", time.as_millis());
        Ok(())
    }

    fn create_index_for(&self, connection: &mut Connection, path: &NotePath) -> anyhow::Result<()> {
        debug!("Start fetching files at {}", path);
        let walker = nfs::get_file_walker(self.workspace_path.clone(), path, false);

        let cached_notes = db::get_notes(connection, &self.workspace_path, path)?;
        let cached_directories = db::get_directories(connection, &self.workspace_path, path)?;
        let mut builder =
            NoteSyncVisitorBuilder::new(&self.workspace_path, cached_notes, cached_directories);
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

        Ok(())
    }

    pub fn load_note<P: Into<NotePath>>(&self, path: P) -> anyhow::Result<String> {
        let os_path = path.into().into_path(&self.workspace_path);
        let file = std::fs::read(&os_path)?;
        let content = String::from_utf8(file)?;
        Ok(content)
    }

    pub fn get_notes_at<P: Into<NotePath>>(
        &self,
        path: P,
        sender: Sender<NoteEntry>,
    ) -> anyhow::Result<()> {
        let start = std::time::SystemTime::now();
        debug!("Start fetching files");
        let workspace_path = self.workspace_path.clone();
        let note_path = path.into();
        let walker = nfs::get_file_walker(self.workspace_path.clone(), &note_path, false);

        let mut connection = ConnectionBuilder::new(workspace_path).build().unwrap();
        let cached_notes = db::get_notes(&mut connection, &self.workspace_path, &note_path)?;
        let cached_directories =
            db::get_directories(&mut connection, &self.workspace_path, &note_path)?;
        let mut builder = NoteListVisitorBuilder::new(
            &self.workspace_path,
            cached_notes,
            cached_directories,
            sender.clone(),
        );
        // We traverse the directory
        walker.visit(&mut builder);
        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!("Files fetched in {} milliseconds", time.as_millis());

        let tx = connection.transaction()?;
        db::delete_notes(&tx, &builder.get_notes_to_delete())?;
        db::insert_notes(&tx, &builder.get_notes_to_add())?;
        db::update_notes(&tx, &builder.get_notes_to_modify())?;
        tx.commit()?;

        connection.close().unwrap();
        let time = std::time::SystemTime::now()
            .duration_since(start)
            .expect("Something's wrong with the time");
        debug!("Files fetched in DB in {} milliseconds", time.as_millis());

        Ok(())
    }
}
