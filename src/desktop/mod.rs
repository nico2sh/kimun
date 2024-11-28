#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod editor;
mod modal;

mod settings;
use dioxus::prelude::*;
use editor::{note_browser::NoteBrowser, text_editor::TextEditor};
use log::info;
use modal::Modal;
use settings::Settings;

use crate::noters::{nfs::NotePath, NoteVault};

#[derive(Debug, Clone)]
pub struct AppContext {
    pub vault: NoteVault,
    pub current_error: Signal<Option<String>>,
}

#[allow(non_snake_case)]
pub fn App() -> Element {
    let settings = use_signal(|| {
        info!("Settings loaded");
        Settings::load().unwrap()
    });
    use_context_provider(|| {
        let error: Signal<Option<String>> = Signal::new(None);
        let workspace_path = settings.read();
        let vault = NoteVault::new(workspace_path.workspace_dir.clone().unwrap()).unwrap();
        AppContext {
            vault,
            current_error: error,
        }
    });

    let app_context: AppContext = use_context();
    let error: Signal<Option<String>> = app_context.current_error;

    let current_note_path: Signal<Option<NotePath>> = use_signal(|| Some(NotePath::root()));
    let mut modal = use_signal(Modal::new);

    rsx! {
        link { rel: "stylesheet", href: "theme.css"}
        link { rel: "stylesheet", href: "main.css"}
        div {
            class: "container",
            onkeydown: move |event: Event<KeyboardData>| {
                let key = event.data.code();
                let modifiers = event.data.modifiers();
                if modifiers.meta() && key == Code::KeyO {
                    info!("Open Note Select");
                    modal.write().set_note_select();
                }
                if modifiers.meta() && key == Code::KeyS {
                    info!("Open Note Search");
                    modal.write().set_note_search();
                }
            },
            // We close any modal if we click on the main UI
            onclick: move |_e| {
                if modal.read().is_open() {
                    modal.write().close();
                    info!("Close dialog");
                }
            },
            aside {
                class: "sidebar",
                NoteBrowser {
                    note_path: current_note_path,
                }
            }
            header {
                class: "header"
            }
            div {
                class: "mainarea",
                { Modal::get_element(modal, current_note_path) },
                TextEditor {
                    note_path: current_note_path,
                }
            }
            footer {
                class: "footer",
                if let Some(err) = &*error.read() {
                        p{"{err}"}
                }
            }
        }
    }
}
