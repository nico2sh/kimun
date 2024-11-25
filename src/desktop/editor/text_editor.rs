use dioxus::prelude::*;

use crate::{
    desktop::AppContext,
    noters::{nfs::NotePath, NoteVault},
};

#[derive(Props, Clone, PartialEq)]
pub struct TextEditorProps {
    note_path: Signal<Option<NotePath>>,
}

#[allow(non_snake_case)]
pub fn TextEditor(props: TextEditorProps) -> Element {
    let app_context: AppContext = use_context();
    let vault: NoteVault = app_context.vault;
    let content = props
        .note_path
        .read()
        .clone()
        .map_or_else(String::new, |e| vault.load_note(&e).unwrap());
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

    rsx! {
        // link { rel: "stylesheet", href: "code-input.css" },
        // script { src: "prism.js" },
        // script { src: "code-input.js" }
        if use_highlighter {
            code-input {
                id: "edit-content",
                language: "Markdown",
                // template: "syntax-highlighted",
                wrap: "hard",
                resize: "none",
                placeholder: "Insert your note text",
                "{content}",
            }
        } else {
            textarea {
                // size full to fill all the space
                class: "edittext",
                id: "edit-content",
                wrap: "hard",
                resize: "none",
                placeholder: "Insert your note text",
                "{content}",
            }
        }
    }
}
