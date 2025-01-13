mod editor;
pub mod icons;
pub mod settings;

use std::path::PathBuf;

use anyhow::anyhow;
use editor::Editor;
use eframe::egui;
// use filtered_list::row::{RowItem, RowMessage};
use icons::set_icon_fonts;
use log::error;
use settings::Settings;

fn main() -> eframe::Result {
    env_logger::Builder::new()
        .filter(Some("notes_"), log::LevelFilter::max())
        .init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Note",
        native_options,
        Box::new(|cc| Ok(Box::new(DesktopApp::new(cc)?))),
    )
}

#[derive(PartialEq, Eq)]
pub enum Message {
    None,
    // SelectionMessage(RowMessage),
    CloseWindow,
}

pub struct DesktopApp {
    settings: Settings,
    main_view: Box<dyn View>,
    left_view: Option<Box<dyn View>>,
    right_view: Option<Box<dyn View>>,
}

impl DesktopApp {
    pub fn new(cc: &eframe::CreationContext) -> anyhow::Result<Self> {
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

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(left_view) = self.left_view.as_mut() {
            egui::SidePanel::left("Left Panel").show(ctx, |ui| {
                if let Err(e) = left_view.view(ui) {
                    error!("Error displaying left view: {}", e);
                }
            });
        }
        if let Some(right_view) = self.right_view.as_mut() {
            egui::SidePanel::right("Right Panel").show(ctx, |ui| {
                if let Err(e) = right_view.view(ui) {
                    error!("Error displaying right view: {}", e);
                }
            });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Err(e) = self.main_view.view(ui) {
                error!("Error displaying main view: {}", e);
            }
        });
    }
}

pub trait View {
    fn view(&mut self, ui: &mut egui::Ui) -> anyhow::Result<()>;
}

fn pick_workspace() -> anyhow::Result<PathBuf> {
    let handle = rfd::FileDialog::new()
        .set_title("Choose a Workspace Directory")
        .pick_folder()
        .ok_or(anyhow!("Dialog Closed"))?;

    Ok(handle.to_path_buf())
}
