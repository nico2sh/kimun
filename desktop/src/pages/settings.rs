use std::{path::PathBuf, sync::Arc};

use dioxus::{html::label, prelude::*};
use kimun_core::NoteVault;

use crate::{
    components::modal::{indexer::IndexType, Modal},
    route::Route,
    settings,
};

#[component]
pub fn Settings() -> Element {
    let mut settings: Signal<settings::AppSettings> = use_context();
    let mut modal = use_signal(Modal::new);
    // let mut settings = use_signal(|| settings::Settings::load_from_disk().unwrap_or_default());

    rsx! {
        div { class: "settings-container",
            {Modal::get_element(modal)}
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
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    if let Ok(path) = pick_workspace() {
                                        settings.write().set_workspace(&path);
                                    }
                                },
                                "Browse"
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
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    if let Some(workspace_dir) = settings().workspace_dir {
                                        let vault = Arc::new(NoteVault::new(workspace_dir).unwrap());
                                        modal.write().set_indexer(vault, IndexType::Fast);
                                    }
                                },
                                disabled: {settings().workspace_dir.is_none()},
                                "Fast Index"
                            }
                        }
                        div { class: "description", "Indexes the notes located in the directory" }
                        div { class: "file-upload-container",
                            button {
                                class: "btn btn-primary",
                                onclick: move |_| {
                                    if let Some(workspace_dir) = settings().workspace_dir {
                                        let vault = Arc::new(NoteVault::new(workspace_dir).unwrap());
                                        modal.write().set_indexer(vault, IndexType::Full);
                                    }
                                },
                                disabled: {settings().workspace_dir.is_none()},
                                "Full Index"
                            }
                        }
                        div { class: "description",
                            "Performs a full index of the notes located in the directory, can take longer time depending on the number of notes"
                        }
                    }
                }
                div { class: "action-buttons",
                    button {
                        class: "btn btn-secondary",
                        onclick: move |_| {
                            navigator().replace(Route::Main {});
                        },
                        "Close without saving"
                    }
                    button {
                        class: "btn btn-primary",

                        onclick: move |_| {
                            let path = &settings.read().workspace_dir;
                            match settings.read().save_to_disk() {
                                Ok(_) => {
                                    if let Some(_p) = path {
                                        navigator().replace(Route::Editor {});
                                    }
                                }
                                Err(_e) => todo!(),
                            };
                        },
                        "Save and Close"
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
