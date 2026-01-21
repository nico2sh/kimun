use std::sync::Arc;

use dioxus::prelude::*;
use kimun_core::{NoteVault, NotesValidation};

use crate::{
    app_state::AppState,
    components::button::{Button, ButtonStyle},
    settings::AppSettings,
};

#[derive(Clone, Debug, PartialEq)]
pub enum IndexType {
    Validate,
    Fast,
    Full,
}

#[component]
pub fn Indexer(vault: Arc<NoteVault>, index_type: IndexType) -> Element {
    let mut app_state: Signal<AppState> = use_context();
    let mut settings: Signal<AppSettings> = use_context();
    let theme = settings().get_theme();

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

    let (index_result, actions_section) = match &*result.read_unchecked() {
        Some(r) => match r {
            Ok(rep) => {
                let duration = rep.duration.as_secs();
                (
                    rsx! {
                        div { onmounted: move |_| { settings.write().report_indexed() },
                            "Done in {duration} seconds"
                        }
                    },
                    rsx! {
                        if confirm_close {
                            Button {
                                action: move |_| {
                                    app_state.write().close_modal();
                                },
                                title: "Close",
                                style: ButtonStyle::Secondary {},
                                theme: theme.clone(),
                            }
                        } else {
                            div {
                                onmounted: move |_| {
                                    app_state.write().close_modal();
                                },
                            }
                        }
                    },
                )
            }
            Err(e) => (
                rsx! { "Error indexing vault: {e}" },
                rsx! {
                    Button {
                        action: move |_| {
                            app_state.write().close_modal();
                        },
                        title: "Close",
                        style: ButtonStyle::Secondary {},
                        theme: theme.clone(),
                    }
                },
            ),
        },
        None => (
            rsx! {
                div {
                    class: "loader",
                    background: "radial-gradient(farthest-side, {theme.accent_yellow} 94%, #0000) left/20px 20px no-repeat {theme.bg_surface}",
                }
            },
            rsx! {},
        ),
    };
    let settings: Signal<AppSettings> = use_context();
    let theme = settings().get_theme();

    rsx! {
        div {
            class: "modal",
            background_color: "{theme.bg_main}",
            border_color: "{theme.border_light}",
            div { class: "modal-header",
                div { class: "modal-title", color: "{theme.text_primary}", "Indexing" }
                div { class: "modal-subtitle", color: "{theme.text_light}", "{description}" }
            }
            div { class: "modal-body", {index_result} }
            div { class: "modal-actions", {actions_section} }
        }
    }
}
