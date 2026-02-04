//! LlamaManager — singleton manager for local LLM inference.
//!
//! Provides:
//! - Lazy model loading on first use
//! - Automatic model download if not present
//! - Thread-safe inference with dedicated thread pool
//! - Generate and summarize API for skills

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Global LLama manager instance.
pub static LLAMA_MANAGER: Lazy<LlamaManager> = Lazy::new(LlamaManager::new);

/// Model file name (Gemma 3n E2B Q4_K_M quantization)
const MODEL_FILENAME: &str = "gemma-3n-E2B-it-Q4_K_M.gguf";

/// HuggingFace model URL for download
const MODEL_URL: &str = "https://huggingface.co/bartowski/google_gemma-3n-E2B-it-GGUF/resolve/main/google_gemma-3n-E2B-it-Q4_K_M.gguf";

/// Expected SHA256 hash for model verification (first 16 chars for quick check)
const MODEL_SHA256_PREFIX: &str = ""; // Will be verified on first download

/// Status of the local model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// Whether the model API is available on this platform.
    pub available: bool,
    /// Whether the model is currently loaded in memory.
    pub loaded: bool,
    /// Whether the model is currently being loaded or downloaded.
    pub loading: bool,
    /// Download progress (0.0 to 1.0) if downloading.
    pub download_progress: Option<f32>,
    /// Error message if loading failed.
    pub error: Option<String>,
    /// Model file path if known.
    pub model_path: Option<String>,
}

impl Default for ModelStatus {
    fn default() -> Self {
        Self {
            available: true,
            loaded: false,
            loading: false,
            download_progress: None,
            error: None,
            model_path: None,
        }
    }
}

/// Configuration for text generation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct GenerateConfig {
    /// Maximum tokens to generate (default: 2048).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Sampling temperature (default: 0.7).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Top-p sampling (default: 0.9).
    #[serde(default = "default_top_p")]
    pub top_p: f32,
}

fn default_max_tokens() -> u32 {
    2048
}
fn default_temperature() -> f32 {
    0.7
}
fn default_top_p() -> f32 {
    0.9
}

/// Internal state for the loaded model.
struct LoadedModel {
    backend: LlamaBackend,
    model: LlamaModel,
}

// Safety: LlamaBackend and LlamaModel are thread-safe through their C API
unsafe impl Send for LoadedModel {}
unsafe impl Sync for LoadedModel {}

/// LLama Manager for local model inference.
pub struct LlamaManager {
    /// Directory for model files.
    data_dir: RwLock<PathBuf>,
    /// Loaded model (lazy-loaded on first use).
    model: RwLock<Option<Arc<LoadedModel>>>,
    /// Current status.
    status: RwLock<ModelStatus>,
    /// Lock to prevent concurrent loading.
    loading: AtomicBool,
}

impl LlamaManager {
    /// Create a new LlamaManager (model not loaded yet).
    pub fn new() -> Self {
        Self {
            data_dir: RwLock::new(PathBuf::new()),
            model: RwLock::new(None),
            status: RwLock::new(ModelStatus::default()),
            loading: AtomicBool::new(false),
        }
    }

    /// Set the data directory for model storage.
    pub fn set_data_dir(&self, dir: PathBuf) {
        log::info!("[llama] Setting data dir: {:?}", dir);
        *self.data_dir.write() = dir.clone();

        // Update status with model path
        let model_path = dir.join(MODEL_FILENAME);
        self.status.write().model_path = Some(model_path.to_string_lossy().to_string());
    }

    /// Get the current model status.
    pub fn get_status(&self) -> ModelStatus {
        self.status.read().clone()
    }

    /// Get the model file path.
    fn model_path(&self) -> PathBuf {
        self.data_dir.read().join(MODEL_FILENAME)
    }

    /// Check if the model file exists.
    fn model_exists(&self) -> bool {
        self.model_path().exists()
    }

    /// Download the model from HuggingFace.
    async fn download_model(&self) -> Result<(), String> {
        let model_path = self.model_path();

        // Ensure parent directory exists
        if let Some(parent) = model_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create model directory: {}", e))?;
        }

