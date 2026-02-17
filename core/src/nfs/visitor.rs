use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc, Mutex},
};

use ignore::{ParallelVisitor, ParallelVisitorBuilder};
use log::error;
use tokio::runtime::Handle;

use crate::{
    nfs::{DirectoryDetails, EntryData, NoteEntryData, VaultEntry, VaultPath},
    note::NoteContentData,
    NotesValidation, SearchResult,
};

struct NoteListVisitor {
    workspace_path: PathBuf,
    validation: NotesValidation,
    notes_to_delete: Arc<Mutex<HashMap<VaultPath, (NoteEntryData, NoteContentData)>>>,
    notes_to_modify: Arc<Mutex<Vec<(NoteEntryData, String)>>>,
    notes_to_add: Arc<Mutex<Vec<(NoteEntryData, String)>>>,
    directories_found: Arc<Mutex<Vec<VaultPath>>>,
    sender: Option<Sender<SearchResult>>,
    handle: Handle,
}

impl NoteListVisitor {
    fn verify_cache(&self, entry: &VaultEntry) {
        let result = match &entry.data {
            EntryData::Note(note_data) => {
                SearchResult::note(&note_data.path, &self.verify_cached_note(note_data))
            }
            EntryData::Directory(directory_data) => {
                let details = DirectoryDetails {
                    path: directory_data.path.clone(),
                };
                self.directories_found
                    .lock()
                    .unwrap()
                    .push(directory_data.path.clone());
                SearchResult::directory(&details.path)
            }
            EntryData::Attachment => SearchResult::attachment(&entry.path),
        };
        if let Some(sender) = &self.sender {
            if let Err(e) = sender.send(result) {
                error!("{}", e)
            }
        }
    }

    // We only check the size and modified
    fn has_changed_fast_check(&self, cached: &NoteEntryData, disk: &NoteEntryData) -> bool {
        let modified_secs = disk.modified_secs;
        let size = disk.size;
        let modified_sec_cached = cached.modified_secs;
        let size_cached = cached.size;
        size != size_cached || modified_secs != modified_sec_cached
    }

    // We check the content hash
    fn has_changed_deep_check(&self, cached: &mut NoteContentData, disk: &NoteEntryData) -> bool {
        let details = self.handle.block_on(disk.load_details(&self.workspace_path, &disk.path)).unwrap();
        let details_hash = details.get_content_data().hash;
        let cached_hash = cached.hash;
        !details_hash.eq(&cached_hash)
    }

    fn verify_cached_note(&self, data: &NoteEntryData) -> NoteContentData {
        let mut ntd = self.notes_to_delete.lock().unwrap();
        let cached_option = ntd.remove(&data.path);

        let content_data = if let Some((cached_data, mut cached_details)) = cached_option {
            // entry exists
            let changed = match self.validation {
                NotesValidation::Full => self.has_changed_deep_check(&mut cached_details, data),
                NotesValidation::Fast => self.has_changed_fast_check(&cached_data, data),
                NotesValidation::None => false,
            };
            if changed {
                let details = self.handle.block_on(data
                    .load_details(&self.workspace_path, &data.path))
                    .expect("Can't get details for note");
                let text = details.raw_text.clone();
                self.notes_to_modify
                    .lock()
                    .unwrap()
                    .push((data.to_owned(), text));
                details.get_content_data()
            } else {
                cached_details
            }
        } else {
            let details = self.handle.block_on(data
                .load_details(&self.workspace_path, &data.path))
                .expect("Can't get Details for note");
            let text = details.raw_text.clone();
            self.notes_to_add
                .lock()
                .unwrap()
                .push((data.to_owned(), text));
            details.get_content_data()
        };
        content_data
    }
}

