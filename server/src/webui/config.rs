//! Config page: renders the form for the persisted TOML, saves edits to the
//! file, and hosts the Restart trigger.

use std::sync::Arc;

use axum::{
    Form,
    extract::State,
    response::{IntoResponse, Response},
};
use maud::{DOCTYPE, Markup, html};

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
    // `llm = None` on save, and is what a server with no configured LLM shows.
    // "openai-local" is the OpenAI wire pointed at a user-supplied
    // endpoint (Ollama, llama.cpp, …) — same table-driven field groups as the
    // embedder select: each option's data-groups names the field divs it shows.
    let providers = [
        ("none", ""),
        ("gemini", "llm"),
        ("claude", "llm"),
        ("openai", "llm"),
        ("mistral", "llm"),
        ("openrouter", "llm llm-reasoning"),
        ("openai-local", "llm llm-url"),
    ];
    let current = c.llm.as_ref().map(|l| l.form_id()).unwrap_or("none");
    let current_reasoning = c.llm.as_ref().and_then(|l| l.reasoning());
    let current_llm_url = c.llm.as_ref().and_then(|l| l.url()).unwrap_or("");
    let can_save = state.config_path.is_some();
    // Embedder section: "none" is the unconfigured sentinel; the
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
        ("ollama", "url emodel"),
        ("openai", "url emodel key keyhelp-openai"),
        ("openrouter", "emodel key keyhelp-openrouter"),
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
        Some(crate::config::EmbedderConfig::OpenRouter { model, .. }) => {
            (String::new(), String::new(), model.clone())
        }
        None => (String::new(), String::new(), String::new()),
    };
    let embedder_key_set = matches!(
        &c.embedder,
        Some(crate::config::EmbedderConfig::OpenAI {
            api_key: Some(_),
            ..
        }) | Some(crate::config::EmbedderConfig::OpenRouter {
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
    // One-line "what is set now" per card, shown in the card header so a
    // collapsed card still answers "what is this configured to?".
    let embedder_sum = match &c.embedder {
        None => "not configured".to_string(),
        Some(_) if current_embedder == "fastembed" => format!("fastembed · {current_fastembed}"),
        Some(_) => format!("{current_embedder} · {current_model}"),
    };
    let vector_db_sum = current_vector_db;
    let reranker_sum = if c.reranker.enabled { "on" } else { "off" };
    let cut_sum = match c.reranker.context_cut {
        crate::config::ContextCut::Fixed => "fixed",
        crate::config::ContextCut::ScoreRange => "score-range",
        crate::config::ContextCut::LargestDrop => "largest-drop",
    };
    let llm_sum = match &c.llm {
        None => "none (semantic-only)".to_string(),
        Some(l) => format!("{} · {}", l.provider(), l.model()),
    };
    let auth_sum = if c.auth.token.is_some() {
        "token required"
    } else {
        "open"
    };
    let server_sum = format!("{}:{}", c.server.host, c.server.port);
    // A tab carries a marker when something on it needs attention.
    let search_needs_attention = c.embedder.is_none();
    let tabs = [
        ("general", "General", false),
        ("search", "Search", search_needs_attention),
        ("ask", "Ask", false),
    ];
    let body = html! {
        h1 { "Configuration" }
        @if let Some(f) = flash { (f) }
        @if !can_save {
            p .flash.err { "No writable config path — edits cannot be saved." }
        }
        // Marks the page as scripted before the panels render, so the tab
        // layout applies without a flash. Without JavaScript every panel
        // stays visible and the tab bar stays hidden.
        script { (maud::PreEscaped("document.documentElement.classList.add('js');")) }
        div .tabs role="tablist" {
            @for (id, label, warn) in tabs {
                button .tab type="button" role="tab" data-tab=(id) {
                    (label)
                    @if warn { " " span .warn title="Needs attention" { "!" } }
                }
            }
        }
        form #config-form method="post" action="/config" {
          div .tabpanel data-panel="general" {
            details .card open {
                summary { h2 { "Server" } span .sum { (server_sum) } }
                div .row {
                    div { label { "Host" } input type="text" name="host" value=(c.server.host); }
                    div { label { "Port" } input type="number" name="port" value=(c.server.port); }
                }
            }
            details .card open {
                summary { h2 { "Auth" } span .sum { (auth_sum) } }
                label { "Bearer token" }
                input type="password" name="auth_token" placeholder=(if c.auth.token.is_some() { "unchanged (a token is set)" } else { "open — no token set" });
                p .muted { "Leave blank to keep the current token. Clearing it (going open) must be done in the config file." }
            }
            details .card open {
                summary { h2 { "Apply changes" } }
                p .muted { "Saved changes take effect the next time the server starts, or when you restart it here." }
                p .muted {
                    "Restart drains in-flight requests, reloads the config file, and rebinds, so every "
                    "setting applies, including the bind address. It uses whatever is in the config file "
                    "right now (web-saved or hand-edited), so it works even when this page cannot save. "
                    "The server is briefly unavailable; connected Kimün clients reconnect on their own."
                }
            }
          }
          div .tabpanel data-panel="search" {
            details .card open {
                summary { h2 { "Embedder" } span .sum { (embedder_sum) } }
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
                    p .muted { "(Without JavaScript all fields are shown: Model applies to fastembed; URL to ollama and openai; Model to ollama, openai and openrouter; API key to openai and openrouter.)" }
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
                // URL and model are separate groups so OpenRouter (fixed URL)
                // shows the model alone; the row's flex layout lets it fill.
                div .row {
                    div data-embedder="url" { label { "URL" } input type="text" name="embedder_url" value=(current_url); }
                    div data-embedder="emodel" { label { "Model" } input type="text" name="embedder_model" value=(current_model); }
                }
                div data-embedder="emodel" {
                    p .muted { "Instruction prefixes (doc_prefix / query_prefix) are file-only settings; a save here keeps them." }
                }
                // The placeholder states only what is true for every provider
                // that shows this field; the env-var fallback differs per
                // provider, so each gets its own help line.
                div data-embedder="key" {
                    label { "API key" }
                    input type="password" name="embedder_api_key" placeholder=(if embedder_key_set { "unchanged (a key is set)" } else { "no key set" });
                    p .muted { "Leave blank to keep the current key." }
                }
                div data-embedder="keyhelp-openai" {
                    p .muted { "Sent as a bearer token. There is no environment-variable fallback; keyless endpoints work with no key." }
                }
                div data-embedder="keyhelp-openrouter" {
                    p .muted { "If no key is set, the server reads OPENROUTER_API_KEY." }
                }
            }
            details .card open {
                summary { h2 { "Vector DB" } span .sum { (vector_db_sum) } }
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
            details .card {
                summary { h2 { "Reranker" } span .sum { (reranker_sum) } }
                div .check { input type="checkbox" name="reranker_enabled" checked[c.reranker.enabled]; label style="margin:0" { "Enabled" } }
                p .muted { "The reranker backend (local model or HTTP endpoint) is a file-only setting; a save here keeps it." }
            }
            details .card {
                summary { h2 { "Context cut" } span .sum { (cut_sum) } }
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
          }
          div .tabpanel data-panel="ask" {
            details .card open {
                summary { h2 { "LLM" } span .sum { (llm_sum) } }
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
                div data-llm="llm-reasoning" {
                    label { "Reasoning effort" }
                    select name="llm_reasoning" {
                        option value="" selected[current_reasoning.is_none()] { "model default (not sent)" }
                        @for e in crate::config::ReasoningEffort::ALL {
                            option value=(e.as_str()) selected[Some(e) == current_reasoning] { (e.as_str()) }
                        }
                    }
                    p .muted { "OpenRouter only. Sent as reasoning.effort; models without reasoning ignore it." }
                }
                div data-llm="llm" {
                    label { "Model" }
                    input type="text" name="model" value=(c.llm.as_ref().map(|l| l.model()).unwrap_or(""));
                    label { "API key" }
                    input type="password" name="api_key" placeholder=(if c.llm.as_ref().and_then(|l| l.api_key()).is_some() { "unchanged (a key is set)" } else { "from environment if blank" });
                    p .muted { "Leave blank to keep the current key (or fall back to the provider env var)." }
                }
            }
          }
          div .savebar {
            @if can_save {
                button type="submit" { "Save to config file" }
            }
            button .danger #restart-btn type="submit" form="restart-form" { "Restart server now" }
            span .hint #dirty-hint { "No unsaved changes." }
          }
        }
        // Separate form: restart is not a save. The button lives in the save
        // bar and points here through its `form` attribute.
        form #restart-form method="post" action="/restart" {}
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

// Tabs: the hash picks the tab (so a tab is linkable); sessionStorage keeps it
// across the POST that a save does, which lands on /config without a hash.
const tabs = [...document.querySelectorAll('.tab')];
const panels = [...document.querySelectorAll('.tabpanel')];
const showTab = (id) => {
  if (!tabs.some(t => t.dataset.tab === id)) id = tabs[0].dataset.tab;
  tabs.forEach(t => t.setAttribute('aria-selected', String(t.dataset.tab === id)));
  panels.forEach(p => p.classList.toggle('on', p.dataset.panel === id));
  try { sessionStorage.setItem('config-tab', id); } catch (e) {}
};
tabs.forEach(t => t.addEventListener('click', () => {
  showTab(t.dataset.tab);
  history.replaceState(null, '', '#' + t.dataset.tab);
}));
let saved = null;
try { saved = sessionStorage.getItem('config-tab'); } catch (e) {}
showTab(location.hash.slice(1) || saved);

// Unsaved-changes guard: the save bar says so, leaving the page asks first,
// and Restart warns that it will not include the pending edits.
const form = document.getElementById('config-form');
const hint = document.getElementById('dirty-hint');
const bar = hint.parentElement;
let dirty = false, submitting = false;
const markDirty = () => {
  dirty = true;
  bar.classList.add('dirty');
  hint.textContent = 'Unsaved changes. Save, then restart to apply them.';
};
// 'change' as well as 'input': not every browser fires 'input' on a select.
form.addEventListener('input', markDirty);
form.addEventListener('change', markDirty);
form.addEventListener('submit', () => { submitting = true; });
window.addEventListener('beforeunload', (e) => {
  if (dirty && !submitting) { e.preventDefault(); e.returnValue = ''; }
});
// A clean form needs no flag: the guard only warns when the form is dirty. The
// flag is set only once the user has accepted losing pending edits, so a failed
// or cancelled restart never leaves the guard off for later edits.
document.getElementById('restart-btn').addEventListener('click', (e) => {
  if (!dirty) return;
  if (confirm('You have unsaved changes that a restart will not include. Restart anyway?')) {
    submitting = true;
  } else {
    e.preventDefault();
  }
});
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
    Form(f): Form<ConfigForm>,
) -> Response {
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

/// POST /restart — asks the binary's serving loop to drain
/// in-flight requests, reload the saved config file, and rebind. Pure
/// trigger: what changed shows up on the reloaded pages afterwards.
pub(super) async fn restart_submit(State(state): State<Arc<AppState>>) -> Response {
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