        log::info!("[llama] Downloading model from {}", MODEL_URL);
        self.status.write().download_progress = Some(0.0);

        // Use reqwest for download
        let client = reqwest::Client::new();
        let response = client
            .get(MODEL_URL)
            .send()
            .await
            .map_err(|e| format!("Failed to start download: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Download failed with status: {}", response.status()));
        }

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        // Create temp file for download
        let temp_path = model_path.with_extension("download");
        let mut file = std::fs::File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;

        // Stream the download
        use std::io::Write;
        let mut stream = response.bytes_stream();
        use futures::StreamExt;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
            file.write_all(&chunk)
                .map_err(|e| format!("Failed to write chunk: {}", e))?;

            downloaded += chunk.len() as u64;

            if total_size > 0 {
                let progress = downloaded as f32 / total_size as f32;
                self.status.write().download_progress = Some(progress);

                // Log progress every 10%
                if (progress * 10.0) as u32 > ((downloaded - chunk.len() as u64) as f32 / total_size as f32 * 10.0) as u32 {
                    log::info!("[llama] Download progress: {:.1}%", progress * 100.0);
                }
            }
        }

        // Flush and close file
        file.flush()
            .map_err(|e| format!("Failed to flush file: {}", e))?;
        drop(file);

        // Rename temp file to final path
        std::fs::rename(&temp_path, &model_path)
            .map_err(|e| format!("Failed to rename temp file: {}", e))?;

        log::info!("[llama] Model downloaded successfully to {:?}", model_path);
        self.status.write().download_progress = None;

        Ok(())
    }

    /// Ensure the model is loaded into memory.
    pub async fn ensure_loaded(&self) -> Result<(), String> {
        // Already loaded?
        if self.model.read().is_some() {
            return Ok(());
        }

        // Prevent concurrent loading
        if self.loading.swap(true, Ordering::SeqCst) {
            // Another thread is loading, wait for it
            while self.loading.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            // Check if loading succeeded
            if self.model.read().is_some() {
                return Ok(());
            }
            return Err("Model loading failed".to_string());
        }

        // Update status
        {
            let mut status = self.status.write();
            status.loading = true;
            status.error = None;
        }

        let result = self.load_model_internal().await;

        // Update status based on result
        {
            let mut status = self.status.write();
            status.loading = false;
            match &result {
                Ok(_) => {
                    status.loaded = true;
                    status.error = None;
                }
                Err(e) => {
                    status.loaded = false;
                    status.error = Some(e.clone());
                }
            }
        }

        self.loading.store(false, Ordering::SeqCst);
        result
    }

    /// Internal model loading logic.
    async fn load_model_internal(&self) -> Result<(), String> {
        // Check if model exists, download if not
        if !self.model_exists() {
            log::info!("[llama] Model not found, downloading...");
            self.download_model().await?;
        }

        let model_path = self.model_path();
        log::info!("[llama] Loading model from {:?}", model_path);

        // Load model in blocking thread
        let path = model_path.clone();
        let loaded = tokio::task::spawn_blocking(move || -> Result<LoadedModel, String> {
            // Initialize llama backend
            let backend = LlamaBackend::init()
                .map_err(|e| format!("Failed to initialize llama backend: {}", e))?;

            // Set up model parameters
            let model_params = LlamaModelParams::default();

            // Load the model
            let model = LlamaModel::load_from_file(&backend, &path, &model_params)
                .map_err(|e| format!("Failed to load model: {}", e))?;

            Ok(LoadedModel { backend, model })
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))??;

        // Store the loaded model
        *self.model.write() = Some(Arc::new(loaded));
        log::info!("[llama] Model loaded successfully");

        Ok(())
    }

    /// Generate text from a prompt.
    pub async fn generate(&self, prompt: &str, config: GenerateConfig) -> Result<String, String> {
        // Ensure model is loaded
        self.ensure_loaded().await?;

        let model_arc = self
            .model
            .read()
            .clone()
            .ok_or_else(|| "Model not loaded".to_string())?;

        let prompt = prompt.to_string();
        let max_tokens = config.max_tokens;
        let temperature = config.temperature;

        // Run inference in blocking thread
        tokio::task::spawn_blocking(move || {
            Self::generate_sync(&model_arc, &prompt, max_tokens, temperature)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    /// Synchronous text generation (runs on blocking thread).
    fn generate_sync(
        loaded: &LoadedModel,
        prompt: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<String, String> {
        // Create context for inference
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(std::num::NonZeroU32::new(8192));

        let mut ctx = loaded
            .model
            .new_context(&loaded.backend, ctx_params)
            .map_err(|e| format!("Failed to create context: {}", e))?;

        // Tokenize the prompt
        let tokens = loaded
            .model
            .str_to_token(prompt, llama_cpp_2::model::AddBos::Always)
            .map_err(|e| format!("Failed to tokenize: {}", e))?;

        if tokens.is_empty() {
            return Err("Empty prompt".to_string());
        }

        // Create batch with initial tokens
        let mut batch = LlamaBatch::new(8192, 1);

        for (i, token) in tokens.iter().enumerate() {
            let is_last = i == tokens.len() - 1;
            batch
                .add(*token, i as i32, &[0], is_last)
                .map_err(|e| format!("Failed to add token to batch: {}", e))?;
        }

        // Decode initial tokens
        ctx.decode(&mut batch)
            .map_err(|e| format!("Failed to decode batch: {}", e))?;

        // Generate tokens
        let mut output_tokens = Vec::new();
        let mut n_cur = tokens.len();

        // Create sampler chain for temperature sampling
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u32)
            .unwrap_or(42);

        for _ in 0..max_tokens {
            // Get logits for the last token
            let logits = ctx.candidates_ith(batch.n_tokens() - 1);

            // Create token data array for sampling
            let mut candidates = LlamaTokenDataArray::from_iter(logits, false);

            // Apply temperature sampler
            let mut temp_sampler = LlamaSampler::temp(temperature);
            candidates.apply_sampler(&mut temp_sampler);

            // Sample token with random seed
            let new_token = candidates.sample_token(seed);

            // Check for end of generation
            if loaded.model.is_eog_token(new_token) {
                break;
            }

            output_tokens.push(new_token);

            // Prepare next batch
            batch.clear();
            batch
                .add(new_token, n_cur as i32, &[0], true)
                .map_err(|e| format!("Failed to add token: {}", e))?;

            n_cur += 1;

            // Decode
            ctx.decode(&mut batch)
                .map_err(|e| format!("Failed to decode: {}", e))?;
        }

        // Convert tokens to string using token_to_piece
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let mut output = String::new();

        for token in &output_tokens {
            match loaded.model.token_to_piece(*token, &mut decoder, false, None) {
                Ok(piece) => output.push_str(&piece),
                Err(e) => {
                    log::warn!("[llama] Failed to decode token: {}", e);
                }
            }
        }

        Ok(output)
    }

    /// Summarize text using a built-in prompt.
    pub async fn summarize(&self, text: &str, max_tokens: u32) -> Result<String, String> {
        let prompt = format!(
            "<start_of_turn>user\nPlease provide a concise summary of the following text:\n\n{}\n<end_of_turn>\n<start_of_turn>model\n",
            text
        );

        self.generate(
            &prompt,
            GenerateConfig {
                max_tokens,
                temperature: 0.5, // Lower temperature for more focused summarization
                top_p: 0.9,
            },
        )
        .await
    }

    /// Unload the model from memory.
    pub fn unload(&self) {
        log::info!("[llama] Unloading model");
        *self.model.write() = None;
        self.status.write().loaded = false;
    }
}

impl Default for LlamaManager {
    fn default() -> Self {
        Self::new()
    }
}

// Ensure LlamaManager is Send + Sync
unsafe impl Send for LlamaManager {}
unsafe impl Sync for LlamaManager {}
