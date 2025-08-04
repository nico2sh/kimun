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
        modal::{indexer::IndexType, Modal, ModalType},
        note_browser::NoteBrowser,
        text_editor::{EditorContent, EditorHeader, NoText, TextEditor},
    },
    global_events::{GlobalEvent, PubSub},
    route::Route,
    settings::AppSettings,
    utils::keys::{get_action, Shortcuts},
};

const EDITOR: &str = "editor";

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

    // Modal setup and Indexing on the first run
    let mut modal_type = use_signal(|| {
        debug!("We set the modal to nothing");
        ModalType::None
    });

    let pub_sub: PubSub<GlobalEvent> = use_context();
    let pc = pub_sub.clone();
    use_effect(move || {
        let editor_path = editor_path.read().clone();
        pc.subscribe(
            EDITOR,
            Callback::new(move |ge| match ge {
                GlobalEvent::Deleted(vault_path) => {
                    debug!("Deleted");
                    let is_current = editor_path.eq(&vault_path);
                    if is_current {
                        let parent = vault_path.get_parent_path().0;
                        navigator().replace(crate::Route::Editor {
                            editor_path: parent,
                            create: false,
                        });
                    } else {
                        debug!("Not current {}, {}", editor_path, vault_path);
                    }
                }
                GlobalEvent::Moved { from, to: _ } => {
                    debug!("Moved");

                    let is_current = editor_path.eq(&from);
                    if is_current {
                        let parent = from.get_parent_path().0;
                        navigator().replace(crate::Route::Editor {
                            editor_path: parent,
                            create: false,
                        });
                    }
                }
                GlobalEvent::Renamed { old_name, new_name } => {
                    debug!("Renamed");

                    let is_current = editor_path.eq(&old_name);
                    if is_current {
                        navigator().replace(crate::Route::Editor {
                            editor_path: new_name,
                            create: false,
                        });
                    }
                }
                _ => {}
            }),
        );
    });
    let drop_pub_sub = pub_sub.clone();
    use_drop(move || drop_pub_sub.unsubscribe(EDITOR));

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

    let editor_signal = use_signal(|| {
        debug!("> We create new dirty status");
        EditorContent::None
    });

    let editor_vault = vault.clone();
    let content_path = use_memo(move || {
        editor_vault.exists(&editor_path.read()).map_or_else(
            || {
                debug!("Path doesn't exist");
                if editor_path.read().is_note() && create {
                    debug!("It's a note and we have to create it");
                    let note_path = editor_path.read().to_owned();
                    match editor_vault.create_note(&note_path, "") {
                        Ok(_) => {
                            pub_sub.publish(GlobalEvent::NewNoteCreated(note_path));
                            PathType::Note
                        }
                        Err(e) => {
                            error!("Error creating note: {}", e);
                            let parent = note_path.get_parent_path().0;
                            PathType::Reroute(parent)
                        }
                    }
                } else {
                    debug!("We reroute to the root");
                    PathType::Reroute(VaultPath::root())
                }
            },
            // Exists, so we see if it's a directory or a note
            |e| match e.data {
                // If it's an attachment, we look for the parent
                EntryData::Note(_nt) => {
                    debug!("Path is a note");
                    PathType::Note
                }
                EntryData::Directory(_dt) => {
                    debug!("Path is a directory");
                    PathType::Directory
                }
                EntryData::Attachment => {
                    debug!("Path is an attachment");
                    PathType::Reroute(e.path.get_parent_path().0)
                }
            },
        )
    });

    // use_wry_event_handler(move |event, _| {
    //     if let Event::WindowEvent {
    //         window_id,
    //         event:
    //             WindowEvent::KeyboardInput {
    //                 device_id,
    //                 event,
    //                 is_synthetic,
    //                 ..
    //             },
    //         ..
    //     } = event
    //     {
    //         event.physical_key;
    //     } else {
    //         todo!()
    //     }
    // });

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
                                TextEditor { note_path: editor_path, vault: vault.clone(), editor_signal }
                            }
                        }
                        PathType::Directory => {
                            debug!("Opening Directory View");
                            rsx! {
                                NoText { path: editor_path, editor_signal }
                            }
                        }
                        PathType::Reroute(new_path) => {
                            let next_path = new_path.clone();
                            rsx! {
                                div {
                                    onmounted: move |_| {
                                        debug!("Rerouting to {}...", next_path);
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
