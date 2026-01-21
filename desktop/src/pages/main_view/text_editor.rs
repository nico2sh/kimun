use std::time::Duration;

use dioxus::prelude::*;

use crate::{
    components::focus_manager::{FocusComponent, FocusManager},
    editor_state::EditorState,
    settings::AppSettings,
    utils::keys::{
        action_shortcuts::{ActionShortcuts, TextAction},
        key_combo::KeyCombo,
        key_strike::KeyStrike,
    },
};

const EVAL_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/scripts/md_shortcuts.js"
));

#[derive(Props, Clone, PartialEq)]
pub struct TextEditorProps {
    content: Signal<String>,
}

#[component]
pub fn TextEditor(props: TextEditorProps) -> Element {
    let settings: Signal<AppSettings> = use_context();
    let focus_manager = use_context::<FocusManager>();
    let mut editor_state: Signal<EditorState> = use_context();

    let TextEditorProps { mut content } = props;
    let text = content.read().to_owned();

    let fm = focus_manager.clone();
    use_drop(move || {
        fm.unregister_focus(FocusComponent::Editor);
    });

    use_effect(move || {
        debug!("===> Initializing Javascript code for Markdown");
        let init_script = r#"
window.editor = new TextareaMarkdown(
    document.getElementById('textEditor')
);
"#;
        spawn(async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Err(e) = document::eval(init_script).await {
                error!("Error initializing editor: {}", e);
            } else {
                debug!("===> Javascript code for Markdown initialized");
            }
        });
    });

    let fm = focus_manager.clone();
    let theme = settings().get_theme();
    rsx! {
        textarea {
            class: "text-editor",
            color: "{theme.text_primary}",
            id: "textEditor",
            autofocus: true,
            onfocus: move |_e| {
                focus_manager.focus(FocusComponent::Editor);
            },
            onmounted: move |e| {
                fm.register_and_focus(FocusComponent::Editor, e.data());
            },
            // onselect: move |e| {
            //     info!("Select event {:?}", e.data());
            // },
            // onselectstart: move |e| {
            //     info!("Select start event {:?}", e.data());
            // },
            // onselectionchange: move |e| {
            //     info!("Select change event {:?}", e.data());
            // },
            oninput: move |e| {
                *content.write() = e.value();
                editor_state.write().mark_content_dirty();
            },
            onkeydown: move |event: Event<KeyboardData>| {
                let data = event.data();
                let key_combo: KeyCombo = data.into();
                if key_combo.key == KeyStrike::Tab {
                    if key_combo.modifiers.is_shift() {
                        eval_action("unindent");
                    } else {
                        eval_action("indent");
                    }
                    event.prevent_default();
                } else if let Some(ActionShortcuts::Text(action)) = settings
                    .read()
                    .key_bindings
                    .get_action(&key_combo)
                {
                    match action {
                        TextAction::Bold => eval_action("bold"),
                        TextAction::Italic => eval_action("italic"),
                        TextAction::Link => eval_action("link"),
                        TextAction::Image => eval_action("image"),
                        TextAction::ToggleHeader => eval_action("toggle_header"),
                        TextAction::Header(n) => eval_action(&format!("heading{}", n)),
                        TextAction::Underline => eval_action("underline"),
                        TextAction::Strikethrough => eval_action("strike"),
                    }
                }
            },
            spellcheck: true,
            wrap: "hard",
            resize: "none",
            placeholder: "Start writing something!",
            value: "{text}",
        }
    }
}

fn eval_action(action: &str) {
    let eval = document::eval(EVAL_JS);
    if let Err(e) = eval.send(action) {
        error!("Error sending value {}: {}", action, e);
    }
}
