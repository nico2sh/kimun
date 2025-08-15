use std::sync::Arc;

use dioxus::{logger::tracing::error, prelude::*};
use kimun_core::{nfs::VaultPath, NoteVault, ResultType, VaultBrowseOptionsBuilder};

use crate::{
    components::{button::ButtonBuilder, modal::ModalType},
    global_events::{GlobalEvent, PubSub},
    route::Route,
};

pub enum ConfirmationType {
    Delete(VaultPath),
    Move(VaultPath),
    Rename(VaultPath),
    NewNote(VaultPath),
    NewDirectory(VaultPath),
}

#[derive(Props, Clone, PartialEq)]
pub struct ConfirmationModalProps {
    modal_type: Signal<ModalType>,
    title: String,
    subtitle: String,
    body: Element,
    buttons: Vec<ButtonBuilder>,
}

// General Confirmation Modal
#[component]
pub fn ConfirmationModal(props: ConfirmationModalProps) -> Element {
    let mut modal = props.modal_type;
    rsx! {
        div {
            class: "modal",
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e: Event<KeyboardData>| async move {
                let key = e.data.code();
                if key == Code::Escape {
                    modal.write().close();
                }
            },
            div { class: "modal-header",
                div { class: "modal-title", {props.title} }
                div { class: "modal-subtitle", {props.subtitle} }
            }
            div { class: "modal-body", {props.body} }
            div { class: "modal-actions",
                for button in props.buttons {
                    {button.build()}
                }
            }
        }
    }
}

#[component]
pub fn Error(modal_type: Signal<ModalType>, message: String, error: String) -> Element {
    rsx! {
        ConfirmationModal {
            modal_type,
            title: "Error",
            subtitle: "{message}",
            body: rsx! {
            "{error}"
            },
            buttons: vec![
                ButtonBuilder::secondary(
                    "Ok",
                    Callback::new(move |_e| {
                        modal_type.write().close();
                    }),
                ),
            ],
        }
    }
}

#[component]
pub fn DeleteConfirm(
    modal_type: Signal<ModalType>,
    vault: Arc<NoteVault>,
    path: VaultPath,
) -> Element {
    let pub_sub: PubSub<GlobalEvent> = use_context();
    let delete_path = path.clone();
    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                modal_type.write().close();
            }),
        ),
        ButtonBuilder::danger(
            "Delete",
            Callback::new(move |_e| {
                // We don't want to auto save the note, so we mark it as saved
                pub_sub.publish(GlobalEvent::MarkNoteClean);

                let is_note = delete_path.is_note();
                let delete_result = if is_note {
                    vault.delete_note(&delete_path)
                } else {
                    vault.delete_directory(&delete_path)
                };
                if let Err(e) = delete_result {
                    error!("Error: {}", e);
                    modal_type.write().set_error(
                        format!(
                            "Error deleting {}: {}",
                            if is_note { "note" } else { "directory" },
                            delete_path
                        ),
                        format!("{}", e),
                    );
                } else {
                    pub_sub.publish(GlobalEvent::Deleted(delete_path.clone()));
                    modal_type.write().close();
                }
            }),
        ),
    ];
    rsx! {
        ConfirmationModal {
            modal_type,
            title: "Delete Note",
            subtitle: "Are you sure you want to delete \"{path}\"?",
            body: rsx! { "This action cannot be undone." },
            buttons,
        }
    }
}

#[component]
pub fn MoveConfirm(
    modal_type: Signal<ModalType>,
    vault: Arc<NoteVault>,
    from_path: VaultPath,
) -> Element {
    let pub_sub: PubSub<GlobalEvent> = use_context();
    let is_note = from_path.is_note();
    let to_path = from_path.clone();
    let mut dest_path = use_signal(|| to_path);
    let (current_base_path, current_note_name) = from_path.get_parent_path();
    let list_vault = vault.clone();
    let list_of_paths = use_resource(move || {
        let vault = list_vault.clone();
        async move {
            let mut entries = vec![];
            let (options, receiver) = VaultBrowseOptionsBuilder::new(&VaultPath::root())
                .recursive()
                .no_validation()
                .build();
            let _ = tokio::spawn(async move {
                vault.browse_vault(options).expect("Error fetching Entries");
            })
            .await;
            while let Ok(res) = receiver.recv() {
                if let ResultType::Directory = res.rtype {
                    entries.push(res.path)
                }
            }
            entries.sort();
            entries
        }
    });
    let from = from_path.clone();
    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                modal_type.write().close();
            }),
        ),
        ButtonBuilder::primary(
            "Move",
            Callback::new(move |_e| {
                pub_sub.publish(GlobalEvent::SaveCurrentNote);
                let to = dest_path.read().append(&VaultPath::new(&current_note_name));
                let is_note = from_path.is_note();
                let move_result = if is_note {
                    vault.rename_note(&from_path, &to)
                } else {
                    vault.rename_directory(&from_path, &to)
                };

                if let Err(e) = move_result {
                    error!("Error: {}", e);
                    modal_type.write().set_error(
                        format!(
                            "Error moving {}: {}",
                            if is_note { "note" } else { "directory" },
                            from_path
                        ),
                        format!("{}", e),
                    );
                } else {
                    pub_sub.publish(GlobalEvent::Moved {
                        from: from_path.clone(),
                        to,
                    });

                    modal_type.write().close();
                }
            }),
        ),
    ];
    rsx! {
        ConfirmationModal {
            modal_type,
            title: if is_note { "Move Note" } else { "Move Directory" },
            subtitle: "Moving: \"{from}\"",
            body: rsx! {
                div { class: "dialog-controls",
                    if let Some(paths) = &*list_of_paths.read() {
                        select {
                            class: "select",
                            onchange: move |e| {
                                dest_path.set(VaultPath::new(e.value()));
                            },
                            for path in paths {
                                option { value: "{path}", selected: current_base_path.eq(path), "{path}" }
                            }
                        }
                    } else {
                        div { class: "info-text", "<Loading...>" }
                    }
                }
            },
            buttons,
        }
    }
}

