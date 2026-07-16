use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
    pub server: ServerConfig,
    pub vector_db: VectorDbConfig,
    /// Which embedder produces the vectors. Optional: with no `[embedder]`
    /// section the server is *unconfigured* — it boots, serves the web UI and
    /// `/health`, and rejects every data operation until an embedder is chosen
    /// (adr/0024). There is deliberately no silent default. A first-run
    /// generated config omits it.
    #[serde(default)]
    pub embedder: Option<EmbedderConfig>,
    /// The LLM used for question-answering. Optional: with no `[llm]` section the
    /// server is *semantic-only* — it answers `/api/embeddings` searches but
    /// rejects `/api/answer` (adr/0022). A first-run generated config omits it.
    #[serde(default)]
    pub llm: Option<LlmConfig>,
    pub reranker: RerankerConfig,
    /// Authentication. Optional so a localhost-only dev server needs no setup;
    /// required in practice once the server binds beyond 127.0.0.1 (adr:
    /// shared bearer token).
    #[serde(default)]
    pub auth: AuthConfig,
}

/// Bearer-token auth. When `token` is set, every API request must present it as
/// `Authorization: Bearer <token>`, and the web UI requires it at sign-in.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,
}

/// Selects the embedder. All collections on a server share one embedder — the
/// same model must embed both documents and queries, so it is an invariant of
/// the stored vectors. Changing it invalidates existing embeddings and forces a
/// full re-index. `doc_prefix`/`query_prefix` are model-specific instruction
/// prefixes (e.g. nomic's `search_document: ` / `search_query: `); leave empty
/// when the model needs none.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EmbedderConfig {
    /// Local, in-process fastembed. No network. `model` names a bundled model
    /// by its fastembed variant (e.g. `BGESmallENV15`) or model code (e.g.
    /// `Xenova/bge-small-en-v1.5`); omit for the default (BGE-Large, 1024 dims).
    #[serde(rename = "fastembed")]
    FastEmbed {
        #[serde(default)]
        model: Option<String>,
    },
    /// An Ollama server's `/api/embed` endpoint.
    #[serde(rename = "ollama")]
    Ollama {
        url: String,
        model: String,
        #[serde(default)]
        doc_prefix: String,
        #[serde(default)]
        query_prefix: String,
    },
    /// Any OpenAI-compatible `/embeddings` endpoint.
    #[serde(rename = "openai")]
    OpenAI {
        url: String,
        model: String,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        doc_prefix: String,
        #[serde(default)]
        query_prefix: String,
    },
}

impl EmbedderConfig {
    /// Short provider id (`fastembed` | `ollama` | `openai`) — used by the web
    /// UI form and the `/health` capability probe (adr/0024).
    pub fn provider(&self) -> &'static str {
        match self {
            EmbedderConfig::FastEmbed { .. } => "fastembed",
            EmbedderConfig::Ollama { .. } => "ollama",
            EmbedderConfig::OpenAI { .. } => "openai",
        }
    }

    /// The embedder fingerprint recorded next to stored vectors: provider,
    /// model (lowercased), and vector dimension. Stored vectors are only
    /// comparable to queries embedded by the same model, and reconciliation
    /// cannot see a model swap (note hashes don't change) — on a fingerprint
    /// mismatch at startup the server wipes all collections (adr/0025).
    pub fn fingerprint(&self, dimension: usize) -> String {
        let model = match self {
            EmbedderConfig::FastEmbed { model } => {
                model.as_deref().unwrap_or("default").to_lowercase()
            }
            EmbedderConfig::Ollama { model, .. } | EmbedderConfig::OpenAI { model, .. } => {
                model.to_lowercase()
            }
        };
        format!("{}:{}:{}", self.provider(), model, dimension)
    }
}

/// Vector store selection. `sqlite` is embedded (local, file-backed, no
/// server); `qdrant` talks to a standalone server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum VectorDbConfig {
    /// Embedded SQLite store. `path` is a local directory holding the
    /// database file (one collection per vault inside it). `lance` is
    /// accepted as a legacy alias: the replaced LanceDB backend's data is
    /// unreadable either way, so an old config boots an empty store and the
    /// clients' next reconciliation re-pushes everything.
    #[serde(rename = "sqlite", alias = "lance")]
    Sqlite {
        #[serde(default = "default_sqlite_path")]
        path: PathBuf,
    },
    #[serde(rename = "qdrant")]
    Qdrant {
        #[serde(default = "default_qdrant_url")]
        url: String,
        #[serde(default = "default_qdrant_collection")]
        collection: String,
    },
}

/// LLM provider selection plus its model and API key. The key lives here
/// (server-owned config, editable in the web UI) rather than only in an env var,
/// so Kimün never handles it (adr: server-owned LLM config). An absent `api_key`
/// falls back to the provider's env var.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum LlmConfig {
    #[serde(rename = "gemini")]
    Gemini {
        #[serde(default = "default_gemini_model")]
        model: String,
        #[serde(default)]
        api_key: Option<String>,
    },
    #[serde(rename = "mistral")]
    Mistral {
        #[serde(default = "default_mistral_model")]
        model: String,
        #[serde(default)]
        api_key: Option<String>,
    },
    #[serde(rename = "claude")]
    Claude {
        #[serde(default = "default_claude_model")]
        model: String,
        #[serde(default)]
        api_key: Option<String>,
    },
    #[serde(rename = "openai")]
    OpenAI {
        #[serde(default = "default_openai_model")]
        model: String,
        #[serde(default)]
        api_key: Option<String>,
        /// Optional OpenAI-compatible endpoint (Ollama, llama.cpp, OpenRouter…);
        /// defaults to api.openai.com when absent.
        #[serde(default)]
        url: Option<String>,
    },
}

