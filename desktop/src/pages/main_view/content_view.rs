use std::{rc::Rc, sync::Arc, time::Duration};

use dioxus::{
    core::use_drop,
    logger::tracing::{debug, error},
    prelude::*,
};
// use dioxus_radio::hooks::{use_radio, Radio};
use futures::StreamExt;
use kimun_core::{nfs::VaultPath, note::NoteDetails, NoteVault};

use crate::{
    components::{
        focus_manager::{FocusComponent, FocusManager},
        modal::ModalType,
        preview::Markdown,
    },
    editor_state::{ContentType, EditorState},
    global_events::{GlobalEvent, PubSub},
    settings::AppSettings,
    utils::keys::action_shortcuts::{ActionShortcuts, TextAction},
    MARKDOWN_JS,
};

const AUTOSAVE_SECS: u64 = 5;
const TEXT_EDITOR: &str = "text_editor";
const EVAL_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/scripts/md_shortcuts.js"
));

#[derive(Clone)]
pub struct EditorSaveManager {
    path: VaultPath,
    vault: Arc<NoteVault>,
    content: Signal<String>,
    app_state: Signal<EditorState>,
}

impl EditorSaveManager {
    async fn save(&mut self) -> anyhow::Result<()> {
        // debug!("Triggered save");
        let dirty_status = self.app_state.read().has_dirty_content();
        if dirty_status {
            debug!("Saving content");
            let path = self.path.clone();
            let text = self.content.peek().clone();
            let vault = self.vault.clone();
            tokio::spawn(async move {
                let _ = vault.save_note(&path, text);
                debug!("Saved at {}", path);
            })
            .await?;
            self.app_state.write().mark_content_clean();
        }
        Ok(())
    }
}

impl Drop for EditorSaveManager {
    fn drop(&mut self) {
        debug!("Dropping Editor Data at path {}", self.path);
        let dirty_status = self.app_state.read().has_dirty_content();
        if dirty_status {
            debug!("Saving so we don't lose data");
            let text = self.content.peek().clone();
            let _ = self.vault.save_note(&self.path, text);
            // self.content.write().mark_clean();
        }
    }
}

pub enum EditorMsg {
    Init { text: String },
    Save,
}

#[component]
pub fn NoText(path: ReadSignal<VaultPath>) -> Element {
    let mut text_area_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);

    let mut app_state: Signal<EditorState> = use_context();
    use_effect(move || app_state.write().set_content_type(ContentType::Directory));

    rsx! {
        div {
            class: "editor-empty",
            onmounted: move |e| {
                text_area_signal.set(Some(e.data()));
            },
            div { class: "title", "Current path: {path}" }
            div { class: "message", "Open or create a new note." }
            img { class: "logo", src: "assets/images/kimun.png" }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Props)]
pub struct TextEditorProps {
    note_path: ReadSignal<VaultPath>,
    vault: Arc<NoteVault>,
    modal_type: Signal<ModalType>,
    preview: bool,
}

