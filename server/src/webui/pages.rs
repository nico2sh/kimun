//! Read-only pages: dashboard, collections, jobs, logs, and the test query.

use std::sync::Arc;

use axum::{Form, extract::State};
use maud::{Markup, html};
use serde::Deserialize;

use crate::server_state::AppState;

use super::shell::shell;

// ============================================================================
// Dashboard
// ============================================================================

pub(super) async fn dashboard(State(state): State<Arc<AppState>>) -> Markup {
    let c = &state.config;
    let vector_db = match &c.vector_db {
        crate::config::VectorDbConfig::Sqlite { path } => {
            format!("SQLite ({})", path.display())
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
    // Glance line: live counts so "is my server fine?" is answered before the
    // config echo. Skipped when the store can't be reached (the pages below
    // surface their own errors).
    let glance = match &state.rag {
        Some(rag) => {
            let active = state
                .job_tracker
                .lock()
                .await
                .list()
                .iter()
                .filter(|j| {
                    matches!(
                        j.status,
                        crate::server_state::JobStatus::Queued
                            | crate::server_state::JobStatus::Processing
                    )
                })
                .count();
            rag.collections().await.ok().map(|cols| {
                let notes: usize = cols.iter().map(|c| c.note_count).sum();
                (cols.len(), notes, active)
            })
        }
        None => None,
    };
    let reranker_active = state.rag.as_ref().is_some_and(|r| r.has_reranker());
    let body = html! {
        h1 { "Dashboard" }
        @if let Some((cols, notes, active)) = glance {
            p .statusline {
                b { (count_noun(cols, "collection")) }
                " · "
                b { (count_noun(notes, "indexed note")) }
                " · "
                @if active == 0 { "idle" } @else { b { (count_noun(active, "active job")) } }
            }
        }
        @if let Some(err) = &state.startup_error {
            p .flash.err {
                "Startup failed — the configured embedder could not be initialized, so indexing "
                "and search are disabled until the problem is fixed and the server restarts. "
                a href="/logs" { "See the logs" } " or " a href="/config" { "review the config" } "."
                br;
                span .mono { (err) }
            }
        }
        @else if c.embedder.is_none() {
            p .flash.err {
                "This server is unconfigured — no embedder is set, so indexing and search are disabled. "
                a href="/config" { "Configure an embedder" } "."
            }
        }
        div .panel {
            dl {
                dt { "Bind address" } dd .mono { (c.server.host) ":" (c.server.port) }
                dt { "Vector DB" } dd { (vector_db) }
                dt { "Embedder" } dd { (embedder) }
                dt { "LLM" } dd {
                    @if let Some(l) = &c.llm {
                        (l.provider()) " · " (l.model())
                        @if let Some(u) = l.url() { " · " (u) }
                    }
                    @else { "not configured (semantic-only)" }
                }
                dt { "LLM key" } dd {
                    @if let Some(l) = &c.llm {
                        @if l.api_key().is_some() { "set in config" } @else { "from environment" }
                    } @else { "—" }
                }
                dt { "Reranker" } dd {
                    // Actual state, not config — matches /health: an enabled
                    // reranker whose init failed must not read as "on".
                    @if reranker_active {
                        "on: " (c.reranker.provider.label()) " (top_k " (c.reranker.top_k) ")"
                        @if let Some(u) = c.reranker.url.as_deref() { " · " (u) }
                    } @else if c.reranker.enabled {
                        "enabled but failed to start — using plain vector ranking. "
                        a href="/logs" { "See the logs" } "."
                        @if let Some(err) = &state.reranker_error { br; span .mono { (err) } }
                    } @else { "off" }
                }
                dt { "Auth" } dd { @if c.auth.token.is_some() { span .badge { "token required" } } @else { span .badge { "open" } } }
            }
        }
        p .muted { "The vector store and embedder are fixed at startup. Change them in Config and restart the server." }
    };
    shell(&state, "/", "Dashboard", body)
}

// ============================================================================
// Collections
// ============================================================================

pub(super) async fn collections_page(State(state): State<Arc<AppState>>) -> Markup {
    let body = match &state.rag {
        None => html! {
            h1 { "Collections" }
            p .flash.err {
                "Server unconfigured — configure an embedder in "
                a href="/config" { "Config" } " to enable indexing."
            }
        },
        Some(rag) => {
            let result = rag.collections().await;
            html! {
                h1 { "Collections" }
                @match result {
                    Ok(cols) if cols.is_empty() => {
                        p .muted {
                            "No collections yet — each vault that syncs here gets one. Push notes from Kimün: "
                            span .mono { "kimun workspace sync" }
                        }
                    },
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
    };
    shell(&state, "/collections", "Collections", body)
}

// ============================================================================
// Jobs
// ============================================================================

pub(super) async fn jobs_page(State(state): State<Arc<AppState>>) -> Markup {
    let table = jobs_table(&state).await;
    let body = html! {
        h1 { "Jobs" }
        p .live { "live — refreshes every 2s while this tab is visible" }
        div #jobs { (table) }
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

pub(super) async fn jobs_fragment(State(state): State<Arc<AppState>>) -> Markup {
    jobs_table(&state).await
}

async fn jobs_table(state: &AppState) -> Markup {
    let jobs = state.job_tracker.lock().await.list();
    html! {
        @if jobs.is_empty() {
            p .muted { "No jobs yet — syncs and questions land here as they run." }
        } @else {
            table {
                thead { tr { th { "Job" } th { "Status" } th { "Detail" } } }
                tbody {
                    @for job in &jobs {
                        tr {
                            td .mono { (short_id(&job.id.to_string())) }
                            td { span class=(format!("status {}", job.status.as_str())) { (job.status.as_str()) } }
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
// Logs
// ============================================================================

pub(super) async fn logs_page(State(state): State<Arc<AppState>>) -> Markup {
    let entries = state.log_buffer.list();
    let body = html! {
        h1 { "Logs" }
        p .muted {
            "Warnings and errors since startup, newest first (last "
            (crate::logbuffer::CAPACITY)
            " kept in memory — the full log is on the server's stdout/journal)."
        }
        @if entries.is_empty() {
            p .muted { "No warnings or errors since startup." }
        } @else {
            table {
                thead { tr { th { "Time" } th { "Level" } th { "Message" } } }
                tbody {
                    @for e in &entries {
                        tr {
                            td .mono { (fmt_time(e.time)) }
                            td {
                                span class=(if e.level == tracing::Level::ERROR { "status failed" } else { "status" }) {
                                    (e.level.as_str())
                                }
                            }
                            td .snippet { (e.message) }
                        }
                    }
                }
            }
        }
    };
    shell(&state, "/logs", "Logs", body)
}

fn fmt_time(t: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%H:%M:%S")
        .to_string()
}

// ============================================================================
// Test query
// ============================================================================

pub(super) async fn query_page(State(state): State<Arc<AppState>>) -> Markup {
    let collections = collection_names(&state).await;
    query_markup(&state, &collections, "", "", None)
}

#[derive(Deserialize)]
pub(super) struct QueryForm {
    vault_id: String,
    query: String,
}

pub(super) async fn query_submit(
    State(state): State<Arc<AppState>>,
    Form(f): Form<QueryForm>,
) -> Markup {
    let collections = collection_names(&state).await;
    let results = run_search(&state, &f.vault_id, &f.query).await;
    query_markup(&state, &collections, &f.vault_id, &f.query, Some(results))
}

/// Where the **context cut** would slice an answer on this query — rendered
/// as a divider in the result list plus a summary line (adr/0027).
struct CutSummary {
    /// Displayed rows whose chunk made the would-be LLM context (they form a
    /// prefix: rows and context are cut from the same score-ordered pool).
    rows_in_context: usize,
    /// Chunks the cut keeps (counts every chunk, including extra sections of
    /// an already-listed note — the rows show only each note's best one).
    context_chunks: usize,
    pool_chunks: usize,
    /// Last kept / first dropped pool score — where the cut actually landed,
    /// which the note-deduped rows usually can't show.
    boundary: Option<(f64, f64)>,
    algorithm: &'static str,
}

/// Hits plus the wall-clock milliseconds the search took — the same duration
/// the API reports as `query_time_ms` — plus the context-cut preview, on
/// both reranker paths (adr/0029).
type SearchOutcome = Result<(Vec<(f64, String, String)>, u64, Option<CutSummary>), String>;

/// The same pipeline the API's `/api/embeddings` runs — the test query shows
/// exactly what clients get: one row per note surviving the context cut.
async fn run_search(state: &AppState, vault_id: &str, query: &str) -> SearchOutcome {
    let Some(rag) = state.rag.as_ref() else {
        return Err("Server unconfigured — configure an embedder first.".into());
    };
    if vault_id.is_empty() || query.trim().is_empty() {
        return Err("Pick a collection and enter a query.".into());
    }
    let collection = crate::CollectionKey::parse(vault_id).map_err(|e| e.to_string())?;
    let top_k = state.config.reranker.top_k;
    let started = std::time::Instant::now();
    let (ranked, preview) = rag
        .search_with_cut_preview(&collection, query, top_k)
        .await
        .map_err(|e| e.to_string())?;
    let query_time_ms = started.elapsed().as_millis() as u64;
    let cut = preview.map(|p| CutSummary {
        rows_in_context: ranked
            .iter()
            .take_while(|(_, chunk)| p.context.contains(chunk))
            .count(),
        context_chunks: p.context.len(),
        pool_chunks: p.pool_chunks,
        boundary: p.boundary,
        algorithm: state.config.reranker.context_cut.label(),
    });
    Ok((
        ranked
            .into_iter()
            .map(|(score, chunk)| (score, chunk.doc_path, chunk.text))
            .collect(),
        query_time_ms,
        cut,
    ))
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
        p .muted { "Runs the same pipeline clients get from the API — one row per surviving note. The configured context cut decides how many: a fixed count under \"fixed\", the pool's score shape under the adaptive cuts." }
        @if state.rag.is_none() {
            p .flash.err {
                "Server unconfigured — configure an embedder in "
                a href="/config" { "Config" } " to enable search."
            }
        } @else {
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
            div #results {
                @match outcome {
                    Err(e) => p .flash.err { (e) },
                    Ok((hits, ms, _)) if hits.is_empty() => p .muted { (format!("No matches ({ms} ms).")) },
                    Ok((hits, ms, cut)) => {
                        h2 { (count_noun(hits.len(), "result")) " " span .muted { (format!("· {ms} ms")) } }
                        @if let Some(cut) = &cut {
                            p .muted {
                                "Answer-context preview (" (cut.algorithm) "): the cut keeps "
                                (cut.context_chunks) " of " (cut.pool_chunks) " pooled chunks"
                                @if cut.context_chunks > cut.rows_in_context {
                                    (format!(" ({} are extra sections of listed notes)", cut.context_chunks - cut.rows_in_context))
                                }
                                @if let Some((last_kept, first_dropped)) = cut.boundary {
                                    (format!(" — pool cut at {last_kept:.3} → {first_dropped:.3}"))
                                }
                                ". Rows above the marker contribute to an answer's LLM context; the pool interleaves every section of every note, so the cut boundary usually falls between scores no row shows."
                            }
                        }
                        @for (i, (score, path, text)) in hits.iter().enumerate() {
                            div .hit {
                                div { span .mono { (path) } span .score { (format!("{score:.3}")) } }
                                div .snippet { (truncate(text, 240)) }
                            }
                            @if let Some(cut) = &cut {
                                @if i + 1 == cut.rows_in_context && i + 1 < hits.len() {
                                    div .muted .mono style="border-top: 1px dashed currentColor; margin: 0.5rem 0; padding-top: 0.25rem; text-align: center;" {
                                        (format!("── answer context cut ({}) ──", cut.algorithm))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        script {
            (maud::PreEscaped(r#"
// A submit re-renders the whole page, but the browser keeps the old page
// visible until the response lands — clear stale results immediately so a
// slow query never shows the previous answer next to a running search.
const qform = document.querySelector('form[action="/query"]');
if (qform) qform.addEventListener('submit', () => {
  const stale = document.getElementById('results');
  if (stale) stale.remove();
  const btn = qform.querySelector('button[type="submit"]');
  if (btn) { btn.disabled = true; btn.textContent = 'Searching…'; }
});
"#))
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

fn count_noun(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
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
