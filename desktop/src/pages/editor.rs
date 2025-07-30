use std::sync::Arc;

use dioxus::{
    logger::tracing::{debug, error, info},
    prelude::*,
};
use kimun_core::{
    nfs::{EntryData, VaultPath},
    NoteVault,
};

use crate::{
    components::{
        modal::{confirmations::ModalAction, indexer::IndexType, Modal, ModalType},
        note_browser::NoteBrowser,
        text_editor::{EditorData, EditorHeader, NoText, TextEditor},
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
    // let editor_path = use_signal(|| editor_path().to_owned());
    let settings: Signal<AppSettings> = use_context();
    let settings_value = settings.read();

    let mut show_browser = use_signal(|| false);

    let vault_path: &std::path::PathBuf = settings_value.workspace_dir.as_ref().unwrap();
    let vault = NoteVault::new(vault_path).unwrap();
    let vault = Arc::new(vault);

    // Modal setup and Indexing on the first run
    let mut modal_type = use_signal(|| {
        debug!("We set the modal to nothing");
        ModalType::None(None)
    });

    // We monitor results from the modal
    let confirm_action = use_memo(move || {
        debug!("Modal changed");
        let mt = modal_type().clone();
        if let ModalType::None(action) = mt {
            action
        } else {
            None
        }
    });
    let action_vault = vault.clone();
    use_effect(move || {
        if let Some(status) = confirm_action() {
            match status {
                ModalAction::Delete(vault_path) => debug!("Deleted"),
                ModalAction::Move { from, to } => {
                    debug!("Move");

                    let is_current = editor_path.read().eq(&from);
                    if is_current {
                        let parent = from.get_parent_path().0;
                        navigator().replace(crate::Route::Editor {
                            editor_path: parent,
                            create: false,
                        });
                    }
                    let is_note = from.is_note();
                    let move_result = if is_note {
                        action_vault.rename_note(&from, &to)
                    } else {
                        action_vault.rename_directory(&from, &to)
                    };

                    if let Err(e) = move_result {
                        error!("Error: {}", e);
                        modal_type.write().set_error(
                            format!(
                                "Error moving {}: {}",
                                if is_note { "note" } else { "directory" },
                                from
                            ),
                            format!("{}", e),
                        );
                    } else {
                        modal_type.write().close_with_action(ModalAction::Move {
                            from: from.clone(),
                            to: to.clone(),
                        });
                    }
                    if editor_path.read().eq(&from) {
                        navigator().replace(crate::Route::Editor {
                            editor_path: to.clone(),
                            create: false,
                        });
                    }
                }
                ModalAction::Rename(vault_path) => debug!("renamed"),
            }
            // We reset the modal status
            modal_type.set(ModalType::default());
        }
    });

    let index_vault = vault.clone();
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

    let editor_vault = vault.clone();
    let editor_signal = use_signal(|| {
        debug!("> We create new dirty status");
        None
    });

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
                EditorHeader { path: editor_path, show_browser, editor_signal }
                div { class: "editor-main",
                    match &*content_path.read() {
                        PathType::Note => {
                            rsx! {
                                TextEditor { note_path: editor_path, editor_signal }
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
