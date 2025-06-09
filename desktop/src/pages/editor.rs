use std::{path::PathBuf, rc::Rc, sync::Arc};

use dioxus::{
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    components::{
        modal::{indexer::IndexType, Modal},
        note_browser::NoteBrowser,
        text_editor::TextEditor,
    },
    route::Route,
    settings::AppSettings,
};

#[component]
pub fn Editor() -> Element {
    let settings: Signal<AppSettings> = use_context();
    let settings = settings.read();
    let last_note_path = settings.last_paths.last().map(|p| p.to_owned());
    let vault_path = settings.workspace_dir.as_ref().unwrap();
    let vault = NoteVault::new(vault_path).unwrap();
    debug!("Opening editor at {:?}", vault.workspace_path);
    let vault = Arc::new(vault);
    let mut note_path = use_signal_sync(|| last_note_path);
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
    if settings.needs_indexing() {
        modal
            .write()
            .set_indexer(vault.clone(), IndexType::Validate);
    }

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
            tabindex: 0,
            class: "editor-container",
            onkeydown: move |event: Event<KeyboardData>| {
                let key = event.data.code();
                let modifiers = event.data.modifiers();
                if modifiers.meta() {
                    match key {
                        Code::KeyO => {
                            debug!("Trigger Open Note Select");
                            modal.write().set_note_select(vault.clone(), note_path);
                        }
                        Code::KeyK => {
                            debug!("Trigger Open Note Search");
                            modal.write().set_note_search(vault.clone(), note_path);
                        }
                        Code::KeyJ => {
                            debug!("New Journal Entry");
                            if let Ok(journal_entry) = vault.journal_entry() {
                                note_path.set(Some(journal_entry.0.path));
                            }
                        }
                        Code::Comma => {
                            debug!("Open Settings");
                            navigator().replace(Route::Settings {});
                        }
                        _ => {}
                    }
                }
            },
            // We close any modal if we click on the main UI
            onclick: move |_e| {
                info!("{:?}", _e);
                if modal.read().is_open() {
                    modal.write().close();
                    info!("Close dialog");
                }
            },
            {Modal::get_element(modal)}
            div { class: "editor-header",
                div { class: "title-section",
                    div { class: "title-text", "{note_path_display}" }
                    div { class: "status-indicator", id: "saveStatus" }
                }
            }
            div { class: "editor-main",
                TextEditor { vault: vault.clone(), note_path, editor_signal }
            }
            div { class: "editor-footer" }
        }
    }
}
