mod row_item;

use std::rc::Rc;

use dioxus::prelude::*;
use log::{error, info};
use nucleo::Matcher;

use crate::noters::{
    nfs::{NoteEntry, NotePath},
    NoteVault,
};

#[derive(Clone, Debug)]
pub struct SelectionState {
    filter_text: String,
    note_path: NotePath,
    entries: Vec<NoteEntry>,
    filtered: Vec<NoteEntry>,
    state: LoadState,
}

impl SelectionState {
    pub fn close_dialog() -> Self {
        Self {
            filter_text: "".to_string(),
            note_path: NotePath::root(),
            entries: vec![],
            filtered: vec![],
            state: LoadState::Unset,
        }
    }
    pub fn open_dialog(path: NotePath) -> Self {
        Self {
            filter_text: "".to_string(),
            note_path: path,
            entries: vec![],
            filtered: vec![],
            state: LoadState::Open,
        }
    }
}

#[derive(Clone, Debug)]
pub enum LoadState {
    Unset,
    Open,
    Loaded,
    Filtered,
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    filter_text: String,
    note_vault: NoteVault,
    state: Signal<SelectionState>,
}

fn open(
    dialog: Signal<Option<Rc<MountedData>>>,
    vault: NoteVault,
    state: &mut Signal<SelectionState>,
) {
    let note_path = state.read().note_path.clone();
    let path = if note_path.is_note() {
        note_path.get_parent_path().0
    } else {
        note_path
    };
    let (tx, rx) = std::sync::mpsc::channel();
    spawn(async move {
        loop {
            if let Some(e) = dialog.with(|f| f.clone()) {
                info!("focus input");
                let _ = e.set_focus(true).await;
                break;
            }
        }
    });
    if let Err(e) = vault.get_notes_at(path, tx, true) {
        error!("{}", e);
    }

    let mut items = vec![];
    while let Ok(row) = rx.recv() {
        if crate::noters::nfs::EntryData::Attachment != row.data {
            items.push(row);
        }
    }
    let mut new_state = state.read().clone();
    new_state.entries = items;
    new_state.state = LoadState::Loaded;
    state.set(new_state);
}

fn filter_items(state: &mut Signal<SelectionState>) {
    let current_state = state.read().clone();
    let items = current_state.entries;
    let mut matcher = Matcher::new(nucleo::Config::DEFAULT);
    let filtered = nucleo::pattern::Pattern::parse(
        current_state.filter_text.as_ref(),
        nucleo::pattern::CaseMatching::Ignore,
        nucleo::pattern::Normalization::Smart,
    )
    .match_list(items.clone(), &mut matcher)
    .iter()
    .map(|e| e.0.to_owned())
    .collect::<Vec<NoteEntry>>();
    let mut new_state = state.read().clone();
    new_state.filtered = filtered;
    new_state.state = LoadState::Filtered;
    state.set(new_state);
}

#[allow(non_snake_case)]
pub fn Selector(props: SelectorProps) -> Element {
    let mut dialog: Signal<Option<Rc<MountedData>>> = use_signal(|| None);

    let mut state = props.state;
    let vault = props.note_vault;
    let current_state = state.read().clone();
    let (visible, rows) = match current_state.state {
        LoadState::Unset => (false, vec![]),
        LoadState::Open => {
            open(dialog, vault, &mut state);
            (true, vec![])
        }
        LoadState::Loaded => {
            filter_items(&mut state);
            (true, vec![])
        }
        LoadState::Filtered => (true, current_state.filtered),
    };

    let filter_text = state.read().filter_text.clone();
    rsx! {
        dialog {
            class: "h-48 p-2 rounded-lg shadow",
            open: visible,
            autofocus: "true",
            onkeydown: move |e: Event<KeyboardData>| {
                let key = e.data.code();
                if key == Code::Escape {
                     state.set(SelectionState::close_dialog());
                }
            },
            div {
                // class: "flex flex-col border-1",
                class: "size-full",
                input {
                    class: "w-full",
                    r#type: "search",
                    value: "{filter_text}",
                    onmounted: move |e| {
                        info!("input");
                        *dialog.write() = Some(e.data());
                    },
                    oninput: move |e| {
                        state.write().filter_text = e.value().clone().to_string();
                        filter_items(&mut state);
                    },
                }
                div {
                    class: "h-full overflow-auto",
                    // style: "max-height: 70%",
                    for row in rows.iter() {
                        div {
                            "{row.to_string()}",
                        }
                    }
                }
            }
        }
    }
}
