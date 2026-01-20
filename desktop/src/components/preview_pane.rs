use std::{fmt::Display, sync::Arc};

use dioxus::prelude::*;
use kimun_core::{nfs::VaultPath, note::MarkdownNote, NoteVault};

use crate::{
    app_state::{AppState, PreviewListState},
    components::{
        focus_manager::FocusComponent,
        note_list::{
            note_browse_entry::NoteBrowseEntry,
            note_list_loader::{no_op, use_note_list, SelectorFunctions},
            NoteElementActions, NoteList, SelectorHandler,
        },
        preview::Markdown,
        search_box::{SearchBox, StringSearch},
    },
    settings::AppSettings,
};

#[derive(Clone, PartialEq, Debug)]
pub enum PreviewListSource {
    FromQuery(String),
    FromList(String, Vec<NoteBrowseEntry>),
}

impl Default for PreviewListSource {
    fn default() -> Self {
        Self::FromQuery("".to_string())
    }
}

impl StringSearch for PreviewListSource {
    fn change_value(&mut self, value: String) {
        *self = PreviewListSource::FromQuery(value);
    }
}

impl Display for PreviewListSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                PreviewListSource::FromList(query, _items) => query,
                PreviewListSource::FromQuery(query) => query,
            }
        )
    }
}

enum PreviewContent {
    None,
    Note(MarkdownNote),
    Err(String),
}

#[derive(Clone, PartialEq, Props)]
pub struct PreviewPaneProps {
    vault: Arc<NoteVault>,
    #[props(default = PreviewListState::default())]
    initial_state: PreviewListState,
}

#[derive(Clone)]
pub struct PreviewListFunctions {
    pub vault: Arc<NoteVault>,
}

impl SelectorFunctions<PreviewListSource> for PreviewListFunctions {
    fn init(&self) -> Vec<NoteBrowseEntry> {
        vec![]
    }

    fn filter(
        &self,
        filter_text: PreviewListSource,
        _initial_items: &[NoteBrowseEntry],
    ) -> Vec<NoteBrowseEntry> {
        match &filter_text {
            PreviewListSource::FromQuery(_query) => {
                let filter_text = filter_text.to_owned();
                match self.vault.search_notes(filter_text.to_string()) {
                    Ok(res) => res
                        .into_iter()
                        .map(|(entry, content)| {
                            if let Some(date) = self.vault.journal_date(&entry.path) {
                                NoteBrowseEntry::from_note_journal(entry.path, content, date)
                            } else {
                                NoteBrowseEntry::from_note_details(entry.path, content)
                            }
                        })
                        .collect::<Vec<NoteBrowseEntry>>(),
                    Err(e) => {
                        error!("Error searching notes: {}", e);
                        vec![]
                    }
                }
            }
            PreviewListSource::FromList(_query, items) => items.to_owned(),
        }
    }
}

