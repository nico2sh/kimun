use std::sync::Arc;

use dioxus::{logger::tracing::error, prelude::*};
use kimun_core::{nfs::VaultPath, NoteVault, ResultType, VaultBrowseOptionsBuilder};

use crate::{
    components::{button::ButtonBuilder, modal::ModalType},
    global_events::{GlobalEvent, PubSub},
};

pub enum ConfirmationType {
    Delete(VaultPath),
    Move(VaultPath),
    Rename(VaultPath),
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
    let pub_sub: Signal<PubSub> = use_context();
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
                pub_sub.read().publish(GlobalEvent::MarkNoteClean);

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
                    pub_sub
                        .read()
                        .publish(GlobalEvent::Deleted(delete_path.clone()));
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
    let pub_sub: Signal<PubSub> = use_context();
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
                pub_sub.read().publish(GlobalEvent::SaveCurrentNote);
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
                    pub_sub.read().publish(GlobalEvent::Moved {
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
                div { class: "controls",
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
    let pub_sub: Signal<PubSub> = use_context();
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
                pub_sub.read().publish(GlobalEvent::SaveCurrentNote);
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
                    pub_sub.read().publish(GlobalEvent::Renamed {
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
