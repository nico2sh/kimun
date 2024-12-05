use core_notes::{nfs::NotePath, NoteVault};
use std::rc::Rc;

use dioxus::prelude::*;

use crate::{editor::markdown::Markdown, AppContext};

#[derive(Props, Clone, PartialEq)]
pub struct TextEditorProps {
    note_path: Signal<Option<NotePath>>,
    editor_signal: Signal<Option<Rc<MountedData>>>,
}

#[allow(non_snake_case)]
pub fn TextEditor(props: TextEditorProps) -> Element {
    // to recover the focus
    let mut editor = props.editor_signal;

    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;
    let note_path = props.note_path;
    let content = use_memo(move || {
        note_path.read().as_ref().map_or_else(String::new, |e| {
            vault.load_note(e).unwrap_or_else(|_e| "".to_string())
        })
    });

    // info!("Content: {}", content.read());
    // let eval = eval(
    //     r#"codeInput.registerTemplate("syntax-highlighted", codeInput.templates.prism(Prism, [] /* Array of plugins */));"#,
    // );
    //
    // let future = use_resource(move || {
    //     to_owned![eval];
    //     async move {
    //         match eval.recv().await {
    //             Ok(res) => info!("Success: {}", res),
    //             Err(err) => error!("Error receiving the result of the Code Input JS: {:?}", err),
    //         }
    //     }
    // });

    // let _use_highlighter = match future.read_unchecked().as_ref() {
    //     Some(_v) => {
    //         info!("Use highlighter");
    //         true
    //     }
    //     None => {
    //         info!("No highlighter");
    //         false
    //     }
    // };
    let use_highlighter = false;
    let class = use_signal(|| String::from("content"));
    rsx! {
        if use_highlighter {
            code-input {
                language: "Markdown",
                // template: "syntax-highlighted",
                // wrap: "hard",
                resize: "none",
                placeholder: "Insert your note text",
                value: "{content}",
            }
        } else {
            Markdown {
                class: class,
                content: "{content}"
            }
            // textarea {
            //     class: "edittext",
            //     onmounted: move |e| {
            //         *editor.write() = Some(e.data());
            //     },
            //     spellcheck: false,
            //     wrap: "hard",
            //     resize: "none",
            //     placeholder: "Insert your note text",
            //     value: "{content}",
            // }
        }
    }
}
