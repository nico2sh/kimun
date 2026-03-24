use std::sync::Arc;

use dioxus::{logger::tracing::error, prelude::*};
use kimun_core::{nfs::VaultPath, NoteVault, ResultType, VaultBrowseOptionsBuilder};

use crate::{
    app_state::AppState,
    components::button::ButtonBuilder,
    global_events::{GlobalEvent, PubSub},
    settings::AppSettings,
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
    title: String,
    subtitle: String,
    body: Element,
    buttons: Vec<ButtonBuilder>,
}

// General Confirmation Modal
#[component]
pub fn ConfirmationModal(props: ConfirmationModalProps) -> Element {
    let mut app_state: Signal<AppState> = use_context();
    let settings: Signal<AppSettings> = use_context();

    let theme = settings().get_theme();

    rsx! {
        div {
            class: "modal",
            background_color: "{theme.bg_main}",
            border_color: "{theme.border_light}",
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e: Event<KeyboardData>| async move {
                let key = e.data.code();
                if key == Code::Escape {
                    app_state.write().close_modal();
                }
            },
            div { class: "modal-header",
                div { class: "modal-title", color: "{theme.text_primary}", {props.title} }
                div { class: "modal-subtitle", color: "{theme.text_light}", {props.subtitle} }
            }
            div { class: "modal-body", {props.body} }
            div { class: "modal-actions",
                for button in props.buttons {
                    {button.build(&theme)}
                }
            }
        }
    }
}

#[component]
pub fn Error(message: String, error: String) -> Element {
    let mut app_state: Signal<AppState> = use_context();

    rsx! {
        ConfirmationModal {
            title: "Error",
            subtitle: "{message}",
            body: rsx! { "{error}" },
            buttons: vec![
                ButtonBuilder::secondary(
                    "Ok",
                    Callback::new(move |_e| {
                        app_state.write().close_modal();
                    }),
                ),
            ],
        }
    }
}

#[component]
pub fn DeleteConfirm(vault: Arc<NoteVault>, path: VaultPath) -> Element {
    let mut app_state: Signal<AppState> = use_context();

    let pub_sub: PubSub<GlobalEvent> = use_context();
    let delete_path = path.clone();
    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                app_state.write().close_modal();
            }),
        ),
        ButtonBuilder::danger(
            "Delete",
            Callback::new(move |_e| {
                // We don't want to auto save the note, so we mark it as saved
                pub_sub.publish(GlobalEvent::MarkNoteClean);

                let is_note = delete_path.is_note();
                let vault = vault.clone();
                let delete_path = delete_path.clone();
                let pub_sub = pub_sub.clone();

                spawn(async move {
                    let delete_result = if is_note {
                        vault.delete_note(&delete_path).await
                    } else {
                        vault.delete_directory(&delete_path).await
                    };
                    if let Err(e) = delete_result {
                        error!("Error: {}", e);
                        app_state.write().get_modal_mut().set_error(
                            format!(
                                "Error deleting {}: {}",
                                if is_note { "note" } else { "directory" },
                                delete_path
                            ),
                            format!("{}", e),
                        );
                    } else {
                        pub_sub.publish(GlobalEvent::Deleted(delete_path.clone()));
                        app_state.write().close_modal();
                    }
                });
            }),
        ),
    ];
    rsx! {
        ConfirmationModal {
            title: "Delete Note",
            subtitle: "Are you sure you want to delete \"{path}\"?",
            body: rsx! { "This action cannot be undone." },
            buttons,
        }
    }
}

#[component]
pub fn MoveConfirm(vault: Arc<NoteVault>, from_path: VaultPath) -> Element {
    let mut app_state: Signal<AppState> = use_context();

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
                vault
                    .browse_vault(options)
                    .await
                    .expect("Error fetching Entries");
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
                app_state.write().close_modal();
            }),
        ),
        ButtonBuilder::primary(
            "Move",
            Callback::new(move |_e| {
                pub_sub.publish(GlobalEvent::SaveCurrentNote);
                let to = dest_path.read().append(&VaultPath::new(&current_note_name));
                let is_note = from_path.is_note();
                let vault = vault.clone();
                let from_path = from_path.clone();
                let pub_sub = pub_sub.clone();

                spawn(async move {
                    let move_result = if is_note {
                        vault.rename_note(&from_path, &to).await
                    } else {
                        vault.rename_directory(&from_path, &to).await
                    };

                    if let Err(e) = move_result {
                        error!("Error: {}", e);
                        app_state.write().get_modal_mut().set_error(
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

                        app_state.write().close_modal();
                    }
                });
            }),
        ),
    ];
    let settings: Signal<AppSettings> = use_context();
    let theme = settings().get_theme();
    let mut select_focused = use_signal(|| false);

    rsx! {
        ConfirmationModal {
            title: if is_note { "Move Note" } else { "Move Directory" },
            subtitle: "Moving: \"{from}\"",
            body: rsx! {
                div { class: "dialog-controls",
                    if let Some(paths) = &*list_of_paths.read() {
                        select {
                            class: "select",
                            border_color: if select_focused() { "{theme.text_primary}" } else { "{theme.border_light}" },
                            onfocusin: move |_e| select_focused.set(true),
                            onfocusout: move |_e| select_focused.set(false),
                            background_color: "{theme.bg_section}",
                            color: "{theme.text_primary}",
                            onchange: move |e| {
                                dest_path.set(VaultPath::new(e.value()));
                            },
                            for path in paths {
                                option { value: "{path}", selected: current_base_path.eq(path), "{path}" }
                            }
                        }
                    } else {
                        div { class: "info-text", color: "{theme.text_light}", "<Loading...>" }
                    }
                }
            },
            buttons,
        }
    }
}

