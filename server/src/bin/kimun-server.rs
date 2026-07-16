use axum::{
    Router,
    routing::{get, post},
};
use clap::Parser;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use kimun_server::{
    KimunRag,
    config::RagConfig,
    handlers::{
        answer_handler, collection_hashes_handler, get_embeddings_handler, index_delete_handler,
        index_docs_handler, job_status_handler,
    },
    server_state::AppState,
};

#[derive(Parser)]
#[command(version, about = "Kimun RAG Server", long_about = None)]
struct Cli {
    /// Path to configuration file (default: ~/.config/kimun/server.toml)
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,

    /// Start with built-in defaults — embedded SQLite plus the local
    /// fastembed embedder (default model) — without reading a config file.
    #[arg(long, conflicts_with = "config")]
    default_config: bool,

    /// Host to bind to (overrides config)
    #[arg(long)]
    host: Option<String>,

    /// Port to bind to (overrides config)
    #[arg(short, long)]
    port: Option<u16>,
}

/// Why one `run_server` iteration ended: an operator asked for an in-process
/// restart (drain, reload the config file, rebind — adr/0028), or the process
/// is done (Ctrl-C).
enum Shutdown {
    Restart,
    Terminate,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing ONCE for the process lifetime. Besides stdout,
    // WARN/ERROR events are copied into an in-memory ring buffer the web UI
    // serves at /logs; it survives in-process restarts so the log page shows
    // what happened across them.
    let log_buffer = kimun_server::logbuffer::LogBuffer::new();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kimun_server=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .with(log_buffer.layer())
        .init();

    let cli = Cli::parse();

    // The restart loop (adr/0028): each iteration builds and serves the whole
    // server; a web-UI restart drains in-flight requests, then the next
    // iteration re-reads the config file and rebinds, so every setting —
    // including the bind address — applies without a supervisor.
    let mut first_run = true;
    loop {
        match run_server(&cli, first_run, log_buffer.clone()).await? {
            Shutdown::Restart => {
                tracing::info!("Restart requested — reloading configuration");
                first_run = false;
            }
            Shutdown::Terminate => return Ok(()),
        }
    }
}

async fn run_server(
    cli: &Cli,
    first_run: bool,
    log_buffer: kimun_server::logbuffer::LogBuffer,
) -> anyhow::Result<Shutdown> {
    // Load configuration (remembering the path so the web UI can persist edits).
    tracing::info!("Loading configuration...");
    let config_path = RagConfig::resolve_path(cli.config.clone());
    let config = if cli.default_config && first_run {
        // Explicit opt-in to local defaults; no file is read. A missing config
        // file is created with these defaults so later file-based starts (and
        // web-UI edits) have a real file; an existing file is left untouched.
        tracing::info!(
            "--default-config: using built-in defaults (SQLite + fastembed), not reading a config file"
        );
        RagConfig::ready_default_persisted(&config_path)?
    } else {
        // On an in-process restart the seeded file (plus any web edits) is the
        // source of truth, --default-config or not.
        RagConfig::load(cli.config.clone())?
    };
    let config = config.merge_with_cli(cli.host.clone(), cli.port);

    tracing::info!("Configuration loaded successfully");
    tracing::debug!("Server: {}:{}", config.server.host, config.server.port);

    // Create RAG instance based on config. `None` = unconfigured (adr/0024).
    // A build failure (embedding model download failed, bad LLM key, …) does
    // NOT abort startup: the server comes up degraded — same 503-everything
    // behavior as unconfigured — so the web UI stays reachable to show the
    // error and fix the config.
    tracing::info!("Initializing RAG system...");
    let (rag, startup_error, reranker_error) = match create_rag_from_config(&config).await {
        Ok(Some((rag, reranker_error))) => (Some(rag), None, reranker_error),
        Ok(None) => (None, None, None),
        Err(e) => {
            let msg = format!("{e:#}");
            tracing::error!(
                "RAG initialization failed: {msg} — starting DEGRADED: indexing and search are \
                 disabled. Check http://{}:{}/logs and fix the config at /config, then restart.",
                config.server.host,
                config.server.port
            );
            (None, Some(msg), None)
        }
    };
    match &rag {
        Some(_) => tracing::info!("RAG system initialized"),
        None if startup_error.is_some() => {} // error logged above
        None => tracing::warn!(
            "No embedder configured — server is UNCONFIGURED: indexing and search are disabled. \
             Open http://{}:{}/config to set up an embedder (and optionally an LLM).",
            config.server.host,
            config.server.port
        ),
    }

    // Create application state. The restart channel is how the web UI's
    // Restart button reaches this loop iteration's graceful shutdown.
    let (restart_tx, mut restart_rx) = tokio::sync::mpsc::channel::<()>(1);
    let state = Arc::new(
        AppState::new(rag, config.clone())
            .with_config_path(config_path)
            .with_log_buffer(log_buffer)
            .with_startup_error(startup_error)
            .with_reranker_error(reranker_error)
            .with_restart(restart_tx),
    );

    // Periodically drop completed/old jobs so the tracker doesn't grow for the
    // life of the server. Holds only a Weak: when this iteration's state is
    // dropped after a restart, the sweep task ends instead of pinning the old
    // pipeline (embedder, store) in memory forever.
    {
        let sweep = Arc::downgrade(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let Some(state) = sweep.upgrade() else { break };
                state.job_tracker.lock().await.cleanup_old_jobs();
            }
        });
    }

    if state.config.auth.token.is_some() {
        tracing::info!("Bearer-token auth enabled on /api routes");
    } else if config.server.host != "127.0.0.1" && config.server.host != "localhost" {
        tracing::warn!(
            "No [auth] token set and bound to {} — the API is OPEN to the network",
            config.server.host
        );
    }

    // `/api` routes require the bearer token (when configured); `/health` is
    // always open for liveness probes.
    let api = Router::new()
        .route("/api/index/docs", post(index_docs_handler))
        .route("/api/index/delete", post(index_delete_handler))
        .route("/api/embeddings", post(get_embeddings_handler))
        .route("/api/answer", post(answer_handler))
        .route(
            "/api/collections/{vault_id}/hashes",
            get(collection_hashes_handler),
        )
        .route("/api/job/{job_id}", get(job_status_handler))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            kimun_server::auth::auth_middleware,
        ));

    let app = Router::new()
        .route("/health", get(health_handler))
        .merge(api)
        .merge(kimun_server::webui::routes(state.clone()))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("RAG server listening on {}", addr);
    tracing::info!("Health check available at http://{}/health", addr);

    // Serve until the web UI requests a restart or the process gets Ctrl-C;
    // either way axum drains in-flight requests before returning. The oneshot
    // smuggles the reason out of the shutdown future.
    let (reason_tx, reason_rx) = tokio::sync::oneshot::channel();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let reason = tokio::select! {
                _ = restart_rx.recv() => Shutdown::Restart,
                _ = tokio::signal::ctrl_c() => Shutdown::Terminate,
            };
            let _ = reason_tx.send(reason);
        })
        .await?;

    Ok(reason_rx.await.unwrap_or(Shutdown::Terminate))
}

