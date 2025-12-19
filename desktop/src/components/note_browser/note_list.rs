use std::rc::Rc;

use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::{
    components::note_select_entry::{NoteBrowseEntry, RowItem},
    utils::sparse_vector::SparseVector,
};

#[derive(Clone, PartialEq, Props)]
pub struct NoteListProps<H>
where
    H: NoteElementActions + Clone + PartialEq + 'static,
{
    entries: Vec<NoteBrowseEntry>,
    active_path: VaultPath,
    element_action: H,
}

pub trait NoteElementActions {
    fn on_hover(&self, entry: NoteBrowseEntry) -> Element;
    fn on_select(&mut self, entry: NoteBrowseEntry);
}

#[component]
pub fn NoteList<H>(props: NoteListProps<H>) -> Element
where
    H: NoteElementActions + Clone + PartialEq + 'static,
{
    let entries = props.entries;
    let num_entries = entries.len();
    let active_path = props.active_path;
    let element_action = props.element_action;

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
                    let mut element_click = element_action.clone();
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
                                element_click.on_select(entry_action.clone());
                            },
                            {entry.get_view()}
                        
                            if slct {
                                {element_action.on_hover(entry)}
                            }
                        }
                    }
                }
            }
        }
    }
}
