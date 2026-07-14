use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::document::FlattenedChunk;

use super::Embedder;

/// Default local model when none is named (BGE-Large-EN-V1.5, 1024 dims).
const DEFAULT_MODEL: EmbeddingModel = EmbeddingModel::BGELargeENV15;

pub struct FastEmbedder {
    model: Arc<Mutex<TextEmbedding>>,
    dimension: usize,
}

impl FastEmbedder {
    /// Builds the local embedder for `model`, or the default when `None`. The
    /// name accepts either the fastembed variant name (e.g. `BGESmallENV15`) or
    /// the model code (e.g. `BAAI/bge-small-en-v1.5`), case-insensitively.
    pub fn new(model: Option<&str>) -> anyhow::Result<Self> {
        let (embedding_model, dimension) = resolve_model(model)?;
        let model = TextEmbedding::try_new(
            InitOptions::new(embedding_model).with_show_download_progress(true),
        )?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            dimension,
        })
    }
}

/// Every bundled fastembed model as `(model code, dimension)` — feeds the web
/// UI's model dropdown so choosing a local model is always explicit (adr/0024).
pub fn supported_models() -> Vec<(String, usize)> {
    TextEmbedding::list_supported_models()
        .into_iter()
        .map(|i| (i.model_code, i.dim))
        .collect()
}

/// The canonical model code for any accepted model name — variant name
/// (`BGESmallENV15`) or model code, case-insensitive. The web UI's dropdown is
/// keyed by model codes, so this is how a config value written in either form
/// matches its option. `None` for an unknown model.
pub fn canonical_model_code(name: &str) -> Option<String> {
    let (model, _) = resolve_model(Some(name)).ok()?;
    TextEmbedding::list_supported_models()
        .iter()
        .find(|i| i.model == model)
        .map(|i| i.model_code.clone())
}

/// Resolves a model name to its `EmbeddingModel` and output dimension.
fn resolve_model(name: Option<&str>) -> anyhow::Result<(EmbeddingModel, usize)> {
    let infos = TextEmbedding::list_supported_models();
    let selected = match name {
        None => DEFAULT_MODEL,
        Some(s) => EmbeddingModel::from_str(s)
            .ok()
            .or_else(|| {
                infos
                    .iter()
                    .find(|i| i.model_code.eq_ignore_ascii_case(s))
                    .map(|i| i.model.clone())
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Unknown fastembed model `{s}` \
                     (use a variant name like `BGESmallENV15`, or a model code \
                     like `Xenova/bge-small-en-v1.5`)"
                )
            })?,
    };
    let dimension = infos
        .iter()
        .find(|i| i.model == selected)
        .map(|i| i.dim)
        .ok_or_else(|| anyhow::anyhow!("No dimension info for model `{selected:?}`"))?;
    Ok((selected, dimension))
}

#[async_trait]
impl Embedder for FastEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    async fn generate_embeddings(
        &self,
        chunks: &[FlattenedChunk],
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let texts: Vec<String> = chunks
            .iter()
            .map(|chunk| format!("passage: {}\n{}", chunk.title, chunk.text))
            .collect();

        let model = self.model.clone();
        let embeds = tokio::task::spawn_blocking(move || {
            let mut model_guard = model.blocking_lock();
            model_guard.embed(texts, None)
        })
        .await??;

        Ok(embeds)
    }

    async fn prompt_embedding(&self, query: &str) -> anyhow::Result<Vec<f32>> {
        let text = format!("query: {}", query);
        let model = self.model.clone();

        let embed = tokio::task::spawn_blocking(move || {
            let mut model_guard = model.blocking_lock();
            model_guard.embed(vec![text], None)
        })
        .await??;

        Ok(embed.into_iter().next().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_default_is_bge_large_1024() {
        let (model, dim) = resolve_model(None).unwrap();
        assert_eq!(model, EmbeddingModel::BGELargeENV15);
        assert_eq!(dim, 1024);
    }

    #[test]
    fn resolve_by_variant_name_case_insensitive() {
        let (model, dim) = resolve_model(Some("bgesmallenv15")).unwrap();
        assert_eq!(model, EmbeddingModel::BGESmallENV15);
        assert_eq!(dim, 384);
    }

    #[test]
    fn resolve_by_model_code() {
        let (_model, dim) = resolve_model(Some("Xenova/bge-small-en-v1.5")).unwrap();
        assert_eq!(dim, 384);
    }

    #[test]
    fn resolve_unknown_model_errors() {
        assert!(resolve_model(Some("not-a-real-model")).is_err());
    }

    #[test]
    fn canonical_model_code_accepts_variant_names_and_codes() {
        // A config may name the model either way (resolve_model accepts both);
        // the web UI dropdown needs the code form to match its options.
        assert_eq!(
            canonical_model_code("BGESmallENV15").as_deref(),
            Some("Xenova/bge-small-en-v1.5")
        );
        assert_eq!(
            canonical_model_code("xenova/BGE-small-en-v1.5").as_deref(),
            Some("Xenova/bge-small-en-v1.5")
        );
        assert!(canonical_model_code("not-a-real-model").is_none());
    }

    #[test]
    fn supported_models_is_nonempty_and_carries_dims() {
        let models = supported_models();
        assert!(!models.is_empty());
        assert!(
            models
                .iter()
                .any(|(code, dim)| code == "Xenova/bge-small-en-v1.5" && *dim == 384)
        );
    }
}
