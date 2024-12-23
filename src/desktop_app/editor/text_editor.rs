use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_logger::tracing::info;
use futures::StreamExt;

use crate::{
    core_notes::{nfs::NotePath, NoteVault},
    desktop_app::AppContext,
};

const AUTOSAVE_SECS: u64 = 5;

enum EditorMsg {
    Init { path: NotePath, text: String },
    Save,
    Update { text: String },
}

struct EditorData {
    text: String,
    path: NotePath,
    is_dirty: bool,
    vault: NoteVault,
}

impl EditorData {
    async fn save(&mut self) {
        let path = self.path.clone();
        let text = self.text.clone();
        let vault = self.vault.clone();
        if self.is_dirty
            && tokio::spawn(async move {
                info!("Saving");
                vault.save_note(&path, text);
            })
            .await
            .is_ok()
        {
            self.is_dirty = false;
        };
    }
}

impl Drop for EditorData {
    fn drop(&mut self) {
        info!("Dropping Editor Data, saving so we don't lose data");
        if self.is_dirty {
            self.vault.save_note(&self.path, &self.text);
        };
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TextEditorProps {
    note_path: SyncSignal<Option<NotePath>>,
    editor_signal: Signal<Option<Rc<MountedData>>>,
}

#[allow(non_snake_case)]
pub fn TextEditor(props: TextEditorProps) -> Element {
    // to recover the focus
    let mut editor_area = props.editor_signal;

    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;
    let content_edit_vault: NoteVault = vault.clone();
    let note_path = props.note_path;

    // This manages the editor state
    let cr = use_coroutine(move |mut rx: UnboundedReceiver<EditorMsg>| {
        let vault = vault.clone();
        async move {
            let mut editor_data: Option<EditorData> = None;
            while let Some(msg) = rx.next().await {
                match msg {
                    EditorMsg::Init { text, path } => {
                        info!("Initializing: {}", path);
                        if let Some(editor_data) = editor_data.as_mut() {
                            editor_data.save().await;
                        }
                        editor_data = Some(EditorData {
                            text,
                            path,
                            is_dirty: false,
                            vault: vault.clone(),
                        })
                    }
                    EditorMsg::Update { text } => {
                        if let Some(editor_data) = editor_data.as_mut() {
                            editor_data.text = text;
                            editor_data.is_dirty = true;
                        }
                    }
                    EditorMsg::Save => {
                        if let Some(editor_data) = editor_data.as_mut() {
                            editor_data.save().await;
                        }
                    }
                }
            }
        }
    });

    // AutoSave every 5 seconds
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(AUTOSAVE_SECS)).await;
            cr.send(EditorMsg::Save);
        }
    });

    let vault = content_edit_vault.clone();
    // TODO: Consider use_resource to async load the content
    let content = use_memo(move || {
        if let Some(path) = &*note_path.read() {
            let text = vault.load_note(path).unwrap_or_else(|_e| "".to_string());
            cr.send(EditorMsg::Init {
                path: path.to_owned(),
                text: text.to_owned(),
            });
            text
        } else {
            "".to_string()
        }
    });

    let disabled = note_path.read().as_ref().is_none();

    rsx! {
        textarea {
            class: "edittext",
            onmounted: move |e| {
                *editor_area.write() = Some(e.data());
            },
            oninput: move |e| {
                cr.send(EditorMsg::Update { text: e.value() });
                // if let Some(content) = &*content_edit.read() { content.update_content(e.value()) }
            },
            spellcheck: false,
            wrap: "hard",
            resize: "none",
            placeholder: if disabled
                { "Create or select a note" }
                    else
                { "Start writing something!" },
            disabled: disabled,
            value: "{content}"
        }
    }
}
