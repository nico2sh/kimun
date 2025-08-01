use std::{rc::Rc, sync::Arc};

use dioxus::{
    logger::tracing::{debug, error, info},
    prelude::*,
};
use futures::StreamExt;
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    global_events::{GlobalEvent, PubSub},
    settings::AppSettings,
};

const AUTOSAVE_SECS: u64 = 5;
const TEXT_EDITOR: &str = "text_editor";

#[derive(Debug, Clone)]
pub struct EditorData {
    path: VaultPath,
    vault: Arc<NoteVault>,
    content: Signal<EditorContent>,
}

#[derive(Debug, Clone, Default)]
pub enum EditorContent {
    #[default]
    None,
    Note {
        text: String,
        dirty_status: bool,
    },
}

impl EditorContent {
    pub fn init(&mut self, new_text: String) {
        // Make sure you saved the content before
        match self {
            EditorContent::None => {
                *self = EditorContent::Note {
                    text: new_text,
                    dirty_status: false,
                }
            }
            EditorContent::Note { text, dirty_status } => {
                *text = new_text;
                *dirty_status = false;
            }
        }
    }

    pub fn update_text(&mut self, new_text: String) {
        match self {
            EditorContent::None => {
                *self = EditorContent::Note {
                    text: new_text,
                    dirty_status: false,
                }
            }
            EditorContent::Note { text, dirty_status } => {
                *text = new_text;
                *dirty_status = true;
            }
        }
    }

    pub fn has_content(&self) -> bool {
        match self {
            EditorContent::Note {
                text: _,
                dirty_status: _,
            } => true,
            _ => false,
        }
    }
    pub fn is_dirty(&self) -> bool {
        match self {
            EditorContent::Note {
                text: _,
                dirty_status,
            } => *dirty_status,
            _ => false,
        }
    }

    pub fn get_text(&self) -> String {
        match self {
            EditorContent::Note {
                text,
                dirty_status: _,
            } => text.to_owned(),
            _ => "".to_string(),
        }
    }

    pub fn mark_clean(&mut self) {
        if let EditorContent::Note {
            text: _,
            dirty_status,
        } = self
        {
            *dirty_status = false
        }
    }
}

impl EditorData {
    pub fn new(path: VaultPath, vault: Arc<NoteVault>, content: Signal<EditorContent>) -> Self {
        Self {
            path,
            vault,
            content,
        }
    }

    async fn save(&mut self) -> anyhow::Result<()> {
        debug!("Triggered save");
        let dirty_status = self.content.read().is_dirty();
        if dirty_status {
            let path = self.path.clone();
            let text = self.content.read().get_text();
            let vault = self.vault.clone();
            tokio::spawn(async move {
                debug!("Saving at {}", path);
                let _ = vault.save_note(&path, text);
            })
            .await?;
            self.content.write().mark_clean();
        }
        Ok(())
    }
}

impl Drop for EditorData {
    fn drop(&mut self) {
        debug!("Dropping Editor Data at path {}", self.path);
        let dirty_status = self.content.read().is_dirty();
        if dirty_status {
            debug!("Saving so we don't lose data");
            let text = self.content.read().get_text();
            let _ = self.vault.save_note(&self.path, text);
        }
    }
}

pub enum EditorMsg {
    Init { text: String },
    Save,
}

#[component]
pub fn EditorHeader(
    path: ReadOnlySignal<VaultPath>,
    show_browser: Signal<bool>,
    editor_signal: Signal<EditorContent>,
) -> Element {
    let note_path_display = path.read().to_string();
    rsx! {
        div { class: "editor-header",
            div { class: "header-left",
                button {
                    class: "sidebar-toggle-main",
                    onclick: move |_| {
                        let showing = *show_browser.read();
                        show_browser.set(!showing);
                    },
                    svg {
                        width: 20,
                        height: 20,
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        line {
                            x1: 3,
                            y1: 6,
                            x2: 21,
                            y2: 6,
                        }
                        line {
                            x1: 3,
                            y1: 12,
                            x2: 21,
                            y2: 12,
                        }
                        line {
                            x1: 3,
                            y1: 18,
                            x2: 21,
                            y2: 18,
                        }
                    }
                }
            }
            div { class: "title-section",
                div { class: "title-text", "{note_path_display}" }
                if editor_signal().has_content() {
                    div {
                        class: if !editor_signal().is_dirty() { "status-indicator" } else { "status-indicator unsaved" },
                        id: "saveStatus",
                    }
                }
            }
        }
    }
}

