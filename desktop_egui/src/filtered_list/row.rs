use core_notes::nfs::NotePath;
use eframe::egui;

pub trait RowItem: AsRef<str> + Send + Sync + Clone {
    fn get_label(&self, ui: &mut egui::Ui) -> egui::Response;
    fn get_sort_string(&self) -> String;
    fn get_message(&self) -> RowMessage;
}

#[derive(PartialEq, Eq, Debug)]
pub enum RowMessage {
    Nothing,
    OpenNote(NotePath),
    OpenDirectory(NotePath),
}
