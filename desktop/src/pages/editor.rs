use std::{rc::Rc, sync::Arc};

use dioxus::{
    logger::tracing::{debug, error, info},
    prelude::*,
};
use futures::StreamExt;
use kimun_core::{
    nfs::{EntryData, VaultPath},
    NoteVault,
};

use crate::{
    components::{
        modal::{indexer::IndexType, Modal},
        note_browser::NoteBrowser,
        text_editor::{EditorHeader, TextEditor},
    },
    route::Route,
    settings::AppSettings,
};

const AUTOSAVE_SECS: u64 = 5;

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

impl Drop for EditorData {
    fn drop(&mut self) {
        debug!(
            "Dropping Editor Data at path {}, saving so we don't lose data",
            self.path
        );
        let _ = self.vault.save_note(&self.path, &self.text);
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
pub enum EditorContent {
    #[default]
    Loading,
    Enabled {
        content: String,
    },
    Disabled,
}

pub enum EditorMsg {
    Init { editor_data: Option<EditorData> },
    Save,
    Update { text: String },
}

#[component]
pub fn Editor(note_path: ReadOnlySignal<VaultPath>, create: bool) -> Element {
    debug!("-== [Editor] Starting Editor ==-");
    let mut settings: Signal<AppSettings> = use_context();
    let settings_value = settings.read();

    let vault_path = settings_value.workspace_dir.as_ref().unwrap();
    let vault = NoteVault::new(vault_path).unwrap();
    debug!("Opening note '{}'", note_path);

    let mut is_dirty = use_signal(|| false);
    let vault = Arc::new(vault);

    let editor_vault = vault.clone();

    let cr = use_coroutine(move |mut rx: UnboundedReceiver<EditorMsg>| async move {
        let mut ed = None;
        while let Some(msg) = rx.next().await {
            match msg {
                EditorMsg::Init { editor_data } => {
                    debug!("Initialized with Editor Data {:?}", editor_data);
                    ed = editor_data;
                }
                EditorMsg::Update { text } => {
                    if let Some(ed) = ed.as_mut() {
                        ed.text = text;
                        is_dirty.set(true);
                    }
                }
                EditorMsg::Save => {
                    if let Some(editor_data) = ed.as_mut() {
                        let is_dirty_val = is_dirty.read().clone();
                        if is_dirty_val {
                            if editor_data.save().await.is_ok() {
                                is_dirty.set(false);
                            }
                        }
                    }
                }
            }
        }
    });

    let initial_content = use_resource(move || {
        debug!("Loading content");
        let editor_vault = editor_vault.clone();
        async move {
            let exists = editor_vault
                .exists(&note_path.read())
                .and_then(|v| match v.data {
                    EntryData::Note(note_entry_data) => Some(note_entry_data),
                    EntryData::Directory(_directory_entry_data) => None,
                    EntryData::Attachment => None,
                });
            debug!("Exists: {:?}", exists);
            match exists {
                Some(entry_data) => {
                    debug!("Loading from path at {}", entry_data.path);
                    let text = editor_vault.load_note(&entry_data.path).map_or_else(
                        |e| {
                            error!("Error loading Note: {}", e);
                            String::new()
                        },
                        |d| {
                            // We save the settings for the last opened notes
                            debug!("Saving path history");
                            settings.write().add_path_history(&entry_data.path);
                            let _r = settings.read().save_to_disk();
                            d.raw_text
                        },
                    );
                    debug!(
                        "Creating new Editor Data for existing note with path: {}",
                        entry_data.path
                    );
                    let editor_data = EditorData {
                        text: text.clone(),
                        path: entry_data.path.to_owned(),
                        vault: editor_vault.clone(),
                    };
                    cr.send(EditorMsg::Init {
                        editor_data: Some(editor_data),
                    });
                    debug!("Message sent");
                    EditorContent::Enabled { content: text }
                }
                None => {
                    if create {
                        let text = "".to_string();
                        debug!(
                            "Creating new Editor Data for new note with path: {}",
                            note_path.read()
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
                    } else {
                        cr.send(EditorMsg::Init { editor_data: None });
                        EditorContent::Disabled
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

    let display_note_path = note_path.clone();
    let note_path_display = use_memo(move || {
        if display_note_path.read().is_note() {
            display_note_path.to_string()
        } else {
            String::new()
        }
    });

    // This is for the autofocus
    let editor_signal: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    // Modal setup and Indexing on the first run
    let mut modal = use_signal(Modal::new);
    let index_vault = vault.clone();
    use_effect(move || {
        if settings.read().needs_indexing() {
            debug!("triggering indexer");
            modal
                .write()
                .set_indexer(index_vault.clone(), IndexType::Validate);
        }
    });

    if !modal.read().is_open() {
        spawn(async move {
            loop {
                if let Some(e) = editor_signal.with(|f| f.clone()) {
                    let _ = e.set_focus(true).await;
                    break;
                }
            }
        });
    }

    let content = initial_content
        .read_unchecked()
        .as_ref()
        .map_or_else(EditorContent::default, |ec| ec.to_owned());

    rsx! {
        div { class: "sidebar open",
            NoteBrowser { vault: vault.clone(), base_path: note_path.read().to_owned() }
        }
        div { class: "editor-area",
            div {
                class: "editor-container",
                tabindex: 0,
                onkeydown: move |event: Event<KeyboardData>| {
                    let key = event.data.code();
                    let modifiers = event.data.modifiers();
                    if modifiers.meta() {
                        match key {
                            Code::KeyO => {
                                debug!("Trigger Open Note Select");
                                modal
                                    .write()
                                    .set_note_select(vault.clone(), note_path.read().clone());
                            }
                            Code::KeyK => {
                                debug!("Trigger Open Note Search");
                                modal.write().set_note_search(vault.clone());
                            }
                            Code::KeyJ => {
                                debug!("New Journal Entry");
                                if let Ok(journal_entry) = vault.journal_entry() {
                                    navigator()
                                        .replace(crate::Route::Editor {
                                            note_path: journal_entry.0.path,
                                            create: true,
                                        });
                                }
                            }
                            Code::Comma => {
                                debug!("Open Settings");
                                navigator().replace(Route::Settings {});
                            }
                            _ => {}
                        }
                    }
                },
                // We close any modal if we click on the main UI
                onclick: move |_e| {
                    if modal.read().is_open() {
                        modal.write().close();
                        info!("Close dialog");
                    }
                },
                {Modal::get_element(modal)}
                EditorHeader { note_path_display, is_dirty }
                div { class: "editor-main",
                    TextEditor { content, editor_signal, cr }
                }
                div { class: "editor-footer" }
            }
        }
    }
}
