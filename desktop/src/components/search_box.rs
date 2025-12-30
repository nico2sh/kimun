use dioxus::prelude::*;

use crate::components::{
    focus_manager::{FocusComponent, FocusManager},
    icons,
    note_select_entry::SortCriteria,
};

#[derive(Clone, PartialEq, Props)]
pub struct SearchBoxProps {
    search_text: Signal<String>,
    sort_criteria: Signal<Option<SortCriteria>>,
    sort_ascending: Signal<bool>,
    input_focus: FocusComponent,

    #[props(default = true)]
    no_default: bool,
}

#[component]
pub fn SearchBox(props: SearchBoxProps) -> Element {
    let SearchBoxProps {
        mut search_text,
        mut sort_criteria,
        mut sort_ascending,
        input_focus,
        no_default,
    } = props;
    let focus_manager = use_context::<FocusManager>();
    let mut show_sort_options = use_signal(|| false);
    let focus_after_sort = focus_manager.clone();

    let fm = focus_manager.clone();
    let ifoc = input_focus.clone();
    use_drop(move || {
        fm.unregister_focus(ifoc);
    });

    let mount_focus = input_focus.clone();
    let return_focus = input_focus.clone();

    rsx! {
        div { class: "search-input-wrapper",
            input {
                class: "search-box",
                r#type: "search",
                placeholder: "Search...",
                value: "{search_text}",
                spellcheck: false,
                onmounted: move |e| {
                    focus_manager.register_and_focus(mount_focus.clone(), e.data());
                },
                oninput: move |e| {
                    search_text.set(e.value().clone().to_string());
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
            }
            div { class: "search-controls",
                div { class: "sort-dropdown",
                    button {
                        class: "icon-button",
                        title: "Sort Options",
                        aria_label: "Sort Options",
                        onclick: move |_e| show_sort_options.set(!show_sort_options()),
                        if let Some(criteria) = sort_criteria() {
                            if criteria == SortCriteria::Title {
                                icons::SortTitle {}
                            } else if criteria == SortCriteria::FileName {
                                icons::SortFileName {}
                            }
                        } else {
                            icons::DoubleCircle {}
                        }
                    }
                    if show_sort_options() {
                        div {
                            class: "sort-menu show",
                            onclick: move |_e| {
                                show_sort_options.set(false);
                                focus_after_sort.focus(return_focus.clone());
                            },
                            if !no_default {
                                div {
                                    class: if sort_criteria().is_none() { "sort-option selected" } else { "sort-option" },
                                    onclick: move |_e| {
                                        sort_criteria.set(None);
                                    },
                                    icons::DoubleCircle {}
                                    "Default"
                                }
                            }
                            div {
                                class: if Some(SortCriteria::Title) == sort_criteria() { "sort-option selected" } else { "sort-option" },
                                onclick: move |_e| {
                                    sort_criteria.set(Some(SortCriteria::Title));
                                },
                                icons::SortTitle {}
                                "Title"
                            }
                            div {
                                class: if Some(SortCriteria::FileName) == sort_criteria() { "sort-option selected" } else { "sort-option" },
                                onclick: move |_e| {
                                    sort_criteria.set(Some(SortCriteria::FileName));
                                },
                                icons::SortFileName {}
                                "FileName"
                            }
                        }
                    }
                }
            }
            button {
                class: if sort_ascending() { "icon-button sort-order ascending" } else { "icon-button sort-order" },
                title: "Sort order: descending",
                disabled: sort_criteria().is_none(),
                aria_label: "Toggle sort order",
                onclick: move |_e| sort_ascending.set(!sort_ascending()),
                svg { view_box: "0 0 24 24",
                    line {
                        x1: "12",
                        y1: "5",
                        x2: "12",
                        y2: "19",
                    }
                    polyline { points: "19 12 12 19 5 12" }
                }
            }
        }
    }
}
