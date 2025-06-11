use std::{rc::Rc, sync::Arc};

use dioxus::{
    logger::tracing::{debug, error, info},
    prelude::*,
};
use futures::{lock::Mutex, StreamExt};
use kimun_core::{nfs::VaultPath, NoteVault};

use crate::{
    components::{
        modal::{indexer::IndexType, Modal},
        text_editor::{EditorHeader, TextEditor},
    },
    route::Route,
    settings::AppSettings,
};

const AUTOSAVE_SECS: u64 = 5;

#[derive(Clone)]
struct EditorData {
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
            info!("Saving");
            let _ = vault.save_note(&path, text);
        })
        .await?;
        Ok(())
    }
}

impl Drop for EditorData {
    fn drop(&mut self) {
        info!("Dropping Editor Data, saving so we don't lose data");
        let _ = self.vault.save_note(&self.path, &self.text);
    }
}

pub enum EditorMsg {
    Save,
    Update { text: String },
}

#[component]
pub fn Editor() -> Element {
    let mut settings: Signal<AppSettings> = use_context();
    let settings_value = settings.read();
    let last_note_path = settings_value.last_paths.last().map(|p| p.to_owned());
    let mut note_path = use_signal_sync(|| last_note_path);
    let disabled = note_path.read().is_none();
    let mut is_dirty = use_signal(|| false);

    let vault_path = settings_value.workspace_dir.as_ref().unwrap();
    let vault = NoteVault::new(vault_path).unwrap();
    debug!("Opening editor at {:?}", vault.workspace_path);

    let vault = Arc::new(vault);

    let editor_vault = vault.clone();
    let editor_data: Arc<Mutex<Option<EditorData>>> = Arc::new(Mutex::new(None));

    let ed = editor_data.clone();
    let initial_content = use_resource(move || {
        let editor_vault = editor_vault.clone();
        let editor_data = ed.clone();
        async move {
            match &*note_path.read() {
                Some(path) => {
                    info!("New path at {}", path);
                    let text = editor_vault.load_note(&path).map_or_else(
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
                    let mut editor_data = editor_data.lock().await;
                    *editor_data = Some(EditorData {
                        text: text.clone(),
                        path: path.to_owned(),
                        vault: editor_vault.clone(),
                    });
                    text
                }
                None => "".to_string(),
            }
        }
    });

    let ed = editor_data.clone();
    let cr = use_coroutine(move |mut rx: UnboundedReceiver<EditorMsg>| {
        let editor_data = ed.clone();
        async move {
            let mut ed = editor_data.lock().await.clone();
            while let Some(msg) = rx.next().await {
                match msg {
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
        }
    });
    // AutoSave every 5 seconds
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(AUTOSAVE_SECS)).await;
            cr.send(EditorMsg::Save);
        }
    });

    let note_path_display = use_memo(move || {
        let np = match note_path.read().to_owned() {
            Some(path) => path,
            None => VaultPath::root(),
        };
        if np.is_note() {
            np.to_string()
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
            info!("triggering indexer");
            modal
                .write()
                .set_indexer(index_vault.clone(), IndexType::Validate);
        }
    });
    if !modal.read().is_open() {
        // TODO: Try with use_future
        spawn(async move {
            loop {
                if let Some(e) = editor_signal.with(|f| f.clone()) {
                    let _ = e.set_focus(true).await;
                    break;
                }
            }
        });
    }

    let initial_content = initial_content
        .read_unchecked()
        .as_ref()
        .map_or_else(|| String::new(), |c| c.to_owned());

    rsx! {
        div {
            tabindex: 0,
            class: "editor-container",
            onkeydown: move |event: Event<KeyboardData>| {
                let key = event.data.code();
                let modifiers = event.data.modifiers();
                if modifiers.meta() {
                    match key {
                        Code::KeyO => {
                            debug!("Trigger Open Note Select");
                            modal.write().set_note_select(vault.clone(), note_path);
                        }
                        Code::KeyK => {
                            debug!("Trigger Open Note Search");
                            modal.write().set_note_search(vault.clone(), note_path);
                        }
                        Code::KeyJ => {
                            debug!("New Journal Entry");
                            if let Ok(journal_entry) = vault.journal_entry() {
                                note_path.set(Some(journal_entry.0.path));
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
                TextEditor {
                    content: initial_content,
                    editor_signal,
                    disabled,
                    cr,
                }
            }
            div { class: "editor-footer" }
        }
    }
}
