//! Server admin web UI: a small server-rendered dashboard (maud templates plus
//! a sprinkle of inline JS for job polling). Everything is embedded in the
//! binary — no build step, no external assets, no node.
//!
//! Scope (P5): view the running configuration, browse collections, watch jobs,
//! and run a test query. Config edits are **persisted to the TOML file and take
//! effect on restart** — the embedder, vector store, and LLM client are all
//! built at startup (changing the embedder even changes the vector width), so
//! the running instance is never mutated live. The page says as much.
//!
//! Auth reuses the server's bearer token: a login form exchanges the token for
//! an `HttpOnly` session cookie holding that same shared secret. With no token
//! configured the UI is open (matching the API's localhost-dev posture).

use std::sync::Arc;

use axum::{
    Form, Router,
    extract::{Request, State},
    http::{
        HeaderMap, HeaderValue,
        header::{COOKIE, HOST, ORIGIN, SET_COOKIE},
    },
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use maud::{DOCTYPE, Markup, html};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::{ConfigForm, RagConfig};
use crate::server_state::AppState;

const SESSION_COOKIE: &str = "kimun_session";

/// The session-cookie value for a token: the token's SHA-256 as hex. Keeping the
/// hash (not the token) in the cookie means the value is always cookie-safe
/// (`0-9a-f`, so a token with spaces/`;`/control chars can't corrupt the cookie
/// or lock the admin out), and a leaked cookie doesn't hand over the raw API
/// secret.
fn session_value(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// Rejects a state-changing POST a browser marks as cross-origin. A same-origin
/// form POST (or a non-browser client like curl/tests) sends no mismatching
/// `Origin`, so it passes; a drive-by CSRF from another site is blocked even in
/// open mode (no token, where there's no SameSite cookie to lean on).
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true; // no Origin → non-browser or not cross-site; allow
    };
    let origin_host = origin.split("://").nth(1).unwrap_or(origin);
    let host = headers
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    origin_host == host
}

/// Web-UI routes. Returned without state applied (main calls `.with_state`); the
/// auth middleware captures its own state clone.
pub fn routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let protected = Router::new()
        .route("/", get(dashboard))
        .route("/config", get(config_page).post(config_submit))
        .route("/collections", get(collections_page))
        .route("/jobs", get(jobs_page))
        .route("/jobs/fragment", get(jobs_fragment))
        .route("/query", get(query_page).post(query_submit))
        .route("/logout", get(logout))
        .route_layer(middleware::from_fn_with_state(state, web_auth));

    Router::new()
        .merge(protected)
        .route("/login", get(login_page).post(login_submit))
}

// ============================================================================
// Auth
// ============================================================================

/// Gates every protected page. Open when no token is configured; otherwise the
/// session cookie must carry the configured token. Unauthorized → `/login`.
async fn web_auth(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let Some(expected) = state.config.auth.token.as_deref() else {
        return next.run(req).await;
    };
    let want = session_value(expected);
    let ok = cookie_value(&req, SESSION_COOKIE)
        .map(|v| crate::auth::constant_time_eq(v.as_bytes(), want.as_bytes()))
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        Redirect::to("/login").into_response()
    }
}

fn cookie_value(req: &Request, name: &str) -> Option<String> {
    let header = req.headers().get(COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

fn redirect_with_cookie(location: &str, cookie: String) -> Response {
    let mut resp = Redirect::to(location).into_response();
    if let Ok(val) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(SET_COOKIE, val);
    }
    resp
}

async fn login_page(State(state): State<Arc<AppState>>) -> Response {
    // No token configured → nothing to log into.
    if state.config.auth.token.is_none() {
        return Redirect::to("/").into_response();
    }
    login_markup(false).into_response()
}

#[derive(Deserialize)]
struct LoginForm {
    token: String,
}

async fn login_submit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    if !same_origin(&headers) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    match state.config.auth.token.as_deref() {
        Some(expected)
            if crate::auth::constant_time_eq(form.token.as_bytes(), expected.as_bytes()) =>
        {
            // The cookie holds the token's hash, not the token — always
            // cookie-safe, and HttpOnly keeps it out of page scripts.
            let cookie = format!(
                "{SESSION_COOKIE}={}; HttpOnly; SameSite=Strict; Path=/",
                session_value(expected)
            );
            redirect_with_cookie("/", cookie)
        }
        Some(_) => login_markup(true).into_response(),
        None => Redirect::to("/").into_response(),
    }
}

