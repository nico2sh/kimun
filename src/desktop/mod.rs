#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod editor;
mod selector;

mod settings;
use dioxus::prelude::*;
use editor::{note_browser::NoteBrowser, text_editor::TextEditor};
use log::info;
use selector::{SelectionState, Selector};
use settings::Settings;

use crate::noters::{
    nfs::{NoteEntry, NotePath},
    NoteVault,
};

pub fn app() -> Element {
    let settings = Settings::load().unwrap();
    info!("Settings loaded");
    let note_vault = NoteVault::new(settings.workspace_dir.unwrap()).unwrap();

    let current_note_path: Signal<Option<NotePath>> = use_signal(|| Some(NotePath::root()));
    let mut selector_open = use_signal(|| SelectionState::Unset);

    rsx! {
        Selector {
            note_vault: note_vault.clone(),
            state: selector_open,
        }
        div {
            class: "flex flex-col h-screen border-solid border-2 border-green-500 p-2",
            onkeydown: move |e: Event<KeyboardData>| {
                let key = e.data.code();
                let modifiers = e.data.modifiers();
                if modifiers.meta() && key == Code::KeyO {
                    info!("Key pressed");
                     *selector_open.write() = SelectionState::Open(NotePath::root());
                }
            },
            // We close the modal if we click on the main UI
            onclick: move |_e| {*selector_open.write() = SelectionState::Unset;
                    info!("Close dialog");},
            div {
                // class: "flex h-full border-solid border-2 border-orange-600",
                class: "flex flex-row h-full border-solid border-2 border-orange-600",

                aside {
                    class: "w-48",
                    NoteBrowser {
                        note_vault: note_vault.clone(),
                        note_path: current_note_path,
                    }
                }
                main {
                    class: "size-full",
                    TextEditor {
                        note_vault: note_vault.clone(),
                        note_path: current_note_path,
                    }
                }
            }
        }
    }
}
