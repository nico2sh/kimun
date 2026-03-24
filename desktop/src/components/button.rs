use std::fmt::Display;

use dioxus::prelude::*;

use crate::themes::Theme;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum ButtonStyle {
    #[default]
    Primary,
    Secondary,
    Danger,
}

impl Display for ButtonStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ButtonStyle::Primary => "primary",
                ButtonStyle::Secondary => "secondary",
                ButtonStyle::Danger => "danger",
            }
        )
    }
}

#[derive(Clone, PartialEq)]
pub struct ButtonBuilder {
    title: String,
    style: ButtonStyle,
    action: EventHandler<MouseEvent>,
}

impl ButtonBuilder {
    //     pub fn new<MaybeAsync, Marker>(mut f: impl FnMut(Args) -> MaybeAsync + 'static) -> Self
    // where
    //     MaybeAsync: SpawnIfAsync<Marker, Ret>,
    //     // Bounds from impl:
    //     Args: 'static,
    //     Ret: 'static,
    pub fn primary(title: &str, action: Callback<MouseEvent>) -> Self {
        Self {
            title: title.to_string(),
            style: ButtonStyle::Primary,
            action,
        }
    }

    pub fn secondary(title: &str, action: Callback<MouseEvent>) -> Self {
        Self {
            title: title.to_string(),
            style: ButtonStyle::Secondary,
            action,
        }
    }

    pub fn danger(title: &str, action: Callback<MouseEvent>) -> Self {
        Self {
            title: title.to_string(),
            style: ButtonStyle::Danger,
            action,
        }
    }

    pub fn build(&self, theme: &Theme) -> Element {
        rsx! {
            Button {
                title: self.title.clone(),
                theme: theme.clone(),
                style: self.style.clone(),
                action: self.action,
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    title: String,
    theme: Theme,
    #[props(default)]
    style: ButtonStyle,
    action: EventHandler<MouseEvent>,
    #[props(default)]
    disabled: bool,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    let color = match props.style {
        ButtonStyle::Primary => (props.theme.accent_blue, props.theme.accent_blue_dark),
        ButtonStyle::Secondary => (props.theme.bg_section, props.theme.bg_hover),
        ButtonStyle::Danger => (props.theme.accent_red, props.theme.accent_red_dark),
    };
    let mut hover = use_signal(|| false);
    rsx! {
        button {
            class: "btn {props.style}",
            border_color: "{props.theme.border_light}",
            onmouseover: move |_e| hover.set(true),
            onmouseleave: move |_e| hover.set(false),
            background_color: if hover() { "{color.1}" } else { "{color.0}" },
            onclick: move |e| {
                props.action.call(e);
            },
            disabled: props.disabled,
            "{props.title}"
        }
    }
}
