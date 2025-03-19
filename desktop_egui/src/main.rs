#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod editor;
pub mod fonts;
pub mod helpers;
pub mod modals;
mod no_note_view;
pub mod settings;

use std::path::PathBuf;

use editor::Editor;
use eframe::egui;
use kimun_core::{nfs::VaultPath, NoteVault};
use log::error;
use no_note_view::NoView;
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
        let ctx = cc.egui_ctx.clone();
        let current_view = match &settings.workspace_dir {
            Some(workspace_dir) => Self::get_first_view(workspace_dir, &settings, ctx)?,
            None => Box::new(SettingsView::new(ctx)?),
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

    fn get_first_view(
        workspace_dir: &PathBuf,
        settings: &Settings,
        ctx: egui::Context,
    ) -> anyhow::Result<Box<dyn MainView>> {
        let last_note = settings.last_paths.last().and_then(|path| {
            if !path.is_note() {
                None
            } else {
                Some(path.to_owned())
            }
        });

        let vault = NoteVault::new(workspace_dir)?;
        let view: Box<dyn MainView> = match last_note {
            Some(path) => Box::new(Editor::new(&vault, &path, ctx)?),
            None => Box::new(NoView::new(&vault)),
        };
        Ok(view)
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let has_window_action = egui::CentralPanel::default()
            .show(ctx, |ui| match self.main_view.update(ui) {
                Ok(ws) => ws,
                Err(e) => {
                    error!("Error displaying main view: {}", e);
                    None
                }
            })
            .inner;

        if let Some(window_switch) = has_window_action {
            match window_switch {
                WindowAction::Editor { vault, note_path } => {
                    match Editor::new(&vault, &note_path, ctx.clone()) {
                        Ok(editor) => {
                            self.main_view = Box::new(editor);
                        }
                        Err(e) => {
                            error!("Can't load the Editor: {}", e);
                        }
                    }
                }
                WindowAction::Settings => match SettingsView::new(ctx.clone()) {
                    Ok(settings_view) => {
                        self.main_view = Box::new(settings_view);
                    }
                    Err(e) => {
                        error!("Can't load the Settings: {}", e);
                    }
                },
                WindowAction::NoNote { vault } => self.main_view = Box::new(NoView::new(&vault)),
            }
        }
    }
}

pub trait MainView {
    fn update(&mut self, ui: &mut egui::Ui) -> anyhow::Result<Option<WindowAction>>;
}

#[derive(Clone)]
pub enum WindowAction {
    Editor {
        vault: NoteVault,
        note_path: VaultPath,
    },
    NoNote {
        vault: NoteVault,
    },
    Settings,
}