#[component]
pub fn RenameConfirm(
    modal_type: Signal<ModalType>,
    vault: Arc<NoteVault>,
    path: VaultPath,
) -> Element {
    let pub_sub: PubSub<GlobalEvent> = use_context();
    let (current_path, current_name) = path.get_parent_path();
    let mut new_name = use_signal(|| current_name.clone());

    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                modal_type.write().close();
            }),
        ),
        ButtonBuilder::primary(
            "Rename",
            Callback::new(move |_e| {
                pub_sub.publish(GlobalEvent::SaveCurrentNote);
                let to = current_path.append(&VaultPath::new(&*new_name.read()));
                let is_note = path.is_note();
                let move_result = if is_note {
                    vault.rename_note(&path, &to)
                } else {
                    vault.rename_directory(&path, &to)
                };

                if let Err(e) = move_result {
                    error!("Error: {}", e);
                    modal_type.write().set_error(
                        format!(
                            "Error renaming {}: {}",
                            if is_note { "note" } else { "directory" },
                            path
                        ),
                        format!("{}", e),
                    );
                } else {
                    pub_sub.publish(GlobalEvent::Renamed {
                        old_name: path.clone(),
                        new_name: to.clone(),
                    });

                    modal_type.write().close();
                }
            }),
        ),
    ];
    rsx! {
        ConfirmationModal {
            modal_type,
            title: "Rename Note",
            subtitle: "Current name: \"{current_name}\"",
            body: rsx! {
                input {
                    r#type: "text",
                    class: "input",
                    value: "{new_name}",
                    placeholder: "Enter new file name",
                    oninput: move |e| {
                        new_name.set(e.value());
                    },
                }
            },
            buttons,
        }
    }
}

#[component]
pub fn CreateNote(
    modal_type: Signal<ModalType>,
    vault: Arc<NoteVault>,
    from_path: VaultPath,
) -> Element {
    let from_path = if from_path.is_note() {
        from_path.get_parent_path().0
    } else {
        from_path
    };
    let new_from_path = from_path.clone();
    let mut new_note_base_path = use_signal(move || new_from_path);
    let mut new_note_name = use_signal(|| "".to_string());
    let mut new_full_path = use_signal(move || new_note_base_path.read().to_owned());
    let mut is_valid = use_signal(|| false);

    let list_vault = vault.clone();
    let list_of_paths = use_resource(move || {
        let vault = list_vault.clone();
        async move {
            let mut entries = vec![];
            let (options, receiver) = VaultBrowseOptionsBuilder::new(&VaultPath::root())
                .recursive()
                .no_validation()
                .build();
            let _ = tokio::spawn(async move {
                vault.browse_vault(options).expect("Error fetching Entries");
            })
            .await;
            while let Ok(res) = receiver.recv() {
                if let ResultType::Directory = res.rtype {
                    entries.push(res.path)
                }
            }
            entries.sort();
            entries
        }
    });
    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                modal_type.write().close();
            }),
        ),
        ButtonBuilder::primary(
            "Create",
            Callback::new(move |_e| {
                if is_valid() {
                    navigator().replace(Route::MainView {
                        editor_path: new_full_path.read().to_owned(),
                        create: true,
                    });
                    modal_type.write().close();
                }
            }),
        ),
    ];

    let vault_select = vault.clone();
    rsx! {
        ConfirmationModal {
            modal_type,
            title: "Create Note",
            subtitle: "Enter a filename and select a directory for your new note",
            body: rsx! {
                div { class: "dialog-controls",
                    label { "Directory" }
                    if let Some(paths) = &*list_of_paths.read() {
                        select {
                            class: "select",
                            onchange: move |e| {
                                new_note_base_path.set(VaultPath::new(e.value()));
                                let (p, valid) = get_path_is_valid(
                                    vault_select.clone(),
                                    &new_note_base_path.read(),
                                    &*new_note_name.read(),
                                    false,
                                );
                                new_full_path.set(p);
                                is_valid.set(valid);
                            },
                            for path in paths {
                                option {
                                    value: "{path}",
                                    selected: new_note_base_path.read().eq(path),
                                    "{path}"
                                }
                            }
                        }
                    } else {
                        div { class: "info-text", "<Loading...>" }
                    }
                    label { "File Name" }
                    input {
                        r#type: "text",
                        class: "input",
                        value: "{new_note_name}",
                        placeholder: "Enter new file name",
                        oninput: move |e| {
                            new_note_name.set(e.value());
                            let (p, valid) = get_path_is_valid(
                                vault.clone(),
                                &new_note_base_path.read(),
                                &*new_note_name.read(),
                                false,
                            );
                            new_full_path.set(p);
                            is_valid.set(valid);
                        },
                    }
                    label { class: if !&is_valid() { "error" } else { "" }, "New note at: {new_full_path}" }
                }
            },
            buttons,
        }
    }
}

