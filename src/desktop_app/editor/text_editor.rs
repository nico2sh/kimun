use crate::{
    core_notes::{nfs::NotePath, NoteVault},
    desktop_app::AppContext,
};
use dioxus_logger::tracing::{debug, info};
use std::{
    fmt::{Display, Formatter},
    rc::Rc,
};

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct TextEditorProps {
    note_path: SyncSignal<Option<NotePath>>,
    editor_signal: Signal<Option<Rc<MountedData>>>,
}

#[allow(non_snake_case)]
pub fn TextEditor(props: TextEditorProps) -> Element {
    // to recover the focus
    let mut editor = props.editor_signal;
    info!("Open Text Editor");

    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;
    let note_path = props.note_path;
    let mut content_edit =
        use_signal_sync(|| ContentEdit::new(&vault, note_path.read().to_owned()));

    let load_vault = vault.clone();
    use_future(move || {
        let vault = load_vault.clone();
        async move {
            let disabled = note_path
                .read()
                .as_ref()
                .map_or_else(|| true, |np| !np.is_note());

            let content = if disabled {
                "".to_string()
            } else {
                let np = &*note_path.read();
                if let Some(path) = np {
                    vault.load_note(path.to_owned()).ok().unwrap_or_default()
                } else {
                    "".to_string()
                }
            };
            content_edit
                .write()
                .replace_content(&content, note_path.read().to_owned());
        }
    });

    use_future(move || {
        // let vault = vault.clone();
        async move {
            loop {
                // smol::Timer::after(Duration::from_secs(5)).await;
                content_edit.write().save();
                // let content = content_edit.read().content.to_owned();
                // let path = content_edit.read().path.to_owned();
                // // gloo_timers::future::TimeoutFuture::new(5_000).await;
                // if let Some(path) = &path {
                //     debug!("SAVING: {}", content);
                //     let vault = vault.clone();
                //     let path = path.clone();
                //     let content = &content_edit.read().content;
                //     let content = content.clone();
                //     smol::spawn(async move {
                //         vault.save_note(path.clone(), content.clone());
                //     })
                //     .await;
                // }
            }
        }
    });
    rsx! {
        // Markdown {
        //     class: class,
        //     content: "{content}"
        // }
        textarea {
            class: "edittext",
            onmounted: move |e| {
                *editor.write() = Some(e.data());
            },
            oninput: move |e| {
                content_edit.write().update_content(e.value().clone().to_string());
            },
            spellcheck: false,
            wrap: "hard",
            resize: "none",
            placeholder: if !content_edit.read().is_enabled() { "Create or select a note" } else { "Start writing something!" },
            disabled: !content_edit.read().is_enabled(),
            value: "{content_edit}",
        }
    }
}

#[derive(Debug, PartialEq)]
struct ContentEdit {
    content: String,
    has_changed: bool,
    vault: NoteVault,
    path: Option<NotePath>,
}

impl Display for ContentEdit {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content.to_owned())
    }
}

impl ContentEdit {
    fn new(vault: &NoteVault, path: Option<NotePath>) -> Self {
        Self {
            content: "".to_string(),
            has_changed: false,
            vault: vault.clone(),
            path,
        }
    }

    fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    fn save(&mut self) {
        if self.has_changed {
            if let Some(path) = self.path.clone() {
                self.has_changed = false;
                debug!("=================");
                debug!("About to Save");
                let vault = self.vault.clone();
                vault.save_note(path, self.content.clone());
                debug!("Content Saved:\n{}", self.content);
                debug!("=================");
            }
        }
    }

    // async fn save_async(&mut self) {
    //     if self.has_changed {
    //         if let Some(path) = self.path.clone() {
    //             self.has_changed = false;
    //             debug!("=================");
    //             debug!("About to Save");
    //             let vault = self.vault.clone();
    //             let path = path.clone();
    //             let content = self.content.clone();
    //             vault.save_note(path, content);
    //             debug!("Content Saved:\n{}", self.content);
    //             debug!("=================");
    //         }
    //     }
    // }

    fn replace_content<S: AsRef<str>>(&mut self, content: S, path: Option<NotePath>) {
        self.save();
        self.content = content.as_ref().to_owned();
        self.path = path;
    }

    fn update_content(&mut self, content: String) {
        // debug!("=================");
        // debug!("Updating content:\n{}", content);
        // debug!("=================");
        self.content = content;
        self.has_changed = true;
    }
}

impl Drop for ContentEdit {
    fn drop(&mut self) {
        debug!("-----------------");
        debug!("Saving content!\n{}", self.content);
        debug!("-----------------");
        self.save();
    }
}
