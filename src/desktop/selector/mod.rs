mod row_item;

use std::rc::Rc;

use dioxus::prelude::*;
use log::{debug, error, info, warn};
use nucleo::Matcher;

use crate::{
    desktop::AppContext,
    noters::{
        nfs::{NoteEntry, NotePath},
        NoteVault,
    },
};

use super::modal::Modal;

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState {
    Closed,
    Open(NotePath),
    Loaded(Vec<NoteEntry>),
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    modal: Signal<Modal>,
    filter_text: String,
}

fn open(note_path: NotePath, vault: &NoteVault) -> Vec<NoteEntry> {
    let path = if note_path.is_note() {
        note_path.get_parent_path().0
    } else {
        note_path
    };
    let (tx, rx) = std::sync::mpsc::channel();
    if let Err(e) = vault.get_notes_at(path, tx, true) {
        error!("{}", e);
    }

    let mut items = vec![];
    while let Ok(row) = rx.recv() {
        if let crate::noters::nfs::EntryData::Note(_) = row.data {
            items.push(row);
        }
    }
    items
}

fn filter_items(items: Vec<NoteEntry>, filter_text: String) -> Vec<NoteEntry> {
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let filtered = nucleo::pattern::Pattern::parse(
        filter_text.as_ref(),
        nucleo::pattern::CaseMatching::Ignore,
        nucleo::pattern::Normalization::Smart,
    )
    .match_list(items.clone(), &mut matcher)
    .iter()
    .map(|e| e.0.to_owned())
    .collect::<Vec<NoteEntry>>();
    filtered
}

#[allow(non_snake_case)]
pub fn Selector(props: SelectorProps) -> Element {
    warn!("Opening Selector");
    let mut modal = props.modal;
    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;
    let mut load_state = use_signal(|| LoadState::Open(NotePath::root()));
    let mut filter_text = use_signal(|| props.filter_text);

    let mut dialog: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let current_state = load_state.read().to_owned();
    let visible = match current_state {
        LoadState::Closed => false,
        LoadState::Open(_note_path) => true,
        LoadState::Loaded(_) => {
            spawn(async move {
                loop {
                    if let Some(e) = dialog.with(|f| f.clone()) {
                        info!("Focus input");
                        let _ = e.set_focus(true).await;
                        break;
                    }
                }
            });
            true
        }
    };

    let _loading_rows = use_resource(move || {
        let vault = vault.clone();
        let current_state = load_state.read().to_owned();
        async move {
            if let LoadState::Open(note_path) = current_state {
                let items = open(note_path.to_owned(), &vault);
                debug!("Loaded {} items", items.len());
                load_state.set(LoadState::Loaded(items));
            }
        }
    });

    // We asynchronously filter
    let rows = use_resource(move || {
        let current_state = load_state.read().to_owned();
        async move {
            if let LoadState::Loaded(items) = current_state {
                // dependencies
                if !items.is_empty() {
                    let filter = filter_text();
                    debug!("filtering {}", filter);
                    filter_items(items, filter)
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        }
    });
    let mut selected = use_signal(|| None);

    rsx! {
        dialog {
            class: "search_modal",
            open: visible,
            autofocus: "true",
            onkeydown: move |e: Event<KeyboardData>| {
                let key = e.data.code();
                if key == Code::Escape {
                    load_state.set(LoadState::Closed);
                    modal.write().close();
                }
                if key == Code::ArrowDown {
                    let r = rows.read().as_ref().map_or_else(|| 0, |e| e.len());
                    let max_items = r;
                    let new_selected = if max_items == 0 {
                        None
                    } else if let Some(ref current_selected) = *selected.read() {
                        let current_selected = current_selected.to_owned();
                        if current_selected < max_items - 1 {
                            Some(current_selected + 1)
                        } else {
                            Some(0)
                        }
                    } else {
                        Some(0)
                    };
                    selected.set(new_selected);
                }
                if key == Code::ArrowUp {
                    let r = rows.read().as_ref().map_or_else(|| 0, |e| e.len());
                    let max_items = r;
                    let new_selected = if max_items == 0 {
                        None
                    } else if let Some(current_selected) = *selected.read() {
                        if current_selected > 0 {
                            Some(current_selected - 1)
                        } else {
                            Some(max_items - 1)
                        }
                    } else {
                        Some(0)
                    };
                    selected.set(new_selected);
                }
            },
            input {
                class: "search_box",
                r#type: "search",
                value: "{filter_text}",
                onmounted: move |e| {
                    info!("input");
                    *dialog.write() = Some(e.data());
                },
                oninput: move |e| {
                    filter_text.set(e.value().clone().to_string());
                },
            }
            div {
                class: "list",
                match &*rows.read() {
                    Some(rows) => rsx! {
                        for row in rows.iter().enumerate() {
                            div {
                                onmouseenter: move |_e| {
                                    *selected.write() = Some(row.0);
                                },
                                class: if *selected.read() == Some(row.0) {
                                    "element selected"
                                } else {
                                    "element"
                                },
                                "{row.1.to_string()}"
                            }
                        }
                    },
                    None =>  rsx! { p { "Loading..." } }
                }
            }
        }
    }
}
