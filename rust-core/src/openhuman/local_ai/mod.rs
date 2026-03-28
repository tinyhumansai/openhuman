use crate::openhuman::config::Config;
use mistralrs::{
    GgufModelBuilder, Model, PagedAttentionMetaBuilder, RequestBuilder, TextMessageRole,
    TextModelBuilder,
};
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
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub download_speed_bps: Option<u64>,
    pub eta_seconds: Option<u64>,
    pub warning: Option<String>,
    pub model_path: Option<String>,
    pub active_backend: String,
    pub backend_reason: Option<String>,
    pub last_latency_ms: Option<u64>,
    pub prompt_toks_per_sec: Option<f32>,
    pub gen_toks_per_sec: Option<f32>,
}

impl LocalAiStatus {
    fn disabled(config: &Config) -> Self {
        Self {
            state: "disabled".to_string(),
            model_id: config.local_ai.model_id.clone(),
            provider: config.local_ai.provider.clone(),
            download_progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            download_speed_bps: None,
            eta_seconds: None,
            warning: None,
            model_path: None,
            active_backend: "cpu".to_string(),
            backend_reason: None,
            last_latency_ms: None,
            prompt_toks_per_sec: None,
            gen_toks_per_sec: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub text: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy)]
enum RuntimeBackend {
    Cpu,
    Metal,
    Cuda,
    Vulkan,
}

impl RuntimeBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Metal => "metal",
            Self::Cuda => "cuda",
            Self::Vulkan => "vulkan",
        }
    }
}

fn supports_backend(backend: RuntimeBackend) -> bool {
    match backend {
        RuntimeBackend::Cpu => true,
        RuntimeBackend::Metal => cfg!(feature = "local-ai-metal"),
        RuntimeBackend::Cuda => cfg!(feature = "local-ai-cuda"),
        // mistralrs v0.7 does not expose a vulkan feature in this workspace yet.
        RuntimeBackend::Vulkan => false,
    }
}