#[component]
pub fn TextEditor(props: TextEditorProps) -> Element {
    debug!(
        "-==== [Text Editor] Starting Editor at '{}' ====-",
        props.note_path
    );
    let mut app_state: Signal<EditorState> = use_context();
    let mut content = use_signal(|| "".to_string());
    let modal_type = props.modal_type;

    let focus_manager = use_context::<FocusManager>();
    let mut settings: Signal<AppSettings> = use_context();

    let editor_vault = props.vault.clone();
    let cr = use_coroutine(move |mut rx: UnboundedReceiver<EditorMsg>| {
        let editor_vault = editor_vault.clone();
        async move {
            debug!("We start listening for editor update events");
            let mut ed: Option<EditorSaveManager> = None;
            while let Some(msg) = rx.next().await {
                match msg {
                    EditorMsg::Init { text } => {
                        debug!("We init with text {text}");
                        // We check if we already have an editor_data and we save
                        if let Some(editor_data) = ed.as_mut() {
                            let _ = editor_data.save().await;
                        }
                        // We create a new instance of the editor data
                        *content.write() = text.clone();
                        app_state.write().set_content_type(
                            crate::editor_state::ContentType::Note { dirty: false },
                        );
                        let editor_data = EditorSaveManager {
                            content,
                            path: props.note_path.read().to_owned(),
                            vault: editor_vault.clone(),
                            app_state,
                        };
                        ed = Some(editor_data);
                    }
                    EditorMsg::Save => {
                        // debug!("Received save signal");
                        if let Some(editor_data) = ed.as_mut() {
                            let _ = editor_data.save().await;
                        }
                    }
                }
            }
        }
    });
    // AutoSave every 5 seconds
    use_future(move || async move {
        debug!("Initializing save loop");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(AUTOSAVE_SECS)).await;
            cr.send(EditorMsg::Save);
        }
    });

    let vault_content = props.vault.clone();
    let mut note_content = use_resource(move || {
        debug!("[Initial Content] Loading text content");
        let vault = vault_content.clone();
        async move {
            let exists = vault.exists(&props.note_path.read()).is_some();
            debug!("[Initial Content] Exists: {:?}", exists);
            let text = if exists {
                debug!("[Initial Content] Loading from path at {}", props.note_path);
                let text = vault.load_note(&props.note_path.read()).map_or_else(
                    |e| {
                        error!("[Initial Content] Error loading Note: {}", e);
                        String::new()
                    },
                    |d| {
                        // We save the settings for the last opened notes
                        debug!("[Initial Content] Saving path history");
                        settings.write().add_path_history(&props.note_path.read());
                        // We don't want the settings to trigger a re-run every time it changes, so we use `peek()` instead of `read()`
                        let _r = settings.peek().save_to_disk();
                        d.raw_text
                    },
                );
                text
            } else {
                "".to_string()
            };
            cr.send(EditorMsg::Init { text: text.clone() });
            debug!("[Initial Content] Init message sent");
            text
        }
    });

    let pub_sub: PubSub<GlobalEvent> = use_context();
    let pc = pub_sub.clone();
    let vault = props.vault.clone();
    use_effect(move || {
        let vault = vault.clone();
        pc.subscribe(
            TEXT_EDITOR,
            Callback::new(move |g| {
                match g {
                    GlobalEvent::SaveCurrentNote => {
                        let dirty_status = app_state.read().has_dirty_content();
                        if dirty_status {
                            debug!("Saving so we don't lose data");
                            let text = content.peek().clone();
                            let _ = vault.save_note(&props.note_path.read(), text);
                            app_state.write().mark_content_clean();
                        }
                    }
                    GlobalEvent::MarkNoteClean => {
                        app_state.write().mark_content_clean();
                    }
                    _ => {}
                }
                debug!("Saving a note");
            }),
        );
    });
    let fm = focus_manager.clone();
    use_drop(move || {
        pub_sub.unsubscribe(TEXT_EDITOR);
        fm.unregister_focus(FocusComponent::Editor);
    });

    use_effect(move || {
        if !props.preview {
            let init_script = r#"
const textEditor = document.getElementById('textEditor');
if (textEditor) {
    window.md_editor = enhanceTextareaWithMarkdown(textEditor);
}
"#;
            spawn(async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                debug!("Initializing Markdown Editor");
                if let Err(e) = document::eval(init_script).await {
                    error!("Error initializing editor: {}", e);
                }
            });
        }
    });
    // let _ = use_global_shortcut("cmd+L", move || {
    //     info!("Command L");
    // });
    use_effect(move || {
        // If we set the preview, we update the content with the current value
        if props.preview {
            note_content.set(Some(content.peek().clone()));
        }
    });

    // This manages the editor state
    rsx! {
        document::Script { src: MARKDOWN_JS }
        div { class: "editor-content",
            {
                let focus = focus_manager.clone();
                match &*note_content.read() {
                    None => rsx! {
                        div { onmounted: move |_e| {}, "Loading..." }
                    },
                    Some(text) => rsx! {
                        if props.preview {
                            {
                                let note_details = NoteDetails::new(
                                    &props.note_path.read(),
                                    content.peek().clone(),
                                );
                                let md_content = note_details.get_markdown_and_links();
                                rsx! {
                                    Markdown {
                                        vault: props.vault.clone(),
                                        note_md: md_content.text,
                                        note_links: md_content.links,
                                        modal_type,
                                        focus_manager,
                                    }
                                }
                            }
                        } else {
                            textarea {
                                class: "text-editor",
                                id: "textEditor",
                                autofocus: true,
                                onfocus: move |_e| {
                                    focus.focus(FocusComponent::Editor);
                                },
                                onmounted: move |e| {
                                    focus_manager.register_and_focus(FocusComponent::Editor, e.data());
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
                                    app_state.write().mark_content_dirty();
                                },
                                onkeydown: move |event: Event<KeyboardData>| {
                                    let data = event.data();
                                    if let Some(ActionShortcuts::Text(action)) = settings
                                        .read()
                                        .key_bindings
                                        .get_action(&data.into())
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
                    },
                }
            }
        }
    }
}

fn eval_action(action: &str) {
    let eval = document::eval(EVAL_JS);
    if let Err(e) = eval.send(action) {
        error!("Error sending value {}: {}", action, e);
    }
}
