use std::sync::Arc;

use dioxus::{
    core::use_drop,
    hooks::use_signal,
    logger::tracing::{debug, info},
    prelude::*,
};
use kimun_core::{nfs::VaultPath, NoteVault, ResultType, VaultBrowseOptionsBuilder};

use crate::{
    app_state::AppState,
    components::{
        focus_manager::FocusComponent,
        modal::confirmations::ConfirmationType,
        note_list::{
            note_browse_entry::{NoteBrowseEntry, NoteEntryType, SortCriteria},
            note_list_loader::{use_note_list, SelectorFunctions, UseNoteList},
            NoteElementActions, NoteList, SelectorHandler,
        },
        search_box::SearchBox,
    },
    global_events::{GlobalEvent, PubSub},
    settings::AppSettings,
};

use super::focus_manager::FocusManager;

const NOTE_BROWSER: &str = "note_browser";

#[component]
pub fn NoteBrowser(vault: Arc<NoteVault>, editor_path: ReadSignal<VaultPath>) -> Element {
    let mut app_state: Signal<AppState> = use_context();
    let settings: Signal<AppSettings> = use_context();
    let theme = settings().get_theme();

    let browsing_directory = use_signal_sync(move || {
        let np = editor_path.read();
        if np.is_note() {
            np.get_parent_path().0
        } else {
            np.to_owned()
        }
    });
    let focus_manager = use_context::<FocusManager>();

    let filter_text = use_signal(|| "".to_string());
    let sort_criteria = use_signal(|| SortCriteria::FileName);
    let sort_ascending = use_signal(|| false);

    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut select_by_mouse = use_signal(|| true);

    let use_note_list = use_note_list(
        filter_text,
        sort_criteria,
        sort_ascending,
        BrowseFuncions {
            vault: vault.clone(),
            browsing_directory,
        },
        move |mut r| {
            if !browsing_directory.read().is_root_or_empty() {
                r.insert(
                    0,
                    NoteBrowseEntry::up_dir_from(browsing_directory()).with_style_icon(),
                );
            }
            r
        },
    );
    let selector_handler = SelectorHandler::build(use_note_list.display_data.clone());

    // Extract state signal before moving use_note_list
    let list_state = use_note_list.state;

    let pub_sub: PubSub<GlobalEvent> = use_context();
    let pc = pub_sub.clone();
    use_effect(move || {
        pc.subscribe(
            NOTE_BROWSER,
            Callback::new(move |g| {
                debug!("event: {:?}", g);
                // all_entries.restart();
            }),
        );
    });
    let fm = focus_manager.clone();
    use_drop(move || {
        fm.unregister_focus(FocusComponent::BrowseSearch);
        pub_sub.unsubscribe(NOTE_BROWSER);
    });

    let new_note_vault = vault.clone();
    let entries = use_note_list.display_data;
    rsx! {
        div {
            class: "sidebar-header",
            background: "{theme.bg_surface}",
            color: "{theme.text_primary}",
            border_bottom: "{theme.border_light}",
            div { class: "sidebar-title", "{browsing_directory}" }
            div { class: "sidebar-header-actions",
                button {
                    class: "sidebar-btn",
                    color: "{theme.text_primary}",
                    title: "Create new note",
                    onclick: move |_e| {
                        app_state
                            .write()
                            .get_modal_mut()
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
                    color: "{theme.text_primary}",
                    title: "Create new directory",
                    onclick: move |_e| {
                        app_state
                            .write()
                            .get_modal_mut()
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
        div {
            class: "sidebar-search",
            background_color: "{theme.bg_surface}",
            border_bottom_color: "{theme.border_light}",
            SearchBox {
                search_text: filter_text,
                sort_criteria,
                sort_ascending,
                input_focus: FocusComponent::BrowseSearch,
                no_default: true,
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
            {
                let vault = vault.clone();
                rsx! {
                    NoteList {
                        entries,
                        active_path: editor_path.read().to_owned(),
                        element_action: NoteBrowserHover {
                            vault,
                            current_browse_path: browsing_directory,
                            use_note_list,
                        },
                        selector_handler,
                        load_state: list_state,
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct NoteBrowserHover {
    vault: Arc<NoteVault>,
    current_browse_path: SyncSignal<VaultPath>,
    use_note_list: UseNoteList,
}

impl NoteElementActions for NoteBrowserHover {
    fn on_hover(&self, entry: &NoteBrowseEntry) -> Element {
        let vault = self.vault.clone();
        let entry_path = entry.get_path().to_owned();
        rsx! {
            if !entry.is_up_dir() {
                NoteActions {
                    vault,
                    entry_path,
                    onclick: move |_e| {
                        info!("Clicked element");
                    },
                }
            }
        }
    }

    fn on_select(&mut self, entry: &NoteBrowseEntry) {
        let mut app_state: Signal<AppState> = use_context();
        match &entry.e_type {
            NoteEntryType::Note {
                title: _,
                search_str: _,
            }
            | NoteEntryType::Journal {
                title: _,
                date_string: _,
                search_str: _,
            } => app_state.write().current_path = entry.path.to_owned(),
            NoteEntryType::Directory { name: _ } => {
                self.current_browse_path.set(entry.path.to_owned());
                self.use_note_list.reset();
            }
            NoteEntryType::Create { name: _ } => {
                warn!("No Create should happen here");
            }
        }
    }
}

#[derive(PartialEq, Clone, Props)]
struct NoteActionsProps {
    vault: Arc<NoteVault>,
    entry_path: VaultPath,
    onclick: EventHandler<MouseEvent>,
}

#[component]
fn NoteActions(props: NoteActionsProps) -> Element {
    let mut app_state: Signal<AppState> = use_context();
    let settings: Signal<AppSettings> = use_context();

    let rename_vault = props.vault.clone();
    let rename_path = props.entry_path.clone();
    let move_vault = props.vault.clone();
    let move_path = props.entry_path.clone();
    let delete_vault = props.vault.clone();

    let theme = settings().get_theme();

    let mut hover_button_num = use_signal(|| 0);

    rsx! {
        div {
            class: "note-actions",
            onclick: move |e| {
                props.onclick.call(e);
            },
            button {
                class: "action-btn rename",
                color: "{theme.text_light}",
                title: "Rename",
                background_color: if hover_button_num() == 1 { "{theme.accent_blue}" } else { "{theme.bg_section}" },
                border_color: if hover_button_num() == 1 { "{theme.border_hover}" } else { "{theme.border_light}" },
                onmouseover: move |_e| hover_button_num.set(1),
                onmouseleave: move |_e| hover_button_num.set(0),
                onclick: move |e| {
                    e.stop_propagation();
                    let rename_path = rename_path.clone();
                    app_state
                        .write()
                        .get_modal_mut()
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
                border_color: "{theme.border_light}",
                color: "{theme.text_light}",
                title: "Move",
                background_color: if hover_button_num() == 2 { "{theme.accent_yellow}" } else { "{theme.bg_section}" },
                border_color: if hover_button_num() == 2 { "{theme.border_hover}" } else { "{theme.border_light}" },
                onmouseover: move |_e| hover_button_num.set(2),
                onmouseleave: move |_e| hover_button_num.set(0),
                onclick: move |e| {
                    e.stop_propagation();
                    let move_path = move_path.clone();
                    app_state
                        .write()
                        .get_modal_mut()
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
                border_color: "{theme.border_light}",
                color: "{theme.text_light}",
                title: "Delete",
                background_color: if hover_button_num() == 3 { "{theme.accent_red}" } else { "{theme.bg_section}" },
                border_color: if hover_button_num() == 3 { "{theme.border_hover}" } else { "{theme.border_light}" },
                onmouseover: move |_e| hover_button_num.set(3),
                onmouseleave: move |_e| hover_button_num.set(0),
                onclick: move |e| {
                    e.stop_propagation();
                    let delete_path = props.entry_path.clone();
                    app_state
                        .write()
                        .get_modal_mut()
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

#[derive(Clone)]
struct BrowseFuncions {
    vault: Arc<NoteVault>,
    browsing_directory: SyncSignal<VaultPath>,
}

impl SelectorFunctions for BrowseFuncions {
    async fn init(&self) -> Vec<NoteBrowseEntry> {
        info!("Load all entries from path {}", self.browsing_directory);
        let mut entries = vec![];
        let (search_options, rx) = VaultBrowseOptionsBuilder::new(&self.browsing_directory.read())
            .full_validation()
            .non_recursive()
            .build();
        let browsing_vault = self.vault.clone();
        browsing_vault
            .browse_vault(search_options)
            .await
            .expect("Error fetching Entries");

        while let Ok(entry) = rx.recv() {
            match &entry.rtype {
                ResultType::Note(note_details) => {
                    let e = if let Some(date) = self.vault.journal_date(&entry.path) {
                        NoteBrowseEntry::from_note_journal(
                            entry.path,
                            note_details.to_owned(),
                            date,
                        )
                        .with_style_icon()
                    } else {
                        NoteBrowseEntry::from_note_details(entry.path, note_details.to_owned())
                            .with_style_icon()
                    };
                    entries.push(e)
                }
                ResultType::Directory => {
                    if entry.path != *self.browsing_directory.read() {
                        let e =
                            NoteBrowseEntry::from_directory_details(entry.path).with_style_icon();
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

    async fn filter(
        &self,
        filter_text: String,
        initial_items: &[NoteBrowseEntry],
    ) -> Vec<NoteBrowseEntry> {
        info!("Filtering entries");
        if !initial_items.is_empty() {
            debug!("Filtering {}", filter_text);
            let filtered = initial_items
                .iter()
                .filter_map(|entry| {
                    let entry_text = entry.as_ref().to_lowercase();
                    if entry_text.contains(&filter_text.to_lowercase()) {
                        Some(entry.to_owned())
                    } else {
                        None
                    }
                })
                .collect::<Vec<NoteBrowseEntry>>();

            filtered
        } else {
            vec![]
        }
    }
}
