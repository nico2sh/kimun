use std::sync::Arc;

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::components::{button::ButtonBuilder, modal::ModalManager};

pub enum ConfirmationType {
    Delete(VaultPath),
    Move(VaultPath),
    Rename(VaultPath),
}

#[derive(Props, Clone, PartialEq)]
pub struct ConfirmationModalProps {
    modal: Signal<ModalManager>,
    title: String,
    subtitle: String,
    body: Element,
    buttons: Vec<ButtonBuilder>,
}

// General Confirmation Modal
#[component]
pub fn ConfirmationModal(props: ConfirmationModalProps) -> Element {
    let mut modal = props.modal;
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
pub fn DeleteConfirm(
    modal: Signal<ModalManager>,
    vault: Arc<NoteVault>,
    path: VaultPath,
) -> Element {
    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                modal.write().close();
            }),
        ),
        ButtonBuilder::danger(
            "Delete",
            Callback::new(move |_e| {
                modal.write().close();
            }),
        ),
    ];
    rsx! {
        ConfirmationModal {
            modal,
            title: "Delete Note",
            subtitle: "Are you sure you want to delete \"{path}\"?",
            body: rsx! { "This action cannot be undone." },
            buttons,
        }
    }
}

#[component]
pub fn MoveConfirm(
    modal: Signal<ModalManager>,
    vault: Arc<NoteVault>,
    from_path: VaultPath,
) -> Element {
    let to_path = from_path.clone();
    let dest_path = use_signal(|| to_path);
    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                modal.write().close();
            }),
        ),
        ButtonBuilder::primary(
            "Move",
            Callback::new(move |_e| {
                modal.write().close();
            }),
        ),
    ];
    rsx! {
        ConfirmationModal {
            modal,
            title: "Move Note",
            subtitle: "Moving: \"{from_path}\"",
            body: rsx! { "<List of paths>" },
            buttons,
        }
    }
}

#[component]
pub fn RenameConfirm(
    modal: Signal<ModalManager>,
    vault: Arc<NoteVault>,
    path: VaultPath,
) -> Element {
    let current_name = path.get_name();
    let mut new_name = use_signal(|| current_name.clone());
    let buttons = vec![
        ButtonBuilder::secondary(
            "Cancel",
            Callback::new(move |_e| {
                modal.write().close();
            }),
        ),
        ButtonBuilder::primary(
            "Rename",
            Callback::new(move |_e| {
                modal.write().close();
            }),
        ),
    ];
    rsx! {
        ConfirmationModal {
            modal,
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