impl LlmConfig {
    /// Short provider id (`gemini` | `mistral` | `claude` | `openai`) — used by
    /// the web UI form and the `/health` probe.
    pub fn provider(&self) -> &'static str {
        match self {
            LlmConfig::Gemini { .. } => "gemini",
            LlmConfig::Mistral { .. } => "mistral",
            LlmConfig::Claude { .. } => "claude",
            LlmConfig::OpenAI { .. } => "openai",
        }
    }

    /// The configured model name.
    pub fn model(&self) -> &str {
        match self {
            LlmConfig::Gemini { model, .. }
            | LlmConfig::Mistral { model, .. }
            | LlmConfig::Claude { model, .. }
            | LlmConfig::OpenAI { model, .. } => model,
        }
    }

    /// The environment variable the provider's client reads its API key from.
    /// Single source of truth for the provider→env-var mapping, shared by the
    /// startup key gate and the web UI's save path.
    pub fn env_var(&self) -> &'static str {
        match self {
            LlmConfig::Gemini { .. } => "GEMINI_API_KEY",
            LlmConfig::Mistral { .. } => "MISTRAL_API_KEY",
            LlmConfig::Claude { .. } => "ANTHROPIC_API_KEY",
            LlmConfig::OpenAI { .. } => "OPENAI_API_KEY",
        }
    }

    /// The configured API key, if any.
    pub fn api_key(&self) -> Option<&str> {
        match self {
            LlmConfig::Gemini { api_key, .. }
            | LlmConfig::Mistral { api_key, .. }
            | LlmConfig::Claude { api_key, .. }
            | LlmConfig::OpenAI { api_key, .. } => api_key.as_deref(),
        }
    }

    /// The configured endpoint override, if any (OpenAI provider only).
    pub fn url(&self) -> Option<&str> {
        match self {
            LlmConfig::OpenAI { url, .. } => url.as_deref(),
            _ => None,
        }
    }

    /// The id the web form uses for this config. Distinguishes the cloud
    /// `openai` provider from `openai-local` (the same OpenAI wire pointed at a
    /// user-supplied endpoint — Ollama, llama.cpp, …), so the form can
    /// pre-select the right option and expose the URL field. Both ids map back
    /// to [`LlmConfig::OpenAI`] on save.
    pub fn form_id(&self) -> &'static str {
        match self {
            LlmConfig::OpenAI { url: Some(_), .. } => "openai-local",
            other => other.provider(),
        }
    }

    /// Builds a config from web-form parts, defaulting the model per provider
    /// when blank. Unknown provider ids are rejected. `url` applies to the
    /// openai provider only (the endpoint override); other providers ignore it.
    pub fn from_parts(
        provider: &str,
        model: Option<String>,
        api_key: Option<String>,
        url: Option<String>,
    ) -> anyhow::Result<Self> {
        let key = api_key.filter(|k| !k.is_empty());
        Ok(match provider {
            "gemini" => LlmConfig::Gemini {
                model: model
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(default_gemini_model),
                api_key: key,
            },
            "mistral" => LlmConfig::Mistral {
                model: model
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(default_mistral_model),
                api_key: key,
            },
            "claude" => LlmConfig::Claude {
                model: model
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(default_claude_model),
                api_key: key,
            },
            "openai" => LlmConfig::OpenAI {
                model: model
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(default_openai_model),
                api_key: key,
                url: url.filter(|u| !u.is_empty()),
            },
            other => anyhow::bail!("unknown LLM provider: {other}"),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankerConfig {
    #[serde(default = "default_reranker_enabled")]
    pub enabled: bool,
    /// The `fixed` context cut's size: search notes / answer chunks,
    /// overridable per request via `context_size`. Ignored by the adaptive
    /// cuts (`score-range`, `largest-drop`) — there the pool's score shape
    /// decides (adr/0029).
    #[serde(default = "default_reranker_top_k")]
    pub top_k: usize,
    /// How reranking runs: the local fastembed cross-encoder (default, model
    /// downloaded on first start) or an external HTTP rerank endpoint. The
    /// provider fields below are file-only knobs — the web UI form edits only
    /// `enabled`/`top_k` and carries these unchanged (like embedder prefixes).
    #[serde(default, rename = "type")]
    pub provider: RerankerProvider,
    /// Base URL for `type = "http"`, including the API version where the
    /// provider has one (e.g. `https://api.cohere.com/v2`); `/rerank` is
    /// appended. Required for `http`, ignored for `fastembed`.
    #[serde(default)]
    pub url: Option<String>,
    /// Model name for `type = "http"` (e.g. `rerank-v3.5`). Optional — some
    /// self-hosted servers serve a single model and don't need it.
    #[serde(default)]
    pub model: Option<String>,
    /// Bearer token for `type = "http"` endpoints that need one.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Which **context cut** sizes both retrieval surfaces (adr/0029):
    /// `search` shows the notes whose best chunk survives it, `answer` feeds
    /// the chunks that survive it — with or without a reranker. Editable from
    /// the web UI Config page.
    #[serde(default)]
    pub context_cut: ContextCut,
    /// The score-range cut's normalized cutoff in `0.0..=1.0` (default 0.4):
    /// chunks at or above this fraction of the pool's (percentile-measured)
    /// score range survive. Ignored by the other cuts; values outside the
    /// range are clamped.
    #[serde(default = "default_score_range_cutoff")]
    pub score_range_cutoff: f64,
    /// The largest-drop cut's search window: the gap is looked for at note
    /// positions `drop_window_min..=drop_window_max` (defaults 3 and 30).
    /// Ignored by the other cuts; sanitized to `min ≥ 1` and `max ≥ min`.
    #[serde(default = "default_drop_window_min")]
    pub drop_window_min: usize,
    #[serde(default = "default_drop_window_max")]
    pub drop_window_max: usize,
}

/// The context cut algorithm — how many retrieved chunks/notes each query
/// surface returns, applied with or without a reranker (adr/0029). `fixed`
/// counts; the other two read the pool's score shape (adr/0027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextCut {
    /// Exactly `top_k` results — search notes / answer chunks — the classic
    /// count cut.
    Fixed,
    /// Min-max normalize the pool's scores; chunks at or above
    /// `score_range_cutoff` (default 0.4) of the range survive. The range is
    /// measured between the pool's 5th/95th score percentiles so outlier
    /// chunks can't stretch it.
    #[default]
    ScoreRange,
    /// Cut at the biggest relative drop between consecutive NOTE scores —
    /// each distinct note's best chunk, `(s[i] − s[i+1]) / s[i]` — searched
    /// within the `drop_window_min..=drop_window_max` note positions; every
    /// chunk at or above the gap-closing note's score is kept. No drop found
    /// means no cut.
    LargestDrop,
}

impl ContextCut {
    /// The config-file name of the algorithm (`fixed` | `score-range` |
    /// `largest-drop`) — the web UI shows it next to the cut preview.
    pub fn label(&self) -> &'static str {
        match self {
            ContextCut::Fixed => "fixed",
            ContextCut::ScoreRange => "score-range",
            ContextCut::LargestDrop => "largest-drop",
        }
    }
}

/// Selects the reranker backend. Unlike the embedder this is not an invariant
/// of the stored vectors — switching rerankers never invalidates the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RerankerProvider {
    /// Local fastembed cross-encoder (BGE-Reranker-Base), no network at query
    /// time but downloads the model from Hugging Face on first start.
    #[default]
    FastEmbed,
    /// A Cohere/Jina-compatible `POST {url}/rerank` endpoint — covers Cohere,
    /// Jina AI, Voyage AI, and self-hosted vLLM or Infinity.
    Http,
}

impl RerankerProvider {
    /// Human-readable backend label — the startup log and the web dashboard
    /// share it (same pattern as [`EmbedderConfig::provider`]).
    pub fn label(&self) -> &'static str {
        match self {
            RerankerProvider::FastEmbed => "local cross-encoder",
            RerankerProvider::Http => "http",
        }
    }
}

// Default value functions
fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    7573
}

fn default_max_concurrent_jobs() -> usize {
    10
}

fn default_sqlite_path() -> PathBuf {
    PathBuf::from("./rag_sqlite")
}

