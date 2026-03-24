use std::rc::Rc;

use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::{
    app_state::AppState, components::note_list::note_browse_entry::NoteBrowseEntry,
    settings::AppSettings, utils::sparse_vector::SparseVector,
};

#[derive(Clone, Debug, PartialEq, Props)]
pub struct NotePickerProps {
    note_list: Vec<(String, VaultPath)>,
}

#[component]
pub fn NotePicker(props: NotePickerProps) -> Element {
    let mut app_state: Signal<AppState> = use_context();
    let settings: Signal<AppSettings> = use_context();

    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut select_by_mouse = use_signal(|| true);
    let mut row_mounts = use_signal(SparseVector::<Rc<MountedData>>::new);

    let entries = props
        .note_list
        .iter()
        .map(|(title, path)| NoteBrowseEntry::new_note(path.to_owned(), title.to_owned()))
        .collect::<Vec<NoteBrowseEntry>>();

    let theme = settings().get_theme();
    rsx! {
        div {
            class: "modal note-picker",
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
                                border_bottom_color: "{theme.border_light}",
                                border_left_color: if slct { "{theme.accent_yellow}" } else { "transparent" },
                                background_color: if slct { "{theme.bg_hover}" } else { "transparent" },
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
                                    app_state.write().current_path = entry.get_path().to_owned();
                                    app_state.write().close_modal();
                                },
                                {entry.get_view(&theme)}
                            }
                        }
                    }
                }
            }
        }
    }
}
