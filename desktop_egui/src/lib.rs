use std::path::PathBuf;

use core_notes::error::UIError;
use editor::Editor;
use eframe::egui;
use eframe::{App, CreationContext};
use filtered_list::row::{RowItem, RowMessage};
use icons::set_icon_fonts;
use settings::Settings;

mod editor;
pub mod filtered_list;
pub mod icons;
pub mod settings;

#[derive(PartialEq, Eq)]
pub enum Message {
    None,
    SelectionMessage(RowMessage),
    CloseWindow,
}

impl RowItem for String {
    fn get_label(&self, ui: &mut egui::Ui) -> egui::Response {
        ui.label(self)
    }

    fn get_sort_string(&self) -> String {
        self.clone()
    }

    fn get_message(&self) -> filtered_list::row::RowMessage {
        filtered_list::row::RowMessage::Nothing
    }
}

pub struct DesktopApp {
    settings: Settings,
    main_view: Box<dyn View>,
    left_view: Option<Box<dyn View>>,
    right_view: Option<Box<dyn View>>,
}

impl DesktopApp {
    pub fn new(cc: &CreationContext) -> anyhow::Result<Self> {
        let mut settings = Settings::load()?;
        set_icon_fonts(&cc.egui_ctx);
        if settings.workspace_dir.is_none() {
            let ws = pick_workspace()?;
            settings.workspace_dir = Some(ws);
            settings.save()?;
        }
        let current_view = Box::new(Editor::new(&settings)?);
        let left_view = None;
        let right_view = None;

        Ok(Self {
            settings,
            main_view: current_view,
            left_view,
            right_view,
        })
    }
}

impl App for DesktopApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        if let Some(left_view) = self.left_view.as_mut() {
            egui::SidePanel::left("Left Panel").show(ctx, |ui| {
                left_view.view(ui);
            });
        }
        if let Some(right_view) = self.right_view.as_mut() {
            egui::SidePanel::right("Right Panel").show(ctx, |ui| {
                right_view.view(ui);
            });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.main_view.view(ui);
        });
    }
}

pub trait View {
    fn view(&mut self, ui: &mut egui::Ui) -> Message;
}

fn pick_workspace() -> anyhow::Result<PathBuf> {
    let handle = rfd::FileDialog::new()
        .set_title("Choose a Workspace Directory")
        .pick_folder()
        .ok_or(UIError::DialogClosed)?;

    Ok(handle.to_path_buf())
}