/// SQLite store path baked into a *generated* first-run config: an absolute
/// path under the OS data dir (`~/.local/share/kimun/rag_sqlite`) so the store
/// does not float with the process's launch directory the way `./rag_sqlite`
/// would (adr/0022). Falls back to the relative default if the data dir can't be
/// resolved.
fn generated_sqlite_path() -> PathBuf {
    match dirs::data_dir() {
        Some(mut dir) => {
            dir.push("kimun");
            dir.push("rag_sqlite");
            dir
        }
        None => default_sqlite_path(),
    }
}

fn default_qdrant_url() -> String {
    "http://localhost:6333".to_string()
}

fn default_qdrant_collection() -> String {
    "kimun_embeddings".to_string()
}

fn default_gemini_model() -> String {
    "gemini-2.5-flash-preview-04-17".to_string()
}

fn default_mistral_model() -> String {
    "mistral-large-latest".to_string()
}

fn default_claude_model() -> String {
    "claude-3-5-sonnet-20241022".to_string()
}

fn default_openai_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_reranker_enabled() -> bool {
    true
}

fn default_reranker_top_k() -> usize {
    20
}

fn default_score_range_cutoff() -> f64 {
    0.4
}

fn default_drop_window_min() -> usize {
    3
}

fn default_drop_window_max() -> usize {
    30
}

/// A zero-config default: SQLite under the data dir, default `127.0.0.1`
/// bind, no embedder (unconfigured, adr/0024), no LLM, no auth. This is what a
/// first run with no config file boots from and writes to disk (adr/0022).
impl Default for RagConfig {
    fn default() -> Self {
        RagConfig {
            server: ServerConfig {
                host: default_host(),
                port: default_port(),
                max_concurrent_jobs: default_max_concurrent_jobs(),
            },
            vector_db: VectorDbConfig::Sqlite {
                path: generated_sqlite_path(),
            },
            embedder: None,
            llm: None,
            reranker: RerankerConfig {
                enabled: default_reranker_enabled(),
                top_k: default_reranker_top_k(),
                provider: RerankerProvider::default(),
                url: None,
                model: None,
                api_key: None,
                context_cut: ContextCut::default(),
                score_range_cutoff: default_score_range_cutoff(),
                drop_window_min: default_drop_window_min(),
                drop_window_max: default_drop_window_max(),
            },
            auth: AuthConfig::default(),
        }
    }
}

impl RagConfig {
    /// A ready-to-run config for `--default-config`: embedded SQLite plus the
    /// local fastembed embedder with its default model. Unlike [`Default`] —
    /// which is *unconfigured* (no embedder, data operations rejected,
    /// adr/0024) — this serves indexing and search out of the box. Still
    /// semantic-only: no LLM, so `/api/answer` is rejected (adr/0022).
    pub fn ready_default() -> Self {
        Self {
            embedder: Some(EmbedderConfig::FastEmbed { model: None }),
            ..Self::default()
        }
    }

    /// [`ready_default`](Self::ready_default), materialized at `path` when no
    /// file exists there yet — `--default-config` leaves a real file behind so
    /// later file-based starts (and web-UI edits) have something to load and
    /// update. An existing file is never touched; the returned config is the
    /// built-in default either way (the flag means "run with defaults", not
    /// "load the file").
    pub fn ready_default_persisted(path: &std::path::Path) -> anyhow::Result<Self> {
        let config = Self::ready_default();
        if !path.exists() {
            config.save_to(path)?;
            tracing::info!("Wrote default config (SQLite + fastembed) to {:?}", path);
        }
        Ok(config)
    }
}

/// The web UI's config form, exactly as submitted. Numeric fields arrive as
/// strings so a non-numeric value yields a friendly flash instead of a bare
/// 400 that discards the whole form. Exposing a new option in the web UI means
/// a field here, a rule in [`RagConfig::apply_form`], and an input in the
/// form's markup — nothing else.
#[derive(Debug, Deserialize)]
pub struct ConfigForm {
    pub host: String,
    pub port: String,
    pub provider: String,
    pub model: String,
    pub api_key: String,
    /// Endpoint URL for the `openai-local` provider; ignored otherwise.
    /// Defaulted so form posts predating the field still deserialize.
    #[serde(default)]
    pub llm_url: String,
    /// `none` | `fastembed` | `ollama` | `openai` — `none` clears the embedder
    /// (unconfigured server, adr/0024).
    pub embedder_provider: String,
    pub fastembed_model: String,
    pub embedder_url: String,
    pub embedder_model: String,
    pub embedder_api_key: String,
    /// `sqlite` | `qdrant`.
    pub vector_db: String,
    /// Aliased so form posts predating the SQLite rename still deserialize.
    #[serde(alias = "lance_path")]
    pub sqlite_path: String,
    pub qdrant_url: String,
    pub qdrant_collection: String,
    #[serde(default)]
    pub reranker_enabled: Option<String>,
    pub reranker_top_k: String,
    /// `fixed` | `score-range` | `largest-drop`. Defaulted (empty = keep the
    /// current setting) so form posts predating the field still deserialize.
    #[serde(default)]
    pub context_cut: String,
    /// Strategy knobs; blank keeps the current value (fields hidden for a
    /// non-selected strategy still post, so blanks only come from stale
    /// forms predating them).
    #[serde(default)]
    pub score_range_cutoff: String,
    #[serde(default)]
    pub drop_window_min: String,
    #[serde(default)]
    pub drop_window_max: String,
    pub auth_token: String,
}

