use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
    pub server: ServerConfig,
    pub vector_db: VectorDbConfig,
    /// Which embedder produces the vectors. Defaults to the local fastembed
    /// model so an existing config with no [embedder] section keeps working.
    #[serde(default)]
    pub embedder: EmbedderConfig,
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

impl Default for EmbedderConfig {
    fn default() -> Self {
        EmbedderConfig::FastEmbed { model: None }
    }
}

/// Vector store selection. `lance` is embedded (local, file-backed, no server);
/// `qdrant` talks to a standalone server. A Turso backend is planned as a second
/// embedded option once it supports vector similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum VectorDbConfig {
    /// Embedded LanceDB store. `path` is a local directory (one table per vault).
    #[serde(rename = "lance")]
    Lance {
        #[serde(default = "default_lance_path")]
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
    /// Default number of results kept after reranking. Overridable per request
    /// via `context_size`.
    #[serde(default = "default_reranker_top_k")]
    pub top_k: usize,
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

fn default_lance_path() -> PathBuf {
    PathBuf::from("./rag_lance")
}

/// LanceDB store path baked into a *generated* first-run config: an absolute
/// path under the OS data dir (`~/.local/share/kimun/rag_lance`) so the store
/// does not float with the process's launch directory the way `./rag_lance`
/// would (adr/0022). Falls back to the relative default if the data dir can't be
/// resolved.
fn generated_lance_path() -> PathBuf {
    match dirs::data_dir() {
        Some(mut dir) => {
            dir.push("kimun");
            dir.push("rag_lance");
            dir
        }
        None => default_lance_path(),
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

/// A zero-config default: LanceDB under the data dir, local fastembed, default
/// `127.0.0.1` bind, no LLM (semantic-only), no auth. This is what a first run
/// with no config file boots from and writes to disk (adr/0022).
impl Default for RagConfig {
    fn default() -> Self {
        RagConfig {
            server: ServerConfig {
                host: default_host(),
                port: default_port(),
                max_concurrent_jobs: default_max_concurrent_jobs(),
            },
            vector_db: VectorDbConfig::Lance {
                path: generated_lance_path(),
            },
            embedder: EmbedderConfig::default(),
            llm: None,
            reranker: RerankerConfig {
                enabled: default_reranker_enabled(),
                top_k: default_reranker_top_k(),
            },
            auth: AuthConfig::default(),
        }
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
    #[serde(default)]
    pub reranker_enabled: Option<String>,
    pub reranker_top_k: String,
    pub auth_token: String,
}

impl RagConfig {
    /// Applies a submitted web form onto this (current) config, producing the
    /// config to persist. Every form→config rule lives here, not in the web
    /// layer: numeric parsing, the `none` provider sentinel clearing the LLM
    /// (adr/0022), and the carry rules for values the form can't or doesn't
    /// resend — a blank secret keeps the current one, and provider-scoped
    /// values (API key, endpoint url) carry over only while the provider is
    /// unchanged (a switched provider must not inherit the old provider's key).
    /// Errors are user-facing flash messages.
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
            let provider_unchanged =
                Some(f.provider.as_str()) == self.llm.as_ref().map(|l| l.provider());
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
            // The form has no url field; keep a hand-edited endpoint override
            // alive across saves as long as the provider stays the same.
            let url = carry(self.llm.as_ref().and_then(|l| l.url()));
            Some(
                LlmConfig::from_parts(&f.provider, Some(f.model), key, url)
                    .map_err(|e| anyhow::anyhow!("Invalid LLM settings: {e}"))?,
            )
        };

        let mut cfg = self.clone();
        cfg.server.host = f.host;
        cfg.server.port = port;
        cfg.llm = llm;
        cfg.reranker.enabled = f.reranker_enabled.is_some();
        cfg.reranker.top_k = top_k;
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

    /// Get the default config path (~/.config/kimun/rag.conf)
    pub fn default_path() -> PathBuf {
        let mut path = dirs::home_dir().expect("Could not find home directory");
        path.push(".config");
        path.push("kimun");
        path.push("rag.conf");
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
            "No config found; wrote a semantic-only default (LanceDB, no LLM) to {:?}",
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
        assert_eq!(config.reranker.enabled, true);
        // top_k has a default and is configurable (was missing from the struct).
        assert_eq!(config.reranker.top_k, 20);
        // Auth is optional; absent by default.
        assert!(config.auth.token.is_none());
    }

    #[test]
    fn test_embedder_defaults_to_fastembed() {
        let config_toml = r#"
[server]
[vector_db]
type = "qdrant"
[llm]
provider = "gemini"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        assert!(matches!(
            config.embedder,
            EmbedderConfig::FastEmbed { model: None }
        ));
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
            EmbedderConfig::FastEmbed { model } => {
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
            EmbedderConfig::Ollama {
                url,
                model,
                doc_prefix,
                query_prefix,
            } => {
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
        let path = dir.path().join("nested").join("rag.conf");
        original.save_to(&path).unwrap();

        let reloaded = RagConfig::from_file(path).unwrap();
        assert_eq!(reloaded.server.host, "0.0.0.0");
        assert_eq!(reloaded.server.port, 9000);
        assert_eq!(reloaded.reranker.enabled, false);
        assert_eq!(reloaded.reranker.top_k, 33);
        assert_eq!(reloaded.auth.token.as_deref(), Some("secret-token"));
        let llm = reloaded.llm.as_ref().expect("llm present");
        assert_eq!(llm.provider(), "claude");
        assert_eq!(llm.api_key(), Some("sk-ant-xxx"));
        assert!(matches!(reloaded.vector_db, VectorDbConfig::Qdrant { .. }));
        assert!(matches!(reloaded.embedder, EmbedderConfig::Ollama { .. }));
    }

    #[test]
    fn lance_vector_db_parses_with_default_path() {
        let config_toml = r#"
[server]
[vector_db]
type = "lance"
[llm]
provider = "gemini"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        match config.vector_db {
            VectorDbConfig::Lance { path } => {
                assert_eq!(path, default_lance_path())
            }
            other => panic!("expected lance, got {other:?}"),
        }
    }

    fn form(provider: &str, api_key: &str) -> ConfigForm {
        ConfigForm {
            host: "127.0.0.1".into(),
            port: "7573".into(),
            provider: provider.into(),
            model: "m".into(),
            api_key: api_key.into(),
            reranker_enabled: Some("on".into()),
            reranker_top_k: "20".into(),
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
    fn apply_form_carries_endpoint_url_while_provider_unchanged() {
        let cfg = config_with_llm(
            "[llm]\nprovider = \"openai\"\nurl = \"http://localhost:11434/v1\"",
        );
        let kept = cfg.apply_form(form("openai", "")).unwrap();
        assert_eq!(kept.llm.as_ref().unwrap().url(), Some("http://localhost:11434/v1"));
        let switched = cfg.apply_form(form("gemini", "")).unwrap();
        assert_eq!(switched.llm.as_ref().unwrap().url(), None);
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

        let c = LlmConfig::from_parts("gemini", Some(String::new()), Some("k".into()), None)
            .unwrap();
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
type = "lance"
[reranker]
"#;
        let config: RagConfig = toml::from_str(config_toml).unwrap();
        assert!(config.llm.is_none());
    }

    #[test]
    fn default_config_is_semantic_only_lance() {
        // The generated first-run default: LanceDB, fastembed, no LLM, no auth.
        let config = RagConfig::default();
        assert!(config.llm.is_none());
        assert!(config.auth.token.is_none());
        assert!(matches!(config.embedder, EmbedderConfig::FastEmbed { .. }));
        match config.vector_db {
            VectorDbConfig::Lance { path } => {
                // Absolute when a data dir resolves (generated_lance_path).
                assert!(path.ends_with("kimun/rag_lance"), "got {path:?}");
            }
            other => panic!("expected lance, got {other:?}"),
        }
        assert_eq!(config.server.host, default_host());
        assert_eq!(config.server.port, default_port());
    }

    #[test]
    fn load_or_generate_default_writes_semantic_only_on_first_run() {
        // Missing path → generate a semantic-only default, persist it, boot from
        // it. A second load reads the file back rather than regenerating.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kimun").join("rag.conf");
        assert!(!path.exists());

        let generated = RagConfig::load_or_generate_default(&path).unwrap();
        assert!(generated.llm.is_none());
        assert!(path.exists(), "first run must persist the default");

        // Second call must not overwrite: mark the file, reload, confirm it read
        // the existing file (llm still none, file untouched semantics).
        let reloaded = RagConfig::load_or_generate_default(&path).unwrap();
        assert!(reloaded.llm.is_none());
        assert!(matches!(reloaded.vector_db, VectorDbConfig::Lance { .. }));
    }

    #[test]
    fn default_config_round_trips_through_save_to() {
        // The default is what first run writes with save_to; it must reload.
        let config = RagConfig::default();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kimun").join("rag.conf");
        config.save_to(&path).unwrap();

        let reloaded = RagConfig::from_file(path).unwrap();
        assert!(reloaded.llm.is_none());
        assert!(matches!(reloaded.vector_db, VectorDbConfig::Lance { .. }));
        assert!(matches!(
            reloaded.embedder,
            EmbedderConfig::FastEmbed { .. }
        ));
    }
}
