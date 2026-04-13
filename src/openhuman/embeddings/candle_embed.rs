//! Pure-Rust local embedding provider using the `candle` ML framework.
//!
//! Replaces the previous `fastembed` + ONNX Runtime stack, eliminating all C++
//! dynamic library dependencies. Uses HuggingFace's `candle` crate to run BERT
//! models (e.g. BGE-small-en-v1.5) entirely in Rust.
//!
//! Model weights and tokenizer files are downloaded from HuggingFace Hub on
//! first use and cached locally.

use std::sync::Arc;

use async_trait::async_trait;
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig, DTYPE};
use parking_lot::Mutex;
use tokenizers::Tokenizer;

use super::EmbeddingProvider;

/// Default HuggingFace model repository for local embeddings.
pub const DEFAULT_HF_REPO: &str = "BAAI/bge-small-en-v1.5";
/// Default embedding dimensions for BGE-small-en-v1.5.
pub const DEFAULT_DIMENSIONS: usize = 384;
/// Default model name used in config and factory.
pub const DEFAULT_MODEL_NAME: &str = "BGESmallENV15";

/// Maximum number of tokens per chunk sent to the model. BGE models support 512
/// but we leave a small margin for special tokens ([CLS], [SEP]).
const MAX_TOKENS: usize = 510;

/// Maps config-level model names to HuggingFace repository IDs.
fn resolve_hf_repo(model_name: &str) -> &str {
    match model_name {
        "BGESmallENV15" | "bge-small-en-v1.5" => "BAAI/bge-small-en-v1.5",
        "BGEBaseENV15" | "bge-base-en-v1.5" => "BAAI/bge-base-en-v1.5",
        "BGELargeENV15" | "bge-large-en-v1.5" => "BAAI/bge-large-en-v1.5",
        "AllMiniLmL6V2" | "all-MiniLM-L6-v2" => "sentence-transformers/all-MiniLM-L6-v2",
        "AllMiniLmL12V2" | "all-MiniLM-L12-v2" => "sentence-transformers/all-MiniLM-L12-v2",
        // Treat unknown names as direct HuggingFace repo IDs.
        other => other,
    }
}

// ── Internal state machine ──────────────────────────────────────

enum ModelState {
    /// Initial state before model is loaded.
    Uninitialized,
    /// Model and tokenizer are loaded and ready.
    Ready {
        model: BertModel,
        tokenizer: Tokenizer,
        device: Device,
    },
    /// Loading failed; cached error message prevents retry loops.
    Failed(String),
}

/// Pure-Rust local embedding provider backed by `candle`.
///
/// Loads a BERT-family model from HuggingFace Hub on first use. All inference
/// runs on CPU (or Metal on macOS when the `metal` feature is enabled).
/// The model is initialized lazily inside a blocking task to avoid stalling
/// the async runtime.
pub struct CandleEmbedding {
    model_name: String,
    hf_repo: String,
    dims: usize,
    state: Arc<Mutex<ModelState>>,
}

impl CandleEmbedding {
    /// Creates a new candle embedding provider.
    ///
    /// `model_name` is a config-level name (e.g. `"BGESmallENV15"`) or a direct
    /// HuggingFace repo ID. `dims` is the expected embedding dimensionality.
    pub fn new(model_name: &str, dims: usize) -> Self {
        let name = if model_name.trim().is_empty() {
            DEFAULT_MODEL_NAME.to_string()
        } else {
            model_name.trim().to_string()
        };
        let hf_repo = resolve_hf_repo(&name).to_string();
        let dims = if dims == 0 { DEFAULT_DIMENSIONS } else { dims };

        tracing::debug!(
            target: "embeddings.candle",
            "[embeddings] CandleEmbedding created: model={name}, repo={hf_repo}, dims={dims}"
        );

        Self {
            model_name: name,
            hf_repo,
            dims,
            state: Arc::new(Mutex::new(ModelState::Uninitialized)),
        }
    }
}

/// Downloads model files from HuggingFace Hub and loads them into candle.
///
/// This is called inside `spawn_blocking` — it does synchronous I/O and
/// potentially large downloads on first run.
fn load_model(
    hf_repo: &str,
    device: &Device,
) -> anyhow::Result<(BertModel, Tokenizer)> {
    tracing::debug!(
        target: "embeddings.candle",
        "[embeddings] downloading/loading model from HuggingFace: {hf_repo}"
    );

    let api = hf_hub::api::sync::Api::new()?;
    let repo = api.model(hf_repo.to_string());

    // Download the three required files.
    let config_path = repo.get("config.json")?;
    let tokenizer_path = repo.get("tokenizer.json")?;
    let weights_path = repo.get("model.safetensors")?;

    tracing::debug!(
        target: "embeddings.candle",
        "[embeddings] model files cached at: config={}, weights={}",
        config_path.display(),
        weights_path.display()
    );

    // Parse BERT config.
    let config_str = std::fs::read_to_string(&config_path)?;
    let config: BertConfig = serde_json::from_str(&config_str)?;

    // Load tokenizer.
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("failed to load tokenizer: {e}"))?;

    // Load model weights from safetensors.
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, device)?
    };
    let model = BertModel::load(vb, &config)?;

    tracing::debug!(
        target: "embeddings.candle",
        "[embeddings] model loaded successfully: {hf_repo}"
    );

    Ok((model, tokenizer))
}

