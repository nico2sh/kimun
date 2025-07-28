use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{
    nfs::{EntryData, VaultPath},
    NoteVault,
};

use crate::{
    components::{
        modal::{indexer::IndexType, Modal, ModalType},
        note_browser::NoteBrowser,
        text_editor::{EditorHeader, NoText, TextEditor},
    },
    route::Route,
    settings::AppSettings,
    utils::keys::{get_action, Shortcuts},
};

#[derive(Debug, PartialEq, Eq)]
enum PathType {
    Note,
    Directory,
    Reroute(VaultPath),
}

#[component]
pub fn Editor(editor_path: ReadOnlySignal<VaultPath>, create: bool) -> Element {
    debug!("-== [Editor] Starting Editor at '{}' ==-", editor_path);
    let settings: Signal<AppSettings> = use_context();
    let settings_value = settings.read();

    let mut show_browser = use_signal(|| false);

    let vault_path: &std::path::PathBuf = settings_value.workspace_dir.as_ref().unwrap();
    let vault = NoteVault::new(vault_path).unwrap();
    let vault = Arc::new(vault);

    let index_vault = vault.clone();

    // Modal setup and Indexing on the first run
    let mut modal_type = use_signal(|| {
        debug!("We initialize the modal manager");
        ModalType::None
    });
    use_effect(move || {
        debug!("We check if we have to trigger the indexer");
        if settings.read().needs_indexing() {
            debug!("Triggering Indexer");
            modal_type
                .write()
                .set_indexer(index_vault.clone(), IndexType::Validate);
        } else {
            debug!("No need to index");
        }
    });

    let dirty_status = use_signal(|| false);

    let editor_vault = vault.clone();
    let content_path = use_memo(move || {
        editor_vault.exists(&editor_path.read()).map_or_else(
            || {
                if editor_path.read().is_note() && create {
                    PathType::Note
                } else {
                    PathType::Reroute(VaultPath::root())
                }
            },
            // Exists, so we see if it's a directory or a note
            |e| match e.data {
                // If it's an attachment, we look for the parent
                EntryData::Note(_nt) => PathType::Note,
                EntryData::Directory(_dt) => PathType::Directory,
                EntryData::Attachment => PathType::Reroute(e.path.get_parent_path().0),
            },
        )
    });

    rsx! {
        if *show_browser.read() {
            div { class: "sidebar",
                NoteBrowser {
                    vault: vault.clone(),
                    editor_path,
                    modal_type,
                    show_browser,
                }
            }
        } else {
            div { class: "sidebar collapsed" }
        }
        div { class: "editor-area",
            div {
                class: "editor-container",
                tabindex: 0,
                onkeydown: move |event: Event<KeyboardData>| {
                    let data = event.data;
                    match get_action(&data) {
                        Shortcuts::None => {}
                        Shortcuts::OpenSettings => {
                            debug!("Open Settings");
                            navigator().replace(Route::Settings {});
                        }
                        Shortcuts::ToggleNoteBrowser => {
                            debug!("Toggle note browser");
                            let shown = *show_browser.read();
                            show_browser.set(!shown);
                        }
                        Shortcuts::SearchNotes => {
                            debug!("Trigger Open Note Search");
                            modal_type.write().set_note_search(vault.clone());
                        }
                        Shortcuts::OpenNote => {
                            debug!("Trigger Open Note Select");
                            modal_type
                                .write()
                                .set_note_select(vault.clone(), editor_path.read().clone());
                        }
                        Shortcuts::NewJournal => {
                            debug!("New Journal Entry");
                            if let Ok(journal_entry) = vault.journal_entry() {
                                navigator()
                                    .replace(crate::Route::Editor {
                                        editor_path: journal_entry.0.path,
                                        create: true,
                                    });
                            }
                        }
                    }
                },
                // We close any modal if we click on the main UI
                onclick: move |_e| {
                    if modal_type.read().is_open() {
                        modal_type.write().close();
                        info!("Close dialog");
                    }
                },
                Modal { modal_type }
                EditorHeader { path: editor_path, show_browser, dirty_status }
                div { class: "editor-main",
                    match &*content_path.read() {
                        PathType::Note => {
                            rsx! {
                                TextEditor { vault: vault.clone(), note_path: editor_path, dirty_status }
                            }
                        }
                        PathType::Directory => {
                            rsx! {
                                NoText { path: editor_path }
                            }
                        }
                        PathType::Reroute(new_path) => {
                            let next_path = new_path.clone();
                            rsx! {
                                div {
                                    onmounted: move |_| {
                                        navigator()
                                            .replace(Route::Editor {
                                                editor_path: next_path.clone(),
                                                create: true,
                                            });
                                    },
                                }
                            }
                        }
                    }
                }
                div { class: "editor-footer" }
            }
        }
    }
}
