pub mod note_search;
pub mod note_select;

use std::rc::Rc;

use dioxus::prelude::*;
use log::{debug, info};

use core_notes::nfs::{NoteDetails, NotePath};

use super::Modal;

pub trait RowItem: PartialEq + Eq + Clone {
    fn on_select(&self) -> Box<dyn FnMut()>;
    fn get_view(&self) -> Element;
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadState<R>
where
    R: RowItem + 'static,
{
    Closed,
    Open,
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
fn SelectorView<R, F, I, P>(
    hint: String,
    filter_text: String,
    mut modal: Signal<Modal>,
    on_init: I,
    on_filter_change: F,
    on_preview: Option<P>,
) -> Element
where
    R: RowItem + Send + Sync + 'static,
    I: Fn() -> Vec<R> + Clone + Send + 'static,
    F: Fn(String, Vec<R>) -> Vec<R> + Clone + Send + 'static,
    P: Fn(&R) -> String + Clone + Send + 'static,
{
    let mut filter_text = use_signal(|| filter_text);
    let mut load_state = use_signal_sync(|| LoadState::Open);
    let mut dialog: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let current_state = load_state.read().to_owned();
    let visible = match &current_state {
        LoadState::Closed => false,
        LoadState::Open => {
            debug!("Opening Dialog View");
            // when the dialog is open and starts initializing
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
        LoadState::Loaded(_) => {
            // when the dialog has initialized
            true
        }
    };
    let mut selected: Signal<Option<usize>> = use_signal(|| None);

    let rows = use_resource(move || {
        let current_state = current_state.clone();
        let filter_text = filter_text.read().clone();
        let on_init = on_init.clone();
        let on_filter_change = on_filter_change.clone();
        async move {
            if let LoadState::Open = current_state {
                let init_task = smol::spawn(async move {
                    let items = on_init();
                    debug!("Loaded {} items", items.len());
                    load_state.set(LoadState::Loaded(items.clone()));
                    on_filter_change(filter_text, items)
                });
                init_task.await
            } else if let LoadState::Loaded(items) = current_state {
                selected.set(None);
                let task = smol::spawn(async move { on_filter_change(filter_text, items) });
                task.await
            } else {
                vec![]
            }
        }
    });

    let show_preview = on_preview.is_some();
    let preview_text = match on_preview {
        Some(on_preview) => use_resource(move || {
            let rows: Vec<R> = rows.value().read().clone().unwrap_or_default();
            let function_preview = on_preview.clone();
            let selected = selected.read().to_owned();
            async move {
                if let Some(selection) = selected {
                    let entry = rows.get(selection);
                    if let Some(value) = entry {
                        let value_copy = value.to_owned();
                        let task = smol::spawn(async move { function_preview(&value_copy) });
                        Some(task.await)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }),
        None => use_resource(move || async move { None }),
    };

    let row_number = rows.value().read().clone().unwrap_or_default().len();

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
                    selected.set(new_selected);
                }
                if key == Code::Enter && row_number > 0 {
                    let current_selected = (*selected.read()).unwrap_or(0);
                    if let Some(rows) = &*rows.value().read() {
                        if let Some(row) = rows.get(current_selected) {
                            row.on_select()();
                            load_state.set(LoadState::Closed);
                            modal.write().close();
                        }
                    }
                }
            },
            div {
                class: "hint",
                "{hint}"
            }
            div {
                class: "search",
                input {
                    class: "search_box",
                    r#type: "search",
                    value: "{filter_text}",
                    spellcheck: false,
                    onmounted: move |e| {
                        *dialog.write() = Some(e.data());
                    },
                    oninput: move |e| {
                        filter_text.set(e.value().clone().to_string());
                    },
                }
                div {
                    class: "list",
                    if let Some(rs) = rows.value().read().clone() {
                        for (index, row) in rs.into_iter().enumerate() {
                            div {
                                onmouseover: move |_e| {
                                    selected.set(Some(index));
                                },
                                onclick: move |_e| {
                                    row.on_select()();
                                    load_state.set(LoadState::Closed);
                                    modal.write().close();
                                },
                                class: if *selected.read() == Some(index) {
                                    "element selected"
                                } else {
                                    "element"
                                },
                                { row.get_view() }
                            }
                        }
                    } else {
                        div {
                            "Loading..."
                        }
                    }
                }
            }
            if show_preview {
                div {
                    class: "preview",
                    match &*preview_text.read() {
                        Some(text) => if let Some(t) = text {
                            rsx! { p { "{t}" } }
                        } else {
                            rsx!{}
                        },
                        None => rsx! { "Loading..." }
                    }
                }
            }
        }
    }
}
