use iced::{
    Task,
    alignment::Horizontal,
    keyboard::{Key, key::Named},
};
use kimun_core::{NoteVault, ResultType, VaultBrowseOptionsBuilder, nfs::VaultPath};
use log::{debug, error};
use rayon::slice::ParallelSliceMut;

use crate::{
    KimunMessage,
    components::{
        KimunComponent,
        filtered_list::{
            FilteredList, FilteredListFunctions, ListViewMsg, SortMode, VaultRow, VaultRowType,
        },
        list::{ListSelector, RowSelection},
    },
    editor::EditorMsg,
};

use super::KimunModal;

pub struct VaultSelector {}

impl ListSelector<VaultRow> for VaultSelector {
    fn on_enter(&mut self, element: VaultRow) -> Task<KimunMessage> {
        match element.entry_type {
            VaultRowType::Note { title: _ } => {
                // We close first the modal, then we open the note
                Task::batch([
                    Task::done(KimunMessage::CloseModal),
                    Task::done(KimunMessage::EditorMessage(EditorMsg::OpenNote(
                        element.path.clone(),
                    ))),
                ])
            }
            VaultRowType::Directory => {
                let directory = element.clone();
                // let new_one = Self::new(self.path.clone(), self.vault.clone());

                Task::done(KimunMessage::ListViewMessage(ListViewMsg::Initializing(
                    Some(directory),
                )))
            }
            VaultRowType::Attachment => Task::none(),
            VaultRowType::NewNote => {
                // We close first the modal, then we open the note
                Task::batch([
                    Task::done(KimunMessage::CloseModal),
                    Task::done(KimunMessage::EditorMessage(EditorMsg::NewNote(
                        element.path.clone(),
                    ))),
                ])
            }
        }
    }
}

pub struct VaultNavigator<F>
where
    F: FilteredListFunctions + 'static,
{
    filtered_list: FilteredList<F, VaultSelector>,
    vault: NoteVault,
    preview_text: String,
}

impl<F> VaultNavigator<F>
where
    F: FilteredListFunctions + 'static,
{
    pub fn new(vault: NoteVault, functions: F) -> (Self, iced::Task<KimunMessage>) {
        let selector = VaultSelector {};
        let (filtered_list, task) = FilteredList::new(functions, selector);
        (
            Self {
                filtered_list,
                vault,
                preview_text: String::new(),
            },
            task,
        )
    }
}

impl<F> KimunModal for VaultNavigator<F>
where
    F: FilteredListFunctions + 'static,
{
    fn view(&self) -> iced::Element<KimunMessage> {
        iced::widget::row![
            self.filtered_list.view(),
            iced::widget::scrollable(
                iced::widget::text(&self.preview_text).align_x(Horizontal::Left)
            )
            .spacing(4)
            .height(iced::Length::Fill)
        ]
        .padding(8)
        .spacing(4)
        .into()
    }

    fn get_width(&self) -> iced::Length {
        800.into()
    }

    fn get_height(&self) -> iced::Length {
        600.into()
    }

    fn update(&mut self, message: KimunMessage) -> iced::Task<KimunMessage> {
        match message {
            KimunMessage::Select(RowSelection::Highlighted(_)) => {
                let vault_row = self.filtered_list.get_selection();
                // We trigger the preview
                let vault = self.vault.clone();
                let mp = vault_row.to_owned();
                Task::perform(
                    async move {
                        match mp {
                            Some(row) => {
                                let path = row.path;
                                if path.is_note() {
                                    match vault.get_note_text(&path) {
                                        Ok(text) => text.replace('\t', "    "),
                                        Err(_e) => "Error Loading Preview".to_string(),
                                    }
                                } else {
                                    String::new()
                                }
                            }
                            None => String::new(),
                        }
                    },
                    |t| ListViewMsg::PreviewUpdated(t).into(),
                )
            }
            KimunMessage::ListViewMessage(ListViewMsg::PreviewUpdated(preview)) => {
                self.preview_text = preview.to_owned();
                Task::none()
            }
            _ => self.filtered_list.update(message),
        }
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        match (key, modifiers) {
            (Key::Named(Named::Escape), _) => Task::done(KimunMessage::CloseModal),
            _ => self.filtered_list.key_press(key, modifiers),
        }
    }

    fn should_close_on_click(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub struct VaultSearchFunctions {
    vault: NoteVault,
}

impl VaultSearchFunctions {
    pub fn new(vault: NoteVault) -> Self {
        Self { vault }
    }
}

impl FilteredListFunctions for VaultSearchFunctions {
    fn init(&mut self, _row: Option<VaultRow>) {}

    fn filter<S: AsRef<str>>(&self, filter_text: S) -> Vec<VaultRow> {
        match self.vault.search_notes(filter_text) {
            Ok(results) => results
                .iter()
                .map(|(data, details)| {
                    let title = details.title.clone();
                    let path = data.path.clone();
                    let file_name = path.get_parent_path().1;
                    let file_name_no_ext =
                        file_name.strip_suffix(".md").unwrap_or(file_name.as_str());
                    let search_str = if title.contains(file_name_no_ext) {
                        title.clone()
                    } else {
                        format!("{} {}", title, file_name_no_ext)
                    };
                    VaultRow {
                        path: path.clone(),
                        path_str: path.get_parent_path().1,
                        search_str,
                        entry_type: VaultRowType::Note { title },
                    }
                })
                .collect(),
            Err(e) => {
                error!("Error retrieving results: {}", e);
                vec![]
            }
        }
    }

    fn header_element(&self, _filter_text: &str) -> Option<VaultRow> {
        None
    }

    fn button_icon(&self) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct VaultBrowseFunctions {
    path: VaultPath,
    vault: NoteVault,
    initial_rows: Vec<VaultRow>,
    sort_mode: SortMode,
}

impl VaultBrowseFunctions {
    pub fn new(path: VaultPath, vault: NoteVault) -> Self {
        Self {
            path,
            vault,
            initial_rows: vec![],
            sort_mode: SortMode::FileDown,
        }
    }
}

impl FilteredListFunctions for VaultBrowseFunctions {
    fn init(&mut self, row: Option<VaultRow>) {
        if let Some(r) = row {
            self.path = r.path;
            self.initial_rows = vec![];
        }
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
        filtered.par_sort_by(|a, b| {
            // We compare first the order of the type priority
            if a.entry_type.get_order() == b.entry_type.get_order() {
                match self.sort_mode {
                    SortMode::FileUp => a.path_str.cmp(&b.path_str),
                    SortMode::FileDown => b.path_str.cmp(&a.path_str),
                    SortMode::TitleUp => a.search_str.cmp(&b.search_str),
                    SortMode::TitleDown => b.search_str.cmp(&a.search_str),
                }
            } else {
                a.entry_type.get_order().cmp(&b.entry_type.get_order())
            }
        });
        if !self.path.get_slices().is_empty() {
            let mut up = vec![VaultRow::up_dir(&self.path)];
            up.append(&mut filtered);
            filtered = up;
        }

        // filtered.par_sort_by(|a, b| a.get_sort_string().cmp(&b.get_sort_string()));

        debug!("filtered {} values", filtered.len());
        filtered
    }

    fn header_element(&self, filter_text: &str) -> Option<VaultRow> {
        if !filter_text.is_empty() {
            Some(VaultRow::create_new_note(&self.path, filter_text))
        } else {
            None
        }
    }

    fn button_icon(&self) -> Option<String> {
        None
    }
}
