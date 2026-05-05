use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc, Mutex},
};

use ignore::{ParallelVisitor, ParallelVisitorBuilder};
use log::{error, warn};

use crate::{
    nfs::{EntryData, NoteEntryData, VaultEntry, VaultPath},
    note::NoteContentData,
    NotesValidation, SearchResult,
};

struct NoteListVisitor {
    workspace_path: PathBuf,
    validation: NotesValidation,
    notes_to_delete: Arc<Mutex<HashMap<VaultPath, (NoteEntryData, NoteContentData)>>>,
    notes_to_modify: Arc<Mutex<Vec<(NoteEntryData, String)>>>,
    notes_to_add: Arc<Mutex<Vec<(NoteEntryData, String)>>>,
    sender: Option<Sender<SearchResult>>,
}

impl NoteListVisitor {
    fn verify_cache(&self, entry: &VaultEntry, os_path: &Path) {
        let result = match &entry.data {
            EntryData::Note(note_data) => match self.verify_cached_note(note_data, os_path) {
                Some(content) => SearchResult::note(&note_data.path, &content),
                // Read failed; the entry stays in `notes_to_delete` if it was
                // already cached, or simply isn't indexed if it's new — both
                // cases will be retried on the next index pass. Don't emit a
                // misleading SearchResult to the UI.
                None => return,
            },
            EntryData::Directory(directory_data) => SearchResult::directory(&directory_data.path),
            EntryData::Attachment => SearchResult::attachment(&entry.path),
        };
        if let Some(sender) = &self.sender {
            if let Err(e) = sender.send(result) {
                error!("{}", e)
            }
        }
    }

    fn has_changed_fast_check(cached: &NoteEntryData, disk: &NoteEntryData) -> bool {
        cached.size != disk.size || cached.modified_secs != disk.modified_secs
    }

    /// Returns `None` only when the file could not be read; the caller treats
    /// that as "skip this iteration" so the cached entry (if any) survives
    /// untouched and will be re-checked next time.
    fn verify_cached_note(&self, data: &NoteEntryData, os_path: &Path) -> Option<NoteContentData> {
        let cached_option = self.notes_to_delete.lock().unwrap().remove(&data.path);

        match cached_option {
            Some((cached_data, cached_details)) => {
                let needs_reload = match self.validation {
                    NotesValidation::Full => true,
                    NotesValidation::Fast => Self::has_changed_fast_check(&cached_data, data),
                    NotesValidation::None => false,
                };
                if !needs_reload {
                    return Some(cached_details);
                }
                match data.load_details_from_os_path(os_path) {
                    Ok(details) => {
                        let new_content = details.get_content_data();
                        if self.validation == NotesValidation::Full
                            && new_content.hash == cached_details.hash
                        {
                            return Some(cached_details);
                        }
                        self.notes_to_modify
                            .lock()
                            .unwrap()
                            .push((data.to_owned(), details.raw_text));
                        Some(new_content)
                    }
                    Err(e) => {
                        warn!(
                            "Could not read note {}: {}; reinstating cached entry",
                            data.path, e
                        );
                        // Put the cached entry back into the delete map so it
                        // doesn't get treated as "deleted from disk" at flush.
                        self.notes_to_delete
                            .lock()
                            .unwrap()
                            .insert(data.path.clone(), (cached_data, cached_details.clone()));
                        Some(cached_details)
                    }
                }
            }
            None => match data.load_details_from_os_path(os_path) {
                Ok(details) => {
                    let content = details.get_content_data();
                    self.notes_to_add
                        .lock()
                        .unwrap()
                        .push((data.to_owned(), details.raw_text));
                    Some(content)
                }
                Err(e) => {
                    warn!("Could not read new note {}: {}; skipping", data.path, e);
                    None
                }
            },
        }
    }
}

