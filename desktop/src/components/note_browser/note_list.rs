use std::rc::Rc;

use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::{components::note_browse_entry::NoteBrowseEntry, utils::sparse_vector::SparseVector};

#[derive(Clone, PartialEq, Props)]
pub struct NoteListProps<H>
where
    H: NoteElementActions + Clone + PartialEq + 'static,
{
    entries: Signal<Vec<NoteBrowseEntry>>,
    active_path: VaultPath,
    element_action: H,
    selector_handler: SelectorHandler,
    #[props(default = false)]
    compact: bool,
}

pub trait NoteElementActions: Clone + PartialEq {
    fn on_hover(&self, entry: &NoteBrowseEntry) -> Element;
    fn on_select(&mut self, entry: &NoteBrowseEntry);
}

#[derive(Clone, PartialEq)]
pub struct SelectorHandler {
    entries: Signal<Vec<NoteBrowseEntry>>,
    selected: Signal<Option<usize>>,
    manually_selected: Signal<usize>,
}

impl SelectorHandler {
    pub fn build(entries: Signal<Vec<NoteBrowseEntry>>) -> Self {
        Self {
            entries,
            selected: use_signal(|| None),
            manually_selected: use_signal(|| 0),
        }
    }

    pub fn set_selected(&self, value: Option<usize>) {
        let mut selected = self.selected;
        *selected.write() = value;
    }

    pub fn get_selected(&self) -> Option<usize> {
        self.selected.read().to_owned()
    }

    pub fn select_next(&self) {
        let max_items = self.entries.peek().len();
        let new_selected = if max_items == 0 {
            None
        } else if let Some(ref current_selected) = self.get_selected() {
            let current_selected = current_selected.to_owned();
            if current_selected < max_items - 1 {
                Some(current_selected + 1)
            } else {
                Some(0)
            }
        } else {
            Some(0)
        };
        if let Some(sel) = new_selected {
            let mut manually_selected = self.manually_selected;
            manually_selected.set(sel);
        }
        self.set_selected(new_selected);
    }

    pub fn select_prev(&self) {
        let max_items = self.entries.peek().len();
        let new_selected = if max_items == 0 {
            None
        } else if let Some(current_selected) = self.get_selected() {
            if current_selected > 0 {
                Some(current_selected - 1)
            } else {
                Some(max_items - 1)
            }
        } else {
            Some(0)
        };
        if let Some(sel) = new_selected {
            let mut manually_selected = self.manually_selected;
            manually_selected.set(sel);
        }
        self.set_selected(new_selected);
    }
}

#[component]
pub fn NoteList<H>(props: NoteListProps<H>) -> Element
where
    H: NoteElementActions + Clone + PartialEq + 'static,
{
    let selector_handler = props.selector_handler;
    let entries = props.entries;

    let num_entries = props.entries.len();
    let active_path: VaultPath = props.active_path;
    let element_action = props.element_action;

    let mut select_by_mouse = use_signal(|| true);
    let mut row_mounts = use_signal(|| SparseVector::<Rc<MountedData>>::with_capacity(num_entries));

    _ = use_resource(move || async move {
        let r = selector_handler.manually_selected.read().to_owned();
        if let Some(mount) = row_mounts().get(r) {
            let _a = mount.scroll_to(ScrollBehavior::Smooth).await;
            select_by_mouse.set(false);
        }
    });

    let item_class = if props.compact {
        "note-item-compact"
    } else {
        "note-item"
    };

    let selector_mouse = selector_handler.clone();
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
                    selector_mouse.set_selected(None);
                }
            },
            for (index , entry) in entries().iter().enumerate() {
                {
                    let entry_path = entry.get_path().to_owned();
                    let slct = selector_handler.get_selected() == Some(index);
                    let active = entry_path.eq(&active_path);
                    let entry_action = entry.clone();
                    let mut element_click = element_action.clone();
                    let cls = format!(
                        "{item_class}{}",
                        if slct { " selected" } else { if active { " active" } else { "" } },
                    );
                    let selector_handler = selector_handler.clone();
                    rsx! {
                        div {
                            class: "{cls}",
                            id: "element-{index}",
                            onmounted: move |e| {
                                row_mounts.write().insert(index, e.data());
                            },
                            onmouseenter: move |_e| {
                                if select_by_mouse() {
                                    selector_handler.set_selected(Some(index));
                                }
                            },
                            onclick: move |e| {
                                info!("Clicked element");
                                e.stop_propagation();
                                element_click.on_select(&entry_action);
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