fn resolve_backend(config: &Config) -> (RuntimeBackend, Option<String>) {
    let preference = config
        .local_ai
        .backend_preference
        .trim()
        .to_ascii_lowercase();
    let requested = match preference.as_str() {
        "metal" => RuntimeBackend::Metal,
        "cuda" => RuntimeBackend::Cuda,
        "vulkan" => RuntimeBackend::Vulkan,
        "cpu" => RuntimeBackend::Cpu,
        _ => RuntimeBackend::Cpu,
    };

    if preference == "auto" || preference.is_empty() {
        if supports_backend(RuntimeBackend::Metal) {
            return (RuntimeBackend::Metal, None);
        }
        if supports_backend(RuntimeBackend::Cuda) {
            return (RuntimeBackend::Cuda, None);
        }
        if supports_backend(RuntimeBackend::Vulkan) {
            return (RuntimeBackend::Vulkan, None);
        }
        return (
            RuntimeBackend::Cpu,
            Some("No GPU runtime is compiled in this build; using CPU.".to_string()),
        );
    }

    if supports_backend(requested) {
        return (requested, None);
    }

    (
        RuntimeBackend::Cpu,
        Some(format!(
            "Requested backend `{}` is unavailable in this build; using CPU.",
            preference
        )),
    )
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
                downloaded_bytes: None,
                total_bytes: None,
                download_speed_bps: None,
                eta_seconds: None,
                warning: None,
                model_path: Some(model_artifact_path(config).display().to_string()),
                active_backend: "cpu".to_string(),
                backend_reason: None,
                last_latency_ms: None,
                prompt_toks_per_sec: None,
                gen_toks_per_sec: None,
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
        status.downloaded_bytes = None;
        status.total_bytes = None;
        status.download_speed_bps = None;
        status.eta_seconds = None;
        status.warning = None;
        status.active_backend = "cpu".to_string();
        status.backend_reason = None;
        status.last_latency_ms = None;
        status.prompt_toks_per_sec = None;
        status.gen_toks_per_sec = None;
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
            if self.effective_download_url(config).is_some() {
                {
                    let mut status = self.status.lock();
                    status.state = "downloading".to_string();
                    status.warning =
                        Some("Downloading local model in background (mistral.rs)".to_string());
                    status.download_progress = Some(0.0);
                    status.downloaded_bytes = Some(0);
                    status.total_bytes = None;
                    status.download_speed_bps = None;
                    status.eta_seconds = None;
                }
                let download_result = self.download_model(config, &artifact_path).await;
                if let Err(err) = download_result {
                    if let Some(repo) = self
                        .effective_download_url(config)
                        .and_then(|url| parse_huggingface_repo(&url))
                    {
                        {
                            let mut status = self.status.lock();
                            status.state = "loading".to_string();
                            status.warning = Some(
                                "GGUF unavailable; loading model directly from Hugging Face"
                                    .to_string(),
                            );
                            status.download_progress = None;
                            status.downloaded_bytes = None;
                            status.total_bytes = None;
                            status.download_speed_bps = None;
                            status.eta_seconds = None;
                        }

                        let loaded_model = self.load_hf_text_model(&repo).await;
                        match loaded_model {
                            Ok(model) => {
                                *self.model.write() = Some(model.clone());
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
                                        status.warning = Some(format!(
                                            "Loaded from Hugging Face repo: {}",
                                            repo
                                        ));
                                        status.download_progress = None;
                                        status.downloaded_bytes = None;
                                        status.total_bytes = None;
                                        status.download_speed_bps = None;
                                        status.eta_seconds = None;
                                        status.model_path = Some(format!("hf://{repo}"));
                                        return;
                                    }
                                    Err(warmup_err) => {
                                        let mut status = self.status.lock();
                                        status.state = "degraded".to_string();
                                        status.warning = Some(format!(
                                            "HF model warmup failed after GGUF download failure ({err}): {warmup_err}"
                                        ));
                                        status.download_progress = None;
                                        status.downloaded_bytes = None;
                                        status.total_bytes = None;
                                        status.download_speed_bps = None;
                                        status.eta_seconds = None;
                                        *self.model.write() = None;
                                        return;
                                    }
                                }
                            }
                            Err(load_err) => {
                                let mut status = self.status.lock();
                                status.state = "degraded".to_string();
                                status.warning = Some(format!(
                                    "Local model download failed ({err}); HF fallback failed: {load_err}"
                                ));
                                status.download_progress = None;
                                status.downloaded_bytes = None;
                                status.total_bytes = None;
                                status.download_speed_bps = None;
                                status.eta_seconds = None;
                                return;
                            }
                        }
                    } else {
                        let mut status = self.status.lock();
                        status.state = "degraded".to_string();
                        status.warning = Some(format!("Local model download failed: {err}"));
                        status.download_progress = None;
                        status.downloaded_bytes = None;
                        status.total_bytes = None;
                        status.download_speed_bps = None;
                        status.eta_seconds = None;
                        return;
                    }
                }
            } else {
                let mut status = self.status.lock();
                status.state = "degraded".to_string();
                status.warning = Some(
                    "Local model artifact missing. Configure local_ai.download_url or place GGUF file."
                        .to_string(),
                );
                status.download_progress = None;
                status.downloaded_bytes = None;
                status.total_bytes = None;
                status.download_speed_bps = None;
                status.eta_seconds = None;
                return;
            }
        }

        {
            let mut status = self.status.lock();
            status.state = "loading".to_string();
            status.warning = Some("Loading model with mistral.rs".to_string());
            status.download_progress = None;
            status.downloaded_bytes = None;
            status.total_bytes = None;
            status.download_speed_bps = None;
            status.eta_seconds = None;
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
                        status.download_progress = None;
                        status.downloaded_bytes = None;
                        status.total_bytes = None;
                        status.download_speed_bps = None;
                        status.eta_seconds = None;
                        status.model_path = Some(artifact_path.display().to_string());
                    }
                    Err(err) => {
                        let mut status = self.status.lock();
                        status.state = "degraded".to_string();
                        status.warning = Some(format!("Local model warmup failed: {err}"));
                        status.download_progress = None;
                        status.downloaded_bytes = None;
                        status.total_bytes = None;
                        status.download_speed_bps = None;
                        status.eta_seconds = None;
                        status.model_path = Some(artifact_path.display().to_string());
                        *self.model.write() = None;
                    }
                }
            }
            Err(err) => {
                let mut status = self.status.lock();
                status.state = "degraded".to_string();
                status.warning = Some(format!("Local model load failed: {err}"));
                status.download_progress = None;
                status.downloaded_bytes = None;
                status.total_bytes = None;
                status.download_speed_bps = None;
                status.eta_seconds = None;
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

    pub async fn prompt(
        &self,
        config: &Config,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let system = if no_think {
            "You are a concise assistant. Return only the final answer. Do not include reasoning or chain-of-thought."
        } else {
            "You are a helpful assistant."
        };
        self.inference(config, system, prompt, max_tokens).await
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
        let raw_url = self
            .effective_download_url(config)
            .ok_or_else(|| "download url not configured".to_string())?;
        let url = self.resolve_download_url(&raw_url).await?;

        if let Some(parent) = artifact_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("failed to create model directory: {e}"))?;
        }

        let tmp_path = artifact_path.with_extension("part");
        let client = reqwest::Client::new();
        let mut response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("download request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("download failed with status {}", response.status()));
        }

        let total = response.content_length();
        {
            let mut status = self.status.lock();
            status.downloaded_bytes = Some(0);
            status.total_bytes = total;
            status.download_speed_bps = Some(0);
            status.eta_seconds = None;
            status.download_progress = total.map(|_| 0.0);
        }
        let mut downloaded: u64 = 0;
        let started_at = std::time::Instant::now();
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
            let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
            let speed_bps = (downloaded as f64 / elapsed).round().max(0.0) as u64;
            let eta_seconds = total.and_then(|total_bytes| {
                if speed_bps == 0 || downloaded >= total_bytes {
                    None
                } else {
                    Some((total_bytes.saturating_sub(downloaded)) / speed_bps.max(1))
                }
            });
            let mut status = self.status.lock();
            status.downloaded_bytes = Some(downloaded);
            status.total_bytes = total;
            status.download_speed_bps = Some(speed_bps);
            status.eta_seconds = eta_seconds;
            status.download_progress =
                total.map(|total_bytes| (downloaded as f32 / total_bytes as f32).clamp(0.0, 1.0));
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

    async fn load_hf_text_model(&self, repo: &str) -> Result<Arc<Model>, String> {
        let mut builder = TextModelBuilder::new(repo.to_string());
        if log::log_enabled!(log::Level::Debug) {
            builder = builder.with_logging();
        }

        let model = builder
            .build()
            .await
            .map_err(|e| format!("mistral.rs HF model load failed ({repo}): {e}"))?;
        Ok(Arc::new(model))
    }

    fn effective_download_url(&self, config: &Config) -> Option<String> {
        if let Some(url) = config.local_ai.download_url.as_deref() {
            if !url.trim().is_empty() {
                return Some(url.to_string());
            }
        }

        // Backward-compatible fallback for older configs that persisted `download_url = null`.
        if config
            .local_ai
            .model_id
            .trim()
            .eq_ignore_ascii_case("qwen3-1.7b")
        {
            return Some("https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q8_0.gguf?download=true".to_string());
        }
        None
    }

    async fn resolve_download_url(&self, raw_url: &str) -> Result<String, String> {
        let repo = parse_huggingface_repo(raw_url);
        if repo.is_none() {
            return Ok(raw_url.to_string());
        }
        let repo = repo.expect("checked is_some");

        let client = reqwest::Client::new();
        let api_url = format!("https://huggingface.co/api/models/{repo}");
        let payload = client
            .get(&api_url)
            .send()
            .await
            .map_err(|e| format!("huggingface model metadata request failed: {e}"))?;
        if !payload.status().is_success() {
            return Err(format!(
                "huggingface model metadata request failed with status {}",
                payload.status()
            ));
        }
        let model: HuggingFaceModel = payload
            .json()
            .await
            .map_err(|e| format!("huggingface model metadata parse failed: {e}"))?;

        let filename = select_best_gguf_file(&model.siblings)
            .ok_or_else(|| "no .gguf artifact found in huggingface repository".to_string())?;

        Ok(format!(
            "https://huggingface.co/{repo}/resolve/main/{}",
            urlencoding::encode(&filename)
        ))
    }
}

