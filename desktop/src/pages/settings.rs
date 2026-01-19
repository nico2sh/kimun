use std::{path::PathBuf, sync::Arc};

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    app_state::AppState,
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

    let theme = settings().get_theme();

    rsx! {
        div { class: "settings-container", background_color: "{theme.bg_main}",
            Modal { modal_type }
            div {
                class: "settings-header",
                background_color: "{theme.bg_head}",
                color: "{theme.text_head}",
                h1 { "Settings" }
                p { "Customize app settings" }
            }
            div { class: "settings-content",
                div {
                    class: "settings-section",
                    background_color: "{theme.bg_section}",
                    border_color: "{theme.border_light}",
                    h2 {
                        class: "section-title",
                        color: "{theme.text_secondary}",
                        "Workspace"
                    }
                    div { class: "form-group",
                        label { class: "form-label", color: "{theme.text_muted}", "Workspace Location" }
                        div { class: "file-upload-container",
                            Button {
                                title: "Browse",
                                theme: theme.clone(),
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
                            color: {
                                if settings().workspace_dir.is_some() {
                                    "{theme.text_light}"
                                } else {
                                    "{theme.accent_green}"
                                }
                            },
                            "{settings().get_workspace_string()}"
                        }
                        div { class: "description", color: "{theme.text_light}",
                            "Sets the directory where your notes are located"
                        }
                    }
                    div { class: "form-group",
                        label { class: "form-label", color: "{theme.text_muted}", "Vault Indexing" }
                        div { class: "file-upload-container",
                            Button {
                                title: "Fast Index",
                                theme: theme.clone(),
                                action: move |_| {
                                    if let Some(workspace_dir) = settings().workspace_dir {
                                        let vault = Arc::new(NoteVault::new(workspace_dir).unwrap());
                                        modal_type.write().set_indexer(vault, IndexType::Fast);
                                    }
                                },
                                disabled: settings().workspace_dir.is_none(),
                            }
                        }
                        div { class: "description", color: "{theme.text_light}",
                            "Indexes the notes located in the directory"
                        }
                        div { class: "file-upload-container",
                            Button {
                                title: "Full Index",
                                theme: theme.clone(),
                                action: move |_| {
                                    if let Some(workspace_dir) = settings().workspace_dir {
                                        let vault = Arc::new(NoteVault::new(workspace_dir).unwrap());
                                        modal_type.write().set_indexer(vault, IndexType::Full);
                                    }
                                },
                                disabled: settings().workspace_dir.is_none(),
                            }
                        }
                        div { class: "description", color: "{theme.text_light}",
                            "Performs a full index of the notes located in the directory, can take longer time depending on the number of notes"
                        }
                    }
                }

                div {
                    class: "settings-section",
                    background_color: "{theme.bg_section}",
                    border_color: "{theme.border_light}",
                    h2 {
                        class: "section-title",
                        color: "{theme.text_secondary}",
                        "Theme"
                    }
                    div { class: "form-group",
                        label { class: "form-label", color: "{theme.text_muted}", "Theme Settings" }
                        div { class: "select-container",
                            select {
                                class: "custom-select",
                                border_color: "{theme.border_light}",
                                color: "{theme.text_secondary}",
                                background_color: "transparent",
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
                        div { class: "description", color: "{theme.text_light}",
                            "Choose your application theme"
                        }
                    }
                }

                div {
                    class: "action-buttons",
                    background: "{theme.bg_section}",
                    border_top_color: "{theme.border_light}",
                    Button {
                        title: "Close without Saving",
                        theme: theme.clone(),
                        style: crate::components::button::ButtonStyle::Secondary,
                        action: move |_| {
                            navigator().replace(Route::Start {});
                        },
                    }
                    Button {
                        title: "Save and Close",
                        theme: theme.clone(),
                        action: move |_| {
                            let path = &settings.read().workspace_dir;
                            match settings.read().save_to_disk() {
                                Ok(_) => {
                                    let mut app_state: Signal<AppState> = use_context();
                                    if let Some(_p) = path {
                                        let editor_path = settings
                                            .read()
                                            .last_paths
                                            .last()
                                            .map_or_else(VaultPath::root, |p| p.to_owned());
                                        app_state.write().current_path = editor_path;
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
