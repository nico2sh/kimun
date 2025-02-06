use crossbeam_channel::Receiver;
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
    filtered_list: FilteredList<VaultBrowseFunctions, Vec<SelectorEntry>, SelectorEntry>,
    message_receiver: Receiver<EditorMessage>,
}

impl NoView {
    pub fn new(vault: &NoteVault) -> Self {
        let (message_sender, message_receiver) = crossbeam_channel::unbounded();
        let filtered_list = FilteredList::new(
            VaultBrowseFunctions::new(VaultPath::root(), vault.clone()),
            message_sender.clone(),
        );
        Self {
            filtered_list,
            message_receiver,
        }
    }
}

impl MainView for NoView {
    fn update(&mut self, ui: &mut egui::Ui) -> anyhow::Result<Option<WindowSwitch>> {
        ui.vertical_centered(|ui| {
            ui.add_space(64.0);
            ui.label("Open or create a note with cmd + O");
            self.filtered_list.update(ui);
        });
        Ok(None)
    }
}
