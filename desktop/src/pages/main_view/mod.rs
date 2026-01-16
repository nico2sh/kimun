use std::{str::FromStr, sync::Arc};

use content_view::{NoText, TextEditor};
use dioxus::{core::use_drop, logger::tracing::debug, prelude::*};
use header::EditorHeader;
use kimun_core::{
    nfs::{EntryData, VaultPath},
    NoteVault,
};

use crate::{
    app_state::AppState,
    components::{
        modal::{indexer::IndexType, Modal, ModalType},
        note_browser::NoteBrowser,
        preview_pane::PreviewPane,
    },
    global_events::{GlobalEvent, PubSub},
    route::Route,
    settings::AppSettings,
    utils::keys::action_shortcuts::ActionShortcuts,
};

mod content_view;
mod header;

const EDITOR: &str = "editor";

#[derive(Debug, PartialEq, Eq)]
enum ContentType {
    Note,
    Directory,
    Reroute(VaultPath),
}

#[component]
pub fn MainView() -> Element {
    let app_state: Signal<AppState> = use_context();
    let editor_path = use_memo(move || app_state.read().current_path.to_owned());
    debug!("-== [Editor] Starting Editor at '{}' ==-", editor_path);
    let settings: Signal<AppSettings> = use_context();
    let settings_value = settings.read();

    let vault_path: &std::path::PathBuf = settings_value.workspace_dir.as_ref().unwrap();
    let vault = Arc::new(NoteVault::new(vault_path)?);

    // Modal setup and Indexing on the first run
    let mut modal_type = use_signal(|| {
        debug!("We set the modal to nothing");
        ModalType::None
    });

    let pub_sub: PubSub<GlobalEvent> = use_context();
    let pc = pub_sub.clone();
    let mut app_state: Signal<AppState> = use_context();
    use_effect(move || {
        pc.subscribe(
            EDITOR,
            Callback::new(move |ge| match ge {
                GlobalEvent::Deleted(vault_path) => {
                    debug!("Deleted");
                    let is_current = editor_path.eq(&vault_path);
                    if is_current {
                        let parent = vault_path.get_parent_path().0;
                        app_state.write().set_path(&parent, false);
                    } else {
                        debug!("Not current {}, {}", editor_path, vault_path);
                    }
                }
                GlobalEvent::Moved { from, to: _ } => {
                    debug!("Moved");

                    let is_current = editor_path.eq(&from);
                    if is_current {
                        let parent = from.get_parent_path().0;
                        app_state.write().set_path(&parent, false);
                    }
                }
                GlobalEvent::Renamed { old_name, new_name } => {
                    debug!("Renamed");

                    let is_current = editor_path.eq(&old_name);
                    if is_current {
                        app_state.write().set_path(&new_name, false);
                    }
                }
                GlobalEvent::OpenPreviewPane(source) => {
                    debug!("Preview pane, with source: {}", source);
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

    let editor_vault = vault.clone();
    let content_path = use_memo(move || {
        editor_vault.exists(&editor_path.read()).map_or_else(
            || {
                debug!("Path doesn't exist");
                if editor_path.read().is_note() && app_state.read().create_if_not_exists {
                    debug!("It's a note and we have to create it");
                    app_state.write().preview_mode = false;
                    let note_path = editor_path.read().to_owned();
                    match editor_vault.create_note(&note_path, "") {
                        Ok(_) => {
                            pub_sub.publish(GlobalEvent::NewNoteCreated(note_path));
                            ContentType::Note
                        }
                        Err(e) => {
                            let parent = note_path.get_parent_path().0;
                            modal_type.set(ModalType::Error {
                                message: "Error Creating new Note".to_string(),
                                error: e.to_string(),
                            });
                            ContentType::Reroute(parent)
                        }
                    }
                } else {
                    debug!("We reroute to the root");
                    ContentType::Reroute(VaultPath::root())
                }
            },
            // Exists, so we see if it's a directory or a note
            |e| match e.data {
                // If it's an attachment, we look for the parent
                EntryData::Note(_nt) => {
                    debug!("Path is a note");
                    ContentType::Note
                }
                EntryData::Directory(_dt) => {
                    debug!("Path is a directory");
                    ContentType::Directory
                }
                EntryData::Attachment => {
                    debug!("Path is an attachment");
                    ContentType::Reroute(e.path.get_parent_path().0)
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
        if app_state.read().show_browser {
            div { class: "sidebar",
                NoteBrowser { vault: vault.clone(), editor_path, modal_type }
            }
        } else {
            div { class: "sidebar collapsed" }
        }
        div {
            class: "editor-container",
            tabindex: 0,
            onkeydown: move |event: Event<KeyboardData>| {
                let data = event.data;
                if let Some(action) = settings.read().key_bindings.get_action(&data.into()) {
                    match action {
                        ActionShortcuts::TogglePreview => {
                            let preview = !app_state.read().preview_mode;
                            debug!("Toggling preview to {}", preview);
                            app_state.write().preview_mode = preview;
                        }
                        ActionShortcuts::OpenSettings => {
                            debug!("Open Settings");
                            navigator().replace(Route::Settings {});
                        }
                        ActionShortcuts::ToggleNoteBrowser => {
                            debug!("Toggle note browser");
                            app_state.write().toggle_browser();
                        }
                        ActionShortcuts::SearchNotes => {
                            debug!("Trigger Open Note Search");
                            modal_type.write().set_note_search(vault.clone());
                        }
                        ActionShortcuts::OpenNote => {
                            debug!("Trigger Open Note Select");
                            modal_type
                                .write()
                                .set_note_select(vault.clone(), editor_path.read().clone());
                        }
                        ActionShortcuts::NewJournal => {
                            debug!("New Journal Entry");
                            if let Ok(journal_entry) = vault.journal_entry() {
                                app_state.write().set_path(&journal_entry.0.path, true);
                            }
                        }
                        _ => {}
                    }
                }
            },
            Modal { modal_type }
            EditorHeader { path: editor_path }
            div { class: "editor-main",
                match &*content_path.read() {
                    ContentType::Note => {
                        rsx! {
                            TextEditor {
                                note_path: editor_path,
                                vault: vault.clone(),
                                modal_type,
                                preview: app_state.read().preview_mode,
                            }
                        }
                    }
                    ContentType::Directory => {
                        debug!("Opening Directory View");
                        rsx! {
                            NoText { path: editor_path }
                        }
                    }
                    ContentType::Reroute(new_path) => {
                        let next_path = new_path.clone();
                        rsx! {
                            div {
                                onmounted: move |_| {
                                    debug!("Rerouting to {}...", next_path);
                                    app_state.write().set_path(&next_path, true);
                                },
                            }
                        }
                    }
                }
                if let Some(source) = &app_state.read().show_preview_pane {
                    div { class: "rightbar",
                        PreviewPane {
                            vault: vault.clone(),
                            initial_state: source.to_owned(),
                            modal_type,
                        }
                    }
                }
            }
            div { class: "editor-footer" }
        }
    }
}