impl RagConfig {
    /// Applies a submitted web form onto this (current) config, producing the
    /// config to persist. Every form→config rule lives here, not in the web
    /// layer: numeric parsing, the `none` sentinels clearing the LLM (adr/0022)
    /// and the embedder (adr/0024), the vector-db selection, and the carry
    /// rules for values the form can't or doesn't resend — a blank secret keeps
    /// the current one, and provider-scoped values (API keys, endpoint url,
    /// embedder prefixes) carry over only while the provider is unchanged (a
    /// switched provider must not inherit the old provider's key). Errors are
    /// user-facing flash messages.
    pub fn apply_form(&self, f: ConfigForm) -> anyhow::Result<RagConfig> {
        let (Ok(port), Ok(top_k)) = (
            f.port.trim().parse::<u16>(),
            f.reranker_top_k.trim().parse::<usize>(),
        ) else {
            anyhow::bail!("Port and top_k must be whole numbers.");
        };

        // "none" → semantic-only: clear the LLM entirely rather than writing a
        // keyless provider that would fail the boot key gate (adr/0022).
        let llm = if f.provider == "none" {
            None
        } else {
            // Compare against form_id, not provider(): "openai-local" and
            // "openai" are distinct form choices (switching between them is a
            // provider change, so secrets don't carry across).
            let provider_unchanged =
                Some(f.provider.as_str()) == self.llm.as_ref().map(|l| l.form_id());
            let carry = |current: Option<&str>| {
                if provider_unchanged {
                    current.map(str::to_string)
                } else {
                    None
                }
            };
            let key = if !f.api_key.is_empty() {
                Some(f.api_key)
            } else {
                carry(self.llm.as_ref().and_then(|l| l.api_key()))
            };
            // "openai-local" is a form-level alias: the OpenAI wire pointed at
            // a user-supplied endpoint (Ollama, llama.cpp, …). It maps to the
            // `openai` provider with a url override; the cloud providers have
            // fixed endpoints and never write one.
            let (provider, url) = if f.provider == "openai-local" {
                let url = f.llm_url.trim();
                if url.is_empty() {
                    anyhow::bail!("The local OpenAI-compatible provider needs a URL.");
                }
                ("openai", Some(url.to_string()))
            } else {
                (f.provider.as_str(), None)
            };
            Some(
                LlmConfig::from_parts(provider, Some(f.model), key, url)
                    .map_err(|e| anyhow::anyhow!("Invalid LLM settings: {e}"))?,
            )
        };

        // "none" → unconfigured: clear the embedder entirely (adr/0024). The
        // fastembed model is always an explicit choice — no hidden default.
        let embedder = match f.embedder_provider.as_str() {
            "none" => None,
            "fastembed" => {
                let model = f.fastembed_model.trim();
                if model.is_empty() {
                    anyhow::bail!("Pick a fastembed model.");
                }
                Some(EmbedderConfig::FastEmbed {
                    model: Some(model.to_string()),
                })
            }
            "ollama" => {
                let (url, model) = (f.embedder_url.trim(), f.embedder_model.trim());
                if url.is_empty() || model.is_empty() {
                    anyhow::bail!("The Ollama embedder needs a URL and a model.");
                }
                // Prefixes are file-only knobs; carry them while the type is
                // unchanged so a web save never erases a hand-edited value.
                let (doc_prefix, query_prefix) = match &self.embedder {
                    Some(EmbedderConfig::Ollama {
                        doc_prefix,
                        query_prefix,
                        ..
                    }) => (doc_prefix.clone(), query_prefix.clone()),
                    _ => (String::new(), String::new()),
                };
                Some(EmbedderConfig::Ollama {
                    url: url.to_string(),
                    model: model.to_string(),
                    doc_prefix,
                    query_prefix,
                })
            }
            "openai" => {
                let (url, model) = (f.embedder_url.trim(), f.embedder_model.trim());
                if url.is_empty() || model.is_empty() {
                    anyhow::bail!("The OpenAI-compatible embedder needs a URL and a model.");
                }
                // Blank key keeps the current one only while the type is
                // unchanged (a switched embedder must not inherit a key);
                // prefixes carry under the same rule.
                let (current_key, doc_prefix, query_prefix) = match &self.embedder {
                    Some(EmbedderConfig::OpenAI {
                        api_key,
                        doc_prefix,
                        query_prefix,
                        ..
                    }) => (api_key.clone(), doc_prefix.clone(), query_prefix.clone()),
                    _ => (None, String::new(), String::new()),
                };
                let api_key = if f.embedder_api_key.is_empty() {
                    current_key
                } else {
                    Some(f.embedder_api_key.clone())
                };
                Some(EmbedderConfig::OpenAI {
                    url: url.to_string(),
                    model: model.to_string(),
                    api_key,
                    doc_prefix,
                    query_prefix,
                })
            }
            other => anyhow::bail!("Unknown embedder provider: {other}"),
        };

        let vector_db = match f.vector_db.as_str() {
            // "lance" accepted from stale forms predating the SQLite rename.
            "sqlite" | "lance" => {
                let path = f.sqlite_path.trim();
                VectorDbConfig::Sqlite {
                    path: if path.is_empty() {
                        generated_sqlite_path()
                    } else {
                        PathBuf::from(path)
                    },
                }
            }
            "qdrant" => VectorDbConfig::Qdrant {
                url: {
                    let url = f.qdrant_url.trim();
                    if url.is_empty() {
                        default_qdrant_url()
                    } else {
                        url.to_string()
                    }
                },
                collection: {
                    let c = f.qdrant_collection.trim();
                    if c.is_empty() {
                        default_qdrant_collection()
                    } else {
                        c.to_string()
                    }
                },
            },
            other => anyhow::bail!("Unknown vector DB: {other}"),
        };

        let mut cfg = self.clone();
        cfg.server.host = f.host;
        cfg.server.port = port;
        cfg.llm = llm;
        cfg.embedder = embedder;
        cfg.vector_db = vector_db;
        cfg.reranker.enabled = f.reranker_enabled.is_some();
        cfg.reranker.top_k = top_k;
        cfg.reranker.context_cut = match f.context_cut.as_str() {
            // Stale form post predating the field: keep the current setting.
            "" => self.reranker.context_cut,
            "fixed" => ContextCut::Fixed,
            "score-range" => ContextCut::ScoreRange,
            "largest-drop" => ContextCut::LargestDrop,
            other => anyhow::bail!("Unknown context cut: {other}"),
        };
        cfg.reranker.score_range_cutoff = match f.score_range_cutoff.trim() {
            "" => self.reranker.score_range_cutoff,
            s => s
                .parse::<f64>()
                .ok()
                .filter(|v| (0.0..=1.0).contains(v))
                .ok_or_else(|| {
                    anyhow::anyhow!("The score-range cutoff must be a number between 0 and 1.")
                })?,
        };
        let window = |value: &str, current: usize| -> anyhow::Result<usize> {
            match value.trim() {
                "" => Ok(current),
                s => s.parse::<usize>().ok().filter(|&v| v >= 1).ok_or_else(|| {
                    anyhow::anyhow!("The drop window bounds must be whole numbers ≥ 1.")
                }),
            }
        };
        cfg.reranker.drop_window_min = window(&f.drop_window_min, self.reranker.drop_window_min)?;
        cfg.reranker.drop_window_max = window(&f.drop_window_max, self.reranker.drop_window_max)?;
        if cfg.reranker.drop_window_min > cfg.reranker.drop_window_max {
            anyhow::bail!("The drop window minimum cannot exceed its maximum.");
        }
        // Blank keeps the current token (the password field is never pre-filled).
        if !f.auth_token.is_empty() {
            cfg.auth.token = Some(f.auth_token);
        }
        Ok(cfg)
    }

