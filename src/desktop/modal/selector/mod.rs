pub mod note_search;
pub mod note_select;
mod row_item;

use std::rc::Rc;

use dioxus::prelude::*;
use log::{debug, info};
use row_item::RowItem;

use crate::noters::nfs::NotePath;

use super::Modal;

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
    modal: Signal<Modal>,
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
    R: RowItem + 'static,
    I: Fn() -> Vec<R> + Clone + 'static,
    F: Fn(String, Vec<R>) -> Vec<R> + Clone + 'static,
    P: Fn(&R) -> String + Clone + 'static,
{
    let mut filter_text = use_signal(|| filter_text);
    let mut load_state = use_signal(|| LoadState::Open);
    let mut dialog: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let current_state = load_state.read().to_owned();
    let visible = match current_state {
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

    let _loading_rows = use_resource(move || {
        let current_state = load_state.read().to_owned();
        let function = on_init.clone();
        async move {
            if let LoadState::Open = current_state {
                let items = function();
                debug!("Loaded {} items", items.len());
                load_state.set(LoadState::Loaded(items));
            }
        }
    });

    let rows = use_resource(move || {
        let current_state = load_state.read().to_owned();
        let filter_text = filter_text.read().to_owned();
        let function = on_filter_change.clone();
        async move {
            if let LoadState::Loaded(items) = current_state {
                selected.set(None);
                function(filter_text, items)
            } else {
                vec![]
            }
        }
    });

    let show_preview = on_preview.is_some();
    let preview_text = match on_preview {
        Some(on_preview) => use_resource(move || {
            let rows: Vec<R> = rows.value().read().clone().unwrap_or_default();
            let function = on_preview.clone();
            async move {
                if let Some(selection) = *selected.read() {
                    let entry = rows.get(selection);
                    entry.map(&function)
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
                    {
                        let rs = rows.value().read().clone().unwrap_or_default();
                        rsx! {
                            for (index, row) in rs.clone().into_iter().enumerate() {
                                    div {
                                        onmouseover: move |_e| {
                                            *selected.write() = Some(index);
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

#[derive(Clone, Eq, PartialEq)]
pub struct PathEntry {
    path: NotePath,
    path_str: String,
    path_signal: Signal<Option<NotePath>>,
}

impl PathEntry {
    pub fn from_note_path(path: NotePath, path_signal: Signal<Option<NotePath>>) -> Self {
        let path_str = path.to_string();
        Self {
            path,
            path_str,
            path_signal,
        }
    }
}

impl AsRef<str> for PathEntry {
    fn as_ref(&self) -> &str {
        self.path_str.as_str()
    }
}

impl RowItem for PathEntry {
    fn on_select(&self) -> Box<dyn FnMut()> {
        let p = self.path.clone();
        let mut s = self.path_signal;
        Box::new(move || s.set(Some(p.clone())))
    }

    fn get_view(&self) -> Element {
        rsx! {
            div {
                "{self.path.to_string()}"
            }
        }
    }
}