#[derive(Debug, Deserialize)]
struct HuggingFaceModel {
    #[serde(default)]
    siblings: Vec<HuggingFaceSibling>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceSibling {
    rfilename: String,
}

fn parse_huggingface_repo(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let without_prefix = trimmed
        .strip_prefix("https://huggingface.co/")
        .or_else(|| trimmed.strip_prefix("http://huggingface.co/"))?;

    if without_prefix.contains("/resolve/") || without_prefix.contains("/blob/") {
        return None;
    }
    if without_prefix.to_ascii_lowercase().ends_with(".gguf") {
        return None;
    }

    let mut parts = without_prefix.split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(format!("{owner}/{repo}"))
}

fn select_best_gguf_file(siblings: &[HuggingFaceSibling]) -> Option<String> {
    fn score(name: &str) -> i32 {
        let lower = name.to_ascii_lowercase();
        let mut s = 0;
        if lower.ends_with(".gguf") {
            s += 100;
        }
        if lower.contains("q4_k_m") {
            s += 40;
        } else if lower.contains("q4") {
            s += 20;
        }
        if lower.contains("instruct") {
            s += 10;
        }
        s
    }

    siblings
        .iter()
        .filter(|f| f.rfilename.to_ascii_lowercase().ends_with(".gguf"))
        .max_by_key(|f| score(&f.rfilename))
        .map(|f| f.rfilename.clone())
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
