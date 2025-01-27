#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod editor;
pub mod fonts;
pub mod helpers;
pub mod settings;

use editor::Editor;
use eframe::egui;
// use filtered_list::row::{RowItem, RowMessage};
use log::error;
use settings::{view::SettingsView, Settings};

fn main() -> eframe::Result {
    env_logger::Builder::new()
        .filter(Some("kimun_"), log::LevelFilter::max())
        .init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Kimün",
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
    main_view: Box<dyn MainView>,
}

impl DesktopApp {
    pub fn new(cc: &eframe::CreationContext) -> anyhow::Result<Self> {
        let settings = Settings::load_from_disk()?;
        let current_view: Box<dyn MainView> = if settings.workspace_dir.is_some() {
            Box::new(Editor::new(&settings, true)?)
        } else {
            Box::new(SettingsView::new(&settings))
        };

        let desktop_app = Self {
            main_view: current_view,
        };
        cc.egui_ctx.style_mut(|style| {
            style.url_in_tooltip = true;
        });
        desktop_app.setup(cc);
        Ok(desktop_app)
    }

    fn setup(&self, cc: &eframe::CreationContext) {
        fonts::set_fonts(&cc.egui_ctx);
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| match self.main_view.update(ui) {
            Ok(Some(switch)) => match Settings::load_from_disk() {
                Ok(settings) => match switch {
                    WindowSwitch::Editor { recreate_index } => {
                        match Editor::new(&settings, recreate_index) {
                            Ok(editor) => {
                                self.main_view = Box::new(editor);
                            }
                            Err(e) => {
                                error!("Can't load the Editor: {}", e);
                            }
                        }
                    }
                    WindowSwitch::Settings => {
                        self.main_view = Box::new(SettingsView::new(&settings));
                    }
                },
                Err(e) => error!("Error loading settings from disk: {}", e),
            },
            Err(e) => {
                error!("Error displaying main view: {}", e);
            }
            _ => {}
        });
    }
}

pub trait MainView {
    fn update(&mut self, ui: &mut egui::Ui) -> anyhow::Result<Option<WindowSwitch>>;
}

#[derive(Clone, Copy)]
pub enum WindowSwitch {
    Editor { recreate_index: bool },
    Settings,
}
