use std::{rc::Rc, sync::Arc};

use dioxus::{
    logger::tracing::{debug, error, info},
    prelude::*,
};
use futures::StreamExt;
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::settings::AppSettings;

const AUTOSAVE_SECS: u64 = 5;

#[derive(Default, Debug, Clone, PartialEq)]
pub enum EditorContent {
    #[default]
    Loading,
    Enabled {
        content: String,
    },
}

#[derive(Debug, Clone)]
pub struct EditorData {
    text: String,
    path: VaultPath,
    vault: Arc<NoteVault>,
}

impl EditorData {
    async fn save(&mut self) -> anyhow::Result<()> {
        let path = self.path.clone();
        let text = self.text.clone();
        let vault = self.vault.clone();
        tokio::spawn(async move {
            debug!("Saving at {}", path);
            let _ = vault.save_note(&path, text);
        })
        .await?;
        Ok(())
    }
}

pub enum EditorMsg {
    Init { editor_data: Option<EditorData> },
    Save,
    Update { text: String },
}

impl Drop for EditorData {
    fn drop(&mut self) {
        debug!(
            "Dropping Editor Data at path {}, saving so we don't lose data",
            self.path
        );
        let _ = self.vault.save_note(&self.path, &self.text);
    }
}

#[component]
pub fn EditorHeader(
    path: ReadOnlySignal<VaultPath>,
    show_browser: Signal<bool>,
    dirty_status: Signal<bool>,
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
                div {
                    class: if !dirty_status() { "status-indicator" } else { "status-indicator unsaved" },
                    id: "saveStatus",
                }
            }
        }
    }
}

#[component]
pub fn NoText(path: ReadOnlySignal<VaultPath>) -> Element {
    let mut editor_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    spawn(async move {
        loop {
            if let Some(e) = editor_signal.with(|f| f.clone()) {
                debug!("Attached main UI for focus");
                let _ = e.set_focus(true).await;
                break;
            }
        }
    });

    rsx! {
        div {
            class: "editor-empty",
            onmounted: move |e| {
                editor_signal.set(Some(e.data()));
            },
            div { class: "title", "Current path: {path}" }
            div { class: "message", "Open or create a new note." }
            img { class: "logo", src: "assets/images/kimun.png" }
        }
    }
}

#[component]
pub fn TextEditor(
    vault: Arc<NoteVault>,
    note_path: ReadOnlySignal<VaultPath>,
    dirty_status: Signal<bool>,
) -> Element {
    debug!(
        "-==== [Text Editor] Starting Editor at '{}' ====-",
        note_path
    );
    // This is for the autofocus
    let mut editor_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    spawn(async move {
        loop {
            if let Some(e) = editor_signal.with(|f| f.clone()) {
                debug!("Attached main UI for focus");
                let _ = e.set_focus(true).await;
                break;
            }
        }
    });

    let mut settings: Signal<AppSettings> = use_context();
    let cr = use_coroutine(move |mut rx: UnboundedReceiver<EditorMsg>| async move {
        debug!("We start listening for editor update events");
        let mut ed = None;
        while let Some(msg) = rx.next().await {
            match msg {
                EditorMsg::Init { editor_data } => {
                    ed = editor_data;
                }
                EditorMsg::Update { text } => {
                    if let Some(ed) = ed.as_mut() {
                        ed.text = text;
                        dirty_status.set(true);
                    }
                }
                EditorMsg::Save => {
                    if let Some(editor_data) = ed.as_mut() {
                        let is_dirty_val = dirty_status.read().clone();
                        if is_dirty_val {
                            if editor_data.save().await.is_ok() {
                                dirty_status.set(false);
                            }
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

    let initial_content = use_resource(move || {
        debug!("[Initial Content] Loading text content");
        let editor_vault = vault.clone();
        async move {
            let exists = editor_vault.exists(&note_path.read()).is_some();
            debug!("[Initial Content] Exists: {:?}", exists);
            if exists {
                debug!("[Initial Content] Loading from path at {}", note_path);
                let text = editor_vault.load_note(&note_path.read()).map_or_else(
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
                let editor_data = EditorData {
                    text: text.clone(),
                    path: note_path.read().to_owned(),
                    vault: editor_vault.clone(),
                };
                cr.send(EditorMsg::Init {
                    editor_data: Some(editor_data),
                });
                debug!("[Initial Content] Init message sent");
                EditorContent::Enabled { content: text }
            } else {
                let text = "".to_string();
                debug!(
                    "[Initial Content] Creating new Editor Data for new note with path: {}",
                    note_path
                );
                let editor_data = EditorData {
                    text: text.clone(),
                    path: note_path.read().to_owned(),
                    vault: editor_vault.clone(),
                };
                cr.send(EditorMsg::Init {
                    editor_data: Some(editor_data),
                });
                EditorContent::Enabled { content: text }
            }
        }
    });

    let content = initial_content
        .read_unchecked()
        .as_ref()
        .map_or_else(EditorContent::default, |ec| ec.to_owned());

    // This manages the editor state
    rsx! {
        div { class: "editor-content",
            {
                match content {
                    EditorContent::Loading => rsx! {
                        div {
                            onmounted: move |e| {
                                *editor_signal.write() = Some(e.data());
                            },
                            "Loading..."
                        }
                    },
                    EditorContent::Enabled { content } => rsx! {
                        textarea {
                            class: "text-editor",
                            id: "textEditor",
                            autofocus: true,
                            onmounted: move |e| {
                                *editor_signal.write() = Some(e.data());
                            },
                            onselect: move |e| {
                                info!("Select event {:?}", e);
                            },
                            oninput: move |e| {
                                cr.send(EditorMsg::Update {
                                    text: e.value(),
                                });
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
