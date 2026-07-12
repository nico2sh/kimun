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

use crate::config::{LlmConfig, RagConfig};
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
        crate::config::EmbedderConfig::FastEmbed { model } => {
            format!(
                "fastembed ({})",
                model.as_deref().unwrap_or("default BGE-Large")
            )
        }
        crate::config::EmbedderConfig::Ollama { url, model, .. } => {
            format!("ollama {model} @ {url}")
        }
        crate::config::EmbedderConfig::OpenAI { url, model, .. } => {
            format!("openai-compatible {model} @ {url}")
        }
    };
    let body = html! {
        h1 { "Dashboard" }
        div .card {
            dl {
                dt { "Bind address" } dd .mono { (c.server.host) ":" (c.server.port) }
                dt { "Vector DB" } dd { (vector_db) }
                dt { "Embedder" } dd { (embedder) }
                dt { "LLM" } dd { (c.llm.provider()) " · " (c.llm.model()) }
                dt { "LLM key" } dd { @if c.llm.api_key().is_some() { "set in config" } @else { "from environment" } }
                dt { "Reranker" } dd { @if c.reranker.enabled { "on (top_k " (c.reranker.top_k) ")" } @else { "off" } }
                dt { "Auth" } dd { @if c.auth.token.is_some() { span .badge { "token required" } } @else { span .badge { "open" } } }
            }
        }
        p .muted { "The vector store and embedder are fixed at startup. Change them in the config file and restart the server." }
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
    let providers = ["gemini", "claude", "openai", "mistral"];
    let current = c.llm.provider();
    let can_save = state.config_path.is_some();
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
                h2 { "LLM" }
                label { "Provider" }
                select name="provider" {
                    @for p in providers {
                        option value=(p) selected[p == current] { (p) }
                    }
                }
                label { "Model" }
                input type="text" name="model" value=(c.llm.model());
                label { "API key" }
                input type="password" name="api_key" placeholder=(if c.llm.api_key().is_some() { "unchanged (a key is set)" } else { "from environment if blank" });
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

// Numeric fields arrive as strings so a non-numeric value yields a friendly
// flash instead of a bare 400 that discards the whole form.
#[derive(Deserialize)]
struct ConfigForm {
    host: String,
    port: String,
    provider: String,
    model: String,
    api_key: String,
    #[serde(default)]
    reranker_enabled: Option<String>,
    reranker_top_k: String,
    auth_token: String,
}

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

    let (Ok(port), Ok(top_k)) = (
        f.port.trim().parse::<u16>(),
        f.reranker_top_k.trim().parse::<usize>(),
    ) else {
        return err_page(
            &state,
            html! { p .flash.err { "Port and top_k must be whole numbers." } },
        );
    };

    // A typed key overwrites; a blank key keeps the current one only when the
    // provider is unchanged — switching provider with a blank key must NOT carry
    // the old provider's key over (it would be wrong), so fall back to the env var.
    let key = if !f.api_key.is_empty() {
        Some(f.api_key)
    } else if f.provider == state.config.llm.provider() {
        state.config.llm.api_key().map(str::to_string)
    } else {
        None
    };
    let llm = match LlmConfig::from_parts(&f.provider, Some(f.model), key) {
        Ok(llm) => llm,
        Err(e) => {
            return err_page(
                &state,
                html! { p .flash.err { "Invalid LLM settings: " (e) } },
            );
        }
    };

    let mut cfg: RagConfig = (*state.config).clone();
    cfg.server.host = f.host;
    cfg.server.port = port;
    cfg.llm = llm;
    cfg.reranker.enabled = f.reranker_enabled.is_some();
    cfg.reranker.top_k = top_k;
    // Blank keeps the current token (the password field is never pre-filled).
    if !f.auth_token.is_empty() {
        cfg.auth.token = Some(f.auth_token);
    }

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
    let embeddings = {
        let rag = state.rag.lock().await;
        rag.embeddings()
    };
    let result = embeddings.list_collections().await;
    let body = html! {
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

async fn run_search(state: &AppState, vault_id: &str, query: &str) -> SearchOutcome {
    if vault_id.is_empty() || query.trim().is_empty() {
        return Err("Pick a collection and enter a query.".into());
    }
    let (embeddings, reranker) = {
        let rag = state.rag.lock().await;
        (rag.embeddings(), rag.get_reranker())
    };
    let top_k = state.config.reranker.top_k;
    let raw = embeddings
        .query_embedding(vault_id, query)
        .await
        .map_err(|e| e.to_string())?;
    let ranked = match reranker {
        Some(r) => r
            .rerank(query, raw, top_k)
            .await
            .map_err(|e| e.to_string())?,
        None => raw.into_iter().take(top_k).collect(),
    };
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
    let embeddings = {
        let rag = state.rag.lock().await;
        rag.embeddings()
    };
    embeddings.collection_names().await.unwrap_or_default()
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
    use crate::dbembeddings::{CollectionInfo, Embeddings, IndexedNote};
    use crate::document::FlattenedChunk;
    use crate::llmclients::LLMClient;
    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header::CONTENT_TYPE};
    use std::collections::HashMap;
    use tower::ServiceExt;

    struct FakeEmbeddings;

    #[async_trait]
    impl Embeddings for FakeEmbeddings {
        async fn init(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn store_embeddings(
            &self,
            _: &str,
            _: &[crate::document::KimunDoc],
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn delete_embeddings(&self, _: &str, _: Vec<&String>) -> anyhow::Result<()> {
            Ok(())
        }
        async fn query_embedding(
            &self,
            _: &str,
            _: &str,
        ) -> anyhow::Result<Vec<(f64, FlattenedChunk)>> {
            Ok(vec![(
                0.9,
                FlattenedChunk {
                    doc_path: "/notes/a.md".into(),
                    doc_hash: "h".into(),
                    title: "A".into(),
                    text: "hello world".into(),
                    date: None,
                },
            )])
        }
        async fn get_indexed_notes(&self, _: &str) -> anyhow::Result<HashMap<String, IndexedNote>> {
            Ok(HashMap::new())
        }
        async fn remove_indexed_note(&self, _: &str, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn list_collections(&self) -> anyhow::Result<Vec<CollectionInfo>> {
            Ok(vec![CollectionInfo {
                name: "vault-1".into(),
                note_count: 3,
            }])
        }
        async fn collection_names(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec!["vault-1".into()])
        }
    }

    struct FakeLlm;

    #[async_trait]
    impl LLMClient for FakeLlm {
        async fn ask(&self, _: &str, _: &[(f64, FlattenedChunk)]) -> anyhow::Result<String> {
            Ok("answer".into())
        }
    }

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
        let rag = KimunRag::new(Arc::new(FakeEmbeddings), Arc::new(FakeLlm));
        let mut st = AppState::new(rag, config);
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
        let form = "host=127.0.0.1&port=7573&provider=claude&model=my-model&api_key=&reranker_top_k=20&auth_token=";
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
        assert_eq!(saved.llm.provider(), "claude");
        assert_eq!(saved.llm.model(), "my-model");
    }

    /// Builds an AppState from a full config TOML, with a writable config path.
    fn state_from(config_toml: &str, path: std::path::PathBuf) -> Arc<AppState> {
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        let rag = KimunRag::new(Arc::new(FakeEmbeddings), Arc::new(FakeLlm));
        Arc::new(AppState::new(rag, config).with_config_path(path))
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
        let form = "host=127.0.0.1&port=7573&provider=openai&model=gpt-4o-mini&api_key=&reranker_top_k=20&auth_token=";
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
        assert_eq!(saved.llm.provider(), "openai");
        assert_eq!(
            saved.llm.api_key(),
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
        let form = "host=127.0.0.1&port=99999999&provider=gemini&model=m&api_key=&reranker_top_k=20&auth_token=";
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
}