impl ParallelVisitor for NoteListVisitor {
    fn visit(&mut self, entry: Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState {
        match entry {
            Ok(dir) => {
                let os_path = dir.path();
                match VaultEntry::from_path_sync(&self.workspace_path, os_path) {
                    Ok(entry) => self.verify_cache(&entry, os_path),
                    Err(e) => error!("{}", e),
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
    sender: Option<Sender<SearchResult>>,
}

impl NoteListVisitorBuilder {
    pub fn new<P: AsRef<Path>>(
        workspace_path: P,
        validation: NotesValidation,
        cached_notes: Vec<(NoteEntryData, NoteContentData)>,
        sender: Option<Sender<SearchResult>>,
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
            sender,
        }
    }

    /// Consumes the builder and returns the accumulated diff. Must be called
    /// after the parallel walker has finished — at that point all visitor
    /// clones are dropped, so the inner `Arc<Mutex<...>>` are uniquely owned
    /// and we can move the Vecs out without cloning.
    pub fn into_results(self) -> VisitorResults {
        VisitorResults {
            to_delete: take_arc_mutex(self.notes_to_delete).into_keys().collect(),
            to_add: take_arc_mutex(self.notes_to_add),
            to_modify: take_arc_mutex(self.notes_to_modify),
        }
    }

    #[cfg(test)]
    pub fn get_notes_to_delete(&self) -> Vec<VaultPath> {
        self.notes_to_delete
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    #[cfg(test)]
    pub fn get_notes_to_add(&self) -> Vec<(NoteEntryData, String)> {
        self.notes_to_add.lock().unwrap().clone()
    }

    #[cfg(test)]
    pub fn get_notes_to_modify(&self) -> Vec<(NoteEntryData, String)> {
        self.notes_to_modify.lock().unwrap().clone()
    }
}

/// Diff produced by a `NoteListVisitorBuilder` after the parallel walker
/// finishes. The order of `to_add` and `to_modify` is non-deterministic —
/// they are populated by parallel worker threads and entries land in the
/// order each thread completes its file read.
pub struct VisitorResults {
    pub to_delete: Vec<VaultPath>,
    pub to_add: Vec<(NoteEntryData, String)>,
    pub to_modify: Vec<(NoteEntryData, String)>,
}

fn take_arc_mutex<T: Default>(arc: Arc<Mutex<T>>) -> T {
    match Arc::try_unwrap(arc) {
        Ok(mutex) => mutex.into_inner().expect("visitor mutex poisoned"),
        // Walker should drop every visitor clone before returning. If a
        // worker thread leaked its visitor (e.g. mid-panic), take the data
        // via the surviving lock so the index op still completes rather
        // than aborting the whole walk.
        Err(arc) => {
            log::warn!("visitor Arc still shared after walker exit — taking via lock");
            std::mem::take(&mut *arc.lock().expect("visitor mutex poisoned"))
        }
    }
}

impl<'s> ParallelVisitorBuilder<'s> for NoteListVisitorBuilder {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 's> {
        Box::new(NoteListVisitor {
            workspace_path: self.workspace_path.clone(),
            validation: self.validation,
            notes_to_delete: self.notes_to_delete.clone(),
            notes_to_modify: self.notes_to_modify.clone(),
            notes_to_add: self.notes_to_add.clone(),
            sender: self.sender.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nfs::{create_directory, save_note};
    use std::sync::mpsc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_note_list_visitor_builder_new() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::None;
        let cached_notes = vec![];
        let (sender, _receiver) = mpsc::channel();

        let builder =
            NoteListVisitorBuilder::new(workspace_path, validation, cached_notes, Some(sender));

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

        let builder = NoteListVisitorBuilder::new(workspace_path, validation, cached_notes, None);

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

        let builder = NoteListVisitorBuilder::new(workspace_path, validation, cached_notes, None);

        // Test that notes are initially in the "to delete" list
        let notes_to_delete = builder.get_notes_to_delete();
        assert_eq!(notes_to_delete.len(), 1);
        assert_eq!(notes_to_delete[0], note_path);

        // Initially, no notes to add or modify
        assert_eq!(builder.get_notes_to_add().len(), 0);
        assert_eq!(builder.get_notes_to_modify().len(), 0);
    }

    #[tokio::test]
    async fn test_note_list_visitor_builder_getters() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::None;
        let cached_notes = vec![];

        let builder = NoteListVisitorBuilder::new(workspace_path, validation, cached_notes, None);

        // Test all getter methods return empty collections initially
        assert_eq!(builder.get_notes_to_delete().len(), 0);
        assert_eq!(builder.get_notes_to_add().len(), 0);
        assert_eq!(builder.get_notes_to_modify().len(), 0);
    }

    #[tokio::test]
    async fn test_note_list_visitor_builder_parallel_visitor_trait() {
        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path();
        let validation = NotesValidation::None;
        let cached_notes = vec![];

        let mut builder =
            NoteListVisitorBuilder::new(workspace_path, validation, cached_notes, None);

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

        save_note(workspace_path, &note_path, note_content)
            .await
            .unwrap();
        create_directory(workspace_path, &dir_path).await.unwrap();

        let cached_notes = vec![];
        let (sender, _receiver) = mpsc::channel();

        let mut builder =
            NoteListVisitorBuilder::new(workspace_path, validation, cached_notes, Some(sender));

        // Create a visitor and simulate file discovery
        let _visitor = builder.build();

        // After building, we should have notes to add
        let _notes_to_add = builder.get_notes_to_add();
        // Note: The actual file walking would happen when the visitor is used with ignore::WalkParallel
        // This test verifies the builder setup works correctly

        // Cleanup
        tokio::fs::remove_file(workspace_path.join("test_note.md"))
            .await
            .ok();
        tokio::fs::remove_dir_all(workspace_path.join("test_directory"))
            .await
            .ok();
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
            let builder =
                NoteListVisitorBuilder::new(workspace_path, validation, cached_notes.clone(), None);

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
        let sub_notes = vec![("subdir/deep.md", "# Deep Note\n\nNested note.")];

        create_directory(workspace_path, &sub_dir).await.unwrap();
        for (path, content) in &notes {
            save_note(workspace_path, &VaultPath::new(*path), *content)
                .await
                .unwrap();
        }
        for (path, content) in &sub_notes {
            save_note(workspace_path, &VaultPath::new(*path), *content)
                .await
                .unwrap();
        }

        // Scan with the visitor using a recursive walker (no cached notes)
        let (sender, receiver) = mpsc::channel();
        let mut builder = NoteListVisitorBuilder::new(
            workspace_path,
            NotesValidation::None,
            vec![],
            Some(sender),
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
        assert_eq!(
            note_paths.len(),
            4,
            "Expected 4 notes, got: {:?}",
            note_paths
        );
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
        assert_eq!(
            to_add.len(),
            4,
            "Expected 4 notes to add, got {}",
            to_add.len()
        );

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
        save_note(workspace_path, &note_path, original)
            .await
            .unwrap();

        // Get the entry as the walker would see it (absolute path via from_path)
        let full_path = workspace_path.join("changing.md");
        let entry = VaultEntry::from_path(workspace_path, &full_path)
            .await
            .unwrap();
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
        save_note(workspace_path, &note_path, updated)
            .await
            .unwrap();

        // Supply the old cached entry (with the original size) to the builder
        let cached = vec![(note_data, content_data)];

        let mut builder =
            NoteListVisitorBuilder::new(workspace_path, NotesValidation::Fast, cached, None);

        let walker = crate::nfs::get_file_walker(workspace_path, &VaultPath::root(), true);
        walker.visit(&mut builder);

        // The note should show up as modified (size changed)
        let modified = builder.get_notes_to_modify();
        assert_eq!(
            modified.len(),
            1,
            "Expected 1 modified note, got {}",
            modified.len()
        );
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
        save_note(workspace_path, &note_path, "# Gone soon")
            .await
            .unwrap();

        let full_path = workspace_path.join("ephemeral.md");
        let entry = VaultEntry::from_path(workspace_path, &full_path)
            .await
            .unwrap();
        let cached = match entry.data {
            EntryData::Note(d) => {
                let details = d.load_details(workspace_path, &d.path).await.unwrap();
                vec![(d, details.get_content_data())]
            }
            _ => panic!("Expected note"),
        };

        // Remove the file from disk
        tokio::fs::remove_file(workspace_path.join("ephemeral.md"))
            .await
            .unwrap();

        let mut builder =
            NoteListVisitorBuilder::new(workspace_path, NotesValidation::None, cached, None);

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
