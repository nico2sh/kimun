use core_notes::{
    nfs::{NoteEntry, NotePath},
    NoteVault,
};
use iced::{
    widget::{button, column, container, horizontal_space, keyed_column, row, text, Container},
    Element, Font, Length,
};
use iced_aw::{style, SelectionList};

use crate::Message;

pub struct NoteBrowser {
    base_path: NotePath,
    status: Status,
    vault: NoteVault,
}

enum Status {
    Init,
    Loading,
    Ready(Vec<NoteEntry>),
}

impl NoteBrowser {
    pub fn new(base_path: &NotePath, vault: &NoteVault) -> Self {
        Self {
            base_path: base_path.clone(),
            status: Status::Init,
            vault: vault.clone(),
        }
    }
}

enum NoteBrowserAction {}

impl NoteBrowser {
    pub fn get_view(&self) -> iced::Element<crate::Message> {
        match &self.status {
            Status::Init => {
                //
                container(horizontal_space()).into()
            }
            Status::Loading => {
                //
                container(horizontal_space()).into()
            }
            Status::Ready(rows) => {
                let items = rows.iter().enumerate().map(|(i, e)| {
                    let row_element = get_row_element(e);
                    (i, row_element)
                });

                let list = keyed_column(items).padding(5);

                container(column![list].spacing(10))
                    .width(Length::Fill)
                    .padding(10)
                    .into()
            }
        }
    }

    fn update(&mut self, action: NoteBrowserAction) -> iced::Task<crate::Message> {
        todo!()
    }
}

fn get_row_element(e: &NoteEntry) -> Element<crate::Message> {
    let button = button(text(e.to_string().clone()));
    row![button].padding(2).into()
}

// #[derive(Clone)]
// enum NavEntry {
//     Note(NotePath),
//     Directory(NotePath),
// }
//
// impl NavEntry {
//     fn sort_string(&self) -> String {
//         match self {
//             NavEntry::Directory(note_path) => format!("1-{}", note_path),
//             NavEntry::Note(note_path) => format!("2-{}", note_path),
//         }
//     }
// }
//
// #[derive(Clone)]
// struct NotesAndDirs {
//     current_path: NotePath,
//     entries: Vec<NavEntry>,
// }
//
// impl NotesAndDirs {
//     fn new(vault: NoteVault, path: NotePath) -> Self {
//         // Since this is a resource that depends on the current_path
//         // the entries change every time the current_path is changed
//         let entries = use_resource(move || {
//             let vault = vault.clone();
//             async move {
//                 let (tx, rx) = mpsc::channel();
//                 let current_path = path.read().clone();
//                 let mut entries = vec![];
//                 vault
//                     .get_notes(
//                         &current_path,
//                         NotesGetterOptions::default()
//                             .set_sender(tx)
//                             .full_validation(),
//                     )
//                     .expect("Error fetching Entries");
//                 while let Ok(entry) = rx.recv() {
//                     match &entry.data {
//                         EntryData::Note(note_data) => {
//                             entries.push(NavEntry::Note(note_data.path.clone()))
//                         }
//                         EntryData::Directory(directory_data) => {
//                             if directory_data.path != current_path {
//                                 entries.push(NavEntry::Directory(directory_data.path.clone()))
//                             }
//                         }
//                         EntryData::Attachment => {}
//                     };
//                 }
//                 entries.sort_by_key(|b| std::cmp::Reverse(b.sort_string()));
//                 entries
//             }
//         });
//         Self {
//             current_path: path,
//             entries,
//         }
//     }
//
//     fn get_entries(&self) -> Vec<NavEntry> {
//         let res = self.entries.value().read().to_owned().unwrap_or_default();
//         res
//     }
//
//     fn get_current(&self) -> NotePath {
//         self.current_path.read().clone()
//     }
// }