/// Builds the query pipeline from config, or `None` on an *unconfigured*
/// server — no embedder means no vector store (its dimension comes from the
/// embedder) and no pipeline at all; every data endpoint rejects with 503
/// until one is configured (adr/0024).
/// Builds the pipeline from config. `Ok(None)` = unconfigured; the second
/// tuple field is why the (non-fatal) reranker failed to initialize, if it
/// did — surfaced via `/health` and the dashboard so `reranker: false` is
/// distinguishable from "disabled by config".
async fn create_rag_from_config(
    config: &RagConfig,
) -> anyhow::Result<Option<(KimunRag, Option<String>)>> {
    use anyhow::Context;
    use kimun_server::{
        config::{EmbedderConfig, VectorDbConfig},
        dbembeddings::{
            embedder::{Embedder, fastembedder::FastEmbedder, http::HttpEmbedder},
            vecqdrant::VecQdrant,
            vecsqlite::VecSqlite,
        },
        llmclients::ChatClient,
    };

    // No embedder → unconfigured: nothing to build (adr/0024).
    let Some(embedder_cfg) = &config.embedder else {
        return Ok(None);
    };

    // Build the embedder (shared by every collection on this server)
    let embedder: Arc<dyn Embedder> = match embedder_cfg {
        EmbedderConfig::FastEmbed { model } => {
            tracing::info!(
                "Using local fastembed embedder (model: {})",
                model.as_deref().unwrap_or("default BGE-Large")
            );
            Arc::new(FastEmbedder::new(model.as_deref()).with_context(|| {
                format!(
                    "could not initialize the fastembed embedder (model {}) — the model is \
                     downloaded on first use, so this usually means the download failed \
                     (offline? proxy?)",
                    model.as_deref().unwrap_or("default BGE-Large")
                )
            })?)
        }
        EmbedderConfig::Ollama {
            url,
            model,
            doc_prefix,
            query_prefix,
        } => {
            tracing::info!("Using Ollama embedder {} at {}", model, url);
            Arc::new(
                HttpEmbedder::ollama(
                    url.clone(),
                    model.clone(),
                    doc_prefix.clone(),
                    query_prefix.clone(),
                )
                .await?,
            )
        }
        EmbedderConfig::OpenAI {
            url,
            model,
            api_key,
            doc_prefix,
            query_prefix,
        } => {
            tracing::info!("Using OpenAI-compatible embedder {} at {}", model, url);
            Arc::new(
                HttpEmbedder::openai(
                    url.clone(),
                    model.clone(),
                    api_key.clone(),
                    doc_prefix.clone(),
                    query_prefix.clone(),
                )
                .await?,
            )
        }
    };
    tracing::info!("Embedder dimension: {}", embedder.dimension());

    // Create the vector store based on config. It only needs the embedder's
    // dimension (its tables/collections are created at that width) — embedding
    // itself happens in the pipeline, above the storage seam.
    let store: Arc<dyn kimun_server::dbembeddings::VectorStore + Send + Sync> = match &config
        .vector_db
    {
        VectorDbConfig::Sqlite { path } => {
            tracing::info!("Using SQLite vector database at {:?}", path);
            Arc::new(VecSqlite::new(path, embedder.dimension()).await?)
        }
        VectorDbConfig::Qdrant { url, collection } => {
            tracing::info!(
                "Using Qdrant vector database at {} (collection: {})",
                url,
                collection
            );
            Arc::new(VecQdrant::new(url.clone(), collection.clone(), embedder.dimension()).await?)
        }
    };

    // Embedder fingerprint (adr/0025): a changed embedder makes every stored
    // vector garbage, and reconciliation can't detect it. The gate is armed on
    // the pipeline (every data op verifies before touching the store) rather
    // than enforced here, so a store that is unreachable at boot (e.g. Qdrant
    // still starting) degrades to failing requests instead of aborting startup.
    let fingerprint = embedder_cfg.fingerprint(embedder.dimension());

    // Create LLM client based on config. `None` on a semantic-only server
    // (adr/0022). The key comes from config or the provider's env var and is
    // handed to the client directly — no env mutation, and a missing key is a
    // clean startup error, not a panic in the client.
    let llm_client: Option<Arc<dyn kimun_server::llmclients::LLMClient + Send + Sync>> =
        match &config.llm {
            Some(llm) => {
                let api_key = llm
                    .api_key()
                    .map(str::to_string)
                    .or_else(|| std::env::var(llm.env_var()).ok())
                    // A custom endpoint (openai-local: Ollama, llama.cpp, …) is
                    // typically keyless — send an empty bearer instead of
                    // refusing to boot. Cloud providers stay gated.
                    .or_else(|| llm.url().map(|_| String::new()))
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Missing API key: set [llm] api_key in config or export {}",
                            llm.env_var()
                        )
                    })?;
                tracing::info!("Using {} LLM with model: {}", llm.provider(), llm.model());
                Some(Arc::new(ChatClient::from_config(llm, api_key)))
            }
            None => {
                tracing::info!("No LLM configured — semantic-only server (search, no Q&A)");
                None
            }
        };

    let mut rag = KimunRag::new(store, embedder, llm_client)
        .with_fingerprint(fingerprint)
        .with_context_cut(config.reranker.context_cut);

    // Enable reranking if configured. Initialization failure (typically: the
    // cross-encoder model download failed — offline, proxy — or an unreachable
    // rerank endpoint) is non-fatal; the server logs a warning, serves with
    // plain vector ranking, and reports the reason via /health.
    let mut reranker_error = None;
    if config.reranker.enabled {
        match kimun_server::reranker::from_config(&config.reranker).await {
            Ok(reranker) => {
                tracing::info!(
                    "Reranking enabled ({}{})",
                    config.reranker.provider.label(),
                    config
                        .reranker
                        .url
                        .as_deref()
                        .map(|u| format!(" at {u}"))
                        .unwrap_or_default()
                );
                rag = rag.with_reranker(reranker);
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!(
                    "Reranker initialization failed ({msg}); continuing without reranking — \
                     semantic search still works, results use plain vector ranking"
                );
                reranker_error = Some(msg);
            }
        }
    } else {
        tracing::info!("Reranking disabled");
    }

    // Best-effort eager fingerprint check: the normal case wipes/records at
    // boot; an unreachable store just defers the gate to the first request.
    if let Err(e) = rag.check_fingerprint().await {
        tracing::warn!(
            "Could not verify the embedder fingerprint at startup ({e}); \
             will retry on first use — data operations fail until the vector store is reachable"
        );
    }

    Ok(Some((rag, reranker_error)))
}

/// Health + capability probe. The client hits this to decide which features to
/// light up (adr: additive surfaces appear only when the server is reachable).
/// `embedder: null` = unconfigured (adr/0024); `llm_provider: null` =
/// semantic-only (adr/0022). A degraded server (embedder configured but its
/// initialization failed at startup) reports `embedder: null` too — the
/// capability is genuinely absent — plus the error under `degraded`.
/// `reranker` likewise reports the *active* reranker, not the config: an
/// enabled reranker whose model download failed shows `false`, with the
/// reason under `reranker_error` (null when off by config or healthy).
async fn health_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> axum::Json<serde_json::Value> {
    let embedder = state
        .rag
        .as_ref()
        .and(state.config.embedder.as_ref())
        .map(|e| e.provider());
    axum::Json(serde_json::json!({
        "status": "ok",
        "reranker": state.rag.as_ref().is_some_and(|r| r.has_reranker()),
        "reranker_error": state.reranker_error,
        "embedder": embedder,
        "llm_provider": state.config.llm.as_ref().map(|l| l.provider()),
        "auth_required": state.config.auth.token.is_some(),
        "degraded": state.startup_error,
    }))
}
