use std::fmt::Display;

use dioxus::prelude::*;

use crate::{
    components::{
        focus_manager::{FocusComponent, FocusManager},
        icons,
        note_list::note_browse_entry::SortCriteria,
    },
    settings::AppSettings,
};

#[derive(Clone, PartialEq, Props)]
pub struct SearchBoxProps<S>
where
    S: StringSearch + Clone + 'static,
{
    search_text: Signal<S>,
    sort_criteria: Signal<SortCriteria>,
    sort_ascending: Signal<bool>,
    input_focus: FocusComponent,

    #[props(default)]
    on_keystroke: Callback<Event<KeyboardData>>,

    #[props(default = false)]
    no_default: bool,
}

pub trait StringSearch: Display + PartialEq + Clone + Send + Default {
    fn change_value(&mut self, value: String);
}

impl StringSearch for String {
    fn change_value(&mut self, value: String) {
        *self = value;
    }
}

#[component]
pub fn SearchBox<S>(props: SearchBoxProps<S>) -> Element
where
    S: StringSearch + Clone,
{
    let SearchBoxProps {
        mut search_text,
        mut sort_criteria,
        mut sort_ascending,
        input_focus,
        on_keystroke,
        no_default,
    } = props;
    let settings: Signal<AppSettings> = use_context();
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
    let theme = settings().get_theme();
    let mut icon_button_hover = use_signal(|| false);
    let mut sort_hover: Signal<Option<SortCriteria>> = use_signal(|| None);

    rsx! {
        div {
            class: "search-input-wrapper",
            background_color: "{theme.bg_main}",
            border_color: "{theme.border_light}",
            input {
                class: "search-box",
                color: "{theme.text_primary}",
                r#type: "search",
                placeholder: "Search...",
                value: "{search_text}",
                spellcheck: false,
                onfocus: move |_e| show_sort_options.set(false),
                onmounted: move |e| {
                    focus_manager.register_and_focus(mount_focus.clone(), e.data());
                },
                oninput: move |e| {
                    search_text.write().change_value(e.value());
                },
                onkeydown: move |e: Event<KeyboardData>| {
                    let key = e.data.code();
                    match key {
                        Code::ArrowDown | Code::ArrowUp | Code::Tab => {
                            e.prevent_default();
                        }
                        _ => {}
                    }
                    on_keystroke.call(e);
                },
            }
            div {
                class: "search-controls",
                border_left_color: "{theme.border_light}",
                div { class: "sort-dropdown",
                    button {
                        class: "icon-button",
                        color: "{theme.text_muted}",
                        background_color: if icon_button_hover() { "{theme.bg_hover}" } else { "transparent" },
                        onfocusin: move |_e| icon_button_hover.set(true),
                        onfocusout: move |_e| icon_button_hover.set(false),
                        title: "Sort Options",
                        aria_label: "Sort Options",
                        onclick: move |_e| show_sort_options.set(!show_sort_options()),
                        if sort_criteria() == SortCriteria::Title {
                            icons::SortTitle {}
                        } else if sort_criteria() == SortCriteria::FileName {
                            icons::SortFileName {}
                        } else if sort_criteria() == SortCriteria::None {
                            icons::DoubleCircle {}
                        }
                    }
                    if show_sort_options() {
                        div {
                            class: "sort-menu",
                            background_color: "{theme.bg_main}",
                            border_color: "{theme.border_light}",
                            onmouseleave: move |_e| {
                                sort_hover.set(None);
                            },
                            onclick: move |_e| {
                                show_sort_options.set(false);
                                focus_after_sort.focus(return_focus.clone());
                            },
                            if !no_default {
                                div {
                                    class: if SortCriteria::None == sort_criteria() { "sort-option selected" } else { "sort-option" },
                                    color: if SortCriteria::None == sort_criteria() { "{theme.accent_blue}" } else { "{theme.text_secondary}" },
                                    background_color: if let Some(SortCriteria::None) = sort_hover() { "{theme.bg_hover}" } else { "transparent" },
                                    onmouseenter: move |_e| sort_hover.set(Some(SortCriteria::None)),
                                    onclick: move |_e| {
                                        sort_criteria.set(SortCriteria::None);
                                    },
                                    icons::DoubleCircle {}
                                    "Default"
                                }
                            }
                            div {
                                class: if SortCriteria::Title == sort_criteria() { "sort-option selected" } else { "sort-option" },
                                color: if SortCriteria::Title == sort_criteria() { "{theme.accent_blue}" } else { "{theme.text_secondary}" },
                                background_color: if let Some(SortCriteria::Title) = sort_hover() { "{theme.bg_hover}" } else { "transparent" },
                                onmouseenter: move |_e| {
                                    sort_hover.set(Some(SortCriteria::Title));
                                },
                                onclick: move |_e| {
                                    sort_criteria.set(SortCriteria::Title);
                                },
                                icons::SortTitle {}
                                "Title"
                            }
                            div {
                                class: if SortCriteria::FileName == sort_criteria() { "sort-option selected" } else { "sort-option" },
                                color: if SortCriteria::FileName == sort_criteria() { "{theme.accent_blue}" } else { "{theme.text_secondary}" },
                                background_color: if let Some(SortCriteria::FileName) = sort_hover() { "{theme.bg_hover}" } else { "transparent" },
                                onmouseenter: move |_e| sort_hover.set(Some(SortCriteria::FileName)),
                                onclick: move |_e| {
                                    sort_criteria.set(SortCriteria::FileName);
                                },
                                icons::SortFileName {}
                                "FileName"
                            }
                        }
                    }
                }
                button {
                    class: if sort_ascending() { "icon-button ascending" } else { "icon-button" },
                    title: if sort_ascending() { "Sort Direction: Ascending" } else { "Sort Direction: Descending" },
                    color: "{theme.text_muted}",
                    disabled: sort_criteria() == SortCriteria::None,
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
}
