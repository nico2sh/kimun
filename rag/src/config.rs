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
    pub llm: LlmConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum VectorDbConfig {
    #[serde(rename = "sqlite")]
    SQLite {
        #[serde(default = "default_sqlite_path")]
        db_path: PathBuf,
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

    /// The configured API key, if any.
    pub fn api_key(&self) -> Option<&str> {
        match self {
            LlmConfig::Gemini { api_key, .. }
            | LlmConfig::Mistral { api_key, .. }
            | LlmConfig::Claude { api_key, .. }
            | LlmConfig::OpenAI { api_key, .. } => api_key.as_deref(),
        }
    }

    /// Builds a config from web-form parts, defaulting the model per provider
    /// when blank. Unknown provider ids are rejected.
    pub fn from_parts(
        provider: &str,
        model: Option<String>,
        api_key: Option<String>,
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

fn default_sqlite_path() -> PathBuf {
    PathBuf::from("./rag_index.sqlite")
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

impl RagConfig {
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

    /// Load config from default path or provided path
    pub fn load(override_path: Option<PathBuf>) -> anyhow::Result<Self> {
        let path = override_path.unwrap_or_else(Self::default_path);

        if !path.exists() {
            anyhow::bail!(
                "Config file not found at {:?}. Please create a config file. See config.example.toml for reference.",
                path
            );
        }

        Self::from_file(path)
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
type = "sqlite"

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
type = "sqlite"
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
type = "sqlite"
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
type = "sqlite"
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
type = "sqlite"
db_path = "./x.sqlite"

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
        assert_eq!(reloaded.llm.provider(), "claude");
        assert_eq!(reloaded.llm.api_key(), Some("sk-ant-xxx"));
        assert!(matches!(reloaded.vector_db, VectorDbConfig::SQLite { .. }));
        assert!(matches!(reloaded.embedder, EmbedderConfig::Ollama { .. }));
    }

    #[test]
    fn llm_from_parts_defaults_model_and_rejects_unknown() {
        let c = LlmConfig::from_parts("openai", None, None).unwrap();
        assert_eq!(c.provider(), "openai");
        assert_eq!(c.model(), &default_openai_model());
        assert!(c.api_key().is_none());

        let c = LlmConfig::from_parts("gemini", Some(String::new()), Some("k".into())).unwrap();
        assert_eq!(c.model(), &default_gemini_model());
        assert_eq!(c.api_key(), Some("k"));

        assert!(LlmConfig::from_parts("bogus", None, None).is_err());
    }

    #[test]
    fn test_auth_and_llm_key_parse() {
        let config_toml = r#"
[server]

[vector_db]
type = "sqlite"

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
            LlmConfig::Claude { api_key, .. } => {
                assert_eq!(api_key.as_deref(), Some("sk-ant-xxx"))
            }
            _ => panic!("expected claude"),
        }
    }
}