#[component]
pub fn CreateDirectory(
    modal_type: Signal<ModalType>,
    vault: Arc<NoteVault>,
    from_path: VaultPath,
) -> Element {
    let from_path = if from_path.is_note() {
        from_path.get_parent_path().0
    } else {
        from_path
    };

    let pub_sub: PubSub<GlobalEvent> = use_context();
    let base_path = from_path.clone();
    let mut new_directory_base_path = use_signal(move || base_path.clone());
    let mut new_directory_name = use_signal(|| "".to_string());
    let mut is_valid = use_signal(|| false);

    let mut new_full_path = use_signal(move || new_directory_base_path.read().to_owned());

    let list_vault = vault.clone();
    let list_of_paths = use_resource(move || {
        let vault = list_vault.clone();
        async move {
            let mut entries = vec![];
            let (options, receiver) = VaultBrowseOptionsBuilder::new(&VaultPath::root())
                .recursive()
                .no_validation()
                .build();
            let _ = tokio::spawn(async move {
                vault.browse_vault(options).expect("Error fetching Entries");
            })
            .await;
            while let Ok(res) = receiver.recv() {
                if let ResultType::Directory = res.rtype {
                    entries.push(res.path)
                }
            }
            entries.sort();
            entries
        }
    });
    let create_vault = vault.clone();
    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                modal_type.write().close();
            }),
        ),
        ButtonBuilder::primary(
            "Create",
            Callback::new(move |_e| {
                if is_valid() {
                    if let Err(e) = create_vault.create_directory(&new_full_path()) {
                        modal_type
                            .write()
                            .set_error("Error creating new Directory".to_string(), e.to_string());
                    } else {
                        pub_sub.publish(GlobalEvent::NewDirectoryCreated(new_full_path()));
                        modal_type.write().close();
                    }
                }
            }),
        ),
    ];

    let vault_select = vault.clone();
    rsx! {
        ConfirmationModal {
            modal_type,
            title: "Create Directory",
            subtitle: "Enter a directory name and optionally select the base directory",
            body: rsx! {
                div { class: "dialog-controls",
                    label { "Base Directory" }
                    if let Some(paths) = &*list_of_paths.read() {
                        select {
                            class: "select",
                            onchange: move |e| {
                                let bd = VaultPath::new(e.value());
                                new_directory_base_path.set(bd.clone());
                                let (p, valid) = get_path_is_valid(
                                    vault_select.clone(),
                                    &new_directory_base_path.read(),
                                    &*new_directory_name.read(),
                                    true,
                                );
                                new_full_path.set(p);
                                is_valid.set(valid);
                            },
                            for path in paths {
                                option {
                                    value: "{path}",
                                    selected: new_directory_base_path.read().eq(path),
                                    "{path}"
                                }
                            }
                        }
                    } else {
                        div { class: "info-text", "<Loading...>" }
                    }
                    label { "Directory Name" }
                    input {
                        r#type: "text",
                        class: "input",
                        value: "{new_directory_name}",
                        placeholder: "Enter new directory name",
                        oninput: move |e| {
                            new_directory_name.set(e.value());
                            let (p, valid) = get_path_is_valid(
                                vault.clone(),
                                &new_directory_base_path.read(),
                                &*new_directory_name.read(),
                                true,
                            );
                            new_full_path.set(p);
                            is_valid.set(valid);
                        },
                    }
                    label { class: if !&is_valid() { "error" } else { "" }, "New directory at: {new_full_path}" }
                }
            },
            buttons,
        }
    }
}

fn get_path_is_valid<S: AsRef<str>>(
    vault: Arc<NoteVault>,
    base_path: &VaultPath,
    name: S,
    directory: bool,
) -> (VaultPath, bool) {
    let valid = !name.as_ref().is_empty();

    let path = base_path.append(&if directory {
        VaultPath::new(name)
    } else {
        VaultPath::note_path_from(name)
    });
    let valid = if valid {
        vault.exists(&path).is_none()
    } else {
        valid
    };

    (path, valid)
}
