use std::rc::Rc;

use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::{
    components::{
        modal::ModalType,
        note_select_entry::{NoteSelectEntry, RowItem},
    },
    utils::sparse_vector::SparseVector,
};

#[derive(Clone, Debug, PartialEq, Props)]
pub struct NotePickerProps {
    modal_type: Signal<ModalType>,
    note_list: Vec<(String, VaultPath)>,
}

#[component]
pub fn NotePicker(props: NotePickerProps) -> Element {
    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut select_by_mouse = use_signal(|| true);
    let mut row_mounts = use_signal(SparseVector::<Rc<MountedData>>::new);
    let mut modal_type = props.modal_type;

    let entries = props
        .note_list
        .iter()
        .map(|(title, path)| NoteSelectEntry::Note {
            path: path.clone(),
            title: title.clone(),
            search_str: title.clone(),
        })
        .collect::<Vec<NoteSelectEntry>>();
    rsx! {
        div { class: "modal note-picker",
            onclick: move |e| e.stop_propagation(),
            div {
                class: "entry-list",
                id: "entryList",
                onmousemove: move |_e| {
                    if !select_by_mouse() {
                        select_by_mouse.set(true);
                    }
                },
                onmouseleave: move |_e| {
                    if select_by_mouse() {
                        selected.set(None);
                    }
                },
                for (index , entry) in entries.into_iter().enumerate() {
                    {
                        let slct = selected() == Some(index);
                        rsx! {
                            div {
                                class: if slct { "note-item selected" } else { "note-item" },
                                id: "element-{index}",
                                onmounted: move |e| {
                                    row_mounts.write().insert(index, e.data());
                                },
                                onmouseenter: move |_e| {
                                    if select_by_mouse() {
                                        selected.set(Some(index));
                                    }
                                },
                                onclick: move |e| {
                                    e.stop_propagation();
                                    let _ = entry.on_select();
                                    modal_type.write().close();
                                },
                                {entry.get_view()}
                            }
                        }
                    }
                }
            }
        }
    }
}
