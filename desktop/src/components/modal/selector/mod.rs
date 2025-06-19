pub mod note_search;
pub mod note_select;

use std::rc::Rc;

use dioxus::{
    logger::tracing::{debug, info},
    prelude::*,
};

use super::Modal;

trait SelectorFunctions<R>: Clone
where
    R: RowItem,
{
    fn init(&self) -> Vec<R>;
    fn filter(&self, filter_text: String, items: &Vec<R>) -> Vec<R>;
    fn preview(&self, element: &R) -> Option<PreviewData>;
}

pub trait RowItem: PartialEq + Eq + Clone {
    fn on_select(&self) -> Box<dyn FnMut() -> bool>;
    fn get_view(&self) -> Element;
}

pub struct PreviewData {
    title: String,
    data: String,
    content: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState<R>
where
    R: RowItem + 'static,
{
    Closed,
    Init,
    Loaded(Vec<R>),
}

#[derive(Props, Clone, PartialEq)]
struct SelectorViewProps<R>
where
    Resource<Vec<R>>: PartialEq,
    R: RowItem + 'static,
{
    filter_text: Signal<String>,
    load_state: Signal<LoadState<R>>,
    modal: SyncSignal<Modal>,
}

#[allow(non_snake_case)]
fn SelectorView<R, F>(
    hint: String,
    filter_text: String,
    mut modal: Signal<Modal>,
    functions: F,
) -> Element
where
    R: RowItem + Send + Clone + Sync + 'static,
    F: SelectorFunctions<R> + Clone + Send + 'static,
{
    let mut filter_text = use_signal(|| filter_text);
    let mut load_state: Signal<LoadState<R>, SyncStorage> = use_signal_sync(|| LoadState::Init);
    // For setting the focus in the text box
    let mut dialog: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut row_mount = use_signal(Vec::<Rc<MountedData>>::new);

    let functions_load = functions.clone();

    let rows = use_resource(move || {
        let filter_text = filter_text.read().clone();
        let current_state = load_state.read().clone();
        let functions = functions_load.clone();
        async move {
            match current_state {
                LoadState::Init => {
                    row_mount.write().clear();
                    tokio::task::spawn(async move {
                        let items = functions.init();
                        load_state.set(LoadState::Loaded(items.clone()));
                    });

                    // We put the focus on the text
                    loop {
                        if let Some(e) = dialog.with(|f| f.clone()) {
                            debug!("Focus input");
                            let _ = e.set_focus(true).await;
                            break;
                        }
                    }
                    vec![]
                }
                LoadState::Loaded(items) => {
                    selected.set(None);
                    tokio::spawn(async move { functions.filter(filter_text, &items) })
                        .await
                        .unwrap()
                }
                LoadState::Closed => vec![],
            }
        }
    });

    let preview_text = use_resource(move || {
        let functions = functions.clone();
        // We get a copy of the selected one so we don't have borrow issues
        let selected = selected.read().to_owned();
        async move {
            if let Some(selection) = selected {
                info!("Preview Text for {}", selected.unwrap());
                let r = rows.read_unchecked();
                let entry = match &*r {
                    Some(rows) => rows.get(selection),
                    None => None,
                };
                if let Some(value) = entry {
                    let value_copy = value.to_owned();
                    tokio::spawn(async move { functions.preview(&value_copy) })
                        .await
                        .unwrap()
                } else {
                    None
                }
            } else {
                // Nothing selected
                None
            }
        }
    });

    let row_number = rows.value().read().clone().unwrap_or_default().len();

    rsx! {
        div {
            class: "notes-modal",
            autofocus: "true",
            onclick: move |e| e.stop_propagation(),
            onkeydown: move |e: Event<KeyboardData>| async move {
                let key = e.data.code();
                if key == Code::Escape {
                    load_state.set(LoadState::Closed);
                    modal.write().close();
                }
                if key == Code::ArrowDown {
                    let max_items = row_number;
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
                    if let Some(sel) = new_selected {
                        if let Some(mount) = row_mount.read().get(sel) {
                            let _a = mount.scroll_to(ScrollBehavior::Smooth).await;
                        }
                    }
                    selected.set(new_selected);
                }
                if key == Code::ArrowUp {
                    let max_items = row_number;
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
                    if let Some(sel) = new_selected {
                        if let Some(mount) = row_mount.read().get(sel) {
                            let _a = mount.scroll_to(ScrollBehavior::Smooth).await;
                        }
                    }
                    selected.set(new_selected);
                }
                if key == Code::Enter && row_number > 0 {
                    let current_selected = (*selected.read()).unwrap_or(0);
                    if let Some(rows) = &*rows.value().read() {
                        if let Some(row) = rows.get(current_selected) {
                            if row.on_select()() {
                                load_state.set(LoadState::Closed);
                                modal.write().close();
                            } else {
                                load_state.set(LoadState::Init);
                            }
                        }
                    }
                }
            },
            div { class: "search-header",
                div { class: "search-title", "Browse Notes" }
                div { class: "header-description", "{hint}" }
                input {
                    class: "search-box",
                    r#type: "search",
                    placeholder: "Search...",
                    value: "{filter_text}",
                    spellcheck: false,
                    onmounted: move |e| {
                        *dialog.write() = Some(e.data());
                    },
                    oninput: move |e| {
                        filter_text.set(e.value().clone().to_string());
                    },
                    onkeydown: move |e: Event<KeyboardData>| {
                        let key = e.data.code();
                        match key {
                            Code::ArrowDown | Code::ArrowUp | Code::Tab => {
                                e.prevent_default();
                            }
                            _ => {}
                        }
                    },
                    onfocusout: move |_e| {
                        modal.write().close();
                    },
                }
            }
            div { class: "notes-list",
                if let Some(rs) = rows.value().read().clone() {
                    for (index , row) in rs.into_iter().enumerate() {
                        div {
                            class: if *selected.read() == Some(index) { "note-item selected" } else { "note-item" },
                            id: "element-{index}",
                            onmounted: move |e| {
                                row_mount.write().push(e.data());
                            },
                            onmouseenter: move |_e| {
                                selected.set(Some(index));
                            },
                            onclick: move |_e| {
                                if row.on_select()() {
                                    load_state.set(LoadState::Closed);
                                    modal.write().close();
                                } else {
                                    load_state.set(LoadState::Init);
                                }
                            },
                            {row.get_view()}
                        }
                    }
                } else {
                    div { "Loading..." }
                }
            }
            div { class: "preview-pane",
                match &*preview_text.read_unchecked() {
                    Some(text) => {
                        if let Some(p) = text {
                            rsx! {
                                div { class: "preview-header",
                                    div { class: "preview-title", "{p.title}" }
                                    div { class: "preview-meta", "{p.data}" }
                                }
                                div { class: "preview-content", "{p.content}" }
                            }
                        } else {
                            rsx! {
                                div { class: "no-preview", "" }
                            }
                        }
                    }
                    None => rsx! { "Loading..." },
                }
            }
        }
    }
}
