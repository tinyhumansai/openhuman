use crate::openhuman::config::Config;
use mistralrs::{GgufModelBuilder, Model, RequestBuilder, TextMessageRole};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiStatus {
    pub state: String,
    pub model_id: String,
    pub provider: String,
    pub download_progress: Option<f32>,
    pub warning: Option<String>,
    pub model_path: Option<String>,
}

impl LocalAiStatus {
    fn disabled(config: &Config) -> Self {
        Self {
            state: "disabled".to_string(),
            model_id: config.local_ai.model_id.clone(),
            provider: config.local_ai.provider.clone(),
            download_progress: None,
            warning: None,
            model_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub text: String,
    pub confidence: f32,
}

pub struct LocalAiService {
    status: parking_lot::Mutex<LocalAiStatus>,
    model: parking_lot::RwLock<Option<Arc<Model>>>,
    bootstrap_lock: tokio::sync::Mutex<()>,
    last_memory_summary_at: parking_lot::Mutex<Option<std::time::Instant>>,
}

impl LocalAiService {
    fn new(config: &Config) -> Self {
        Self {
            status: parking_lot::Mutex::new(LocalAiStatus {
                state: "idle".to_string(),
                model_id: config.local_ai.model_id.clone(),
                provider: config.local_ai.provider.clone(),
                download_progress: None,
                warning: None,
                model_path: Some(model_artifact_path(config).display().to_string()),
            }),
            model: parking_lot::RwLock::new(None),
            bootstrap_lock: tokio::sync::Mutex::new(()),
            last_memory_summary_at: parking_lot::Mutex::new(None),
        }
    }

    pub fn status(&self) -> LocalAiStatus {
        self.status.lock().clone()
    }

    pub fn reset_to_idle(&self, config: &Config) {
        let mut status = self.status.lock();
        status.state = "idle".to_string();
        status.model_id = config.local_ai.model_id.clone();
        status.provider = config.local_ai.provider.clone();
        status.download_progress = None;
        status.warning = None;
        *self.model.write() = None;
    }

    pub async fn bootstrap(&self, config: &Config) {
        let _guard = self.bootstrap_lock.lock().await;
        if !config.local_ai.enabled {
            *self.status.lock() = LocalAiStatus::disabled(config);
            return;
        }

        if matches!(self.status.lock().state.as_str(), "ready") && self.model.read().is_some() {
            return;
        }

        let artifact_path = model_artifact_path(config);
        if !artifact_path.exists() {
            if config.local_ai.download_url.is_some() {
                {
                    let mut status = self.status.lock();
                    status.state = "downloading".to_string();
                    status.warning =
                        Some("Downloading local model in background (mistral.rs)".to_string());
                    status.download_progress = Some(0.0);
                }
                if let Err(err) = self.download_model(config, &artifact_path).await {
                    let mut status = self.status.lock();
                    status.state = "degraded".to_string();
                    status.warning = Some(format!("Local model download failed: {err}"));
                    status.download_progress = None;
                    return;
                }
            } else {
                let mut status = self.status.lock();
                status.state = "degraded".to_string();
                status.warning = Some(
                    "Local model artifact missing. Configure local_ai.download_url or place GGUF file."
                        .to_string(),
                );
                status.download_progress = None;
                return;
            }
        }

        {
            let mut status = self.status.lock();
            status.state = "loading".to_string();
            status.warning = Some("Loading model with mistral.rs".to_string());
            status.download_progress = None;
        }

        match self.load_mistral_model(config, &artifact_path).await {
            Ok(model) => {
                *self.model.write() = Some(model.clone());

                // Warmup generation once so the runtime is fully initialized.
                match self
                    .run_request(
                        &model,
                        "You are a local model warmup assistant.",
                        "Reply with exactly: ok",
                        Some(8),
                    )
                    .await
                {
                    Ok(_) => {
                        let mut status = self.status.lock();
                        status.state = "ready".to_string();
                        status.warning = None;
                        status.model_path = Some(artifact_path.display().to_string());
                    }
                    Err(err) => {
                        let mut status = self.status.lock();
                        status.state = "degraded".to_string();
                        status.warning = Some(format!("Local model warmup failed: {err}"));
                        status.model_path = Some(artifact_path.display().to_string());
                        *self.model.write() = None;
                    }
                }
            }
            Err(err) => {
                let mut status = self.status.lock();
                status.state = "degraded".to_string();
                status.warning = Some(format!("Local model load failed: {err}"));
                status.model_path = Some(artifact_path.display().to_string());
            }
        }
    }

    pub async fn summarize(
        &self,
        config: &Config,
        text: &str,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let system = "You summarize internal assistant context. Keep concise bullet points.";
        let prompt = format!(
            "Summarize this text in concise bullet points. Preserve decisions and commitments.\n\n{}",
            text
        );
        self.inference(config, system, &prompt, max_tokens).await
    }

    pub async fn suggest_questions(
        &self,
        config: &Config,
        context: &str,
    ) -> Result<Vec<Suggestion>, String> {
        if !config.local_ai.enabled {
            return Ok(Vec::new());
        }
        let system = "You create short suggested user prompts.";
        let prompt = format!(
            "Given this conversation context, produce up to {} short suggested next user prompts. \
Return one prompt per line with no numbering.\n\n{}",
            config.local_ai.max_suggestions.max(1),
            context
        );
        let raw = self.inference(config, system, &prompt, Some(128)).await?;
        Ok(parse_suggestions(
            &raw,
            config.local_ai.max_suggestions.max(1),
        ))
    }

    pub fn should_run_memory_autosummary(&self, config: &Config) -> bool {
        let mut guard = self.last_memory_summary_at.lock();
        let now = std::time::Instant::now();
        match *guard {
            Some(last)
                if now.duration_since(last).as_millis()
                    < u128::from(config.local_ai.autosummary_debounce_ms) =>
            {
                false
            }
            _ => {
                *guard = Some(now);
                true
            }
        }
    }

    async fn inference(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if self.model.read().is_none() {
            self.bootstrap(config).await;
        }
        let model = self
            .model
            .read()
            .as_ref()
            .cloned()
            .ok_or_else(|| "local model not ready".to_string())?;
        self.run_request(&model, system, prompt, max_tokens).await
    }

    async fn run_request(
        &self,
        model: &Model,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        let mut request = RequestBuilder::new()
            .add_message(TextMessageRole::System, system)
            .add_message(TextMessageRole::User, prompt)
            .set_sampler_temperature(0.2)
            .with_truncate_sequence(true);
        if let Some(limit) = max_tokens {
            request = request.set_sampler_max_len(limit as usize);
        }

        let response = model
            .send_chat_request(request)
            .await
            .map_err(|e| format!("mistral.rs request failed: {e}"))?;
        let content = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .unwrap_or_default();
        if content.trim().is_empty() {
            Err("mistral.rs returned empty content".to_string())
        } else {
            Ok(content)
        }
    }

    async fn load_mistral_model(
        &self,
        config: &Config,
        artifact_path: &Path,
    ) -> Result<Arc<Model>, String> {
        let model_dir = artifact_path
            .parent()
            .ok_or_else(|| "model path missing parent directory".to_string())?;
        let file_name = artifact_path
            .file_name()
            .and_then(|f| f.to_str())
            .ok_or_else(|| "invalid model filename".to_string())?;

        let mut builder =
            GgufModelBuilder::new(model_dir.to_string_lossy().to_string(), vec![file_name]);
        if log::log_enabled!(log::Level::Debug) {
            builder = builder.with_logging();
        }

        let model = builder.build().await.map_err(|e| {
            format!(
                "mistral.rs GGUF load failed ({}): {e}",
                config.local_ai.model_id
            )
        })?;
        Ok(Arc::new(model))
    }

    async fn download_model(&self, config: &Config, artifact_path: &Path) -> Result<(), String> {
        let url = config
            .local_ai
            .download_url
            .as_deref()
            .ok_or_else(|| "download url not configured".to_string())?;

        if let Some(parent) = artifact_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("failed to create model directory: {e}"))?;
        }

        let tmp_path = artifact_path.with_extension("part");
        let client = reqwest::Client::new();
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("download request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("download failed with status {}", response.status()));
        }

        let total = response.content_length();
        let mut downloaded: u64 = 0;
        let mut file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|e| format!("failed to create temporary model file: {e}"))?;

        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("download stream error: {e}"))?
        {
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("failed to write model chunk: {e}"))?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            if let Some(total_bytes) = total {
                let progress = (downloaded as f32 / total_bytes as f32).clamp(0.0, 1.0);
                self.status.lock().download_progress = Some(progress);
            }
        }

        file.flush()
            .await
            .map_err(|e| format!("failed to flush model file: {e}"))?;

        if let Some(expected) = config.local_ai.checksum_sha256.as_deref() {
            verify_sha256(&tmp_path, expected).await?;
        }

        tokio::fs::rename(&tmp_path, artifact_path)
            .await
            .map_err(|e| format!("failed to finalize model file: {e}"))?;
        Ok(())
    }
}

static LOCAL_AI: once_cell::sync::OnceCell<Arc<LocalAiService>> = once_cell::sync::OnceCell::new();

pub fn global(config: &Config) -> Arc<LocalAiService> {
    LOCAL_AI
        .get_or_init(|| Arc::new(LocalAiService::new(config)))
        .clone()
}

pub fn model_artifact_path(config: &Config) -> PathBuf {
    let root = config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config.workspace_dir.clone());
    root.join("models")
        .join("local-ai")
        .join(&config.local_ai.artifact_name)
}

fn parse_suggestions(raw: &str, limit: usize) -> Vec<Suggestion> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == '-'))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(limit)
        .map(|text| Suggestion {
            text: text.to_string(),
            confidence: 0.65,
        })
        .collect()
}

async fn verify_sha256(path: &Path, expected_hex: &str) -> Result<(), String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("failed to read downloaded model for checksum: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual = hex::encode(hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(format!(
            "checksum mismatch: expected {}, got {}",
            expected_hex, actual
        ))
    }
}
