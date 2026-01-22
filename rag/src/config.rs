use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
    pub server: ServerConfig,
    pub vault: VaultConfig,
    pub vector_db: VectorDbConfig,
    pub llm: LlmConfig,
    pub reranker: RerankerConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    pub path: PathBuf,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum LlmConfig {
    #[serde(rename = "gemini")]
    Gemini {
        #[serde(default = "default_gemini_model")]
        model: String,
    },
    #[serde(rename = "mistral")]
    Mistral {
        #[serde(default = "default_mistral_model")]
        model: String,
    },
    #[serde(rename = "claude")]
    Claude {
        #[serde(default = "default_claude_model")]
        model: String,
    },
    #[serde(rename = "openai")]
    OpenAI {
        #[serde(default = "default_openai_model")]
        model: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankerConfig {
    #[serde(default = "default_reranker_enabled")]
    pub enabled: bool,
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
    pub fn merge_with_cli(
        mut self,
        host: Option<String>,
        port: Option<u16>,
        vault_path: Option<PathBuf>,
    ) -> Self {
        if let Some(host) = host {
            self.server.host = host;
        }
        if let Some(port) = port {
            self.server.port = port;
        }
        if let Some(vault_path) = vault_path {
            self.vault.path = vault_path;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_values() {
        let config_toml = r#"
[server]

[vault]
path = "/tmp/notes"

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
        assert_eq!(config.reranker.top_k, 20);
    }
}
