//! Startup wiring — everything `kimun-server` does between "config loaded"
//! and "listener bound": the config → **query pipeline** build, the route
//! table with its bearer gate, the `/health` capability probe, and the job
//! sweep. It is library code so each piece has tests (adr 0046); the binary
//! is the in-process restart loop of adr 0028 and nothing else.

use std::sync::Arc;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};

use crate::KimunRag;
use crate::config::{EmbedderConfig, LlmConfig, RagConfig, VectorDbConfig};
use crate::dbembeddings::VectorStore;
use crate::dbembeddings::embedder::{Embedder, fastembedder::FastEmbedder, http::HttpEmbedder};
use crate::dbembeddings::vecqdrant::VecQdrant;
use crate::dbembeddings::vecsqlite::VecSqlite;
use crate::handlers::{
    answer_handler, collection_hashes_handler, get_embeddings_handler, index_delete_handler,
    index_docs_handler, job_status_handler,
};
use crate::llmclients::{ChatClient, LLMClient};
use crate::server_state::AppState;

/// What building the pipeline from a config yields — the three facts
/// `AppState` carries about the server's state:
///
/// - `rag: Some` — configured; semantic-only or full by
///   [`KimunRag::can_answer`].
/// - `rag: None`, `startup_error: None` — *unconfigured*: no `[embedder]`.
/// - `rag: None`, `startup_error: Some` — *degraded*: an embedder is
///   configured but the build failed (model download, unreachable endpoint,
///   missing LLM key). Behaves like unconfigured, with a visible cause.
///
/// `reranker_error` is why the (non-fatal) reranker failed to initialize, if
/// it did, so `/health`'s `reranker: false` is distinguishable from "off by
/// config".
pub struct Parts {
    pub rag: Option<KimunRag>,
    pub startup_error: Option<String>,
    pub reranker_error: Option<String>,
}

/// Builds the query pipeline from config. Never fails: a build error
/// (embedding model download failed, bad LLM key, …) does not abort startup —
/// the server comes up *degraded*, with the same 503-everything behaviour as
/// unconfigured, so the web UI stays reachable to show the error and fix the
/// config.
pub async fn build(config: &RagConfig) -> Parts {
    build_with(config, build_embedder).await
}

/// [`build`] with the embedder factory injected. The embedder is the one part
/// of the pipeline that must reach the network (or download a model) to come
/// up; tests pass a factory yielding a fake and drive everything else — store,
/// key gate, reranker, fingerprint, the `Parts` wrapping — for real.
pub(crate) async fn build_with(
    config: &RagConfig,
    make_embedder: impl AsyncFnOnce(&EmbedderConfig) -> anyhow::Result<Arc<dyn Embedder>>,
) -> Parts {
    tracing::info!("Initializing RAG system...");
    // No embedder → unconfigured: no vector store either (its dimension comes
    // from the embedder), so there is nothing to build.
    let Some(embedder_cfg) = &config.embedder else {
        tracing::warn!(
            "No embedder configured — server is UNCONFIGURED: indexing and search are disabled. \
             Open http://{}:{}/config to set up an embedder (and optionally an LLM).",
            config.server.host,
            config.server.port
        );
        return Parts {
            rag: None,
            startup_error: None,
            reranker_error: None,
        };
    };

    let built = async {
        let embedder = make_embedder(embedder_cfg).await?;
        assemble(config, embedder_cfg, embedder).await
    }
    .await;
    match built {
        Ok((rag, reranker_error)) => {
            tracing::info!("RAG system initialized");
            Parts {
                rag: Some(rag),
                startup_error: None,
                reranker_error,
            }
        }
        Err(e) => {
            let msg = format!("{e:#}");
            tracing::error!(
                "RAG initialization failed: {msg} — starting DEGRADED: indexing and search are \
                 disabled. Check http://{}:{}/logs and fix the config at /config, then restart.",
                config.server.host,
                config.server.port
            );
            Parts {
                rag: None,
                startup_error: Some(msg),
                reranker_error: None,
            }
        }
    }
}

