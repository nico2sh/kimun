use axum::{
    Router,
    routing::{get, post},
};
use clap::Parser;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use kimun_rag::{
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
    /// Path to configuration file (default: ~/.config/kimun/rag.conf)
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,

    /// Host to bind to (overrides config)
    #[arg(long)]
    host: Option<String>,

    /// Port to bind to (overrides config)
    #[arg(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kimun_rag=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    // Load configuration (remembering the path so the web UI can persist edits).
    tracing::info!("Loading configuration...");
    let config_path = RagConfig::resolve_path(cli.config.clone());
    let config = RagConfig::load(cli.config)?;
    let config = config.merge_with_cli(cli.host, cli.port);

    tracing::info!("Configuration loaded successfully");
    tracing::debug!("Server: {}:{}", config.server.host, config.server.port);

    // Create RAG instance based on config
    tracing::info!("Initializing RAG system...");
    let rag = create_rag_from_config(&config).await?;
    tracing::info!("RAG system initialized");

    // Create application state
    let state = Arc::new(AppState::new(rag, config.clone()).with_config_path(config_path));

    // Periodically drop completed/old jobs so the tracker doesn't grow for the
    // life of the process.
    {
        let sweep = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                sweep.job_tracker.lock().await.cleanup_old_jobs();
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
            kimun_rag::auth::auth_middleware,
        ));

    let app = Router::new()
        .route("/health", get(health_handler))
        .merge(api)
        .merge(kimun_rag::webui::routes(state.clone()))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("RAG server listening on {}", addr);
    tracing::info!("Health check available at http://{}/health", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

/// Create RAG instance based on configuration
async fn create_rag_from_config(config: &RagConfig) -> anyhow::Result<KimunRag> {
    use kimun_rag::{
        config::{EmbedderConfig, VectorDbConfig},
        dbembeddings::{
            embedder::{Embedder, fastembedder::FastEmbedder, http::HttpEmbedder},
            veclance::VecLance,
            vecqdrant::VecQdrant,
        },
        llmclients::ChatClient,
    };

    // Build the embedder (shared by every collection on this server)
    let embedder: Arc<dyn Embedder> = match &config.embedder {
        EmbedderConfig::FastEmbed { model } => {
            tracing::info!(
                "Using local fastembed embedder (model: {})",
                model.as_deref().unwrap_or("default BGE-Large")
            );
            Arc::new(FastEmbedder::new(model.as_deref())?)
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
    let store: Arc<dyn kimun_rag::dbembeddings::VectorStore + Send + Sync> =
        match &config.vector_db {
            VectorDbConfig::Lance { path } => {
                tracing::info!("Using LanceDB vector database at {:?}", path);
                Arc::new(VecLance::new(path, embedder.dimension()).await?)
            }
            VectorDbConfig::Qdrant { url, collection } => {
                tracing::info!(
                    "Using Qdrant vector database at {} (collection: {})",
                    url,
                    collection
                );
                Arc::new(
                    VecQdrant::new(url.clone(), collection.clone(), embedder.dimension()).await?,
                )
            }
        };

    // Create LLM client based on config. `None` on a semantic-only server
    // (adr/0022). The key comes from config or the provider's env var and is
    // handed to the client directly — no env mutation, and a missing key is a
    // clean startup error, not a panic in the client.
    let llm_client: Option<Arc<dyn kimun_rag::llmclients::LLMClient + Send + Sync>> =
        match &config.llm {
            Some(llm) => {
                let api_key = llm
                    .api_key()
                    .map(str::to_string)
                    .or_else(|| std::env::var(llm.env_var()).ok())
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

    let mut rag = KimunRag::new(store, embedder, llm_client);

    // Enable reranking if configured
    if config.reranker.enabled {
        tracing::info!("Enabling reranking");
        rag = rag.with_reranking()?;
    } else {
        tracing::info!("Reranking disabled");
    }

    Ok(rag)
}

/// Health + capability probe. The client hits this to decide which features to
/// light up (adr: additive surfaces appear only when the server is reachable).
async fn health_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "reranker": state.config.reranker.enabled,
        "llm_provider": state.config.llm.as_ref().map(|l| l.provider()),
        "auth_required": state.config.auth.token.is_some(),
    }))
}