    /// Load configuration from a TOML file
    pub fn from_file(path: PathBuf) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file at {:?}: {}", path, e))?;
        let config: RagConfig = toml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse config file: {}", e))?;
        Ok(config)
    }

    /// Get the default config path (~/.config/kimun/server.toml)
    pub fn default_path() -> PathBuf {
        let mut path = dirs::home_dir().expect("Could not find home directory");
        path.push(".config");
        path.push("kimun");
        path.push("server.toml");
        path
    }

    /// Resolve the config path from an optional override, falling back to the
    /// default location. Same resolution [`load`](Self::load) uses, exposed so
    /// callers can persist edits back to the file the server was loaded from.
    pub fn resolve_path(override_path: Option<PathBuf>) -> PathBuf {
        override_path.unwrap_or_else(Self::default_path)
    }

    /// Serialize the config back to a TOML file. The web UI uses this to persist
    /// edits; the running server keeps its in-memory config until restarted.
    pub fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let toml =
            toml::to_string_pretty(self).map_err(|e| anyhow::anyhow!("serialize config: {e}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("create config dir {:?}: {e}", parent))?;
        }
        std::fs::write(path, toml).map_err(|e| anyhow::anyhow!("write config {:?}: {e}", path))?;
        Ok(())
    }

    /// Load config from default path or provided path.
    ///
    /// An explicit `--config` path that does not exist is an error — an explicit
    /// path asserts the file is there, so a typo fails loud. But when no override
    /// is given and the *default* path is missing, this is a first run: generate
    /// a semantic-only default config, persist it there, and boot from it
    /// (adr/0022).
    pub fn load(override_path: Option<PathBuf>) -> anyhow::Result<Self> {
        if let Some(path) = override_path {
            if !path.exists() {
                anyhow::bail!(
                    "Config file not found at {:?}. Please create a config file. See config.example.toml for reference.",
                    path
                );
            }
            return Self::from_file(path);
        }

        Self::load_or_generate_default(&Self::default_path())
    }

    /// Load the config at `path`, or — when it does not exist — generate a
    /// semantic-only default, persist it there, and return it (adr/0022). Split
    /// out from [`load`] so the first-run generation is testable against a temp
    /// path instead of the hardcoded [`default_path`](Self::default_path).
    fn load_or_generate_default(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            return Self::from_file(path.to_path_buf());
        }

        let config = Self::default();
        config.save_to(path)?;
        tracing::info!(
            "No config found; wrote an unconfigured default (SQLite, no embedder, no LLM) to {:?} — open the web UI /config to set up an embedder",
            path
        );
        Ok(config)
    }

    /// Merge configuration with CLI arguments
    pub fn merge_with_cli(mut self, host: Option<String>, port: Option<u16>) -> Self {
        if let Some(host) = host {
            self.server.host = host;
        }
        if let Some(port) = port {
            self.server.port = port;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_values() {
        // Push-only server: no [vault] section exists anymore (adr/0018).
        let config_toml = r#"
[server]

[vector_db]
type = "qdrant"

[llm]
provider = "gemini"

[reranker]
"#;

        let config: RagConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 7573);
        assert!(config.reranker.enabled);
        // top_k has a default and is configurable (was missing from the struct).
        assert_eq!(config.reranker.top_k, 20);
        // Auth is optional; absent by default.
        assert!(config.auth.token.is_none());
    }

    #[test]
    fn reranker_provider_defaults_to_fastembed_and_parses_http() {
        // Bare [reranker] (and legacy configs) keep the local cross-encoder.
        let cfg: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n").unwrap();
        assert_eq!(cfg.reranker.provider, RerankerProvider::FastEmbed);
        assert!(cfg.reranker.url.is_none());
        assert_eq!(cfg.reranker.context_cut, ContextCut::ScoreRange);
        assert_eq!(cfg.reranker.score_range_cutoff, 0.4);

        let cfg: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\nscore_range_cutoff = 0.5\n",
        )
        .unwrap();
        assert_eq!(cfg.reranker.score_range_cutoff, 0.5);

        let cfg: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\ncontext_cut = \"largest-drop\"\n",
        )
        .unwrap();
        assert_eq!(cfg.reranker.context_cut, ContextCut::LargestDrop);

        let cfg: RagConfig = toml::from_str(
            r#"
[server]
[vector_db]
type = "qdrant"
[reranker]
type = "http"
url = "https://api.cohere.com/v2"
model = "rerank-v3.5"
api_key = "k"
"#,
        )
        .unwrap();
        assert_eq!(cfg.reranker.provider, RerankerProvider::Http);
        assert_eq!(
            cfg.reranker.url.as_deref(),
            Some("https://api.cohere.com/v2")
        );
        assert_eq!(cfg.reranker.model.as_deref(), Some("rerank-v3.5"));
        assert_eq!(cfg.reranker.api_key.as_deref(), Some("k"));
    }

    #[test]
    fn web_form_edits_the_context_cut() {
        let cfg: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\n").unwrap();

        // The form sends the select's value.
        let mut f = form("none", "");
        f.context_cut = "largest-drop".into();
        assert_eq!(
            cfg.apply_form(f).unwrap().reranker.context_cut,
            ContextCut::LargestDrop
        );

        // A stale form post predating the field (empty value) keeps the
        // current setting instead of silently resetting it.
        let cfg: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\ncontext_cut = \"largest-drop\"\n",
        )
        .unwrap();
        assert_eq!(
            cfg.apply_form(form("none", ""))
                .unwrap()
                .reranker
                .context_cut,
            ContextCut::LargestDrop
        );

        // Garbage is a user-facing error, not a silent default.
        let mut f = form("none", "");
        f.context_cut = "biggest-elbow".into();
        assert!(cfg.apply_form(f).is_err());
    }

    #[test]
    fn web_form_edits_the_strategy_knobs() {
        let cfg: RagConfig =
            toml::from_str("[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\n").unwrap();
        assert_eq!(cfg.reranker.drop_window_min, 3);
        assert_eq!(cfg.reranker.drop_window_max, 30);

        let mut f = form("none", "");
        f.context_cut = "fixed".into();
        f.score_range_cutoff = "0.55".into();
        f.drop_window_min = "5".into();
        f.drop_window_max = "40".into();
        let saved = cfg.apply_form(f).unwrap();
        assert_eq!(saved.reranker.context_cut, ContextCut::Fixed);
        assert_eq!(saved.reranker.score_range_cutoff, 0.55);
        assert_eq!(saved.reranker.drop_window_min, 5);
        assert_eq!(saved.reranker.drop_window_max, 40);

        // Blanks (stale form) keep current values.
        let kept = saved.apply_form(form("none", "")).unwrap();
        assert_eq!(kept.reranker.score_range_cutoff, 0.55);
        assert_eq!(kept.reranker.drop_window_min, 5);

        // Out-of-range cutoff and inverted window are user-facing errors.
        let mut f = form("none", "");
        f.score_range_cutoff = "1.5".into();
        assert!(cfg.apply_form(f).is_err());
        let mut f = form("none", "");
        f.drop_window_min = "10".into();
        f.drop_window_max = "5".into();
        assert!(cfg.apply_form(f).is_err());
    }

    #[test]
    fn web_form_save_carries_http_reranker_fields() {
        // The form only edits enabled/top_k; provider fields are file-only
        // knobs (like embedder prefixes) and must survive a web UI save.
        let cfg: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"sqlite\"\n[reranker]\ntype = \"http\"\nurl = \"https://api.jina.ai/v1\"\nmodel = \"jina-reranker-v2-base-multilingual\"\ncontext_cut = \"largest-drop\"\n",
        )
        .unwrap();
        let saved = cfg.apply_form(form("none", "")).unwrap();
        assert_eq!(saved.reranker.provider, RerankerProvider::Http);
        assert_eq!(
            saved.reranker.url.as_deref(),
            Some("https://api.jina.ai/v1")
        );
        assert_eq!(saved.reranker.context_cut, ContextCut::LargestDrop);

        // And the saved config must survive the serialize → reload cycle the
        // web UI persists through (save_to), including the tagged type field.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        saved.save_to(&path).unwrap();
        let reloaded = RagConfig::from_file(path).unwrap();
        assert_eq!(reloaded.reranker.provider, RerankerProvider::Http);
        assert_eq!(reloaded.reranker.context_cut, ContextCut::LargestDrop);
        assert_eq!(
            reloaded.reranker.url.as_deref(),
            Some("https://api.jina.ai/v1")
        );
        assert_eq!(
            reloaded.reranker.model.as_deref(),
            Some("jina-reranker-v2-base-multilingual")
        );
        assert!(reloaded.reranker.api_key.is_none());
    }

    #[test]
    fn config_without_embedder_section_is_unconfigured() {
        // No [embedder] section → None (unconfigured server, adr/0024). The old
        // silent fastembed fallback is gone deliberately.
        let config_toml = r#"
[server]
[vector_db]
type = "qdrant"
[llm]
provider = "gemini"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        assert!(config.embedder.is_none());
    }

    #[test]
    fn embedder_provider_ids() {
        let fe = EmbedderConfig::FastEmbed { model: None };
        assert_eq!(fe.provider(), "fastembed");
        let ol: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[embedder]\ntype = \"ollama\"\nurl = \"u\"\nmodel = \"m\"\n[reranker]\n",
        )
        .unwrap();
        assert_eq!(ol.embedder.unwrap().provider(), "ollama");
    }

    #[test]
    fn embedder_fingerprint_is_provider_model_dim() {
        let fe = EmbedderConfig::FastEmbed {
            model: Some("BGESmallENV15".into()),
        };
        assert_eq!(fe.fingerprint(384), "fastembed:bgesmallenv15:384");
        let fe_default = EmbedderConfig::FastEmbed { model: None };
        assert_eq!(fe_default.fingerprint(1024), "fastembed:default:1024");
        let ol = EmbedderConfig::Ollama {
            url: "http://x".into(),
            model: "Nomic-Embed-Text".into(),
            doc_prefix: String::new(),
            query_prefix: String::new(),
        };
        assert_eq!(ol.fingerprint(768), "ollama:nomic-embed-text:768");
    }

    #[test]
    fn test_fastembed_model_parse() {
        let config_toml = r#"
[server]
[vector_db]
type = "qdrant"
[embedder]
type = "fastembed"
model = "BGESmallENV15"
[llm]
provider = "gemini"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        match config.embedder {
            Some(EmbedderConfig::FastEmbed { model }) => {
                assert_eq!(model.as_deref(), Some("BGESmallENV15"))
            }
            other => panic!("expected fastembed, got {other:?}"),
        }
    }

    #[test]
    fn test_embedder_ollama_parse() {
        let config_toml = r#"
[server]
[vector_db]
type = "qdrant"
[embedder]
type = "ollama"
url = "http://localhost:11434"
model = "nomic-embed-text"
doc_prefix = "search_document: "
query_prefix = "search_query: "
[llm]
provider = "gemini"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        match config.embedder {
            Some(EmbedderConfig::Ollama {
                url,
                model,
                doc_prefix,
                query_prefix,
            }) => {
                assert_eq!(url, "http://localhost:11434");
                assert_eq!(model, "nomic-embed-text");
                assert_eq!(doc_prefix, "search_document: ");
                assert_eq!(query_prefix, "search_query: ");
            }
            other => panic!("expected ollama, got {other:?}"),
        }
    }

    #[test]
    fn save_to_round_trips_through_toml() {
        // The web UI persists edits via save_to; the internally-tagged enums
        // (llm provider, vector_db type, embedder type) must survive a
        // serialize → write → reload cycle.
        let config_toml = r#"
[server]
host = "0.0.0.0"
port = 9000

[vector_db]
type = "qdrant"
url = "http://localhost:6333"
collection = "kimun_embeddings"

[embedder]
type = "ollama"
url = "http://localhost:11434"
model = "nomic-embed-text"
doc_prefix = "d: "
query_prefix = "q: "

[llm]
provider = "claude"
model = "claude-3-5-sonnet-20241022"
api_key = "sk-ant-xxx"

[reranker]
enabled = false
top_k = 33

[auth]
token = "secret-token"
"#;
        let original: RagConfig = toml::from_str(config_toml).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("server.toml");
        original.save_to(&path).unwrap();

        let reloaded = RagConfig::from_file(path).unwrap();
        assert_eq!(reloaded.server.host, "0.0.0.0");
        assert_eq!(reloaded.server.port, 9000);
        assert!(!reloaded.reranker.enabled);
        assert_eq!(reloaded.reranker.top_k, 33);
        assert_eq!(reloaded.reranker.provider, RerankerProvider::FastEmbed);
        assert_eq!(reloaded.auth.token.as_deref(), Some("secret-token"));
        let llm = reloaded.llm.as_ref().expect("llm present");
        assert_eq!(llm.provider(), "claude");
        assert_eq!(llm.api_key(), Some("sk-ant-xxx"));
        assert!(matches!(reloaded.vector_db, VectorDbConfig::Qdrant { .. }));
        assert!(matches!(
            reloaded.embedder,
            Some(EmbedderConfig::Ollama { .. })
        ));
    }

    #[test]
    fn sqlite_vector_db_parses_with_default_path() {
        let config_toml = r#"
[server]
[vector_db]
type = "sqlite"
[llm]
provider = "gemini"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        match config.vector_db {
            VectorDbConfig::Sqlite { path } => {
                assert_eq!(path, default_sqlite_path())
            }
            other => panic!("expected sqlite, got {other:?}"),
        }
    }

    /// Configs written before the LanceDB → SQLite replacement still parse:
    /// `type = "lance"` is a legacy alias for the SQLite store.
    #[test]
    fn legacy_lance_type_parses_as_sqlite() {
        let config_toml = r#"
[server]
[vector_db]
type = "lance"
path = "/data/old_lance"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        match config.vector_db {
            VectorDbConfig::Sqlite { path } => {
                assert_eq!(path, PathBuf::from("/data/old_lance"))
            }
            other => panic!("expected sqlite, got {other:?}"),
        }
    }

    fn form(provider: &str, api_key: &str) -> ConfigForm {
        ConfigForm {
            host: "127.0.0.1".into(),
            port: "7573".into(),
            provider: provider.into(),
            model: "m".into(),
            api_key: api_key.into(),
            llm_url: String::new(),
            embedder_provider: "none".into(),
            fastembed_model: String::new(),
            embedder_url: String::new(),
            embedder_model: String::new(),
            embedder_api_key: String::new(),
            vector_db: "sqlite".into(),
            sqlite_path: "./rag_sqlite".into(),
            qdrant_url: String::new(),
            qdrant_collection: String::new(),
            reranker_enabled: Some("on".into()),
            reranker_top_k: "20".into(),
            context_cut: String::new(),
            score_range_cutoff: String::new(),
            drop_window_min: String::new(),
            drop_window_max: String::new(),
            auth_token: String::new(),
        }
    }

    fn config_with_llm(llm_toml: &str) -> RagConfig {
        toml::from_str(&format!(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n{llm_toml}\n[reranker]\n"
        ))
        .unwrap()
    }

    #[test]
    fn apply_form_rejects_non_numeric_port_without_mutating() {
        let cfg = config_with_llm("[llm]\nprovider = \"gemini\"");
        let mut f = form("gemini", "");
        f.port = "not-a-port".into();
        let err = cfg.apply_form(f).unwrap_err();
        assert!(err.to_string().contains("whole numbers"));
    }

    #[test]
    fn apply_form_blank_key_keeps_current_only_when_provider_unchanged() {
        let cfg = config_with_llm("[llm]\nprovider = \"gemini\"\napi_key = \"gem-key\"");
        // Same provider, blank key → carried.
        let kept = cfg.apply_form(form("gemini", "")).unwrap();
        assert_eq!(kept.llm.as_ref().unwrap().api_key(), Some("gem-key"));
        // Switched provider, blank key → NOT carried.
        let switched = cfg.apply_form(form("openai", "")).unwrap();
        assert_eq!(switched.llm.as_ref().unwrap().api_key(), None);
    }

    #[test]
    fn apply_form_none_provider_clears_llm() {
        let cfg = config_with_llm("[llm]\nprovider = \"gemini\"\napi_key = \"k\"");
        let out = cfg.apply_form(form("none", "")).unwrap();
        assert!(out.llm.is_none());
    }

    #[test]
    fn apply_form_openai_local_maps_to_openai_with_url() {
        let cfg = config_with_llm("");
        let mut f = form("openai-local", "");
        f.llm_url = "http://localhost:11434/v1".into();
        let out = cfg.apply_form(f).unwrap();
        let llm = out.llm.as_ref().unwrap();
        assert_eq!(llm.provider(), "openai");
        assert_eq!(llm.form_id(), "openai-local");
        assert_eq!(llm.url(), Some("http://localhost:11434/v1"));
    }

    #[test]
    fn apply_form_openai_local_requires_url() {
        let cfg = config_with_llm("");
        let err = cfg.apply_form(form("openai-local", "")).unwrap_err();
        assert!(err.to_string().contains("needs a URL"));
    }

    #[test]
    fn apply_form_openai_local_and_cloud_openai_are_distinct_providers() {
        // A configured local endpoint round-trips: blank key is carried while
        // the form choice stays openai-local.
        let cfg = config_with_llm(
            "[llm]\nprovider = \"openai\"\napi_key = \"local-key\"\nurl = \"http://localhost:11434/v1\"",
        );
        assert_eq!(cfg.llm.as_ref().unwrap().form_id(), "openai-local");
        let mut f = form("openai-local", "");
        f.llm_url = "http://localhost:11434/v1".into();
        let kept = cfg.apply_form(f).unwrap();
        assert_eq!(kept.llm.as_ref().unwrap().api_key(), Some("local-key"));
        // Switching to cloud openai is a provider change: the url is dropped
        // and the local key is NOT inherited.
        let cloud = cfg.apply_form(form("openai", "")).unwrap();
        let llm = cloud.llm.as_ref().unwrap();
        assert_eq!(llm.url(), None);
        assert_eq!(llm.api_key(), None);
    }

    #[test]
    fn apply_form_blank_auth_token_keeps_current() {
        let cfg: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n[auth]\ntoken = \"secret\"\n",
        )
        .unwrap();
        let out = cfg.apply_form(form("none", "")).unwrap();
        assert_eq!(out.auth.token.as_deref(), Some("secret"));
        // A typed token replaces it.
        let mut f = form("none", "");
        f.auth_token = "new-secret".into();
        let out = cfg.apply_form(f).unwrap();
        assert_eq!(out.auth.token.as_deref(), Some("new-secret"));
    }

    #[test]
    fn apply_form_embedder_none_clears_embedder() {
        let cfg: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"sqlite\"\n[embedder]\ntype = \"fastembed\"\nmodel = \"BGESmallENV15\"\n[reranker]\n",
        )
        .unwrap();
        let out = cfg.apply_form(form("none", "")).unwrap();
        assert!(out.embedder.is_none(), "none must clear to unconfigured");
    }

    #[test]
    fn apply_form_fastembed_requires_a_model() {
        let cfg = RagConfig::default();
        let mut f = form("none", "");
        f.embedder_provider = "fastembed".into();
        f.fastembed_model = String::new();
        let err = cfg.apply_form(f).unwrap_err();
        assert!(err.to_string().contains("model"));

        let mut f = form("none", "");
        f.embedder_provider = "fastembed".into();
        f.fastembed_model = "Xenova/bge-small-en-v1.5".into();
        let out = cfg.apply_form(f).unwrap();
        match out.embedder {
            Some(EmbedderConfig::FastEmbed { model }) => {
                assert_eq!(model.as_deref(), Some("Xenova/bge-small-en-v1.5"))
            }
            other => panic!("expected fastembed, got {other:?}"),
        }
    }

    #[test]
    fn apply_form_ollama_requires_url_and_model_and_carries_prefixes() {
        let cfg: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"sqlite\"\n[embedder]\ntype = \"ollama\"\nurl = \"http://old:11434\"\nmodel = \"old\"\ndoc_prefix = \"d: \"\nquery_prefix = \"q: \"\n[reranker]\n",
        )
        .unwrap();
        // Missing url → friendly error.
        let mut f = form("none", "");
        f.embedder_provider = "ollama".into();
        f.embedder_model = "nomic-embed-text".into();
        assert!(cfg.apply_form(f).is_err());
        // Same provider → prefixes (file-only knobs) carry over.
        let mut f = form("none", "");
        f.embedder_provider = "ollama".into();
        f.embedder_url = "http://new:11434".into();
        f.embedder_model = "nomic-embed-text".into();
        let out = cfg.apply_form(f).unwrap();
        match out.embedder {
            Some(EmbedderConfig::Ollama {
                url,
                model,
                doc_prefix,
                query_prefix,
            }) => {
                assert_eq!(url, "http://new:11434");
                assert_eq!(model, "nomic-embed-text");
                assert_eq!(doc_prefix, "d: ");
                assert_eq!(query_prefix, "q: ");
            }
            other => panic!("expected ollama, got {other:?}"),
        }
    }

    #[test]
    fn apply_form_openai_embedder_key_carries_only_while_type_unchanged() {
        let cfg: RagConfig = toml::from_str(
            "[server]\n[vector_db]\ntype = \"sqlite\"\n[embedder]\ntype = \"openai\"\nurl = \"https://api.openai.com/v1\"\nmodel = \"text-embedding-3-small\"\napi_key = \"emb-key\"\n[reranker]\n",
        )
        .unwrap();
        // Same type, blank key → carried.
        let mut f = form("none", "");
        f.embedder_provider = "openai".into();
        f.embedder_url = "https://api.openai.com/v1".into();
        f.embedder_model = "text-embedding-3-small".into();
        let out = cfg.apply_form(f).unwrap();
        match &out.embedder {
            Some(EmbedderConfig::OpenAI { api_key, .. }) => {
                assert_eq!(api_key.as_deref(), Some("emb-key"))
            }
            other => panic!("expected openai, got {other:?}"),
        }
        // Switched type → key must not leak into the new embedder.
        let mut f = form("none", "");
        f.embedder_provider = "ollama".into();
        f.embedder_url = "http://x:11434".into();
        f.embedder_model = "m".into();
        let out = cfg.apply_form(f).unwrap();
        assert!(matches!(out.embedder, Some(EmbedderConfig::Ollama { .. })));
    }

    #[test]
    fn apply_form_vector_db_switch() {
        let cfg = RagConfig::default();
        // sqlite with explicit path
        let mut f = form("none", "");
        f.vector_db = "sqlite".into();
        f.sqlite_path = "/data/sqlite".into();
        let out = cfg.apply_form(f).unwrap();
        match out.vector_db {
            VectorDbConfig::Sqlite { path } => assert_eq!(path, PathBuf::from("/data/sqlite")),
            other => panic!("expected sqlite, got {other:?}"),
        }
        // sqlite with blank path → generated default (data-dir absolute)
        let mut f = form("none", "");
        f.vector_db = "sqlite".into();
        f.sqlite_path = String::new();
        let out = cfg.apply_form(f).unwrap();
        assert!(matches!(out.vector_db, VectorDbConfig::Sqlite { .. }));
        // "lance" from a stale form still selects the SQLite store.
        let mut f = form("none", "");
        f.vector_db = "lance".into();
        f.sqlite_path = "/data/old".into();
        let out = cfg.apply_form(f).unwrap();
        assert!(matches!(out.vector_db, VectorDbConfig::Sqlite { .. }));
        // qdrant with blanks → defaults
        let mut f = form("none", "");
        f.vector_db = "qdrant".into();
        let out = cfg.apply_form(f).unwrap();
        match out.vector_db {
            VectorDbConfig::Qdrant { url, collection } => {
                assert_eq!(url, default_qdrant_url());
                assert_eq!(collection, default_qdrant_collection());
            }
            other => panic!("expected qdrant, got {other:?}"),
        }
    }

    #[test]
    fn llm_from_parts_defaults_model_and_rejects_unknown() {
        let c = LlmConfig::from_parts("openai", None, None, None).unwrap();
        assert_eq!(c.provider(), "openai");
        assert_eq!(c.model(), &default_openai_model());
        assert!(c.api_key().is_none());
        assert!(c.url().is_none());

        let c = LlmConfig::from_parts(
            "openai",
            None,
            None,
            Some("http://localhost:11434/v1".into()),
        )
        .unwrap();
        assert_eq!(c.url(), Some("http://localhost:11434/v1"));

        let c =
            LlmConfig::from_parts("gemini", Some(String::new()), Some("k".into()), None).unwrap();
        assert_eq!(c.model(), &default_gemini_model());
        assert_eq!(c.api_key(), Some("k"));
        assert!(c.url().is_none(), "url is openai-only");

        assert!(LlmConfig::from_parts("bogus", None, None, None).is_err());
    }

    #[test]
    fn test_auth_and_llm_key_parse() {
        let config_toml = r#"
[server]

[vector_db]
type = "qdrant"

[llm]
provider = "claude"
model = "claude-3-5-sonnet-20241022"
api_key = "sk-ant-xxx"

[reranker]
top_k = 40

[auth]
token = "secret-token"
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.reranker.top_k, 40);
        assert_eq!(config.auth.token.as_deref(), Some("secret-token"));
        match &config.llm {
            Some(LlmConfig::Claude { api_key, .. }) => {
                assert_eq!(api_key.as_deref(), Some("sk-ant-xxx"))
            }
            _ => panic!("expected claude"),
        }
    }

    #[test]
    fn config_without_llm_section_is_semantic_only() {
        // No [llm] section → llm is None (semantic-only server, adr/0022).
        let config_toml = r#"
[server]
[vector_db]
type = "sqlite"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        assert!(config.llm.is_none());
    }

    #[test]
    fn default_config_is_unconfigured_sqlite() {
        // The generated first-run default: SQLite, no embedder, no LLM, no
        // auth (adr/0024).
        let config = RagConfig::default();
        assert!(config.llm.is_none());
        assert!(config.auth.token.is_none());
        assert!(config.embedder.is_none());
        match config.vector_db {
            VectorDbConfig::Sqlite { path } => {
                // Absolute when a data dir resolves (generated_sqlite_path).
                assert!(path.ends_with("kimun/rag_sqlite"), "got {path:?}");
            }
            other => panic!("expected sqlite, got {other:?}"),
        }
        assert_eq!(config.server.host, default_host());
        assert_eq!(config.server.port, default_port());
    }

    #[test]
    fn load_or_generate_default_writes_semantic_only_on_first_run() {
        // Missing path → generate a semantic-only default, persist it, boot from
        // it. A second load reads the file back rather than regenerating.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kimun").join("server.toml");
        assert!(!path.exists());

        let generated = RagConfig::load_or_generate_default(&path).unwrap();
        assert!(generated.llm.is_none());
        assert!(path.exists(), "first run must persist the default");

        // Second call must not overwrite: mark the file, reload, confirm it read
        // the existing file (llm still none, file untouched semantics).
        let reloaded = RagConfig::load_or_generate_default(&path).unwrap();
        assert!(reloaded.llm.is_none());
        assert!(matches!(reloaded.vector_db, VectorDbConfig::Sqlite { .. }));
    }

    #[test]
    fn ready_default_is_fastembed_on_sqlite_semantic_only() {
        let cfg = RagConfig::ready_default();
        assert!(matches!(
            cfg.embedder,
            Some(EmbedderConfig::FastEmbed { model: None })
        ));
        assert!(matches!(cfg.vector_db, VectorDbConfig::Sqlite { .. }));
        assert!(cfg.llm.is_none());
    }

    #[test]
    fn ready_default_persisted_writes_once_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kimun").join("server.toml");

        // Missing file → written, and it round-trips as the ready default.
        RagConfig::ready_default_persisted(&path).unwrap();
        let on_disk = RagConfig::from_file(path.clone()).unwrap();
        assert!(matches!(
            on_disk.embedder,
            Some(EmbedderConfig::FastEmbed { model: None })
        ));

        // Existing file → untouched, even when its content differs.
        std::fs::write(
            &path,
            "[server]\nport = 9999\n[vector_db]\ntype = \"qdrant\"\n[reranker]\n",
        )
        .unwrap();
        let returned = RagConfig::ready_default_persisted(&path).unwrap();
        assert!(
            returned.embedder.is_some(),
            "still runs the built-in default"
        );
        let untouched = RagConfig::from_file(path).unwrap();
        assert_eq!(untouched.server.port, 9999);
    }

    #[test]
    fn default_config_round_trips_through_save_to() {
        // The default is what first run writes with save_to; it must reload.
        let config = RagConfig::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kimun").join("server.toml");
        config.save_to(&path).unwrap();

        let reloaded = RagConfig::from_file(path).unwrap();
        assert!(reloaded.llm.is_none());
        assert!(matches!(reloaded.vector_db, VectorDbConfig::Sqlite { .. }));
        assert!(reloaded.embedder.is_none());
    }
}
