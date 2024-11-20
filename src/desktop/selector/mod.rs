mod row_item;

use std::{
    rc::Rc,
    sync::{mpsc::Receiver, Arc, Mutex},
};

use dioxus::prelude::*;
use log::{error, info};

use crate::noters::{
    nfs::{NoteEntry, NotePath},
    NoteVault,
};

#[derive(Clone, Debug)]
pub enum SelectionState {
    Unset,
    Open(NotePath),
    Loading(Rc<Receiver<NoteEntry>>),
    Loaded(Vec<NoteEntry>),
    Filtered {
        entries: Vec<NoteEntry>,
        filtered: Vec<NoteEntry>,
    },
}

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    note_vault: NoteVault,
    state: Signal<SelectionState>,
}

fn open(
    dialog: Signal<Option<Rc<MountedData>>>,
    note_path: NotePath,
    vault: NoteVault,
    state: &mut Signal<SelectionState>,
) {
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

    *state.write() = SelectionState::Loading(Rc::new(rx));
}

fn load_items(rx: Rc<Receiver<NoteEntry>>, state: &mut Signal<SelectionState>) {
    let mut items = vec![];
    while let Ok(row) = rx.recv() {
        if crate::noters::nfs::EntryData::Attachment != row.data {
            items.push(row);
        }
    }
    state.set(SelectionState::Loaded(items));
}

fn filter_items(items: Vec<NoteEntry>, state: &mut Signal<SelectionState>) {
    let filtered = items.clone();
    std::thread::spawn(move || {});
    state.set(SelectionState::Filtered {
        entries: items,
        filtered,
    });
}

#[allow(non_snake_case)]
pub fn Selector(props: SelectorProps) -> Element {
    let mut state = props.state;
    let vault = props.note_vault;
    let mut dialog: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let current_state = state.read().clone();
    let (visible, rows) = match current_state {
        SelectionState::Unset => (false, vec![]),
        SelectionState::Open(note_path) => {
            open(dialog, note_path, vault, &mut state);
            (true, vec![])
        }
        SelectionState::Loading(rx) => {
            load_items(rx, &mut state);
            (true, vec![])
        }
        SelectionState::Loaded(vec) => {
            filter_items(vec, &mut state);
            (true, vec![])
        }
        SelectionState::Filtered {
            entries: _,
            filtered,
        } => (true, filtered),
    };

    rsx! {
        dialog {
            class: "h-48 p-2 rounded-lg shadow",
            open: visible,
            autofocus: "true",
            onkeydown: move |e: Event<KeyboardData>| {
                let key = e.data.code();
                if key == Code::Escape {
                     state.set(SelectionState::Unset);
                }
            },
            div {
                // class: "flex flex-col border-1",
                class: "size-full",
                input {
                    class: "w-full",
                    r#type: "text",
                    onmounted: move |e| {
                        info!("input");
                        *dialog.write() = Some(e.data());
                    },
                    "search"
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
