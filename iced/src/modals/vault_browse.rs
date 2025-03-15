use iced::{
    Task,
    keyboard::{Key, key::Named},
};
use kimun_core::{NoteVault, ResultType, VaultBrowseOptionsBuilder, nfs::VaultPath};
use log::debug;
use rayon::slice::ParallelSliceMut;

use crate::{
    KimunMessage,
    components::{
        KimunComponent, VaultRow, VaultRowType,
        filtered_list::{
            FilteredList, FilteredListFunctions, RowSelection, SortMode, VaultListMessage,
            state_data::StateData,
        },
    },
    editor::EditorMessage,
};

use super::KimunModal;

pub struct VaultBrowse {
    filtered_list: FilteredList<VaultBrowseFunctions>,
}

impl VaultBrowse {
    pub fn new(path: VaultPath, vault: NoteVault) -> (Self, iced::Task<KimunMessage>) {
        let (filtered_list, task) = FilteredList::new(VaultBrowseFunctions::new(path, vault));
        (Self { filtered_list }, task)
    }
}

impl KimunModal for VaultBrowse {
    fn view(&self) -> iced::Element<KimunMessage> {
        self.filtered_list.view()
    }

    fn update(&mut self, message: KimunMessage) -> anyhow::Result<iced::Task<KimunMessage>> {
        if let Ok(msg) = message.try_into() {
            self.filtered_list.update(msg)
        } else {
            Ok(Task::none())
        }
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        match (key, modifiers) {
            (Key::Named(Named::Escape), _) => Task::done(KimunMessage::CloseModal),
            (Key::Named(Named::ArrowDown), _) => {
                Task::done(VaultListMessage::Select(RowSelection::Next).into())
            }
            (Key::Named(Named::ArrowUp), _) => {
                Task::done(VaultListMessage::Select(RowSelection::Previous).into())
            }
            (Key::Named(Named::Enter), _) => Task::done(VaultListMessage::Enter.into()),
            _ => Task::none(),
        }
    }
}

#[derive(Debug, Clone)]
struct VaultBrowseFunctions {
    path: VaultPath,
    vault: NoteVault,
    initial_rows: Vec<VaultRow>,
    sort_mode: SortMode,
}

impl VaultBrowseFunctions {
    fn new(path: VaultPath, vault: NoteVault) -> Self {
        Self {
            path,
            vault,
            initial_rows: vec![],
            sort_mode: SortMode::FileDown,
        }
    }
}

impl FilteredListFunctions for VaultBrowseFunctions {
    fn init(&mut self) {
        let search_path = if self.path.is_note() {
            self.path.get_parent_path().0
        } else {
            self.path.to_owned()
        };
        debug!("Search path is {}", search_path);
        let (browse_options, receiver) = VaultBrowseOptionsBuilder::new(&search_path).build();

        debug!("Retreiving notes for dialog");
        self.vault
            .browse_vault(browse_options)
            .expect("Error getting notes");

        let mut results = vec![];
        while let Ok(entry) = receiver.recv() {
            match &entry.rtype {
                ResultType::Note(_content_data) => results.push(entry.into()),
                ResultType::Directory => {
                    if entry.path != self.path {
                        results.push(entry.into());
                    }
                }
                ResultType::Attachment => {}
            }
        }
        debug!("Retrieved {} elements", results.len());
        self.initial_rows = results;
    }

    fn filter<S: AsRef<str>>(&self, filter_text: S) -> Vec<VaultRow> {
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let mut filtered = nucleo::pattern::Pattern::parse(
            filter_text.as_ref(),
            nucleo::pattern::CaseMatching::Ignore,
            nucleo::pattern::Normalization::Smart,
        )
        .match_list(&self.initial_rows, &mut matcher)
        .iter()
        .map(|e| e.0.to_owned())
        .collect::<Vec<VaultRow>>();
        if self.path != VaultPath::root() {
            filtered.push(VaultRow::up_dir(&self.path));
        }
        filtered.par_sort_by(|a, b| match self.sort_mode {
            SortMode::FileUp => a.path_str.cmp(&b.path_str),
            SortMode::FileDown => b.path_str.cmp(&a.path_str),
            SortMode::TitleUp => a.search_str.cmp(&b.search_str),
            SortMode::TitleDown => b.search_str.cmp(&a.search_str),
        });
        // filtered.par_sort_by(|a, b| a.get_sort_string().cmp(&b.get_sort_string()));

        debug!("filtered {} values", filtered.len());
        filtered
    }

    fn on_entry(&mut self, element: &VaultRow) -> Option<KimunMessage> {
        match element.entry_type {
            VaultRowType::Note { title: _ } => Some(KimunMessage::EditorMessage(
                EditorMessage::OpenNote(element.path.clone()),
            )),
            VaultRowType::Directory => {
                let directory = element.path.clone();
                debug!("new path: {}", directory);
                // let new_one = Self::new(self.path.clone(), self.vault.clone());
                self.initial_rows = vec![];
                self.path = directory;

                // self.path = directory;
                Some(KimunMessage::ListViewMessage(
                    VaultListMessage::Initializing,
                ))
            }
            VaultRowType::Attachment => None,
            VaultRowType::NewNote => Some(KimunMessage::EditorMessage(EditorMessage::NewNote(
                element.path.clone(),
            ))),
        }
    }

    fn header_element(&self, state_data: &StateData) -> Option<VaultRow> {
        if !state_data.filter_text.is_empty() {
            Some(VaultRow::create_new_note(
                &self.path,
                &state_data.filter_text,
            ))
        } else {
            None
        }
    }

    fn button_icon(&self) -> Option<String> {
        None
    }
}
