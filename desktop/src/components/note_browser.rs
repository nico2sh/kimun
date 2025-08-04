use std::{rc::Rc, sync::Arc};

use dioxus::{
    hooks::use_signal,
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{nfs::VaultPath, NoteVault, ResultType, VaultBrowseOptionsBuilder};

use crate::{
    components::{
        modal::{confirmations::ConfirmationType, ModalType},
        note_select_entry::{NoteSelectEntry, NoteSelectEntryListStatus, RowItem, SortCriteria},
    },
    global_events::{GlobalEvent, PubSub},
    utils::sparse_vector::SparseVector,
};

const NOTE_BROWSER: &str = "note_browser";

#[derive(Clone, Eq, PartialEq)]
pub struct Sort {
    criteria: SortCriteria,
    ascending: bool,
}

impl Sort {
    fn set_criteria(&mut self, criteria: SortCriteria) {
        self.criteria = criteria;
    }

    fn toggle_order(&mut self) {
        self.ascending = !self.ascending;
    }
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            criteria: SortCriteria::FileName,
            ascending: false,
        }
    }
}

#[component]
pub fn NoteBrowser(
    vault: Arc<NoteVault>,
    editor_path: ReadOnlySignal<VaultPath>,
    modal_type: Signal<ModalType>,
    show_browser: Signal<bool>,
) -> Element {
    let browsing_directory = use_signal_sync(move || {
        let np = editor_path.read();
        if np.is_note() {
            np.get_parent_path().0
        } else {
            np.to_owned()
        }
    });

    let mut sort = use_signal(|| Sort::default());

    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut row_mounts = use_signal(SparseVector::<Rc<MountedData>>::new);
    let mut select_by_mouse = use_signal(|| true);

    let mut filter_text = use_signal(|| "".to_string());

    // Since this is a resource that depends on the current_path
    // the entries change every time the current_path is changed
    let vault_fetch = vault.clone();
    let mut all_entries = use_resource(move || {
        let vault = vault_fetch.clone();
        async move {
            info!("Load all entries");
            let mut entries = vec![];
            let (search_options, rx) = VaultBrowseOptionsBuilder::new(&browsing_directory())
                .full_validation()
                .non_recursive()
                .build();
            let browsing_vault = vault.clone();
            let _ = tokio::spawn(async move {
                browsing_vault
                    .browse_vault(search_options)
                    .expect("Error fetching Entries");
            })
            .await;

            while let Ok(entry) = rx.recv() {
                match &entry.rtype {
                    ResultType::Note(note_details) => {
                        let e = if let Some(date) = vault.journal_date(&entry.path) {
                            NoteSelectEntry::from_note_journal(
                                entry.path,
                                note_details.to_owned(),
                                date,
                            )
                        } else {
                            NoteSelectEntry::from_note_details(entry.path, note_details.to_owned())
                        };
                        entries.push(e)
                    }
                    ResultType::Directory => {
                        if entry.path != browsing_directory() {
                            let e = NoteSelectEntry::from_directory_details(
                                entry.path,
                                browsing_directory,
                            );
                            entries.push(e)
                        }
                    }
                    ResultType::Attachment => {
                        // Do nothing
                    }
                };
            }
            entries
        }
    });

    // Filter Entries
    let filtered_entries = use_resource(move || async move {
        info!("Filtering entries");
        let res = if let Some(entries) = &*all_entries.read() {
            if !entries.is_empty() {
                debug!("Filtering {}", filter_text);
                let filtered = entries
                    .iter()
                    .filter_map(|entry| {
                        let entry_text = entry.as_ref().to_lowercase();
                        if entry_text.contains(&filter_text.read().to_lowercase()) {
                            Some(entry.to_owned())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<NoteSelectEntry>>();

                NoteSelectEntryListStatus::Loaded(filtered)
            } else {
                NoteSelectEntryListStatus::Loaded(vec![])
            }
        } else {
            NoteSelectEntryListStatus::Loading
        };
        row_mounts.write().truncate(res.len());
        res
    });

    // Sort Entries
    let sorted_entries = use_resource(move || async move {
        info!("Sorting entries");
        let mut filtered_entries = filtered_entries.read().to_owned();
        if let Some(NoteSelectEntryListStatus::Loaded(result)) = filtered_entries.as_mut() {
            if sort.read().ascending {
                result.sort_by_key(|b| b.sort_string_for(&sort.read().criteria));
            } else {
                result.sort_by_key(|b| std::cmp::Reverse(b.sort_string_for(&sort.read().criteria)));
            };
            if !browsing_directory.read().is_root_or_empty() {
                result.insert(
                    0,
                    NoteSelectEntry::Directory {
                        path: browsing_directory.read().get_parent_path().0,
                        name: "..".to_string(),
                        browse_path_signal: browsing_directory,
                    },
                );
            }
            NoteSelectEntryListStatus::Loaded(result.to_owned())
        } else {
            NoteSelectEntryListStatus::Loading
        }
    });

    let pub_sub: PubSub<GlobalEvent> = use_context();
    let pc = pub_sub.clone();
    use_effect(move || {
        pc.subscribe(
            NOTE_BROWSER,
            Callback::new(move |g| {
                debug!("event: {:?}", g);
                all_entries.restart();
            }),
        );
    });
    use_drop(move || {
        pub_sub.unsubscribe(NOTE_BROWSER);
    });

    let new_note_vault = vault.clone();
    rsx! {
        div { class: "sidebar-header",
            div { class: "sidebar-title", "{browsing_directory}" }
            div { class: "sidebar-header-actions",
                button {
                    class: "sidebar-btn",
                    title: "Create new note",
                    onclick: move |_e| {
                        modal_type
                            .write()
                            .set_confirm(
                                new_note_vault.clone(),
                                ConfirmationType::NewNote(browsing_directory()),
                            );
                    },
                    svg {
                        width: 16,
                        height: 16,
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        path { d: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6M12 13v6M9 16h6" }
                    }
                }
                button {
                    class: "sidebar-btn",
                    title: "Create new directory",
                    onclick: move |_e| {
                        modal_type
                            .write()
                            .set_confirm(
                                vault.clone(),
                                ConfirmationType::NewDirectory(browsing_directory()),
                            );
                    },
                    svg {
                        width: 16,
                        height: 16,
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        path { d: "M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2v11zM12 13v6M9 16h6" }
                    }
                }
            }
        }
        div { class: "sidebar-search",
            input {
                r#type: "text",
                class: "input",
                placeholder: "search",
                value: "{filter_text}",
                oninput: move |e| {
                    filter_text.set(e.value().to_string());
                },
            }
        }
        div { class: "sidebar-controls",
            div { class: "sort-controls",
                select {
                    class: "select",
                    id: "sortBy",
                    onchange: move |e| {
                        let val = e.value();
                        if val.eq("title") {
                            sort.write().set_criteria(SortCriteria::Title);
                        }
                        if val.eq("filename") {
                            sort.write().set_criteria(SortCriteria::FileName);
                        }
                    },
                    option {
                        value: "filename",
                        selected: if sort.read().criteria == SortCriteria::FileName { true },
                        "File Name"
                    }
                    option {
                        value: "title",
                        selected: if sort.read().criteria == SortCriteria::Title { true },
                        "Title"
                    }
                }
                button {
                    class: if sort.read().ascending { "sort-order ascending" } else { "sort-order" },
                    id: "sortOrder",
                    onclick: move |_e| {
                        sort.write().toggle_order();
                    },
                    title { "Toggle sort Order" }
                    svg {
                        width: 14,
                        height: 14,
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        path { d: "M3 6h18M7 12h10M11 18h2" }
                    }
                }
            }
        }
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
            if let Some(NoteSelectEntryListStatus::Loaded(entries)) = sorted_entries
                .value()
                .read()
                .clone()
            {
                for (index , entry) in entries.into_iter().enumerate() {
                    {
                        let entry_path = entry.get_path().to_owned();
                        let slct = selected() == Some(index);
                        let active = entry_path.eq(&*editor_path.read());
                        let vault = vault.clone();
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
                                div {
                                    class: "note-item-content",
                                    onclick: move |e| {
                                        info!("Clicked element");
                                        e.stop_propagation();
                                        let _ = entry.on_select();
                                    },
                                    {entry.get_view()}
                                }
                                if !entry.is_up_dir() && slct {
                                    NoteActions {
                                        vault,
                                        modal_type,
                                        entry_path,
                                        onclick: move |_e| {
                                            info!("Clicked element");
                                            let _ = entry_action.on_select();
                                        },
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "controls",
                    div { class: "info-text", "Loading..." }
                }
            }
        }
    }
}

#[derive(PartialEq, Clone, Props)]
struct NoteActionsProps {
    vault: Arc<NoteVault>,
    modal_type: Signal<ModalType>,
    entry_path: VaultPath,
    onclick: EventHandler<MouseEvent>,
}

#[component]
fn NoteActions(props: NoteActionsProps) -> Element {
    let rename_vault = props.vault.clone();
    let rename_path = props.entry_path.clone();
    let move_vault = props.vault.clone();
    let move_path = props.entry_path.clone();
    let delete_vault = props.vault.clone();

    let mut modal_type = props.modal_type;

    rsx! {
        div {
            class: "note-actions",
            onclick: move |e| {
                props.onclick.call(e);
            },
            button {
                class: "action-btn rename",
                title: "Rename",
                onclick: move |e| {
                    e.stop_propagation();
                    let rename_path = rename_path.clone();
                    modal_type
                        .write()
                        .set_confirm(rename_vault.clone(), ConfirmationType::Rename(rename_path));
                },
                svg {
                    width: 12,
                    height: 12,
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: 2,
                    path { d: "M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z" }
                }
            }
            button {
                class: "action-btn move",
                title: "Move",
                onclick: move |e| {
                    e.stop_propagation();
                    let move_path = move_path.clone();
                    modal_type
                        .write()
                        .set_confirm(move_vault.clone(), ConfirmationType::Move(move_path.clone()));
                },
                svg {
                    width: 12,
                    height: 12,
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: 2,
                    path { d: "M21 9l-9-9-9 9h4v11h10V9h4z" }
                }
            }
            button {
                class: "action-btn delete",
                title: "Delete",
                onclick: move |e| {
                    e.stop_propagation();
                    let delete_path = props.entry_path.clone();
                    modal_type
                        .write()
                        .set_confirm(
                            delete_vault.clone(),
                            ConfirmationType::Delete(delete_path.clone()),
                        );
                },
                svg {
                    width: 12,
                    height: 12,
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: 2,
                    path { d: "M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m3 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" }
                }
            }
        }
    }
}
