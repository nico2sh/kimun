use std::sync::Arc;

use dioxus::prelude::*;
use kimun_core::{NoteVault, NotesValidation};

use crate::{components::modal::Modal, settings::AppSettings};

#[derive(Clone, Debug, PartialEq)]
pub enum IndexType {
    Validate,
    Fast,
    Full,
}

#[component]
pub fn Indexer(modal: Signal<Modal>, vault: Arc<NoteVault>, index_type: IndexType) -> Element {
    let mut settings: Signal<AppSettings> = use_context();

    let (description, confirm_close) = match &index_type {
        IndexType::Validate => ("Validating the Vault", false),
        IndexType::Fast => ("Fast checking data", false),
        IndexType::Full => (
            "Running a full validation, this may take a while with large vaults",
            true,
        ),
    };
    let result = use_resource(move || {
        let index_type = index_type.clone();
        let vault = vault.clone();
        async move {
            tokio::spawn(async move {
                match index_type {
                    IndexType::Validate => vault.init_and_validate(),
                    IndexType::Fast => vault.index_notes(NotesValidation::Fast),
                    IndexType::Full => vault.recreate_index(),
                }
            })
            .await
            .unwrap()
        }
    });

    let index_result = match &*result.read_unchecked() {
        Some(r) => match r {
            Ok(rep) => {
                let duration = rep.duration.as_secs();
                rsx! {
                    div { onmounted: move |_| { settings.write().report_indexed() },
                        "Done in {duration} seconds"
                    }
                    {
                        if confirm_close {
                            rsx! {
                                button {
                                    class: "btn btn-primary",
                                    onclick: move |_| {
                                        modal.write().close();
                                    },
                                    "Close"
                                }
                            }
                        } else {
                            rsx! {
                                div {
                                    onmounted: move |_| {
                                        modal.write().close();
                                    },
                                }
                            }
                        }
                    }
                }

                // let duration = rep.duration.as_secs();
                // if confirm_close {
                //     rsx! {
                //         div { "Done in {duration} seconds" }
                //         button {
                //             class: "btn btn-primary",
                //             onclick: move |_| {
                //                 modal.write().close();
                //             },
                //             "Close"
                //         }
                //     }
                // } else {
                //     modal.write().close();
                //     rsx! {}
                // }
            }
            Err(e) => rsx! { "Error indexing vault: {e}" },
        },
        None => rsx! {
            progress { class: "index-progress" }
        },
    };

    rsx! {
        div { class: "index-modal",
            h3 { "Indexing" }
            {index_result}
            div { class: "description", "{description}" }
        }
    }
}