/// The embedder, shared by every collection on this server — the factory
/// [`build`] hands to [`build_with`].
async fn build_embedder(cfg: &EmbedderConfig) -> anyhow::Result<Arc<dyn Embedder>> {
    let embedder: Arc<dyn Embedder> = match cfg {
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
    Ok(embedder)
}

/// Everything past the embedder, given one: the vector store at the
/// embedder's width, the LLM client behind the key gate, the context-cut
/// settings, the reranker (non-fatal), and the best-effort eager fingerprint
/// check. Returns the pipeline and why the reranker failed to initialize, if
/// it did. `embedder_cfg` is the `[embedder]` section `embedder` was built
/// from; it names the fingerprint.
async fn assemble(
    config: &RagConfig,
    embedder_cfg: &EmbedderConfig,
    embedder: Arc<dyn Embedder>,
) -> anyhow::Result<(KimunRag, Option<String>)> {
    // The store only needs the embedder's dimension (its tables/collections
    // are created at that width) — embedding itself happens in the pipeline,
    // above the storage seam.
    let store: Arc<dyn VectorStore + Send + Sync> = match &config.vector_db {
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

    // Embedder fingerprint: a changed embedder makes every stored vector
    // garbage, and reconciliation can't detect it. The gate is armed on the
    // pipeline (every data op verifies before touching the store) rather than
    // enforced here, so a store that is unreachable at boot (e.g. Qdrant
    // still starting) degrades to failing requests instead of aborting
    // startup.
    let fingerprint = embedder_cfg.fingerprint(embedder.dimension());

    // `None` on a semantic-only server. The key is handed to the client
    // directly — no env mutation.
    let llm_client: Option<Arc<dyn LLMClient + Send + Sync>> = match &config.llm {
        Some(llm) => {
            let api_key = resolve_api_key(llm, |var| std::env::var(var).ok())?;
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
        .with_context_cut(config.reranker.context_cut)
        .with_score_range_cutoff(config.reranker.score_range_cutoff)
        .with_drop_window(
            config.reranker.drop_window_min,
            config.reranker.drop_window_max,
        );

    // Reranker initialization failure (typically: the cross-encoder model
    // download failed — offline, proxy — or an unreachable rerank endpoint)
    // is non-fatal: the server serves with plain vector ranking and reports
    // the reason via /health.
    let mut reranker_error = None;
    if config.reranker.enabled {
        match crate::reranker::from_config(&config.reranker).await {
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

    Ok((rag, reranker_error))
}

/// The LLM key gate: the config's key, else the provider's env var (read
/// through `env`, so tests need not touch the process environment), else —
/// for a custom endpoint (openai-local: Ollama, llama.cpp, …), which is
/// typically keyless — an empty bearer. A cloud provider with no key anywhere
/// is a clean startup error, not a panic in the client.
fn resolve_api_key(
    llm: &LlmConfig,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<String> {
    llm.api_key()
        .map(str::to_string)
        .or_else(|| env(llm.env_var()))
        .or_else(|| llm.url().map(|_| String::new()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Missing API key: set [llm] api_key in config or export {}",
                llm.env_var()
            )
        })
}

/// The route table over a built state: `/health` (always open, for liveness
/// probes and the client's capability probe), the `/api` data routes behind
/// the bearer token when one is configured, and the web UI.
pub fn router(state: Arc<AppState>) -> Router {
    if state.config.auth.token.is_some() {
        tracing::info!("Bearer-token auth enabled on /api routes");
    } else if !state.config.server.binds_loopback() {
        tracing::warn!(
            "No [auth] token set and bound to {} — the API is OPEN to the network",
            state.config.server.host
        );
    }

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
            crate::auth::auth_middleware,
        ));

    Router::new()
        .route("/health", get(health_handler))
        .merge(api)
        .merge(crate::webui::routes(state.clone()))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// Health + capability probe. The client hits this to decide which features
/// to light up (adr: additive surfaces appear only when the server is
/// reachable). `embedder: null` = unconfigured; `llm_provider: null` =
/// semantic-only. A degraded server (embedder configured but its
/// initialization failed at startup) reports `embedder: null` too — the
/// capability is genuinely absent — plus the error under `degraded`.
/// `reranker` likewise reports the *active* reranker, not the config: an
/// enabled reranker whose model download failed shows `false`, with the
/// reason under `reranker_error` (null when off by config or healthy).
pub async fn health_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let embedder = state
        .rag
        .as_ref()
        .and(state.config.embedder.as_ref())
        .map(|e| e.provider());
    Json(serde_json::json!({
        "status": "ok",
        "reranker": state.rag.as_ref().is_some_and(|r| r.has_reranker()),
        "reranker_error": state.reranker_error,
        "embedder": embedder,
        "llm_provider": state.config.llm.as_ref().map(|l| l.provider()),
        "auth_required": state.config.auth.token.is_some(),
        "degraded": state.startup_error,
    }))
}

/// Periodically drops completed/old jobs so the tracker doesn't grow for the
/// life of the server. Holds only a `Weak`: when a restart drops this
/// iteration's state, the sweep ends instead of pinning the old pipeline
/// (embedder model included) in memory forever (adr 0028).
pub fn spawn_job_sweep(state: &Arc<AppState>) {
    let sweep = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let Some(state) = sweep.upgrade() else { break };
            state.job_tracker.lock().await.cleanup_old_jobs();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeEmbedder, FakeLlm, FakeVectorStore};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode, header::AUTHORIZATION};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    const UNCONFIGURED: &str = "[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\n";
    // `[reranker]` defaults to enabled with the local cross-encoder, whose
    // model would be downloaded at assemble time — off in every test config.
    const SEMANTIC_ONLY: &str = "[server]\n[vector_db]\ntype = \"sqlite\"\n[embedder]\ntype = \"fastembed\"\n[reranker]\nenabled = false\n";
    const FULL: &str = "[server]\n[vector_db]\ntype = \"sqlite\"\n[embedder]\ntype = \"fastembed\"\n[llm]\nprovider = \"gemini\"\napi_key = \"k\"\n[reranker]\nenabled = false\n";

    fn config(toml: &str) -> RagConfig {
        toml::from_str(toml).unwrap()
    }

    // ------------------------------------------------------------------
    // build: config → parts
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn no_embedder_builds_an_unconfigured_server() {
        let parts = build(&config(UNCONFIGURED)).await;
        assert!(parts.rag.is_none());
        assert!(
            parts.startup_error.is_none(),
            "unconfigured is not an error"
        );
        assert!(parts.reranker_error.is_none());
    }

    #[tokio::test]
    async fn an_embedder_that_cannot_come_up_degrades_instead_of_aborting() {
        let parts = build_with(&config(SEMANTIC_ONLY), async |_| {
            anyhow::bail!("model download failed: connection refused")
        })
        .await;
        assert!(parts.rag.is_none());
        assert_eq!(
            parts.startup_error.as_deref(),
            Some("model download failed: connection refused"),
            "degraded carries the cause verbatim"
        );
        assert!(parts.reranker_error.is_none());
    }

    /// `build` with the fake embedder and an embedded store in `dir`.
    async fn built(toml: &str, dir: &std::path::Path) -> Parts {
        let mut cfg = config(toml);
        cfg.vector_db = VectorDbConfig::Sqlite {
            path: dir.join("vectors"),
        };
        build_with(&cfg, async |_| {
            Ok(Arc::new(FakeEmbedder) as Arc<dyn Embedder>)
        })
        .await
    }

    #[tokio::test]
    async fn embedder_without_llm_is_semantic_only() {
        let dir = tempfile::tempdir().unwrap();
        let parts = built(SEMANTIC_ONLY, dir.path()).await;
        let rag = parts.rag.expect("configured");
        assert!(!rag.can_answer());
        assert!(!rag.has_reranker(), "reranker is off by config");
        assert!(parts.startup_error.is_none());
        assert!(parts.reranker_error.is_none());
    }

    #[tokio::test]
    async fn embedder_and_keyed_llm_is_full() {
        let dir = tempfile::tempdir().unwrap();
        let parts = built(FULL, dir.path()).await;
        assert!(parts.rag.expect("configured").can_answer());
    }

    #[tokio::test]
    async fn the_fingerprint_is_recorded_at_boot() {
        let dir = tempfile::tempdir().unwrap();
        built(SEMANTIC_ONLY, dir.path())
            .await
            .rag
            .expect("configured");
        let store = VecSqlite::new(dir.path().join("vectors"), FakeEmbedder.dimension())
            .await
            .unwrap();
        assert_eq!(
            store.read_fingerprint().await.unwrap().as_deref(),
            Some("fastembed:default:8"),
            "provider:model:dimension of the configured embedder"
        );
    }

    #[tokio::test]
    async fn a_reranker_that_cannot_come_up_is_non_fatal() {
        let dir = tempfile::tempdir().unwrap();
        // An HTTP reranker at an unparseable URL: its startup probe fails on
        // the URL itself, before any network — asserted below so a future
        // URL normalization can't silently turn this into a real request.
        let parts = built(
            "[server]\n[vector_db]\ntype = \"sqlite\"\n[embedder]\ntype = \"fastembed\"\n[reranker]\nenabled = true\ntype = \"http\"\nurl = \"not a url\"\n",
            dir.path(),
        )
        .await;
        let rag = parts.rag.expect("the pipeline still builds");
        assert!(!rag.has_reranker());
        assert!(parts.startup_error.is_none());
        let why = parts
            .reranker_error
            .expect("/health must be able to say why reranker is false");
        assert!(
            why.contains("relative URL"),
            "URL parse error, not a request: {why}"
        );
    }

    #[tokio::test]
    async fn an_unwritable_store_path_degrades_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, b"").unwrap();
        // A directory cannot be created under a regular file.
        let parts = built(SEMANTIC_ONLY, &file).await;
        assert!(parts.rag.is_none());
        assert!(
            parts.startup_error.is_some(),
            "store creation failure must surface"
        );
    }

    #[test]
    fn the_llm_key_gate_prefers_config_then_env_then_keyless_endpoint() {
        let gemini_keyed = LlmConfig::Gemini {
            model: "m".into(),
            api_key: Some("cfg-key".into()),
        };
        let gemini_bare = LlmConfig::Gemini {
            model: "m".into(),
            api_key: None,
        };
        let openai_local = LlmConfig::OpenAI {
            model: "m".into(),
            api_key: None,
            url: Some("http://localhost:11434/v1".into()),
        };
        let openai_cloud = LlmConfig::OpenAI {
            model: "m".into(),
            api_key: None,
            url: None,
        };
        let no_env = |_: &str| None;
        let env = |var: &str| (var == "GEMINI_API_KEY").then(|| "env-key".to_string());

        assert_eq!(resolve_api_key(&gemini_keyed, no_env).unwrap(), "cfg-key");
        assert_eq!(
            resolve_api_key(&gemini_keyed, env).unwrap(),
            "cfg-key",
            "config wins over the environment"
        );
        assert_eq!(resolve_api_key(&gemini_bare, env).unwrap(), "env-key");
        let missing = resolve_api_key(&gemini_bare, no_env).unwrap_err();
        assert!(
            missing.to_string().contains("GEMINI_API_KEY"),
            "the error names the env var: {missing}"
        );
        assert_eq!(
            resolve_api_key(&openai_local, no_env).unwrap(),
            "",
            "a custom endpoint is keyless"
        );
        assert!(
            resolve_api_key(&openai_cloud, no_env).is_err(),
            "the cloud provider stays gated"
        );
    }

    // ------------------------------------------------------------------
    // router: /health and the bearer gate
    // ------------------------------------------------------------------

    fn fake_rag(with_llm: bool) -> KimunRag {
        KimunRag::new(
            Arc::new(FakeVectorStore::default()),
            Arc::new(FakeEmbedder),
            with_llm.then(|| Arc::new(FakeLlm) as Arc<dyn LLMClient + Send + Sync>),
        )
    }

    async fn get(app: Router, path: &str, bearer: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::get(path);
        if let Some(token) = bearer {
            req = req.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn health_reports_each_server_state() {
        let degraded = AppState::new(None, config(SEMANTIC_ONLY))
            .with_startup_error(Some("model download failed".into()));
        let semantic_only = AppState::new(Some(fake_rag(false)), config(SEMANTIC_ONLY))
            .with_reranker_error(Some("rerank endpoint unreachable".into()));
        let table = [
            (
                "unconfigured",
                AppState::new(None, config(UNCONFIGURED)),
                json!({
                    "status": "ok",
                    "reranker": false,
                    "reranker_error": null,
                    "embedder": null,
                    "llm_provider": null,
                    "auth_required": false,
                    "degraded": null,
                }),
            ),
            (
                "degraded",
                degraded,
                json!({
                    "status": "ok",
                    "reranker": false,
                    "reranker_error": null,
                    "embedder": null,
                    "llm_provider": null,
                    "auth_required": false,
                    "degraded": "model download failed",
                }),
            ),
            (
                "semantic-only",
                semantic_only,
                json!({
                    "status": "ok",
                    "reranker": false,
                    "reranker_error": "rerank endpoint unreachable",
                    "embedder": "fastembed",
                    "llm_provider": null,
                    "auth_required": false,
                    "degraded": null,
                }),
            ),
            (
                "full",
                AppState::new(Some(fake_rag(true)), config(FULL)),
                json!({
                    "status": "ok",
                    "reranker": false,
                    "reranker_error": null,
                    "embedder": "fastembed",
                    "llm_provider": "gemini",
                    "auth_required": false,
                    "degraded": null,
                }),
            ),
        ];
        for (name, state, expected) in table {
            let (status, body) = get(router(Arc::new(state)), "/health", None).await;
            assert_eq!(status, StatusCode::OK, "{name}");
            assert_eq!(body, expected, "{name}");
        }
    }

    #[tokio::test]
    async fn bearer_gates_the_api_routes_only() {
        let state = Arc::new(AppState::new(
            None,
            config(
                "[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\n[auth]\ntoken = \"secret\"\n",
            ),
        ));
        let job = format!("/api/job/{}", uuid::Uuid::new_v4());

        let (status, _) = get(router(state.clone()), &job, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "no token");
        let (status, _) = get(router(state.clone()), &job, Some("nope")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "wrong token");
        let (status, body) = get(router(state.clone()), &job, Some("secret")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|e| e.contains("not found")),
            "right token reaches the handler (unknown job), not the route fallback: {body}"
        );

        let (status, body) = get(router(state.clone()), "/health", None).await;
        assert_eq!(status, StatusCode::OK, "/health is never gated");
        assert_eq!(body["auth_required"], true);

        let (status, _) = get(router(state), "/login", None).await;
        assert_eq!(status, StatusCode::OK, "the web UI is merged in");
    }

    #[tokio::test]
    async fn without_a_token_the_api_is_open() {
        let state = Arc::new(AppState::new(None, config(UNCONFIGURED)));
        let job = format!("/api/job/{}", uuid::Uuid::new_v4());
        let (status, body) = get(router(state), &job, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|e| e.contains("not found")),
            "the handler answered, not the route fallback: {body}"
        );
    }
}
