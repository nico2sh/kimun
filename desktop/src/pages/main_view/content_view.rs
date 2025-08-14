use std::{fmt::Display, rc::Rc, sync::Arc};

use dioxus::{
    logger::tracing::{debug, error, info},
    prelude::*,
};
use dioxus_radio::hooks::{use_radio, Radio};
use futures::StreamExt;
use kimun_core::{nfs::VaultPath, note::NoteDetails, NoteVault};

use crate::{
    components::{modal::ModalType, preview::Markdown},
    global_events::{GlobalEvent, PubSub},
    settings::AppSettings,
    state::{AppState, ContentType, KimunChannel},
};

const AUTOSAVE_SECS: u64 = 5;
const TEXT_EDITOR: &str = "text_editor";

#[derive(Debug, Clone, Default)]
pub enum EditorContentState {
    #[default]
    None,
    Note {
        text: String,
    },
}

impl Display for EditorContentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get_text())
    }
}

impl EditorContentState {
    pub fn init(&mut self, new_text: String) {
        // Make sure you saved the content before
        match self {
            EditorContentState::None => *self = EditorContentState::Note { text: new_text },
            EditorContentState::Note { text } => {
                *text = new_text;
            }
        }
    }

    pub fn update_text(&mut self, new_text: String) {
        match self {
            EditorContentState::None => *self = EditorContentState::Note { text: new_text },
            EditorContentState::Note { text } => {
                *text = new_text;
            }
        }
    }

    pub fn get_text(&self) -> String {
        match self {
            EditorContentState::Note { text } => text.to_owned(),
            _ => "".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct EditorSaveManager {
    path: VaultPath,
    vault: Arc<NoteVault>,
    content: Signal<EditorContentState>,
    app_state: Radio<AppState, KimunChannel>,
}

impl EditorSaveManager {
    async fn save(&mut self) -> anyhow::Result<()> {
        debug!("Triggered save");
        let dirty_status = self.app_state.read().has_dirty_content();
        if dirty_status {
            let path = self.path.clone();
            let text = self.content.read().get_text();
            let vault = self.vault.clone();
            tokio::spawn(async move {
                debug!("Saving at {}", path);
                let _ = vault.save_note(&path, text);
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
            let text = self.content.read().get_text();
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
pub fn NoText(path: ReadOnlySignal<VaultPath>) -> Element {
    let mut text_area_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);

    let mut app_state = use_radio::<AppState, KimunChannel>(KimunChannel::Header);
    use_effect(move || app_state.write().set_content_type(ContentType::Directory));
    // spawn(async move {
    //     loop {
    //         if let Some(e) = text_area_signal.with(|f| f.clone()) {
    //             debug!("Attached main UI for focus");
    //             let _ = e.set_focus(true).await;
    //             break;
    //         }
    //     }
    // });

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
    note_path: ReadOnlySignal<VaultPath>,
    vault: Arc<NoteVault>,
    modal_type: Signal<ModalType>,
    preview: Signal<bool>,
}

#[component]
pub fn TextEditor(props: TextEditorProps) -> Element {
    debug!(
        "-==== [Text Editor] Starting Editor at '{}' ====-",
        props.note_path
    );
    let mut app_state = use_radio::<AppState, KimunChannel>(KimunChannel::Header);
    let mut content_state = use_signal(|| EditorContentState::None);
    let modal_type = props.modal_type;

    // This is for the autofocus
    let mut text_area_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    spawn(async move {
        loop {
            if let Some(e) = text_area_signal.with(|f| f.clone()) {
                if !modal_type.read().is_open() {
                    debug!("Attached main UI for focus");
                    let _ = e.set_focus(true).await;
                }
                break;
            }
        }
    });

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
                        // We check if we already have an editor_data and we save
                        if let Some(editor_data) = ed.as_mut() {
                            let _ = editor_data.save().await;
                        }
                        // We create a new instance of the editor data
                        content_state.write().init(text.clone());
                        app_state
                            .write()
                            .set_content_type(crate::state::ContentType::Note { dirty: false });
                        let editor_data = EditorSaveManager {
                            content: content_state,
                            path: props.note_path.read().to_owned(),
                            vault: editor_vault.clone(),
                            app_state,
                        };
                        ed = Some(editor_data);
                    }
                    EditorMsg::Save => {
                        debug!("Received save signal");
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
    let note_content = use_resource(move || {
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
                            let text = content_state.peek().get_text();
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
    use_drop(move || {
        pub_sub.unsubscribe(TEXT_EDITOR);
    });

    // This manages the editor state
    rsx! {
        div { class: "editor-content",
            {
                match &*note_content.read() {
                    None => rsx! {
                        div {
                            onmounted: move |e| {
                                *text_area_signal.write() = Some(e.data());
                            },
                            "Loading..."
                        }
                    },
                    Some(_) => rsx! {
                        if *props.preview.read() {
                            {
                                let note_details = NoteDetails::new(&props.note_path.read(), content_state.read().get_text());
                                let md_content = note_details.get_markdown_and_links();
                                rsx!{
                                    Markdown { vault: props.vault.clone(), note_md: md_content.text, note_links: md_content.links, modal_type }
                                }
                            }
                        } else {
                            textarea {
                                class: "text-editor",
                                id: "textEditor",
                                autofocus: true,
                                onmounted: move |e| {
                                    *text_area_signal.write() = Some(e.data());
                                },
                                onselect: move |e| {
                                    info!("Select event {:?}", e);
                                },
                                oninput: move |e| {
                                    content_state.write().update_text(e.value());
                                    app_state.write().mark_content_dirty();
                                },
                                onkeydown: move |e| {
                                    if e.key() == Key::Tab {
                                        e.prevent_default();
                                    }
                                },
                                spellcheck: false,
                                wrap: "hard",
                                resize: "none",
                                placeholder: "Start writing something!",
                                value: "{content_state.peek()}",
                            }
                        }
                    },
                }
            }
        }
    }
}
