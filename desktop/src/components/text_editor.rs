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
pub enum EditorStatus {
    #[default]
    Loading,
    Enabled {
        content: String,
    },
}

#[derive(Debug, Clone)]
pub struct EditorData {
    path: VaultPath,
    vault: Arc<NoteVault>,
    content: Signal<Option<EditorContent>>,
}

#[derive(Debug, Clone)]
pub struct EditorContent {
    text: String,
    dirty_status: bool,
}

impl EditorData {
    pub fn new(
        path: VaultPath,
        vault: Arc<NoteVault>,
        content: Signal<Option<EditorContent>>,
    ) -> Self {
        Self {
            path,
            vault,
            content,
        }
    }

    async fn save(&mut self) -> anyhow::Result<()> {
        if let Some(content) = self.content.as_mut() {
            if content.dirty_status {
                let path = self.path.clone();
                let text = content.text.clone();
                let vault = self.vault.clone();
                tokio::spawn(async move {
                    debug!("Saving at {}", path);
                    let _ = vault.save_note(&path, text);
                })
                .await?;
                content.dirty_status = false;
            }
        }
        Ok(())
    }

    pub fn update_text(&mut self, text: String) {
        if let Some(content) = self.content.as_mut() {
            content.text = text;
            content.dirty_status = true;
        } else {
            self.content = Some(EditorContent {
                text,
                dirty_status: false,
            })
        }
    }
}

impl Drop for EditorData {
    fn drop(&mut self) {
        debug!("Dropping Editor Data at path {}", self.path);
        if let Some(content) = self.content.as_mut() {
            if content.dirty_status {
                debug!("Saving so we don't lose data");
                let _ = self.vault.save_note(&self.path, &content.text);
            }
        }
    }
}

pub enum EditorMsg {
    Save,
}

#[component]
pub fn EditorHeader(
    path: ReadOnlySignal<VaultPath>,
    show_browser: Signal<bool>,
    editor_signal: Signal<EditorData>,
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
                if let Some(content) = editor_signal().content.as_ref() {
                    div {
                        class: if !content.dirty_status { "status-indicator" } else { "status-indicator unsaved" },
                        id: "saveStatus",
                    }
                }
            }
        }
    }
}

#[component]
pub fn NoText(path: ReadOnlySignal<VaultPath>) -> Element {
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
    editor_signal: Signal<Option<EditorContent>>,
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
    let cr = use_coroutine(move |mut rx: UnboundedReceiver<EditorMsg>| async move {
        debug!("We start listening for editor update events");
        while let Some(msg) = rx.next().await {
            match msg {
                EditorMsg::Save => {
                    debug!("Received save signal");
                    let _ = editor_signal.write().save();
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
        let vault = editor_signal.peek().vault.clone();
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
                editor_signal.write().update_text(text.clone());
                debug!("[Initial Content] Init message sent");
                EditorStatus::Enabled { content: text }
            } else {
                let text = "".to_string();
                debug!(
                    "[Initial Content] Creating new Editor Data for new note with path: {}",
                    note_path
                );
                editor_signal.write().update_text(text.clone());
                EditorStatus::Enabled { content: text }
            }
        }
    });

    let content = initial_content
        .read_unchecked()
        .as_ref()
        .map_or_else(EditorStatus::default, |ec| ec.to_owned());

    // This manages the editor state
    rsx! {
        div { class: "editor-content",
            {
                match content {
                    EditorStatus::Loading => rsx! {
                        div {
                            onmounted: move |e| {
                                *text_area_signal.write() = Some(e.data());
                            },
                            "Loading..."
                        }
                    },
                    EditorStatus::Enabled { content } => rsx! {
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