impl ParallelVisitor for NoteListVisitor {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState {
        match entry {
            Ok(dir) => {
                // debug!("Scanning: {}", dir.path().as_os_str().to_string_lossy());
                let npe = self.handle.block_on(VaultEntry::from_path(&self.workspace_path, dir.path()));
                match npe {
                    Ok(entry) => {
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
    validation: NotesValidation,
    notes_to_delete: Arc<Mutex<HashMap<VaultPath, (NoteEntryData, NoteContentData)>>>,
    notes_to_modify: Arc<Mutex<Vec<(NoteEntryData, String)>>>,
    notes_to_add: Arc<Mutex<Vec<(NoteEntryData, String)>>>,
    directories_found: Arc<Mutex<Vec<VaultPath>>>,
    sender: Option<Sender<SearchResult>>,
    handle: Handle,
}

impl NoteListVisitorBuilder {
    pub fn new<P: AsRef<Path>>(
        workspace_path: P,
        validation: NotesValidation,
        cached_notes: Vec<(NoteEntryData, NoteContentData)>,
        sender: Option<Sender<SearchResult>>,
        handle: Handle,
    ) -> Self {
        let mut notes_to_delete = HashMap::new();
        for cached in cached_notes {
            let path = cached.0.path.clone();
            notes_to_delete.insert(path, cached);
        }
        Self {
            workspace_path: workspace_path.as_ref().to_path_buf(),
            validation,
            notes_to_delete: Arc::new(Mutex::new(notes_to_delete)),
            notes_to_modify: Arc::new(Mutex::new(Vec::new())),
            notes_to_add: Arc::new(Mutex::new(Vec::new())),
            directories_found: Arc::new(Mutex::new(Vec::new())),
            sender,
            handle,
        }
    }

    pub fn get_notes_to_delete(&self) -> Vec<VaultPath> {
        self.notes_to_delete
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.0.to_owned())
            .collect()
    }

    pub fn get_notes_to_add(&self) -> Vec<(NoteEntryData, String)> {
        self.notes_to_add
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.to_owned())
            .collect()
    }

    pub fn get_notes_to_modify(&self) -> Vec<(NoteEntryData, String)> {
        self.notes_to_modify
            .lock()
            .unwrap()
            .iter()
            .map(|n| n.to_owned())
            .collect()
    }

    pub fn get_directories_found(&self) -> Vec<VaultPath> {
        self.directories_found
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
            validation: self.validation,
            notes_to_delete: self.notes_to_delete.clone(),
            notes_to_modify: self.notes_to_modify.clone(),
            notes_to_add: self.notes_to_add.clone(),
            directories_found: self.directories_found.clone(),
            sender: self.sender.clone(),
            handle: self.handle.clone(),
        };
        Box::new(dbv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use tempfile::TempDir;
    use crate::nfs::{save_note, create_directory};

    #[tokio::test]
    async fn test_note_list_visitor_builder_new() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::None;
        let cached_notes = vec![];
        let (sender, _receiver) = mpsc::channel();

        let builder = NoteListVisitorBuilder::new(
            workspace_path,
            validation,
            cached_notes,
            Some(sender),
            Handle::current(),
        );

        assert_eq!(builder.workspace_path, workspace_path);
        assert_eq!(builder.validation, validation);
        assert!(builder.sender.is_some());
    }

    #[tokio::test]
    async fn test_note_list_visitor_builder_without_sender() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::Fast;
        let cached_notes = vec![];

        let builder = NoteListVisitorBuilder::new(
            workspace_path,
            validation,
            cached_notes,
            None,
            Handle::current(),
        );

        assert_eq!(builder.workspace_path, workspace_path);
        assert_eq!(builder.validation, validation);
        assert!(builder.sender.is_none());
    }

    #[tokio::test]
    async fn test_note_list_visitor_builder_with_cached_notes() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::Full;

        // Create some cached notes
        let note_path = VaultPath::new("cached_note.md");
        let note_content = crate::note::NoteContentData::new("Test Note".to_string(), 12345);
        let note_entry = crate::nfs::NoteEntryData {
            path: note_path.clone(),
            size: 100,
            modified_secs: 1234567890,
        };
        let cached_notes = vec![(note_entry, note_content)];

        let builder = NoteListVisitorBuilder::new(
            workspace_path,
            validation,
            cached_notes,
            None,
            Handle::current(),
        );

        // Test that notes are initially in the "to delete" list
        let notes_to_delete = builder.get_notes_to_delete();
        assert_eq!(notes_to_delete.len(), 1);
        assert_eq!(notes_to_delete[0], note_path);

        // Initially, no notes to add or modify
        assert_eq!(builder.get_notes_to_add().len(), 0);
        assert_eq!(builder.get_notes_to_modify().len(), 0);
        assert_eq!(builder.get_directories_found().len(), 0);
    }

    #[tokio::test]
    async fn test_note_list_visitor_builder_getters() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::None;
        let cached_notes = vec![];

        let builder = NoteListVisitorBuilder::new(
            workspace_path,
            validation,
            cached_notes,
            None,
            Handle::current(),
        );

