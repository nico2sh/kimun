use std::{path::PathBuf, sync::Arc};

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    components::{
        button::Button,
        modal::{indexer::IndexType, Modal, ModalType},
    },
    route::Route,
    settings,
};

#[component]
pub fn Settings() -> Element {
    let mut settings: Signal<settings::AppSettings> = use_context();
    let mut modal_type = use_signal(|| ModalType::None);

    rsx! {
        div { class: "settings-container",
            Modal { modal_type }
            div { class: "settings-header",
                h1 { "Settings" }
                p { "Customize app settings" }
            }
            div { class: "settings-content",
                div { class: "settings-section",
                    h2 { class: "section-title", "Workspace" }
                    div { class: "form-group",
                        label { class: "form-label", "Workspace Location" }
                        div { class: "file-upload-container",
                            Button {
                                title: "Browse",
                                action: move |_| {
                                    if let Ok(path) = pick_workspace() {
                                        settings.write().set_workspace(&path);
                                    }
                                },
                            }
                        }
                        div {
                            id: "config-filename",
                            class: {
                                if settings().workspace_dir.is_some() {
                                    "file-name file-selected"
                                } else {
                                    "file-name"
                                }
                            },
                            "{settings().get_workspace_string()}"
                        }
                        div { class: "description", "Sets the directory where your notes are located" }
                    }
                    div { class: "form-group",
                        label { class: "form-label", "Vault Indexing" }
                        div { class: "file-upload-container",
                            Button {
                                title: "Fast Index",
                                action: move |_| {
                                    if let Some(workspace_dir) = settings().workspace_dir {
                                        let vault = Arc::new(NoteVault::new(workspace_dir).unwrap());
                                        modal_type.write().set_indexer(vault, IndexType::Fast);
                                    }
                                },
                                disabled: settings().workspace_dir.is_none(),
                            }
                        }
                        div { class: "description", "Indexes the notes located in the directory" }
                        div { class: "file-upload-container",
                            Button {
                                title: "Full Index",
                                action: move |_| {
                                    if let Some(workspace_dir) = settings().workspace_dir {
                                        let vault = Arc::new(NoteVault::new(workspace_dir).unwrap());
                                        modal_type.write().set_indexer(vault, IndexType::Full);
                                    }
                                },
                                disabled: settings().workspace_dir.is_none(),
                            }
                        }
                        div { class: "description",
                            "Performs a full index of the notes located in the directory, can take longer time depending on the number of notes"
                        }
                    }
                }

                div { class: "settings-section",
                    h2 { class: "section-title", "Theme" }
                    div { class: "form-group",
                        label { class: "form-label", "Theme Settings" }
                        div { class: "select-container",
                            select {
                                class: "custom-select",
                                id: "theme-select",
                                onchange: move |e| {
                                    settings.write().set_theme(e.data().value());
                                },
                                for theme in settings().theme_list {
                                    option {
                                        value: "{theme.name}",
                                        selected: settings().theme == theme.name,
                                        "{theme.name}"
                                    }
                                }
                            }
                        }
                        div { class: "description", "Choose your application theme" }
                    }
                }

                div { class: "action-buttons",
                    Button {
                        title: "Close without Saving",
                        style: crate::components::button::ButtonStyle::Secondary,
                        action: move |_| {
                            navigator().replace(Route::Start {});
                        },
                    }
                    Button {
                        title: "Save and Close",
                        action: move |_| {
                            let path = &settings.read().workspace_dir;
                            match settings.read().save_to_disk() {
                                Ok(_) => {
                                    if let Some(_p) = path {
                                        let editor_path = settings
                                            .read()
                                            .last_paths
                                            .last()
                                            .map_or_else(VaultPath::root, |p| p.to_owned());
                                        navigator()
                                            .replace(Route::MainView {
                                                editor_path,
                                                create: false,
                                            });
                                    }
                                }
                                Err(_e) => todo!(),
                            };
                        },
                    }
                }
            }
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
