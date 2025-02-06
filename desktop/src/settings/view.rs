use std::path::PathBuf;

use eframe::egui::{self, CollapsingHeader};
use kimun_core::{error::VaultError, utilities::path_to_string, NoteVault, NotesValidation};
use log::{error, info};

use crate::{MainView, WindowSwitch};

use super::Settings;

pub struct SettingsView {
    settings: Settings,
}

impl SettingsView {
    pub fn new() -> anyhow::Result<Self> {
        let settings = Settings::load_from_disk()?;
        Ok(Self {
            settings: settings.to_owned(),
        })
    }

    fn add_workspace_button(
        &mut self,
        ui: &mut egui::Ui,
        button: egui::Button<'_>,
    ) -> egui::Response {
        if self.settings.workspace_dir.is_some() {
            ui.add_enabled(true, button)
        } else {
            ui.add_enabled(false, button)
        }
    }
}

impl MainView for SettingsView {
    fn update(&mut self, ui: &mut eframe::egui::Ui) -> anyhow::Result<Option<WindowSwitch>> {
        let mut should_close = false;
        let mut workspace_changed = false;
        egui::TopBottomPanel::bottom("Settings buttons")
            .resizable(false)
            .min_height(0.0)
            .show_inside(ui, |ui| {
                ui.add_space(8.0);
                let close_button = self.add_workspace_button(ui, egui::Button::new("Close"));
                if close_button.clicked() {
                    info!("Closing");
                    should_close = true;
                }
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical(|ui| ui.heading("Settings"));
                ui.separator();
                ui.add_space(8.0);
                CollapsingHeader::new("Workspace")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.group(|ui| {
                            let workspace_label = &self
                                .settings
                                .workspace_dir
                                .as_ref()
                                .map_or_else(|| "<None>".to_string(), path_to_string)
                                .to_string();
                            ui.label(egui::RichText::new("Main Workspace Directory: ").strong());
                            ui.horizontal(|ui| {
                                egui::Frame::default()
                                    .fill(ui.visuals().noninteractive().weak_bg_fill)
                                    // .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                                    .rounding(ui.visuals().widgets.noninteractive.rounding)
                                    .show(ui, |ui| {
                                        ui.label(workspace_label);
                                    });
                                let button = ui.button("Browse");
                                if button.clicked() {
                                    if let Ok(path) = pick_workspace() {
                                        if self.settings.workspace_dir.as_ref().map_or_else(
                                            || true,
                                            |workspace_dir| &path != workspace_dir,
                                        ) {
                                            workspace_changed = true;
                                        }
                                        self.settings.set_workspace(&path);
                                        if let Err(e) = self.settings.save_to_disk() {
                                            error!("Error setting the workspace: {}", e);
                                        }
                                    }
                                }
                            });
                            ui.label(egui::RichText::new("Vault DB:").strong());
                            ui.horizontal(|ui| {
                                if self
                                    .add_workspace_button(ui, egui::Button::new("Re-Index"))
                                    .clicked()
                                {
                                    if let Some(workspace_path) = &self.settings.workspace_dir {
                                        let res: Result<_, VaultError> =
                                            match NoteVault::new(workspace_path) {
                                                // TODO: Show a modal while executing
                                                Ok(vault) => {
                                                    vault.index_notes(NotesValidation::Full)
                                                }
                                                Err(e) => Err(e),
                                            };
                                        if let Err(e) = res {
                                            error!("Error reindexing the DB: {}", e);
                                        }
                                    };
                                }
                                if self
                                    .add_workspace_button(ui, egui::Button::new("Rebuild DB"))
                                    .clicked()
                                {
                                    if let Some(workspace_path) = &self.settings.workspace_dir {
                                        let res: Result<_, VaultError> =
                                            match NoteVault::new(workspace_path) {
                                                // TODO: Show a modal while executing
                                                Ok(vault) => vault.recreate_index(),
                                                Err(e) => Err(e),
                                            };
                                        if let Err(e) = res {
                                            error!("Error reindexing the DB: {}", e);
                                        }
                                    };
                                }
                            })
                        });
                    })
            });
        });
        if should_close {
            if let Some(workspace_dir) = &self.settings.workspace_dir {
                let vault = NoteVault::new(workspace_dir)?;
                if let Some(note_path) = self.settings.last_paths.last() {
                    Ok(Some(WindowSwitch::Editor {
                        vault,
                        note_path: note_path.to_owned(),
                    }))
                } else {
                    Ok(Some(WindowSwitch::NoNote { vault }))
                }
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
}

fn pick_workspace() -> anyhow::Result<PathBuf> {
    let handle = rfd::FileDialog::new()
        .set_title("Choose a Workspace Directory")
        .pick_folder()
        .ok_or(anyhow::anyhow!("Dialog Closed"))?;

    Ok(handle.to_path_buf())
}
