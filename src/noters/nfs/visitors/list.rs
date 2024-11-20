use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc, Mutex},
};

use ignore::{ParallelVisitor, ParallelVisitorBuilder};
use log::{debug, error};

use crate::noters::nfs::{
    DirectoryData, DirectoryDetails, EntryData, NoteData, NoteDetails, NoteEntry, NotePath,
};

struct NoteListVisitor {
    workspace_path: PathBuf,
    notes_to_delete: Arc<Mutex<HashMap<NotePath, NoteData>>>,
    directories_to_delete: Arc<Mutex<HashSet<NotePath>>>,
    notes_to_modify: Arc<Mutex<Vec<(NoteData, NoteDetails)>>>,
    notes_to_add: Arc<Mutex<Vec<(NoteData, NoteDetails)>>>,
    directories_to_add: Arc<Mutex<Vec<NotePath>>>,
    sender: Sender<NoteEntry>,
}

impl NoteListVisitor {
    fn verify_cache(&self, entry: &NoteEntry) {
        match &entry.data {
            EntryData::Note(note_data) => self.verify_cached_note(note_data),
            EntryData::Directory(directory_data) => self.verify_cached_directory(directory_data),
            EntryData::Attachment => {}
        }
    }

    fn verify_cached_note(&self, data: &NoteData) {
        let modified_secs = data.modified_secs;
        let size = data.size;

        let mut ntd = self.notes_to_delete.lock().unwrap();
        let cached_option = ntd.remove(&data.path);
        if let Some(cached) = cached_option {
            let modified_sec_cached = cached.modified_secs;
            let size_cached = cached.size;
            // entry exists
            if size != size_cached || modified_secs != modified_sec_cached {
                debug!("Modify note, size: {} != {}", size, size_cached);
                debug!(
                    "Modify note, modified: {} != {}",
                    modified_secs, modified_sec_cached
                );
                let details = data
                    .get_details(&self.workspace_path, &data.path)
                    .expect("Can't get details for note");
                self.notes_to_modify
                    .lock()
                    .unwrap()
                    .push((data.to_owned(), details.to_owned()));
            }
        } else {
            debug!("Add note: {}", data.path);
            let details = data
                .get_details(&self.workspace_path, &data.path)
                .expect("Can't get Details for note");
            self.notes_to_add
                .lock()
                .unwrap()
                .push((data.to_owned(), details.to_owned()));
        }
    }

    fn verify_cached_directory(&self, data: &DirectoryData) {
        let mut dtd = self.directories_to_delete.lock().unwrap();
        if !dtd.remove(&data.path) {
            // debug!("Add dir: {}", data.path);
            self.directories_to_add
                .lock()
                .unwrap()
                .push(data.path.clone());
        }
    }
}

impl ParallelVisitor for NoteListVisitor {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState {
        match entry {
            Ok(dir) => {
                // debug!("Scanning: {}", dir.path().as_os_str().to_string_lossy());
                let npe = NoteEntry::from_path(&self.workspace_path, dir.path());
                match npe {
                    Ok(entry) => {
                        if let Err(e) = self.sender.send(entry.clone()) {
                            error!("{}", e)
                        }
                        // artificially adding latency
                        // std::thread::sleep(Duration::from_millis(200));
                        self.verify_cache(&entry);
                    }
                    Err(e) => {
                        error!("{}", e);
                    }
                }
                ignore::WalkState::Continue
            }
            Err(e) => {
                error!("{}", e);
                ignore::WalkState::Continue
            }
        }
    }
}

pub struct NoteListVisitorBuilder {
    workspace_path: PathBuf,
    notes_to_delete: Arc<Mutex<HashMap<NotePath, NoteData>>>,
    directories_to_delete: Arc<Mutex<HashSet<NotePath>>>,
    notes_to_modify: Arc<Mutex<Vec<(NoteData, NoteDetails)>>>,
    notes_to_add: Arc<Mutex<Vec<(NoteData, NoteDetails)>>>,
    directories_to_add: Arc<Mutex<Vec<NotePath>>>,
    sender: Sender<NoteEntry>,
}

impl NoteListVisitorBuilder {
    pub fn new<P: AsRef<Path>>(
        workspace_path: P,
        cached_notes: Vec<(NoteData, NoteDetails)>,
        cached_directories: Vec<(DirectoryData, DirectoryDetails)>,
        sender: Sender<NoteEntry>,
    ) -> Self {
        let mut notes_to_delete = HashMap::new();
        let mut directories_to_delete = HashSet::new();
        for entry in cached_notes {
            let path = entry.1.note_path.clone();
            notes_to_delete.insert(path, entry.0);
        }
        for entry in cached_directories {
            directories_to_delete.insert(entry.0.path);
        }
        Self {
            workspace_path: workspace_path.as_ref().to_path_buf(),
            notes_to_delete: Arc::new(Mutex::new(notes_to_delete)),
            notes_to_modify: Arc::new(Mutex::new(Vec::new())),
            notes_to_add: Arc::new(Mutex::new(Vec::new())),
            directories_to_delete: Arc::new(Mutex::new(directories_to_delete)),
            directories_to_add: Arc::new(Mutex::new(Vec::new())),
            sender,
        }
    }

    pub fn get_notes_to_delete(&self) -> Vec<NotePath> {
        self.notes_to_delete
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.0.to_owned())
            .collect()
    }

    pub fn get_notes_to_add(&self) -> Vec<(NoteData, NoteDetails)> {
        self.notes_to_add
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.to_owned())
            .collect()
    }

    pub fn get_notes_to_modify(&self) -> Vec<(NoteData, NoteDetails)> {
        self.notes_to_modify
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.to_owned())
            .collect()
    }

    pub fn get_directories_to_add(&self) -> Vec<NotePath> {
        self.directories_to_add
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.to_owned())
            .collect()
    }

    pub fn get_directories_to_delete(&self) -> Vec<NotePath> {
        self.directories_to_delete
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.to_owned())
            .collect()
    }
}

impl<'s> ParallelVisitorBuilder<'s> for NoteListVisitorBuilder {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 's> {
        let dbv = NoteListVisitor {
            workspace_path: self.workspace_path.clone(),
            notes_to_delete: self.notes_to_delete.clone(),
            notes_to_modify: self.notes_to_modify.clone(),
            notes_to_add: self.notes_to_add.clone(),
            directories_to_delete: self.directories_to_delete.clone(),
            directories_to_add: self.directories_to_add.clone(),
            sender: self.sender.clone(),
        };
        Box::new(dbv)
    }
}
