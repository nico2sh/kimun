use std::{rc::Rc, sync::Arc};

use chrono::Datelike;
use dioxus::{
    hooks::use_signal,
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{nfs::VaultPath, NoteVault, ResultType, VaultBrowseOptionsBuilder};

use crate::{
    components::note_select_entry::{NoteSelectEntry, RowItem, SortCriteria},
    utils::sparse_vector::SparseVector,
};

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
    note_path: ReadOnlySignal<VaultPath>,
    show_browser: Signal<bool>,
) -> Element {
    let browsing_directory = use_signal_sync(move || {
        let np = note_path.read();
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
    let current_path = browsing_directory.read().clone();

    // Since this is a resource that depends on the current_path
    // the entries change every time the current_path is changed
    let all_entries = use_resource(move || {
        let vault = vault.clone();
        async move {
            info!("Load all entries");
            let mut entries = vec![];
            let (search_options, rx) = VaultBrowseOptionsBuilder::new(&*browsing_directory.read())
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
                        if entry.path != *browsing_directory.read() {
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
                filtered
            } else {
                vec![]
            }
        } else {
            vec![]
        };
        row_mounts.write().truncate(res.len());
        res
    });

    let sorted_entries = use_resource(move || async move {
        info!("Sorting entries");
        let mut filtered_entries = filtered_entries.read().to_owned();
        if let Some(result) = filtered_entries.as_mut() {
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
            result.to_owned()
        } else {
            vec![]
        }
    });

    rsx! {
        div { class: "sidebar-header",
            div { class: "sidebar-title", "{current_path.to_string()}" }
            button {
                class: "sidebar-toggle",
                onclick: move |_| {
                    show_browser.set(false);
                },
                svg {
                    width: 16,
                    height: 16,
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: 2,
                    path { d: "M18 6L6 18M6 6l12 12" }
                }
            }
        }
        div { class: "sidebar-search",
            input {
                r#type: "text",
                class: "search-input",
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
                    class: "sort-select",
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
                    class: if sort.read().ascending { "sort-order" } else { "sort-order descending" },
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
                if !*select_by_mouse.read() {
                    select_by_mouse.set(true);
                }
            },
            onmouseleave: move |_e| {
                if *select_by_mouse.read() {
                    selected.set(None);
                }
            },
            if let Some(entries) = sorted_entries.value().read().clone() {
                for (index , entry) in entries.into_iter().enumerate() {
                    div {
                        class: if *selected.read() == Some(index) { "note-item selected" } else { if entry.get_path().eq(&*note_path.read()) {
                            "note-item active"
                        } else {
                            "note-item"
                        } },
                        id: "element-{index}",
                        onmounted: move |e| {
                            row_mounts.write().insert(index, e.data());
                        },
                        onmouseenter: move |_e| {
                            if *select_by_mouse.read() {
                                selected.set(Some(index));
                            }
                        },
                        onclick: move |e| {
                            info!("Clicked element");
                            e.stop_propagation();
                            let _ = entry.on_select()();
                        },
                        {entry.get_view()}
                    }
                }
            } else {
                div { "Loading..." }
            }
        }
    }
}
