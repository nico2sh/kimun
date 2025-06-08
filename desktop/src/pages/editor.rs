use std::{path::PathBuf, rc::Rc, sync::Arc};

use dioxus::{
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    components::{modal::Modal, note_browser::NoteBrowser, text_editor::TextEditor},
    settings::Settings,
};

#[component]
pub fn Editor(note_path: Option<VaultPath>) -> Element {
    let settings: Signal<Settings> = use_context();
    let settings = settings.read();
    let vault_path = settings.workspace_dir.as_ref().unwrap();
    let vault = NoteVault::new(vault_path).unwrap();
    debug!("Opening editor at {:?}", vault.workspace_path);
    let vault = Arc::new(vault);
    let note_path = use_signal_sync(|| note_path);
    let note_path_display = use_memo(move || {
        let np = match note_path.read().to_owned() {
            Some(path) => path,
            None => VaultPath::root(),
        };
        if np.is_note() {
            np.to_string()
        } else {
            String::new()
        }
    });
    let mut modal = use_signal(Modal::new);
    let editor_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    if !modal.read().is_open() {
        // TODO: Try with use_future
        spawn(async move {
            loop {
                if let Some(e) = editor_signal.with(|f| f.clone()) {
                    let _ = e.set_focus(true).await;
                    break;
                }
            }
        });
    }

    rsx! {
        div {
            class: "container",
            onkeydown: move |event: Event<KeyboardData>| {
                let key = event.data.code();
                let modifiers = event.data.modifiers();
                if modifiers.meta() && key == Code::KeyO {
                    debug!("Trigger Open Note Select");
                    modal.write().set_note_select();
                }
                if modifiers.meta() && key == Code::KeyK {
                    debug!("Trigger Open Note Search");
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
            aside { class: "sidebar",
                NoteBrowser { vault: vault.clone(), note_path }
            }
            header { class: "header",
                div { class: "path", "{note_path_display}" }
            }
            div { class: "mainarea",
                {Modal::get_element(modal, vault.clone(), note_path)}
                TextEditor { vault: vault.clone(), note_path, editor_signal }
            }
            footer { class: "footer" }
        }
    }
}