/// Runs BERT inference on a batch of texts and returns normalized embeddings.
///
/// Uses CLS-token pooling (first token of each sequence) followed by L2
/// normalization, which is the standard approach for BGE models.
fn embed_batch(
    model: &BertModel,
    tokenizer: &Tokenizer,
    device: &Device,
    texts: &[String],
) -> anyhow::Result<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_embeddings = Vec::with_capacity(texts.len());

    // Process one text at a time to avoid padding complexity.
    // For the typical use case (embedding document chunks or queries) this is
    // fast enough, and avoids the need for dynamic padding / attention masks.
    for text in texts {
        let encoding = tokenizer
            .encode(text.as_str(), true)
            .map_err(|e| anyhow::anyhow!("tokenizer encode failed: {e}"))?;

        // Truncate to model maximum.
        let mut token_ids = encoding.get_ids().to_vec();
        if token_ids.len() > MAX_TOKENS + 2 {
            // Keep [CLS] ... truncated ... [SEP]
            let sep = token_ids[token_ids.len() - 1];
            token_ids.truncate(MAX_TOKENS + 1);
            token_ids.push(sep);
        }
        let token_type_ids = vec![0u32; token_ids.len()];
        let seq_len = token_ids.len();

        let token_ids_t = Tensor::new(token_ids.as_slice(), device)?.unsqueeze(0)?;
        let token_type_ids_t = Tensor::new(token_type_ids.as_slice(), device)?.unsqueeze(0)?;

        // Forward pass — returns (batch, seq_len, hidden_size).
        let output = model.forward(&token_ids_t, &token_type_ids_t, None)?;

        // CLS pooling: take the first token's hidden state.
        let cls = output.narrow(1, 0, 1)?.squeeze(1)?; // (1, hidden_size)

        // L2 normalize.
        let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = cls.broadcast_div(&norm)?;

        let embedding: Vec<f32> = normalized.squeeze(0)?.to_vec1()?;
        let embedding_dims = embedding.len();
        all_embeddings.push(embedding);

        tracing::trace!(
            target: "embeddings.candle",
            "[embeddings] embedded text ({seq_len} tokens) → {embedding_dims} dims",
        );
    }

    Ok(all_embeddings)
}

#[async_trait]
impl EmbeddingProvider for CandleEmbedding {
    fn name(&self) -> &str {
        "candle"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    /// Generates embeddings using a blocking task to prevent executor starvation.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let items: Vec<String> = texts.iter().map(|t| (*t).to_string()).collect();
        let state = Arc::clone(&self.state);
        let hf_repo = self.hf_repo.clone();
        let model_name = self.model_name.clone();

        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Vec<f32>>> {
            let mut guard = state.lock();

            // Lazy initialization.
            if matches!(*guard, ModelState::Uninitialized) {
                tracing::debug!(
                    target: "embeddings.candle",
                    "[embeddings] initializing candle model: {model_name} ({hf_repo})"
                );

                #[cfg(feature = "metal")]
                let device = Device::new_metal(0).unwrap_or(Device::Cpu);
                #[cfg(not(feature = "metal"))]
                let device = Device::Cpu;

                match load_model(&hf_repo, &device) {
                    Ok((model, tokenizer)) => {
                        tracing::info!(
                            target: "embeddings.candle",
                            "[embeddings] candle model ready: {model_name}"
                        );
                        *guard = ModelState::Ready {
                            model,
                            tokenizer,
                            device,
                        };
                    }
                    Err(err) => {
                        let message =
                            format!("candle model init failed for {model_name}: {err}");
                        tracing::error!(
                            target: "embeddings.candle",
                            "[embeddings] {message}"
                        );
                        *guard = ModelState::Failed(message);
                    }
                }
            }

            match &*guard {
                ModelState::Ready {
                    model,
                    tokenizer,
                    device,
                } => embed_batch(model, tokenizer, device, &items),
                ModelState::Failed(msg) => Err(anyhow::anyhow!(msg.clone())),
                ModelState::Uninitialized => {
                    Err(anyhow::anyhow!("candle provider did not initialize"))
                }
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("candle embed task join failed: {e}"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_known_models() {
        assert_eq!(resolve_hf_repo("BGESmallENV15"), "BAAI/bge-small-en-v1.5");
        assert_eq!(resolve_hf_repo("BGEBaseENV15"), "BAAI/bge-base-en-v1.5");
        assert_eq!(
            resolve_hf_repo("AllMiniLmL6V2"),
            "sentence-transformers/all-MiniLM-L6-v2"
        );
    }

    #[test]
    fn resolve_unknown_model_passes_through() {
        assert_eq!(
            resolve_hf_repo("my-org/custom-model"),
            "my-org/custom-model"
        );
    }

    #[test]
    fn new_defaults() {
        let p = CandleEmbedding::new("", 0);
        assert_eq!(p.model_name, DEFAULT_MODEL_NAME);
        assert_eq!(p.dims, DEFAULT_DIMENSIONS);
        assert_eq!(p.hf_repo, DEFAULT_HF_REPO);
    }

    #[test]
    fn new_custom() {
        let p = CandleEmbedding::new("BGEBaseENV15", 768);
        assert_eq!(p.model_name, "BGEBaseENV15");
        assert_eq!(p.dims, 768);
        assert_eq!(p.hf_repo, "BAAI/bge-base-en-v1.5");
    }

    #[test]
    fn name_is_candle() {
        let p = CandleEmbedding::new("BGESmallENV15", 384);
        assert_eq!(p.name(), "candle");
    }

    #[tokio::test]
    async fn empty_input_returns_empty() {
        let p = CandleEmbedding::new("BGESmallENV15", 384);
        let result = p.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }
}
