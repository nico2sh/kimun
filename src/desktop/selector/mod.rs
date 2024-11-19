use std::rc::Rc;

use dioxus::prelude::*;
use log::info;

#[derive(Props, Clone, PartialEq)]
pub struct SelectorProps {
    open: Signal<bool>,
}

#[allow(non_snake_case)]
pub fn Selector(props: SelectorProps) -> Element {
    let mut open = props.open;
    let mut element: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    if *open.read() {
        spawn(async move {
            loop {
                if let Some(e) = element.with(|f| f.clone()) {
                    info!("focus input");
                    let _ = e.set_focus(true).await;
                    break;
                }
            }
        });
    }

    rsx! {
        dialog {
            class: "w-screen p-2 h-10 rounded-lg shadow",
            open: *open.read(),
            autofocus: "true",
            onkeydown: move |e: Event<KeyboardData>| {
                let key = e.data.code();
                if key == Code::Escape {
                     *open.write() = false;
                }
            },
            div {
                class: "flex flex-col border-1",
                input {
                    r#type: "text",
                    onmounted: move |e| {
                        info!("input");
                        *element.write() = Some(e.data());
                    },
                    "search"
                }
            }
        }
    }
}