async fn logout() -> Response {
    redirect_with_cookie("/login", format!("{SESSION_COOKIE}=; Max-Age=0; Path=/"))
}

fn login_markup(error: bool) -> Markup {
    html! {
        (DOCTYPE)
        html {
            head { meta charset="utf-8"; title { "Kimün RAG — Sign in" } (styles()) }
            body {
                main .login {
                    h1 { "Kimün RAG" }
                    @if error { p .flash.err { "Incorrect token." } }
                    form method="post" action="/login" {
                        label { "Server token" }
                        input type="password" name="token" autofocus?;
                        button type="submit" { "Sign in" }
                    }
                }
            }
        }
    }
}

// ============================================================================
// Layout
// ============================================================================

fn shell(state: &AppState, active: &str, title: &str, body: Markup) -> Markup {
    let auth_on = state.config.auth.token.is_some();
    let nav = [
        ("/", "Dashboard"),
        ("/config", "Config"),
        ("/collections", "Collections"),
        ("/jobs", "Jobs"),
        ("/query", "Test query"),
    ];
    html! {
        (DOCTYPE)
        html {
            head { meta charset="utf-8"; title { "Kimün RAG — " (title) } (styles()) }
            body {
                nav {
                    span .brand { "Kimün RAG" }
                    @for (href, label) in nav {
                        a href=(href) .active[active == href] { (label) }
                    }
                    @if auth_on { a href="/logout" .right { "Sign out" } }
                }
                main { (body) }
            }
        }
    }
}

