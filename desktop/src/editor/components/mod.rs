use eframe::egui;

use super::EditorMessage;

pub mod filtered_list;
pub mod preview_list;
pub mod vault_browse;

pub trait EditorComponent {
    fn update(&mut self, ui: &mut egui::Ui) -> Option<EditorMessage>;
}
