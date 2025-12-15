use std::rc::Rc;

use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::{
    components::note_select_entry::{NoteSelectEntry, RowItem},
    utils::sparse_vector::SparseVector,
};

#[derive(Clone, PartialEq, Props)]
pub struct NoteListProps<H>
where
    H: HoverElement + Clone + PartialEq + 'static,
{
    entries: Vec<NoteSelectEntry>,
    active_path: VaultPath,
    hover_action: H,
}

pub trait HoverElement {
    fn on_hover(&self, entry: NoteSelectEntry) -> Element;
}

#[component]
pub fn NoteList<H>(props: NoteListProps<H>) -> Element
where
    H: HoverElement + Clone + PartialEq + 'static,
{
    let entries = props.entries;
    let num_entries = entries.len();
    let active_path = props.active_path;
    let hover_action = props.hover_action;

    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut select_by_mouse = use_signal(|| true);
    let mut row_mounts = use_signal(|| SparseVector::<Rc<MountedData>>::with_capacity(num_entries));

    rsx! {
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
                    let entry_path = entry.get_path().to_owned();
                    let slct = selected() == Some(index);
                    let active = entry_path.eq(&active_path);
                    let entry_action = entry.clone();
                    rsx! {
                        div {
                            class: if slct { "note-item selected" } else { if active { "note-item active" } else { "note-item" } },
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
                                info!("Clicked element");
                                e.stop_propagation();
                                let _ = entry_action.on_select();
                            },
                            {entry.get_view()}

                            if slct {
                                {hover_action.on_hover(entry)}
                            }
                        }
                    }
                }
            }
        }
    }
}