#[component]
pub fn NoText(path: ReadOnlySignal<VaultPath>, editor_signal: Signal<EditorContent>) -> Element {
    use_effect(move || *editor_signal.write() = EditorContent::None);

    let mut text_area_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
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

#[component]
pub fn TextEditor(
    note_path: ReadOnlySignal<VaultPath>,
    vault: Arc<NoteVault>,
    editor_signal: Signal<EditorContent>,
) -> Element {
    debug!(
        "-==== [Text Editor] Starting Editor at '{}' ====-",
        note_path
    );
    // This is for the autofocus
    let mut text_area_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    spawn(async move {
        loop {
            if let Some(e) = text_area_signal.with(|f| f.clone()) {
                debug!("Attached main UI for focus");
                let _ = e.set_focus(true).await;
                break;
            }
        }
    });

    let mut settings: Signal<AppSettings> = use_context();

    let editor_vault = vault.clone();
    let cr = use_coroutine(move |mut rx: UnboundedReceiver<EditorMsg>| {
        let editor_vault = editor_vault.clone();
        async move {
            debug!("We start listening for editor update events");
            let mut ed: Option<EditorData> = None;
            while let Some(msg) = rx.next().await {
                match msg {
                    EditorMsg::Init { text } => {
                        // We check if we already have an editor_data and we save
                        if let Some(editor_data) = ed.as_mut() {
                            let _ = editor_data.save().await;
                        }
                        // We create a new instance of the editor data

                        editor_signal.write().init(text.clone());
                        let editor_data = EditorData {
                            content: editor_signal,
                            path: note_path.read().to_owned(),
                            vault: editor_vault.clone(),
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

    let vault_content = vault.clone();
    let note_content = use_resource(move || {
        debug!("[Initial Content] Loading text content");
        let vault = vault_content.clone();
        async move {
            let exists = vault.exists(&note_path.read()).is_some();
            debug!("[Initial Content] Exists: {:?}", exists);
            if exists {
                debug!("[Initial Content] Loading from path at {}", note_path);
                let text = vault.load_note(&note_path.read()).map_or_else(
                    |e| {
                        error!("[Initial Content] Error loading Note: {}", e);
                        String::new()
                    },
                    |d| {
                        // We save the settings for the last opened notes
                        debug!("[Initial Content] Saving path history");
                        settings.write().add_path_history(&note_path.read());
                        // We don't want the settings to trigger a re-run every time it changes, so we use `peek()` instead of `read()`
                        let _r = settings.peek().save_to_disk();
                        d.raw_text
                    },
                );
                debug!(
                    "[Initial Content] Creating Editor Data for existing note with path: {}",
                    note_path
                );
                cr.send(EditorMsg::Init { text: text.clone() });
                debug!("[Initial Content] Init message sent");
                text
            } else {
                let text = "".to_string();
                debug!(
                    "[Initial Content] Creating new Editor Data for new note with path: {}",
                    note_path
                );
                cr.send(EditorMsg::Init { text: text.clone() });
                debug!("[Initial Content] Init message sent");
                text
            }
        }
    });

    let mut pub_sub: Signal<PubSub> = use_context();
    use_effect(move || {
        let vault = vault.clone();
        pub_sub.write().subscribe(
            TEXT_EDITOR.to_string(),
            Callback::new(move |g| {
                match g {
                    GlobalEvent::SaveCurrentNote => {
                        let dirty_status = editor_signal.peek().is_dirty();
                        if dirty_status {
                            debug!("Saving so we don't lose data");
                            let text = editor_signal.peek().get_text();
                            let _ = vault.save_note(&note_path.read(), text);
                            editor_signal.write().mark_clean();
                        }
                    }
                    GlobalEvent::MarkNoteClean => {
                        editor_signal.write().mark_clean();
                    }
                    _ => {}
                }
                debug!("Saving a note");
            }),
        );
    });
    use_drop(move || {
        pub_sub.write().unsubscribe(TEXT_EDITOR.to_string());
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
                    Some(content) => rsx! {
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
                                editor_signal.write().update_text(e.value());
                            },
                            onkeydown: move |e| {
                                match e.key() {
                                    Key::Tab => {
                                        e.prevent_default();
                                    }
                                    _ => {}
                                }
                            },
                            spellcheck: false,
                            wrap: "hard",
                            resize: "none",
                            placeholder: "Start writing something!",
                            value: "{content}",
                        }
                    },
                }
            }
        }
    }
}