#[component]
pub fn PreviewPane(props: PreviewPaneProps) -> Element {
    let mut app_state: Signal<AppState> = use_context();
    let settings: Signal<AppSettings> = use_context();
    let theme = settings().get_theme();

    let mut show_browser = use_signal(|| true);

    let vault = props.vault;
    let active_path = use_signal(|| VaultPath::root());
    let mut show_search = use_signal(|| false);

    let PreviewListState {
        source: state_source,
        sort_criteria: state_criteria,
        sort_ascending: state_ascending,
    } = props.initial_state;

    let source = use_signal(|| state_source);

    let sort_criteria = use_signal(|| state_criteria);
    let sort_ascending = use_signal(|| state_ascending);

    let functions = PreviewListFunctions {
        vault: vault.clone(),
    };

    let loaded_note_list = use_note_list(source, sort_criteria, sort_ascending, functions, no_op);
    let selector_handler = SelectorHandler::build(loaded_note_list.display_data.clone());
    let entries = loaded_note_list.display_data;

    use_drop(move || {
        debug!("We close the pane");
        app_state
            .write()
            .set_preview_pane_state(PreviewListState::new(
                source(),
                sort_criteria(),
                sort_ascending(),
            ));

        // We cache the results
        // app_state
        //     .write()
        //     .set_preview_pane_state(PreviewListState::new(
        //         PreviewList::FromList(
        //             source().to_string(),
        //             loaded_note_list.display_data.read().to_owned(),
        //         ),
        //         sort_criteria(),
        //         sort_ascending(),
        //     ));
    });

    let preview_vault = vault.clone();
    let preview_content = use_resource(move || {
        let vault_content = preview_vault.clone();
        async move {
            if active_path.read().is_root_or_empty() {
                PreviewContent::None
            } else {
                match vault_content.load_note(&active_path.read()) {
                    Ok(note) => PreviewContent::Note(note.get_markdown_and_links()),
                    Err(e) => PreviewContent::Err(format!("Error loading Note: {}", e)),
                }
            }
        }
    });

    let mut is_title_hovered = use_signal(|| false);
    let mut is_search_hovered = use_signal(|| false);
    rsx! {
        div { class: "bar-preview-header",
            div {
                class: "bar-preview-header-top",
                background_color: "{theme.bg_surface}",
                border_bottom_color: "{theme.border_light}",
                button {
                    class: "bar-preview-title-btn",
                    color: "{theme.text_secondary}",
                    onmouseenter: move |_e| is_title_hovered.set(true),
                    onmouseleave: move |_e| is_title_hovered.set(false),
                    background_color: if is_title_hovered() { "{theme.bg_hover}" } else { "transparent" },
                    onclick: move |_e| show_browser.set(!show_browser()),
                    span { class: "bar-preview-title", "Quick Browser" }
                    svg {
                        class: if show_browser() { "icon" } else { "icon collapsed" },
                        fill: "none",
                        stroke: "currentColor",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            stroke_width: "2",
                            d: "M19 9l-7 7-7-7",
                        }
                    }
                }
                button {
                    class: "bar-preview-search-btn",
                    color: "{theme.text_secondary}",
                    onmouseenter: move |_e| is_search_hovered.set(true),
                    onmouseleave: move |_e| is_search_hovered.set(false),
                    background_color: if is_search_hovered() { "{theme.bg_hover}" } else { "transparent" },
                    onclick: move |_e| {
                        show_search.set(!show_search());
                    },
                    svg {
                        class: "icon",
                        fill: "none",
                        stroke: "currentColor",
                        view_box: "0 0 24 24",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            stroke_width: "2",
                            d: "M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z",
                        }
                    }
                }
            }
        }
        if show_browser() {
            div {
                class: "bar-preview-browser",
                border_top_color: "{theme.border_light}",
                border_bottom_color: "{theme.border_light}",
                {
                    rsx! {
                        NoteList {
                            entries,
                            active_path: active_path.read().to_owned(),
                            element_action: NoHoverAction { active_path },
                            selector_handler,
                            compact: true,
                        }
                    }
                }

            }
        }
        if show_search() {
            {
                rsx! {
                    div {
                        class: "bar-preview-search-popup-overlay",
                        onclick: move |_e| {
                            show_search.set(false);
                        },
                        div {
                            class: "bar-preview-search-popup",
                            onclick: |e| e.stop_propagation(),
                            background_color: "{theme.bg_head}",
                            border_color: "{theme.border_light}",
                            SearchBox {
                                search_text: source,
                                sort_criteria,
                                sort_ascending,
                                input_focus: FocusComponent::PreviewPane,
                                on_keystroke: move |e: Event<KeyboardData>| {
                                    let key = e.data.code();
                                    match key {
                                        Code::Escape | Code::Enter => {
                                            show_search.set(!show_search());
                                        }
                                        _ => {}
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }
        div { class: "bar-preview-content", background_color: "{theme.bg_main}",
            match &*preview_content.read() {
                Some(content) => {
                    match content {
                        PreviewContent::None => rsx! {
                            div { class: "info" }
                        },
                        PreviewContent::Note(markdown_note) => rsx! {
                            Markdown {
                                vault: vault.clone(),
                                note_md: markdown_note.text.clone(),
                                note_links: markdown_note.links.clone(),
                            }
                        },
                        PreviewContent::Err(e) => rsx! {
                            div { class: "info", "{e}" }
                        },
                    }
                }
                None => rsx! {
                    div { class: "info", "Loading..." }
                },
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct NoHoverAction {
    active_path: Signal<VaultPath>,
}

impl NoteElementActions for NoHoverAction {
    fn on_hover(&self, _entry: &NoteBrowseEntry) -> Element {
        rsx! {}
    }

    fn on_select(&mut self, entry: &NoteBrowseEntry) {
        self.active_path.set(entry.get_path().to_owned());
    }
}
