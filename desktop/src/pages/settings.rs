use std::path::PathBuf;

use dioxus::prelude::*;

use crate::{route::Route, settings};

#[component]
pub fn Settings() -> Element {
    let mut settings: Signal<settings::AppSettings> = use_context();
    // let mut settings = use_signal(|| settings::Settings::load_from_disk().unwrap_or_default());

    rsx! {
        div { id: "settings",
            div { class: "settings_section",
                div { class: "settings_title", "Workspace Path:" }
                div { class: "settings_content", "{settings().get_workspace_string()}" }
                button {
                    onclick: move |_| {
                        if let Ok(path) = pick_workspace() {
                            settings.write().set_workspace(&path);
                        }
                    },
                    "Browse"
                }
            }
            div { class: "bottom",
                button {
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

fn pick_workspace() -> anyhow::Result<PathBuf> {
    let handle = rfd::FileDialog::new()
        .set_title("Choose a Workspace Directory")
        .pick_folder()
        .ok_or(anyhow::anyhow!("Dialog Closed"))?;

    Ok(handle.to_path_buf())
}