fn styles() -> Markup {
    html! {
        style {
            (maud::PreEscaped(r#"
:root{--fg:#1c1c1e;--muted:#6b7280;--bg:#f7f7f8;--card:#fff;--line:#e5e7eb;--accent:#2563eb;}
*{box-sizing:border-box}
body{margin:0;font:15px/1.5 system-ui,-apple-system,Segoe UI,Roboto,sans-serif;color:var(--fg);background:var(--bg)}
nav{display:flex;gap:.25rem;align-items:center;padding:.6rem 1rem;background:var(--card);border-bottom:1px solid var(--line)}
nav .brand{font-weight:700;margin-right:1rem}
nav a{padding:.35rem .7rem;border-radius:6px;color:var(--muted);text-decoration:none}
nav a:hover{background:var(--bg)}
nav a.active{color:var(--accent);font-weight:600}
nav a.right{margin-left:auto}
main{max-width:900px;margin:1.5rem auto;padding:0 1rem}
main.login{max-width:340px;margin-top:12vh}
h1{font-size:1.4rem}h2{font-size:1.05rem;margin-top:1.6rem}
.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:1rem 1.2rem;margin:1rem 0}
table{width:100%;border-collapse:collapse}
th,td{text-align:left;padding:.45rem .6rem;border-bottom:1px solid var(--line);vertical-align:top}
th{color:var(--muted);font-weight:600;font-size:.82rem;text-transform:uppercase;letter-spacing:.03em}
dl{display:grid;grid-template-columns:auto 1fr;gap:.35rem 1rem;margin:0}
dt{color:var(--muted)}dd{margin:0;font-variant-numeric:tabular-nums}
label{display:block;margin:.6rem 0 .2rem;color:var(--muted);font-size:.9rem}
input,select{width:100%;padding:.45rem .55rem;border:1px solid var(--line);border-radius:6px;font:inherit;background:var(--card)}
.row{display:flex;gap:1rem}.row>div{flex:1}
.check{display:flex;align-items:center;gap:.5rem}.check input{width:auto}
button{margin-top:1rem;padding:.5rem 1rem;border:0;border-radius:6px;background:var(--accent);color:#fff;font:inherit;font-weight:600;cursor:pointer}
.flash{padding:.6rem .8rem;border-radius:6px;margin:.8rem 0}
.flash.ok{background:#ecfdf5;color:#047857}.flash.err{background:#fef2f2;color:#b91c1c}
.muted{color:var(--muted)}.mono{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:.86rem}
.snippet{color:var(--muted);font-size:.9rem}
.badge{display:inline-block;padding:.1rem .5rem;border-radius:999px;font-size:.78rem;background:var(--bg);border:1px solid var(--line)}
"#))
        }
    }
}

// ============================================================================
// Dashboard
// ============================================================================

async fn dashboard(State(state): State<Arc<AppState>>) -> Markup {
    let c = &state.config;
    let vector_db = match &c.vector_db {
        crate::config::VectorDbConfig::Lance { path } => {
            format!("LanceDB ({})", path.display())
        }
        crate::config::VectorDbConfig::Qdrant { url, collection } => {
            format!("Qdrant ({url}, prefix `{collection}`)")
        }
    };
    let embedder = match &c.embedder {
        None => "not configured (unconfigured server)".to_string(),
        Some(crate::config::EmbedderConfig::FastEmbed { model }) => {
            format!(
                "fastembed ({})",
                model.as_deref().unwrap_or("default BGE-Large")
            )
        }
        Some(crate::config::EmbedderConfig::Ollama { url, model, .. }) => {
            format!("ollama {model} @ {url}")
        }
        Some(crate::config::EmbedderConfig::OpenAI { url, model, .. }) => {
            format!("openai-compatible {model} @ {url}")
        }
    };
    let body = html! {
        h1 { "Dashboard" }
        @if c.embedder.is_none() {
            p .flash.err {
                "This server is unconfigured — no embedder is set, so indexing and search are disabled. "
                a href="/config" { "Configure an embedder" } "."
            }
        }
        div .card {
            dl {
                dt { "Bind address" } dd .mono { (c.server.host) ":" (c.server.port) }
                dt { "Vector DB" } dd { (vector_db) }
                dt { "Embedder" } dd { (embedder) }
                dt { "LLM" } dd {
                    @if let Some(l) = &c.llm { (l.provider()) " · " (l.model()) }
                    @else { "not configured (semantic-only)" }
                }
                dt { "LLM key" } dd {
                    @if let Some(l) = &c.llm {
                        @if l.api_key().is_some() { "set in config" } @else { "from environment" }
                    } @else { "—" }
                }
                dt { "Reranker" } dd { @if c.reranker.enabled { "on (top_k " (c.reranker.top_k) ")" } @else { "off" } }
                dt { "Auth" } dd { @if c.auth.token.is_some() { span .badge { "token required" } } @else { span .badge { "open" } } }
            }
        }
        p .muted { "The vector store and embedder are fixed at startup. Change them in Config and restart the server." }
    };
    shell(&state, "/", "Dashboard", body)
}

// ============================================================================
// Config
// ============================================================================

async fn config_page(State(state): State<Arc<AppState>>) -> Markup {
    let cfg = state.config.clone();
    config_markup(&state, &cfg, None)
}

/// Renders the config form from `c` (the running config, or the just-saved one
/// after a successful write so the fields reflect what's on disk).
fn config_markup(state: &AppState, c: &RagConfig, flash: Option<Markup>) -> Markup {
    // "none" is the semantic-only sentinel (search, no Q&A). It maps to
    // `llm = None` on save, and is what a server with no configured LLM shows
    // (adr/0022).
    let providers = ["none", "gemini", "claude", "openai", "mistral"];
    let current = c.llm.as_ref().map(|l| l.provider()).unwrap_or("none");
    let can_save = state.config_path.is_some();
    // Embedder section (adr/0024): "none" is the unconfigured sentinel; the
    // fastembed model is a dropdown so a local model is always an explicit
    // choice, never a hidden default.
    let embedder_providers = ["none", "fastembed", "ollama", "openai"];
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
    let (current_vector_db, lance_path, qdrant_url, qdrant_collection) = match &c.vector_db {
        crate::config::VectorDbConfig::Lance { path } => (
            "lance",
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
            div .card {
                h2 { "Server" }
                div .row {
                    div { label { "Host" } input type="text" name="host" value=(c.server.host); }
                    div { label { "Port" } input type="number" name="port" value=(c.server.port); }
                }
            }
            div .card {
                h2 { "Embedder" }
                @if c.embedder.is_none() {
                    p .flash.err { "No embedder configured — the server is unconfigured: indexing and search are disabled until one is set." }
                }
                label { "Provider" }
                select name="embedder_provider" {
                    @for p in embedder_providers {
                        option value=(p) selected[p == current_embedder] {
                            @if p == "none" { "— none (unconfigured) —" } @else { (p) }
                        }
                    }
                }
                p .muted { "Changing the embedder invalidates all indexed data — on the next start the server wipes stored vectors and every vault re-indexes on its next sync." }
                label { "Fastembed model (fastembed only)" }
                select name="fastembed_model" {
                    option value="" selected[current_fastembed.is_empty()] { "— pick a model —" }
                    @for (code, dim) in &fastembed_models {
                        option value=(code) selected[code.eq_ignore_ascii_case(&current_fastembed)] {
                            (code) " (" (dim) " dims)"
                        }
                    }
                }
                div .row {
                    div { label { "URL (ollama / openai)" } input type="text" name="embedder_url" value=(current_url); }
                    div { label { "Model (ollama / openai)" } input type="text" name="embedder_model" value=(current_model); }
                }
                label { "API key (openai only)" }
                input type="password" name="embedder_api_key" placeholder=(if embedder_key_set { "unchanged (a key is set)" } else { "from environment if blank" });
                p .muted { "Instruction prefixes (doc_prefix / query_prefix) are file-only settings; a save here keeps them." }
            }
            div .card {
                h2 { "Vector DB" }
                label { "Backend" }
                select name="vector_db" {
                    option value="lance" selected[current_vector_db == "lance"] { "LanceDB (embedded, local)" }
                    option value="qdrant" selected[current_vector_db == "qdrant"] { "Qdrant (server)" }
                }
                label { "LanceDB path (lance only)" }
                input type="text" name="lance_path" value=(lance_path) placeholder="default: data dir";
                div .row {
                    div { label { "Qdrant URL (qdrant only)" } input type="text" name="qdrant_url" value=(qdrant_url) placeholder="http://localhost:6333"; }
                    div { label { "Qdrant collection prefix (qdrant only)" } input type="text" name="qdrant_collection" value=(qdrant_collection) placeholder="kimun_embeddings"; }
                }
            }
            div .card {
                h2 { "LLM" }
                label { "Provider" }
                select name="provider" {
                    @for p in providers {
                        option value=(p) selected[p == current] {
                            @if p == "none" { "— none (semantic-only) —" } @else { (p) }
                        }
                    }
                }
                p .muted { "Select — none — for a search-only server (no question-answering)." }
                label { "Model" }
                input type="text" name="model" value=(c.llm.as_ref().map(|l| l.model()).unwrap_or(""));
                label { "API key" }
                input type="password" name="api_key" placeholder=(if c.llm.as_ref().and_then(|l| l.api_key()).is_some() { "unchanged (a key is set)" } else { "from environment if blank" });
                p .muted { "Leave blank to keep the current key (or fall back to the provider env var)." }
            }
            div .card {
                h2 { "Reranker" }
                div .check { input type="checkbox" name="reranker_enabled" checked[c.reranker.enabled]; label style="margin:0" { "Enabled" } }
                label { "Default results (top_k)" }
                input type="number" name="reranker_top_k" value=(c.reranker.top_k);
            }
            div .card {
                h2 { "Auth" }
                label { "Bearer token" }
                input type="password" name="auth_token" placeholder=(if c.auth.token.is_some() { "unchanged (a token is set)" } else { "open — no token set" });
                p .muted { "Leave blank to keep the current token. Clearing it (going open) must be done in the config file." }
            }
            @if can_save {
                button type="submit" { "Save to config file" }
                p .muted { "Saved changes take effect the next time the server starts." }
            }
        }
    };
    shell(state, "/config", "Configuration", body)
}

/// Pure web plumbing: origin check, writable-path check, then hand the form to
/// [`RagConfig::apply_form`] — every form→config rule lives there, so a new
/// web-exposed option touches the config module and the form markup, not this
/// handler.
async fn config_submit(
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
            Some(html! { p .flash.ok { "Saved to " span .mono { (path.display()) } ". Restart the server to apply." } }),
        )
        .into_response(),
        Err(e) => err_page(&state, html! { p .flash.err { "Could not write config: " (e) } }),
    }
}

// ============================================================================
// Collections
// ============================================================================

async fn collections_page(State(state): State<Arc<AppState>>) -> Markup {
    let body = match &state.rag {
        None => html! {
            h1 { "Collections" }
            div .card {
                p .flash.err {
                    "Server unconfigured — configure an embedder in "
                    a href="/config" { "Config" } " to enable indexing."
                }
            }
        },
        Some(rag) => {
            let result = rag.collections().await;
            html! {
                h1 { "Collections" }
                div .card {
                    @match result {
                        Ok(cols) if cols.is_empty() => p .muted { "No collections yet — push some notes from Kimün." },
                        Ok(cols) => table {
                            thead { tr { th { "Vault id" } th { "Indexed notes" } } }
                            tbody {
                                @for col in &cols {
                                    tr { td .mono { (col.name) } td { (col.note_count) } }
                                }
                            }
                        },
                        Err(e) => p .flash.err { "Could not list collections: " (e) },
                    }
                }
            }
        }
    };
    shell(&state, "/collections", "Collections", body)
}

// ============================================================================
// Jobs
// ============================================================================

async fn jobs_page(State(state): State<Arc<AppState>>) -> Markup {
    let table = jobs_table(&state).await;
    let body = html! {
        h1 { "Jobs" }
        div .card #jobs { (table) }
        script {
            (maud::PreEscaped(r#"
setInterval(async () => {
  if (document.visibilityState !== 'visible') return;
  try {
    const r = await fetch('/jobs/fragment');
    if (r.redirected) { location.href = '/login'; return; }
    if (r.ok) document.getElementById('jobs').innerHTML = await r.text();
  } catch (e) {}
}, 2000);
"#))
        }
    };
    shell(&state, "/jobs", "Jobs", body)
}

async fn jobs_fragment(State(state): State<Arc<AppState>>) -> Markup {
    jobs_table(&state).await
}

async fn jobs_table(state: &AppState) -> Markup {
    let jobs = state.job_tracker.lock().await.list();
    html! {
        @if jobs.is_empty() {
            p .muted { "No jobs yet." }
        } @else {
            table {
                thead { tr { th { "Job" } th { "Status" } th { "Detail" } } }
                tbody {
                    @for job in &jobs {
                        tr {
                            td .mono { (short_id(&job.id.to_string())) }
                            td { span .badge { (job.status.as_str()) } }
                            td .snippet {
                                @if let Some(err) = &job.error { (err) }
                                @else if let Some(res) = &job.result { (truncate(res, 160)) }
                                @else { "—" }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ============================================================================
// Test query
// ============================================================================

async fn query_page(State(state): State<Arc<AppState>>) -> Markup {
    let collections = collection_names(&state).await;
    query_markup(&state, &collections, "", "", None)
}

#[derive(Deserialize)]
struct QueryForm {
    vault_id: String,
    query: String,
}

async fn query_submit(State(state): State<Arc<AppState>>, Form(f): Form<QueryForm>) -> Markup {
    let collections = collection_names(&state).await;
    let results = run_search(&state, &f.vault_id, &f.query).await;
    query_markup(&state, &collections, &f.vault_id, &f.query, Some(results))
}

type SearchOutcome = Result<Vec<(f64, String, String)>, String>;

/// The same pipeline the API's `/api/embeddings` runs — the test query shows
/// exactly what clients get (one row per note, top_k notes).
async fn run_search(state: &AppState, vault_id: &str, query: &str) -> SearchOutcome {
    let Some(rag) = state.rag.as_ref() else {
        return Err("Server unconfigured — configure an embedder first.".into());
    };
    if vault_id.is_empty() || query.trim().is_empty() {
        return Err("Pick a collection and enter a query.".into());
    }
    let collection = crate::CollectionKey::parse(vault_id).map_err(|e| e.to_string())?;
    let top_k = state.config.reranker.top_k;
    let ranked = rag
        .search(&collection, query, top_k)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ranked
        .into_iter()
        .map(|(score, chunk)| (score, chunk.doc_path, chunk.text))
        .collect())
}

fn query_markup(
    state: &AppState,
    collections: &[String],
    vault_id: &str,
    query: &str,
    results: Option<SearchOutcome>,
) -> Markup {
    let body = html! {
        h1 { "Test query" }
        @if state.rag.is_none() {
            div .card {
                p .flash.err {
                    "Server unconfigured — configure an embedder in "
                    a href="/config" { "Config" } " to enable search."
                }
            }
        } @else {
            div .card {
                form method="post" action="/query" {
                    label { "Collection" }
                    select name="vault_id" {
                        option value="" { "— select —" }
                        @for c in collections {
                            option value=(c) selected[c == vault_id] { (c) }
                        }
                    }
                    label { "Query" }
                    input type="text" name="query" value=(query) autofocus?;
                    button type="submit" { "Search" }
                }
            }
        }
        @if let Some(outcome) = results {
            div .card {
                @match outcome {
                    Err(e) => p .flash.err { (e) },
                    Ok(hits) if hits.is_empty() => p .muted { "No matches." },
                    Ok(hits) => {
                        h2 { (hits.len()) " results" }
                        @for (score, path, text) in &hits {
                            div style="margin:.8rem 0" {
                                div { span .mono { (path) } " " span .muted { (format!("{score:.3}")) } }
                                div .snippet { (truncate(text, 240)) }
                            }
                        }
                    }
                }
            }
        }
    };
    shell(state, "/query", "Test query", body)
}

// ============================================================================
// Helpers
// ============================================================================

async fn collection_names(state: &AppState) -> Vec<String> {
    match &state.rag {
        Some(rag) => rag.collection_names().await.unwrap_or_default(),
        None => Vec::new(),
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        let cut: String = trimmed.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KimunRag;
    use crate::test_support::{FakeEmbedder, FakeLlm, FakeVectorStore};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header::CONTENT_TYPE};
    use tower::ServiceExt;

    fn state(token: Option<&str>, config_path: Option<std::path::PathBuf>) -> Arc<AppState> {
        let config_toml = format!(
            r#"
[server]
[vector_db]
type = "qdrant"
[llm]
provider = "gemini"
[reranker]
{}
"#,
            token
                .map(|t| format!("[auth]\ntoken = \"{t}\""))
                .unwrap_or_default()
        );
        let config: RagConfig = toml::from_str(&config_toml).unwrap();
        let rag = KimunRag::new(
            Arc::new(FakeVectorStore::default()),
            Arc::new(FakeEmbedder),
            Some(Arc::new(FakeLlm)),
        );
        let mut st = AppState::new(Some(rag), config);
        if let Some(p) = config_path {
            st = st.with_config_path(p);
        }
        Arc::new(st)
    }

    fn app(state: Arc<AppState>) -> Router {
        Router::new().merge(routes(state.clone())).with_state(state)
    }

    async fn body_text(resp: axum::response::Response) -> String {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn dashboard_renders_when_open() {
        let app = app(state(None, None));
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("Dashboard"));
    }

    #[tokio::test]
    async fn collections_page_lists_store_collections() {
        let app = app(state(None, None));
        let resp = app
            .oneshot(Request::get("/collections").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(body_text(resp).await.contains("vault-1"));
    }

    #[tokio::test]
    async fn protected_route_redirects_to_login_without_cookie() {
        let app = app(state(Some("secret"), None));
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers()["location"], "/login");
    }

    #[tokio::test]
    async fn valid_session_cookie_grants_access() {
        let app = app(state(Some("secret"), None));
        let cookie = format!("kimun_session={}", session_value("secret"));
        let resp = app
            .oneshot(
                Request::get("/")
                    .header(COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn raw_token_cookie_is_rejected() {
        // The cookie must carry the hash, not the token itself.
        let app = app(state(Some("secret"), None));
        let resp = app
            .oneshot(
                Request::get("/")
                    .header(COOKIE, "kimun_session=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn login_sets_cookie_on_correct_token() {
        let app = app(state(Some("secret"), None));
        let resp = app
            .oneshot(
                Request::post("/login")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("token=secret"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let cookie = resp.headers()[SET_COOKIE].to_str().unwrap();
        assert!(cookie.contains(&format!("kimun_session={}", session_value("secret"))));
        assert!(!cookie.contains("kimun_session=secret;")); // not the raw token
        assert!(cookie.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn login_survives_token_with_special_chars() {
        // A token containing ';' and a space must not break the cookie or lock
        // out the admin (regression: raw-token cookie).
        let token = "a b;c";
        let app = app(state(Some(token), None));
        let resp = app
            .clone()
            .oneshot(
                Request::post("/login")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    // URL-encoded form value for "a b;c".
                    .body(Body::from("token=a%20b%3Bc"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let set = resp.headers()[SET_COOKIE].to_str().unwrap().to_string();
        // The session cookie value round-trips: use it to reach a protected page.
        let jar = set.split(';').next().unwrap().to_string();
        let resp2 = app
            .oneshot(
                Request::get("/")
                    .header(COOKIE, jar)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn config_submit_persists_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag.conf");
        let app = app(state(None, Some(path.clone())));
        let form = "host=127.0.0.1&port=7573&provider=claude&model=my-model&api_key=&embedder_provider=none&fastembed_model=&embedder_url=&embedder_model=&embedder_api_key=&vector_db=qdrant&lance_path=&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("Saved"));

        let saved = RagConfig::from_file(path).unwrap();
        let llm = saved.llm.as_ref().expect("llm saved");
        assert_eq!(llm.provider(), "claude");
        assert_eq!(llm.model(), "my-model");
    }

    /// Builds an AppState from a full config TOML, with a writable config path.
    fn state_from(config_toml: &str, path: std::path::PathBuf) -> Arc<AppState> {
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        let rag = KimunRag::new(
            Arc::new(FakeVectorStore::default()),
            Arc::new(FakeEmbedder),
            Some(Arc::new(FakeLlm)),
        );
        Arc::new(AppState::new(Some(rag), config).with_config_path(path))
    }

    /// An unconfigured server: no embedder in config, no pipeline (adr/0024).
    fn unconfigured_state() -> Arc<AppState> {
        let config: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"lance\"\n[reranker]\n").unwrap();
        assert!(config.embedder.is_none());
        Arc::new(AppState::new(None, config))
    }

    #[tokio::test]
    async fn unconfigured_dashboard_points_to_config() {
        let app = app(unconfigured_state());
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let html = body_text(resp).await;
        assert!(html.contains("unconfigured"), "dashboard must flag the state");
        assert!(html.contains("/config"), "and link to the config page");
    }

    #[tokio::test]
    async fn unconfigured_collections_page_shows_banner_not_error() {
        let app = app(unconfigured_state());
        let resp = app
            .oneshot(Request::get("/collections").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("unconfigured"));
    }

    #[tokio::test]
    async fn unconfigured_query_page_disables_search() {
        let app = app(unconfigured_state());
        let resp = app
            .oneshot(Request::get("/query").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(resp).await;
        assert!(html.contains("unconfigured"));
        assert!(
            !html.contains(r#"<button type="submit">Search"#),
            "no live search form"
        );
    }

    #[tokio::test]
    async fn provider_switch_with_blank_key_does_not_carry_old_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag.conf");
        let app = app(state_from(
            r#"
[server]
[vector_db]
type = "qdrant"
[llm]
provider = "gemini"
api_key = "gemini-key"
[reranker]
"#,
            path.clone(),
        ));
        // Switch to openai, leave key blank → must NOT reuse the gemini key.
        let form = "host=127.0.0.1&port=7573&provider=openai&model=gpt-4o-mini&api_key=&embedder_provider=none&fastembed_model=&embedder_url=&embedder_model=&embedder_api_key=&vector_db=qdrant&lance_path=&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let saved = RagConfig::from_file(path).unwrap();
        let llm = saved.llm.as_ref().expect("llm saved");
        assert_eq!(llm.provider(), "openai");
        assert_eq!(
            llm.api_key(),
            None,
            "old provider's key must not carry over"
        );
    }

    #[tokio::test]
    async fn invalid_port_flashes_error_without_400_or_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag.conf");
        let app = app(state_from(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[llm]\nprovider = \"gemini\"\n[reranker]\n",
            path.clone(),
        ));
        let form = "host=127.0.0.1&port=99999999&provider=gemini&model=m&api_key=&embedder_provider=none&fastembed_model=&embedder_url=&embedder_model=&embedder_api_key=&vector_db=qdrant&lance_path=&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK); // friendly page, not a bare 400
        assert!(body_text(resp).await.contains("must be whole numbers"));
        assert!(
            !path.exists(),
            "invalid input must not write the config file"
        );
    }

    #[tokio::test]
    async fn saving_provider_none_clears_llm_to_semantic_only() {
        // Selecting "none" disables Q&A: llm is cleared to None, not written as a
        // keyless provider that would fail the boot key gate (adr/0022).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag.conf");
        let app = app(state_from(
            r#"
[server]
[vector_db]
type = "qdrant"
[llm]
provider = "gemini"
api_key = "gemini-key"
[reranker]
"#,
            path.clone(),
        ));
        let form = "host=127.0.0.1&port=7573&provider=none&model=&api_key=&embedder_provider=none&fastembed_model=&embedder_url=&embedder_model=&embedder_api_key=&vector_db=qdrant&lance_path=&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let saved = RagConfig::from_file(path).unwrap();
        assert!(saved.llm.is_none(), "provider=none must clear the LLM");
    }

    #[tokio::test]
    async fn semantic_only_config_page_defaults_provider_to_none() {
        // With no [llm], the provider select must pre-select the none sentinel.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag.conf");
        let app = app(state_from(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n",
            path,
        ));
        let resp = app
            .oneshot(Request::get("/config").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(resp).await;
        assert!(
            html.contains(r#"<option value="none" selected"#),
            "none must be the selected provider on a semantic-only server"
        );
    }

    #[tokio::test]
    async fn config_form_saves_embedder_and_vector_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag.conf");
        let app = app(state_from(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n",
            path.clone(),
        ));
        let form = "host=127.0.0.1&port=7573&provider=none&model=&api_key=&embedder_provider=fastembed&fastembed_model=Xenova%2Fbge-small-en-v1.5&embedder_url=&embedder_model=&embedder_api_key=&vector_db=lance&lance_path=%2Fdata%2Flance&qdrant_url=&qdrant_collection=&reranker_top_k=20&auth_token=";
        let resp = app
            .oneshot(
                Request::post("/config")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_text(resp).await.contains("Saved"));

        let saved = RagConfig::from_file(path).unwrap();
        match saved.embedder {
            Some(crate::config::EmbedderConfig::FastEmbed { model }) => {
                assert_eq!(model.as_deref(), Some("Xenova/bge-small-en-v1.5"))
            }
            other => panic!("expected fastembed, got {other:?}"),
        }
        assert!(matches!(
            saved.vector_db,
            crate::config::VectorDbConfig::Lance { .. }
        ));
    }

    /// An unconfigured server with a writable config path (for the form page).
    fn unconfigured_state_with_path() -> Arc<AppState> {
        let config: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"lance\"\n[reranker]\n").unwrap();
        Arc::new(
            AppState::new(None, config).with_config_path(std::path::PathBuf::from("/dev/null")),
        )
    }

    #[tokio::test]
    async fn config_page_preselects_fastembed_model_saved_as_variant_name() {
        // A config may name the model by fastembed variant (`BGESmallENV15`);
        // the dropdown is keyed by model code. Without canonicalization no
        // option is selected, the browser submits the empty placeholder, and
        // any unrelated save fails with "Pick a fastembed model."
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rag.conf");
        let app = app(state_from(
            "[server]\n[vector_db]\ntype = \"lance\"\n[embedder]\ntype = \"fastembed\"\nmodel = \"BGESmallENV15\"\n[reranker]\n",
            path,
        ));
        let resp = app
            .oneshot(Request::get("/config").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(resp).await;
        assert!(
            html.contains(r#"<option value="Xenova/bge-small-en-v1.5" selected"#),
            "variant-name config must pre-select its model-code option"
        );
    }

    #[tokio::test]
    async fn unconfigured_config_page_preselects_embedder_none_and_warns() {
        let app = app(unconfigured_state_with_path());
        let resp = app
            .oneshot(Request::get("/config").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let html = body_text(resp).await;
        assert!(
            html.contains(r#"<select name="embedder_provider""#),
            "embedder select must render"
        );
        assert!(html.contains("invalidates"), "wipe warning must render");
    }

    #[tokio::test]
    async fn answer_handler_rejects_when_semantic_only() {
        // A semantic-only server (no [llm]) must reject /api/answer at submit
        // time with 503, not mint a job that can only fail (adr/0022).
        let config: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n").unwrap();
        assert!(config.llm.is_none());
        let rag = KimunRag::new(Arc::new(FakeVectorStore::default()), Arc::new(FakeEmbedder), None);
        let state = Arc::new(AppState::new(Some(rag), config));

        let req = crate::handlers::AnswerRequest {
            vault_id: "vault-1".into(),
            query: "hello".into(),
            context_size: None,
        };
        let err = crate::handlers::answer_handler(axum::extract::State(state), axum::Json(req))
            .await
            .expect_err("semantic-only server must reject answering");
        assert!(matches!(err, crate::RagError::SemanticOnly));
        assert_eq!(
            err.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
