use eframe::egui;
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    editor::{
        modals::{
            filtered_list::FilteredList,
            vault_browse::{SelectorEntry, VaultBrowseFunctions},
            EditorModal,
        },
        EditorMessage,
    },
    MainView, WindowSwitch,
};

pub struct NoView {
    vault: NoteVault,
    filtered_list: FilteredList<VaultBrowseFunctions, Vec<SelectorEntry>, SelectorEntry>,
}

impl NoView {
    pub fn new(vault: &NoteVault) -> Self {
        let filtered_list =
            FilteredList::new(VaultBrowseFunctions::new(VaultPath::root(), vault.clone()));
        Self {
            vault: vault.clone(),
            filtered_list,
        }
    }
}

impl MainView for NoView {
    fn update(&mut self, ui: &mut egui::Ui) -> anyhow::Result<Option<WindowSwitch>> {
        let message = ui
            .vertical_centered(|ui| {
                ui.add_space(64.0);
                ui.label("Open or create a new note");
                self.filtered_list.update(ui)
            })
            .inner;
        if let Some(message) = message {
            let switch = match message {
                EditorMessage::OpenNote(vault_path) => Some(WindowSwitch::Editor {
                    vault: self.vault.clone(),
                    note_path: vault_path,
                }),
                EditorMessage::NewNote(vault_path) => {
                    self.vault.create_note(&vault_path, String::new())?;
                    Some(WindowSwitch::Editor {
                        vault: self.vault.clone(),
                        note_path: vault_path,
                    })
                }
                EditorMessage::NewJournal => {
                    let (note_details, _text) = self.vault.journal_entry()?;
                    Some(WindowSwitch::Editor {
                        vault: self.vault.clone(),
                        note_path: note_details.path,
                    })
                }
                EditorMessage::OpenSettings => Some(WindowSwitch::Settings),
                _ => None,
            };
            Ok(switch)
        } else {
            Ok(None)
        }
    }
}
