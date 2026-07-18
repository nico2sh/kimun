//! Config page: renders the form for the persisted TOML, saves edits to the
//! file, and hosts the Restart trigger (adr/0028).

use std::sync::Arc;

use axum::{
    Form,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use maud::{DOCTYPE, Markup, html};

use crate::auth::session::same_origin;
use crate::config::{ConfigForm, RagConfig};
use crate::server_state::AppState;

use super::shell::shell;

pub(super) async fn config_page(State(state): State<Arc<AppState>>) -> Markup {
    let cfg = state.config.clone();
    config_markup(&state, &cfg, None)
}

/// Renders the config form from `c` (the running config, or the just-saved one
/// after a successful write so the fields reflect what's on disk).
fn config_markup(state: &AppState, c: &RagConfig, flash: Option<Markup>) -> Markup {
    // "none" is the semantic-only sentinel (search, no Q&A). It maps to
    // `llm = None` on save, and is what a server with no configured LLM shows
    // (adr/0022). "openai-local" is the OpenAI wire pointed at a user-supplied
    // endpoint (Ollama, llama.cpp, …) — same table-driven field groups as the
    // embedder select: each option's data-groups names the field divs it shows.
    let providers = [
        ("none", ""),
        ("gemini", "llm"),
        ("claude", "llm"),
        ("openai", "llm"),
        ("mistral", "llm"),
        ("openai-local", "llm llm-url"),
    ];
    let current = c.llm.as_ref().map(|l| l.form_id()).unwrap_or("none");
    let current_llm_url = c.llm.as_ref().and_then(|l| l.url()).unwrap_or("");
    let can_save = state.config_path.is_some();
    // Embedder section (adr/0024): "none" is the unconfigured sentinel; the
    // fastembed model is a dropdown so a local model is always an explicit
    // choice, never a hidden default.
    //
    // Each provider carries the form field groups it uses (rendered as
    // data-groups on its <option>; the field divs below are tagged with one
    // group each and the page script shows only the selected option's groups).
    // This table is the single place a provider's fields are declared — adding
    // a provider without naming its groups leaves its fields hidden, so keep
    // apply_form's dispatch in config.rs in step with it.
    let embedder_providers = [
        ("none", ""),
        ("fastembed", "model"),
        ("ollama", "http"),
        ("openai", "http key"),
    ];
    let current_embedder = c.embedder.as_ref().map(|e| e.provider()).unwrap_or("none");
    let fastembed_models = crate::dbembeddings::embedder::fastembedder::supported_models();
    // Canonicalize a configured fastembed model to its model code — configs may
    // use variant names (`BGESmallENV15`), but the dropdown options are keyed
    // by code, and an unmatched value would silently deselect the model and
    // fail the next save.
    let (current_fastembed, current_url, current_model) = match &c.embedder {
        Some(crate::config::EmbedderConfig::FastEmbed { model }) => (
            model
                .as_deref()
                .map(|m| {
                    crate::dbembeddings::embedder::fastembedder::canonical_model_code(m)
                        .unwrap_or_else(|| m.to_string())
                })
                .unwrap_or_default(),
            String::new(),
            String::new(),
        ),
        Some(crate::config::EmbedderConfig::Ollama { url, model, .. })
        | Some(crate::config::EmbedderConfig::OpenAI { url, model, .. }) => {
            (String::new(), url.clone(), model.clone())
        }
        None => (String::new(), String::new(), String::new()),
    };
    let embedder_key_set = matches!(
        &c.embedder,
        Some(crate::config::EmbedderConfig::OpenAI {
            api_key: Some(_),
            ..
        })
    );
    let (current_vector_db, sqlite_path, qdrant_url, qdrant_collection) = match &c.vector_db {
        crate::config::VectorDbConfig::Sqlite { path } => (
            "sqlite",
            path.display().to_string(),
            String::new(),
            String::new(),
        ),
        crate::config::VectorDbConfig::Qdrant { url, collection } => {
            ("qdrant", String::new(), url.clone(), collection.clone())
        }
    };
    let body = html! {
        h1 { "Configuration" }
        @if let Some(f) = flash { (f) }
        @if !can_save {
            p .flash.err { "No writable config path — edits cannot be saved." }
        }
        form method="post" action="/config" {
            section .group {
                h2 { "Server" }
                div .row {
                    div { label { "Host" } input type="text" name="host" value=(c.server.host); }
                    div { label { "Port" } input type="number" name="port" value=(c.server.port); }
                }
            }
            section .group {
                h2 { "Embedder" }
                @if c.embedder.is_none() {
                    p .flash.err { "No embedder configured — the server is unconfigured: indexing and search are disabled until one is set." }
                }
                label { "Provider" }
                select name="embedder_provider" {
                    @for (p, groups) in embedder_providers {
                        option value=(p) data-groups=(groups) selected[p == current_embedder] {
                            @if p == "none" { "— none (unconfigured) —" } @else { (p) }
                        }
                    }
                }
                p .muted { "Changing the embedder invalidates all indexed data — on the next start the server wipes stored vectors and every vault re-indexes on its next sync." }
                noscript {
                    p .muted { "(Without JavaScript all fields are shown: Model applies to fastembed; URL and Model to ollama and openai; API key to openai only.)" }
                }
                // Provider-specific field groups: the script at the bottom of
                // the page shows only the groups named by the selected option's
                // data-groups. Hidden fields still submit, but apply_form
                // dispatches on the provider and ignores the rest.
                div data-embedder="model" {
                    label { "Model" }
                    select name="fastembed_model" {
                        option value="" selected[current_fastembed.is_empty()] { "— pick a model —" }
                        @for (code, dim, desc) in &fastembed_models {
                            option value=(code) selected[code.eq_ignore_ascii_case(&current_fastembed)] title=(desc) data-desc=(desc) {
                                (code) " (" (dim) " dims)"
                            }
                        }
                    }
                    // Filled by the page script with the selected option's
                    // data-desc; hover on an option shows it too (title).
                    p .muted #fastembed-desc {}
                }
                div data-embedder="http" {
                    div .row {
                        div { label { "URL" } input type="text" name="embedder_url" value=(current_url); }
                        div { label { "Model" } input type="text" name="embedder_model" value=(current_model); }
                    }
                    p .muted { "Instruction prefixes (doc_prefix / query_prefix) are file-only settings; a save here keeps them." }
                }
                div data-embedder="key" {
                    label { "API key" }
                    input type="password" name="embedder_api_key" placeholder=(if embedder_key_set { "unchanged (a key is set)" } else { "from environment if blank" });
                }
            }
            section .group {
                h2 { "Vector DB" }
                label { "Backend" }
                select name="vector_db" {
                    option value="sqlite" data-groups="sqlite" selected[current_vector_db == "sqlite"] { "SQLite (embedded, local)" }
                    option value="qdrant" data-groups="qdrant" selected[current_vector_db == "qdrant"] { "Qdrant (server)" }
                }
                noscript {
                    p .muted { "(Without JavaScript all fields are shown: the path applies to SQLite; URL and collection prefix to Qdrant.)" }
                }
                div data-vectordb="sqlite" {
                    label { "SQLite path" }
                    input type="text" name="sqlite_path" value=(sqlite_path) placeholder="default: data dir";
                }
                div .row data-vectordb="qdrant" {
                    div { label { "Qdrant URL" } input type="text" name="qdrant_url" value=(qdrant_url) placeholder="http://localhost:6333"; }
                    div { label { "Qdrant collection prefix" } input type="text" name="qdrant_collection" value=(qdrant_collection) placeholder="kimun_embeddings"; }
                }
            }
            section .group {
                h2 { "LLM" }
                label { "Provider" }
                select name="provider" {
                    @for (p, groups) in providers {
                        option value=(p) data-groups=(groups) selected[p == current] {
                            @match p {
                                "none" => { "— none (semantic-only) —" }
                                "openai-local" => { "openai-compatible (local: Ollama, llama.cpp, …)" }
                                _ => { (p) }
                            }
                        }
                    }
                }
                p .muted { "Select — none — for a search-only server (no question-answering)." }
                noscript {
                    p .muted { "(Without JavaScript all fields are shown: URL applies to the local OpenAI-compatible provider only.)" }
                }
                div data-llm="llm-url" {
                    label { "URL" }
                    input type="text" name="llm_url" value=(current_llm_url) placeholder="http://localhost:11434/v1";
                    p .muted { "The endpoint's OpenAI-compatible base URL. Keyless local servers work — leave the API key blank." }
                }
                div data-llm="llm" {
                    label { "Model" }
                    input type="text" name="model" value=(c.llm.as_ref().map(|l| l.model()).unwrap_or(""));
                    label { "API key" }
                    input type="password" name="api_key" placeholder=(if c.llm.as_ref().and_then(|l| l.api_key()).is_some() { "unchanged (a key is set)" } else { "from environment if blank" });
                    p .muted { "Leave blank to keep the current key (or fall back to the provider env var)." }
                }
            }
            section .group {
                h2 { "Reranker" }
                div .check { input type="checkbox" name="reranker_enabled" checked[c.reranker.enabled]; label style="margin:0" { "Enabled" } }
                p .muted { "The reranker backend (local model or HTTP endpoint) is a file-only setting; a save here keeps it." }
            }
            section .group {
                h2 { "Context cut" }
                label { "Strategy" }
                select name="context_cut" {
                    option value="fixed" data-groups="fixed" selected[c.reranker.context_cut == crate::config::ContextCut::Fixed] {
                        "fixed — exactly top_k results, the classic count cut"
                    }
                    option value="score-range" data-groups="score-range" selected[c.reranker.context_cut == crate::config::ContextCut::ScoreRange] {
                        "score-range — keep chunks above a cutoff of the normalized score range"
                    }
                    option value="largest-drop" data-groups="largest-drop" selected[c.reranker.context_cut == crate::config::ContextCut::LargestDrop] {
                        "largest-drop — cut at the biggest relative gap between consecutive note scores"
                    }
                }
                p .muted { "Sizes both query surfaces from the ranked pool — search shows the notes that survive the cut, answers feed the surviving chunks to the LLM — with or without reranking." }
                noscript {
                    p .muted { "(Without JavaScript all knobs are shown; only the selected strategy's applies.)" }
                }
                // Hidden knobs still post their values, so a save never
                // resets a non-selected strategy's tuning.
                div data-cut="fixed" {
                    label { "Results (top_k) — search notes / answer chunks" }
                    input type="number" name="reranker_top_k" value=(c.reranker.top_k);
                    p .muted { "Per-request context_size (small/medium/large) overrides this under fixed only." }
                }
                div data-cut="score-range" {
                    label { "Normalized cutoff (0..1)" }
                    input type="number" name="score_range_cutoff" step="0.05" min="0" max="1" value=(c.reranker.score_range_cutoff);
                    p .muted { "Higher = stricter. The range is measured between the pool's 5th/95th score percentiles." }
                }
                div .row data-cut="largest-drop" {
                    div {
                        label { "Gap search window — from note position" }
                        input type="number" name="drop_window_min" min="1" value=(c.reranker.drop_window_min);
                    }
                    div {
                        label { "to note position" }
                        input type="number" name="drop_window_max" min="1" value=(c.reranker.drop_window_max);
                    }
                }
            }
            section .group {
                h2 { "Auth" }
                label { "Bearer token" }
                input type="password" name="auth_token" placeholder=(if c.auth.token.is_some() { "unchanged (a token is set)" } else { "open — no token set" });
                p .muted { "Leave blank to keep the current token. Clearing it (going open) must be done in the config file." }
            }
            @if can_save {
                button type="submit" { "Save to config file" }
                p .muted { "Saved changes take effect the next time the server starts — or use Restart below." }
            }
        }
        // Separate form: restart is not a save. Applies whatever is in the
        // config file right now (web-saved or hand-edited), so it is useful
        // even when the path is not writable from here.
        form method="post" action="/restart" {
            button .danger type="submit" { "Restart server now" }
            p .muted {
                "Drains in-flight requests, reloads the config file, and rebinds — every "
                "setting applies, including the bind address. The server is briefly "
                "unavailable; connected Kimün clients reconnect on their own."
            }
        }
        script {
            (maud::PreEscaped(r#"
const bindVisibility = (selectName, attr) => {
  const sel = document.querySelector('select[name="' + selectName + '"]');
  const apply = () => {
    const groups = (sel.selectedOptions[0].getAttribute('data-groups') || '').split(' ');
    document.querySelectorAll('[data-' + attr + ']').forEach(el => {
      el.style.display = groups.includes(el.getAttribute('data-' + attr)) ? '' : 'none';
    });
  };
  sel.addEventListener('change', apply);
  // pageshow: Firefox restores form state on reload/back AFTER scripts run,
  // without firing 'change' — re-apply so visibility tracks the restored value.
  window.addEventListener('pageshow', apply);
  apply();
};
bindVisibility('embedder_provider', 'embedder');
bindVisibility('vector_db', 'vectordb');
bindVisibility('provider', 'llm');
bindVisibility('context_cut', 'cut');
const bindDesc = (selectName, targetId) => {
  const sel = document.querySelector('select[name="' + selectName + '"]');
  const out = document.getElementById(targetId);
  const apply = () => {
    out.textContent = sel.selectedOptions[0].getAttribute('data-desc') || '';
  };
  sel.addEventListener('change', apply);
  window.addEventListener('pageshow', apply);
  apply();
};
bindDesc('fastembed_model', 'fastembed-desc');
"#))
        }
    };
    shell(state, "/config", "Configuration", body)
}

/// Pure web plumbing: origin check, writable-path check, then hand the form to
/// [`RagConfig::apply_form`] — every form→config rule lives there, so a new
/// web-exposed option touches the config module and the form markup, not this
/// handler.
pub(super) async fn config_submit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(f): Form<ConfigForm>,
) -> Response {
    if !same_origin(&headers) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    let err_page = |state: &AppState, msg: Markup| {
        config_markup(state, &state.config.clone(), Some(msg)).into_response()
    };

    let Some(path) = state.config_path.clone() else {
        return err_page(
            &state,
            html! { p .flash.err { "No writable config path — nothing was saved." } },
        );
    };

    let cfg = match state.config.apply_form(f) {
        Ok(cfg) => cfg,
        Err(e) => return err_page(&state, html! { p .flash.err { (e) } }),
    };

    match cfg.save_to(&path) {
        Ok(()) => config_markup(
            &state,
            &cfg,
            Some(html! { p .flash.ok { "Saved to " span .mono { (path.display()) } ". Use the Restart button below to apply now, or restart the server yourself." } }),
        )
        .into_response(),
        Err(e) => err_page(&state, html! { p .flash.err { "Could not write config: " (e) } }),
    }
}

/// POST /restart — asks the binary's serving loop (adr/0028) to drain
/// in-flight requests, reload the saved config file, and rebind. Pure
/// trigger: what changed shows up on the reloaded pages afterwards.
pub(super) async fn restart_submit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    if !same_origin(&headers) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    if !state.request_restart() {
        return config_markup(
            &state,
            &state.config.clone(),
            Some(html! { p .flash.err {
                "In-process restart is not available here — restart the server manually to apply the config."
            } }),
        )
        .into_response();
    }
    // Standalone page, no shell: the server goes down right after this
    // response, so nav links would dead-end anyway. Meta-refresh returns to
    // the dashboard once the server is back.
    html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                meta http-equiv="refresh" content="4;url=/";
                title { "Restarting — Kimün server" }
            }
            body {
                p { "Restarting: draining requests, reloading the config file, rebinding." }
                p {
                    "Returning to the dashboard in a few seconds. If you changed the "
                    "bind address, open the server at its new address instead."
                }
            }
        }
    }
    .into_response()
}
