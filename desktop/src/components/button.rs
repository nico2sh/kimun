use std::fmt::Display;

use dioxus::prelude::*;

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
    action: Callback<MouseEvent>,
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

    pub fn build(&self) -> Element {
        rsx! {
            Button {
                title: self.title.clone(),
                style: self.style.clone(),
                action: self.action,
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ButtonProps {
    title: String,
    #[props(default)]
    style: ButtonStyle,
    action: Callback<MouseEvent>,
    #[props(default)]
    disabled: bool,
}

#[component]
pub fn Button(props: ButtonProps) -> Element {
    rsx! {
        button {
            class: "btn {props.style}",
            onclick: move |e| {
                props.action.call(e);
            },
            disabled: props.disabled,
            "{props.title}"
        }
    }
}