#[component]
pub fn RenameConfirm(vault: Arc<NoteVault>, path: VaultPath) -> Element {
    let mut app_state: Signal<AppState> = use_context();
    let pub_sub: PubSub<GlobalEvent> = use_context();

    let (current_path, current_name) = path.get_parent_path();
    let mut new_name = use_signal(|| current_name.clone());

    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                app_state.write().close_modal();
            }),
        ),
        ButtonBuilder::primary(
            "Rename",
            Callback::new(move |_e| {
                pub_sub.publish(GlobalEvent::SaveCurrentNote);
                let to = current_path.append(&VaultPath::new(&*new_name.read()));
                let is_note = path.is_note();
                let vault = vault.clone();
                let path = path.clone();
                let pub_sub = pub_sub.clone();

                spawn(async move {
                    let move_result = if is_note {
                        vault.rename_note(&path, &to).await
                    } else {
                        vault.rename_directory(&path, &to).await
                    };

                    if let Err(e) = move_result {
                        error!("Error: {}", e);
                        app_state.write().get_modal_mut().set_error(
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

                        app_state.write().close_modal();
                    }
                });
            }),
        ),
    ];
    let settings: Signal<AppSettings> = use_context();
    let theme = settings().get_theme();
    let mut input_focused = use_signal(|| false);

    rsx! {
        ConfirmationModal {
            title: "Rename Note",
            subtitle: "Current name: \"{current_name}\"",
            body: rsx! {
                input {
                    r#type: "text",
                    class: "input",
                    border_color: if input_focused() { "{theme.text_primary}" } else { "{theme.border_light}" },
                    onfocusin: move |_e| input_focused.set(true),
                    onfocusout: move |_e| input_focused.set(false),
                    background_color: "{theme.bg_main}",
                    color: "{theme.text_primary}",
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
pub fn CreateNote(vault: Arc<NoteVault>, from_path: VaultPath) -> Element {
    let mut app_state: Signal<AppState> = use_context();

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
                vault
                    .browse_vault(options)
                    .await
                    .expect("Error fetching Entries");
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
                app_state.write().close_modal();
            }),
        ),
        ButtonBuilder::primary(
            "Create",
            Callback::new(move |_e| {
                if is_valid() {
                    app_state.write().set_path(&new_full_path.read(), true);
                    app_state.write().close_modal();
                }
            }),
        ),
    ];

    let vault_select = vault.clone();
    let settings: Signal<AppSettings> = use_context();
    let theme = settings().get_theme();
    let mut select_focused = use_signal(|| false);
    let mut input_focused = use_signal(|| false);

    rsx! {
        ConfirmationModal {
            title: "Create Note",
            subtitle: "Enter a filename and select a directory for your new note",
            body: rsx! {
                div { class: "dialog-controls",
                    label { color: "{theme.text_light}", "Directory" }
                    if let Some(paths) = &*list_of_paths.read() {
                        select {
                            class: "select",
                            background_color: "{theme.bg_section}",
                            color: "{theme.text_primary}",
                            border_color: if select_focused() { "{theme.text_primary}" } else { "{theme.border_light}" },
                            onfocusin: move |_e| select_focused.set(true),
                            onfocusout: move |_e| select_focused.set(false),
                            onchange: move |e: Event<FormData>| {
                                let vault = vault_select.clone();
                                async move {
                                    new_note_base_path.set(VaultPath::new(e.value()));
                                    let base = new_note_base_path.read().clone();
                                    let name = new_note_name.read().clone();
                                    let (p, valid) = get_path_is_valid(
                                        vault,
                                        &base,
                                        &name,
                                        false,
                                    ).await;
                                    new_full_path.set(p);
                                    is_valid.set(valid);
                                }
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
                        div { class: "info-text", color: "{theme.text_light}", "<Loading...>" }
                    }
                    label { color: "{theme.text_light}", "File Name" }
                    input {
                        r#type: "text",
                        class: "input",
                        border_color: if input_focused() { "{theme.text_primary}" } else { "{theme.border_light}" },
                        onfocusin: move |_e| input_focused.set(true),
                        onfocusout: move |_e| input_focused.set(false),
                        background_color: "{theme.bg_main}",
                        color: "{theme.text_primary}",
                        value: "{new_note_name}",
                        placeholder: "Enter new file name",
                        oninput: move |e: Event<FormData>| {
                            let vault = vault.clone();
                            async move {
                                new_note_name.set(e.value());
                                let base = new_note_base_path.read().clone();
                                let name = new_note_name.read().clone();
                                let (p, valid) = get_path_is_valid(
                                    vault,
                                    &base,
                                    &name,
                                    false,
                                ).await;
                                new_full_path.set(p);
                                is_valid.set(valid);
                            }
                        },
                    }
                    label { color: if !&is_valid() { "{theme.accent_red}" } else { "{theme.text_light}" },
                        "New note at: {new_full_path}"
                    }
                }
            },
            buttons,
        }
    }
}

#[component]
pub fn CreateDirectory(vault: Arc<NoteVault>, from_path: VaultPath) -> Element {
    let mut app_state: Signal<AppState> = use_context();

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
                vault
                    .browse_vault(options)
                    .await
                    .expect("Error fetching Entries");
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
                app_state.write().close_modal();
            }),
        ),
        ButtonBuilder::primary(
            "Create",
            Callback::new(move |_e| {
                let create_vault = create_vault.clone();
                let pub_sub = pub_sub.clone();
                async move {
                    if is_valid() {
                        let new_path = new_full_path();
                        if let Err(e) = create_vault.create_directory(&new_path).await {
                            app_state.write().get_modal_mut().set_error(
                                "Error creating new Directory".to_string(),
                                e.to_string(),
                            );
                        } else {
                            pub_sub.publish(GlobalEvent::NewDirectoryCreated(new_path));
                            app_state.write().close_modal();
                        }
                    }
                }
            }),
        ),
    ];

    let vault_select = vault.clone();
    let settings: Signal<AppSettings> = use_context();
    let theme = settings().get_theme();

    let mut select_focused = use_signal(|| false);
    let mut input_focused = use_signal(|| false);
    rsx! {
        ConfirmationModal {
            title: "Create Directory",
            subtitle: "Enter a directory name and optionally select the base directory",
            body: rsx! {
                div { class: "dialog-controls",
                    label { color: "{theme.text_light}", "Base Directory" }
                    if let Some(paths) = &*list_of_paths.read() {
                        select {
                            class: "select",
                            background_color: "{theme.bg_section}",
                            color: "{theme.text_primary}",
                            border_color: if select_focused() { "{theme.text_primary}" } else { "{theme.border_light}" },
                            onfocusin: move |_e| select_focused.set(true),
                            onfocusout: move |_e| select_focused.set(false),
                            onchange: move |e: Event<FormData>| {
                                let vault = vault_select.clone();
                                async move {
                                    let bd = VaultPath::new(e.value());
                                    new_directory_base_path.set(bd.clone());
                                    let base = new_directory_base_path.read().clone();
                                    let name = new_directory_name.read().clone();
                                    let (p, valid) = get_path_is_valid(
                                        vault,
                                        &base,
                                        &name,
                                        true,
                                    ).await;
                                    new_full_path.set(p);
                                    is_valid.set(valid);
                                }
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
                        div { class: "info-text", color: "{theme.text_light}", "<Loading...>" }
                    }
                    label { color: "{theme.text_light}", "Directory Name" }
                    input {
                        r#type: "text",
                        class: "input",
                        border_color: if input_focused() { "{theme.text_primary}" } else { "transparent" },
                        onfocusin: move |_e| input_focused.set(true),
                        onfocusout: move |_e| input_focused.set(false),
                        background_color: "{theme.bg_main}",
                        color: "{theme.text_primary}",
                        value: "{new_directory_name}",
                        placeholder: "Enter new directory name",
                        oninput: move |e: Event<FormData>| {
                            let vault = vault.clone();
                            async move {
                                new_directory_name.set(e.value());
                                let base = new_directory_base_path.read().clone();
                                let name = new_directory_name.read().clone();
                                let (p, valid) = get_path_is_valid(
                                    vault,
                                    &base,
                                    &name,
                                    true,
                                ).await;
                                new_full_path.set(p);
                                is_valid.set(valid);
                            }
                        },
                    }
                    label { color: if !&is_valid() { "{theme.accent_red}" } else { "{theme.text_light}" },
                        "New directory at: {new_full_path}"
                    }
                }
            },
            buttons,
        }
    }
}

async fn get_path_is_valid<S: AsRef<str>>(
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
        vault.exists(&path).await.is_none()
    } else {
        valid
    };

    (path, valid)
}