        // Test all getter methods return empty collections initially
        assert_eq!(builder.get_notes_to_delete().len(), 0);
        assert_eq!(builder.get_notes_to_add().len(), 0);
        assert_eq!(builder.get_notes_to_modify().len(), 0);
        assert_eq!(builder.get_directories_found().len(), 0);
    }

    #[tokio::test]
    async fn test_note_list_visitor_builder_parallel_visitor_trait() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::None;
        let cached_notes = vec![];

        let mut builder = NoteListVisitorBuilder::new(
            workspace_path,
            validation,
            cached_notes,
            None,
            Handle::current(),
        );

        // Test that we can build a parallel visitor
        let _visitor = builder.build();
        // If this compiles and runs, the trait is implemented correctly
    }

    #[tokio::test]
    async fn test_visitor_with_real_file_operations() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::None;

        // Create a test note and directory
        let note_path = VaultPath::new("test_note.md");
        let dir_path = VaultPath::new("test_directory");
        let note_content = "# Test Note\n\nThis is a test note.";

        save_note(workspace_path, &note_path, note_content).await.unwrap();
        create_directory(workspace_path, &dir_path).await.unwrap();

        let cached_notes = vec![];
        let (sender, _receiver) = mpsc::channel();

        let mut builder = NoteListVisitorBuilder::new(
            workspace_path,
            validation,
            cached_notes,
            Some(sender),
            Handle::current(),
        );

        // Create a visitor and simulate file discovery
        let _visitor = builder.build();

        // After building, we should have notes to add
        let _notes_to_add = builder.get_notes_to_add();
        // Note: The actual file walking would happen when the visitor is used with ignore::WalkParallel
        // This test verifies the builder setup works correctly

        // Cleanup
        tokio::fs::remove_file(workspace_path.join("test_note.md")).await.ok();
        tokio::fs::remove_dir_all(workspace_path.join("test_directory")).await.ok();
    }

    #[tokio::test]
    async fn test_note_list_visitor_different_validation_modes() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let cached_notes = vec![];

        // Test each validation mode
        let validation_modes = [
            NotesValidation::None,
            NotesValidation::Fast,
            NotesValidation::Full,
        ];

        for validation in validation_modes {
            let builder = NoteListVisitorBuilder::new(
                workspace_path,
                validation,
                cached_notes.clone(),
                None,
                Handle::current(),
            );

            assert_eq!(builder.validation, validation);

            // Ensure builder can create visitor for each validation mode
            let mut builder_mut = builder;
            let _visitor = builder_mut.build();
        }
    }

    #[tokio::test]
    async fn test_channel_communication() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::None;
        let cached_notes = vec![];
        let (sender, receiver) = mpsc::channel();

        let _builder = NoteListVisitorBuilder::new(
            workspace_path,
            validation,
            cached_notes,
            Some(sender.clone()),
            Handle::current(),
        );

        // Test that we can send a search result through the channel
        let test_path = VaultPath::new("test.md");
        let test_result = SearchResult::directory(&test_path);

        sender.send(test_result.clone()).unwrap();
        let received = receiver.recv().unwrap();

        assert_eq!(received.path, test_result.path);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_scan_multiple_markdown_files() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Create several markdown files in the root and a subdirectory
        let notes = vec![
            ("note_a.md", "# Note A\n\nFirst note content."),
            ("note_b.md", "# Note B\n\nSecond note with more text."),
            ("note_c.md", "---\ntitle: Note C\n---\n\nFrontmatter note."),
        ];
        let sub_dir = VaultPath::new("subdir");
        let sub_notes = vec![
            ("subdir/deep.md", "# Deep Note\n\nNested note."),
        ];

        create_directory(workspace_path, &sub_dir).await.unwrap();
        for (path, content) in &notes {
            save_note(workspace_path, &VaultPath::new(*path), *content).await.unwrap();
        }
        for (path, content) in &sub_notes {
            save_note(workspace_path, &VaultPath::new(*path), *content).await.unwrap();
        }

        // Scan with the visitor using a recursive walker (no cached notes)
        let (sender, receiver) = mpsc::channel();
        let mut builder = NoteListVisitorBuilder::new(
            workspace_path,
            NotesValidation::None,
            vec![],
            Some(sender),
            Handle::current(),
        );

        let walker = crate::nfs::get_file_walker(workspace_path, &VaultPath::root(), true);
        walker.visit(&mut builder);

        // Collect all SearchResults from the channel
        let mut results: Vec<SearchResult> = Vec::new();
        while let Ok(r) = receiver.try_recv() {
            results.push(r);
        }

        // Separate notes and directories from the results
        let note_paths: Vec<String> = results
            .iter()
            .filter_map(|r| match &r.rtype {
                crate::ResultType::Note(_) => Some(r.path.to_string()),
                _ => None,
            })
            .collect();

        let dir_paths: Vec<String> = results
            .iter()
            .filter_map(|r| match &r.rtype {
                crate::ResultType::Directory => Some(r.path.to_string()),
                _ => None,
            })
            .collect();

        // All four notes should be discovered
        assert_eq!(note_paths.len(), 4, "Expected 4 notes, got: {:?}", note_paths);
        for expected in &["note_a.md", "note_b.md", "note_c.md", "deep.md"] {
            assert!(
                note_paths.iter().any(|p| p.contains(expected)),
                "Missing note {} in {:?}",
                expected,
                note_paths
            );
        }

        // The subdirectory should be found
        assert!(
            dir_paths.iter().any(|p| p.contains("subdir")),
            "Missing subdir in {:?}",
            dir_paths
        );

        // All notes should be in the "to add" list since there were no cached notes
        let to_add = builder.get_notes_to_add();
        assert_eq!(to_add.len(), 4, "Expected 4 notes to add, got {}", to_add.len());

        // Nothing to delete or modify
        assert!(builder.get_notes_to_delete().is_empty());
        assert!(builder.get_notes_to_modify().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_scan_detects_modified_note() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        let note_path = VaultPath::new("changing.md");
        let original = "# Original\n\nOriginal content.";
        save_note(workspace_path, &note_path, original).await.unwrap();

        // Get the entry as the walker would see it (absolute path via from_path)
        let full_path = workspace_path.join("changing.md");
        let entry = VaultEntry::from_path(workspace_path, &full_path).await.unwrap();
        let (note_data, content_data) = match entry.data {
            EntryData::Note(d) => {
                let details = d.load_details(workspace_path, &d.path).await.unwrap();
                let cd = details.get_content_data();
                (d, cd)
            }
            _ => panic!("Expected note"),
        };

        // Overwrite with different content so the file size changes
        let updated = "# Updated\n\nThis content is deliberately much longer to change the file size on disk.";
        save_note(workspace_path, &note_path, updated).await.unwrap();

        // Supply the old cached entry (with the original size) to the builder
        let cached = vec![(note_data, content_data)];

        let mut builder = NoteListVisitorBuilder::new(
            workspace_path,
            NotesValidation::Fast,
            cached,
            None,
            Handle::current(),
        );

        let walker = crate::nfs::get_file_walker(workspace_path, &VaultPath::root(), true);
        walker.visit(&mut builder);

        // The note should show up as modified (size changed)
        let modified = builder.get_notes_to_modify();
        assert_eq!(modified.len(), 1, "Expected 1 modified note, got {}", modified.len());
        assert!(modified[0].0.path.to_string().contains("changing.md"));

        // Nothing to add (it was already cached) and nothing to delete (still on disk)
        assert!(builder.get_notes_to_add().is_empty());
        assert!(builder.get_notes_to_delete().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_scan_detects_deleted_note() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();

        // Create a note, get its cached data, then delete it from disk
        let note_path = VaultPath::new("ephemeral.md");
        save_note(workspace_path, &note_path, "# Gone soon").await.unwrap();

        let full_path = workspace_path.join("ephemeral.md");
        let entry = VaultEntry::from_path(workspace_path, &full_path).await.unwrap();
        let cached = match entry.data {
            EntryData::Note(d) => {
                let details = d.load_details(workspace_path, &d.path).await.unwrap();
                vec![(d, details.get_content_data())]
            }
            _ => panic!("Expected note"),
        };

        // Remove the file from disk
        tokio::fs::remove_file(workspace_path.join("ephemeral.md")).await.unwrap();

        let mut builder = NoteListVisitorBuilder::new(
            workspace_path,
            NotesValidation::None,
            cached,
            None,
            Handle::current(),
        );

        let walker = crate::nfs::get_file_walker(workspace_path, &VaultPath::root(), true);
        walker.visit(&mut builder);

        // The note should appear in the delete list (cached but not on disk)
        let to_delete = builder.get_notes_to_delete();
        assert_eq!(to_delete.len(), 1);
        assert!(to_delete[0].to_string().contains("ephemeral.md"));

        assert!(builder.get_notes_to_add().is_empty());
        assert!(builder.get_notes_to_modify().is_empty());
    }
}
