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
        answer_handler, get_embeddings_handler, index_all_handler, index_docs_handler,
        index_single_handler, job_status_handler,
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

    /// Vault path (overrides config)
    #[arg(long)]
    vault_path: Option<std::path::PathBuf>,
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

    // Load configuration
    tracing::info!("Loading configuration...");
    let config = RagConfig::load(cli.config)?;
    let config = config.merge_with_cli(cli.host, cli.port, cli.vault_path);

    tracing::info!("Configuration loaded successfully");
    tracing::debug!("Server: {}:{}", config.server.host, config.server.port);
    tracing::debug!("Vault path: {:?}", config.vault.path);

    // Create RAG instance based on config
    tracing::info!("Initializing RAG system...");
    let rag = create_rag_from_config(&config).await?;

    // Initialize the RAG system
    rag.init().await?;
    tracing::info!("RAG system initialized");

    // Create application state
    let state = Arc::new(AppState::new(rag, config.clone()));

    // Build router
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/api/index/all", post(index_all_handler))
        .route("/api/index/single", post(index_single_handler))
        .route("/api/index/docs", post(index_docs_handler))
        .route("/api/embeddings", post(get_embeddings_handler))
        .route("/api/answer", post(answer_handler))
        .route("/api/job/{job_id}", get(job_status_handler))
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
        config::{LlmConfig, VectorDbConfig},
        dbembeddings::{vecqdrant::VecQdrant, vecsqlite::VecSQLite},
        llmclients::claude::ClaudeClient,
        llmclients::gemini::GeminiClient,
        llmclients::mistral::MistralClient,
        llmclients::openai::OpenAIClient,
    };

    // Create embeddings based on config
    let embeddings: Arc<dyn kimun_rag::dbembeddings::Embeddings + Send + Sync> =
        match &config.vector_db {
            VectorDbConfig::SQLite { db_path } => {
                tracing::info!("Using SQLite vector database at {:?}", db_path);
                Arc::new(VecSQLite::new(db_path))
            }
            VectorDbConfig::Qdrant { url, collection } => {
                tracing::info!(
                    "Using Qdrant vector database at {} (collection: {})",
                    url,
                    collection
                );
                Arc::new(VecQdrant::new(url.clone(), collection.clone()).await?)
            }
            // LanceDB temporarily disabled due to dependency issue
            // See veclancedb.rs for details
            VectorDbConfig::LanceDB { .. } => {
                anyhow::bail!(
                    "LanceDB is currently disabled due to a dependency compatibility issue. \
                     Please use Qdrant (recommended) or SQLite instead. \
                     See rag/src/dbembeddings/veclancedb.rs for more information."
                )
            }
        };

    // Create LLM client based on config
    let llm_client: Arc<dyn kimun_rag::llmclients::LLMClient + Send + Sync> = match &config.llm {
        LlmConfig::Gemini { model } => {
            tracing::info!("Using Gemini LLM with model: {}", model);
            Arc::new(GeminiClient::new(model))
        }
        LlmConfig::Mistral { model } => {
            tracing::info!("Using Mistral LLM");
            Arc::new(MistralClient::new(model))
        }
        LlmConfig::Claude { model } => {
            tracing::info!("Using Claude LLM with model: {}", model);
            Arc::new(ClaudeClient::new(model))
        }
        LlmConfig::OpenAI { model } => {
            tracing::info!("Using OpenAI LLM with model: {}", model);
            Arc::new(OpenAIClient::new(model))
        }
    };

    let mut rag = KimunRag::new(embeddings, llm_client);

    // Enable reranking if configured
    if config.reranker.enabled {
        tracing::info!("Enabling reranking");
        rag = rag.with_reranking()?;
    } else {
        tracing::info!("Reranking disabled");
    }

    Ok(rag)
}

/// Health check endpoint
async fn health_handler() -> &'static str {
    "OK"
}
