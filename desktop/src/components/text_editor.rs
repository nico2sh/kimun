use std::{rc::Rc, sync::Arc};

use crate::settings::AppSettings;
use dioxus::{
    logger::tracing::{error, info},
    prelude::*,
};
use futures::StreamExt;
use kimun_core::{nfs::VaultPath, NoteVault};

const AUTOSAVE_SECS: u64 = 5;

enum EditorMsg {
    Init { path: VaultPath, text: String },
    Save,
    Update { text: String },
}

struct EditorData {
    text: String,
    path: VaultPath,
    is_dirty: bool,
    vault: Arc<NoteVault>,
}

impl EditorData {
    async fn save(&mut self) {
        let path = self.path.clone();
        let text = self.text.clone();
        let vault = self.vault.clone();
        if self.is_dirty
            && tokio::spawn(async move {
                info!("Saving");
                let _ = vault.save_note(&path, text);
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
            let _ = self.vault.save_note(&self.path, &self.text);
        };
    }
}

#[component]
pub fn TextEditor(
    vault: Arc<NoteVault>,
    note_path: SyncSignal<Option<VaultPath>>,
    editor_signal: Signal<Option<Rc<MountedData>>>,
) -> Element {
    // to recover the focus
    let content_edit_vault = vault.clone();

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
    let mut settings: Signal<AppSettings> = use_context();

    // TODO: Consider use_resource to async load the content
    let content = use_memo(move || {
        if let Some(path) = &*note_path.read() {
            let text = vault.load_note(path).map_or_else(
                |e| {
                    error!("Error loading Note: {}", e);
                    String::new()
                },
                |d| {
                    // We save the settings for the last opened notes
                    settings.write().add_path_history(path);
                    let _r = settings.read().save_to_disk();
                    d.raw_text
                },
            );
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
                *editor_signal.write() = Some(e.data());
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
            placeholder: if disabled { "Create or select a note" } else { "Start writing something!" },
            value: "{content}",
        }
    }
}
