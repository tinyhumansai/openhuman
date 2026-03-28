use crate::openhuman::config::Config;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_OLLAMA_MODEL: &str = "qwen2.5:1.5b";

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
            model_id: LocalAiService::effective_model_id(config),
            provider: "ollama".to_string(),
            download_progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            download_speed_bps: None,
            eta_seconds: None,
            warning: None,
            model_path: None,
            active_backend: "ollama".to_string(),
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

pub struct LocalAiService {
    status: parking_lot::Mutex<LocalAiStatus>,
    bootstrap_lock: tokio::sync::Mutex<()>,
    last_memory_summary_at: parking_lot::Mutex<Option<std::time::Instant>>,
    http: reqwest::Client,
}

impl LocalAiService {
    fn effective_model_id(config: &Config) -> String {
        let raw = config.local_ai.model_id.trim();
        if raw.is_empty() {
            return DEFAULT_OLLAMA_MODEL.to_string();
        }
        let lower = raw.to_ascii_lowercase();
        if lower.ends_with(".gguf")
            || lower.contains("huggingface.co/")
            || lower == "qwen3-1.7b"
            || lower == "qwen2.5-1.5b-instruct"
        {
            return DEFAULT_OLLAMA_MODEL.to_string();
        }
        raw.to_string()
    }

    fn new(config: &Config) -> Self {
        let model_id = Self::effective_model_id(config);
        Self {
            status: parking_lot::Mutex::new(LocalAiStatus {
                state: "idle".to_string(),
                model_id: model_id.clone(),
                provider: "ollama".to_string(),
                download_progress: None,
                downloaded_bytes: None,
                total_bytes: None,
                download_speed_bps: None,
                eta_seconds: None,
                warning: None,
                model_path: Some(format!("ollama://{}", model_id)),
                active_backend: "ollama".to_string(),
                backend_reason: None,
                last_latency_ms: None,
                prompt_toks_per_sec: None,
                gen_toks_per_sec: None,
            }),
            bootstrap_lock: tokio::sync::Mutex::new(()),
            last_memory_summary_at: parking_lot::Mutex::new(None),
            http: reqwest::Client::new(),
        }
    }

    pub fn status(&self) -> LocalAiStatus {
        self.status.lock().clone()
    }

    pub fn reset_to_idle(&self, config: &Config) {
        let model_id = Self::effective_model_id(config);
        let mut status = self.status.lock();
        status.state = "idle".to_string();
        status.model_id = model_id.clone();
        status.provider = "ollama".to_string();
        status.download_progress = None;
        status.downloaded_bytes = None;
        status.total_bytes = None;
        status.download_speed_bps = None;
        status.eta_seconds = None;
        status.warning = None;
        status.model_path = Some(format!("ollama://{}", model_id));
        status.active_backend = "ollama".to_string();
        status.backend_reason = None;
        status.last_latency_ms = None;
        status.prompt_toks_per_sec = None;
        status.gen_toks_per_sec = None;
    }

    pub async fn bootstrap(&self, config: &Config) {
        let _guard = self.bootstrap_lock.lock().await;
        if !config.local_ai.enabled {
            *self.status.lock() = LocalAiStatus::disabled(config);
            return;
        }

        if matches!(self.status.lock().state.as_str(), "ready") {
            return;
        }

        {
            let mut status = self.status.lock();
            status.state = "loading".to_string();
            status.warning = Some("Connecting to local Ollama runtime".to_string());
            status.download_progress = None;
            status.downloaded_bytes = None;
            status.total_bytes = None;
            status.download_speed_bps = None;
            status.eta_seconds = None;
            status.active_backend = "ollama".to_string();
            status.backend_reason = Some("Inference delegated to Ollama runtime".to_string());
            status.model_path = Some(format!("ollama://{}", Self::effective_model_id(config)));
        }

        if let Err(err) = self.ensure_ollama_server().await {
            let mut status = self.status.lock();
            status.state = "degraded".to_string();
            status.warning = Some(err);
            return;
        }

        if let Err(err) = self.ensure_model_available(config).await {
            let mut status = self.status.lock();
            status.state = "degraded".to_string();
            status.warning = Some(err);
            return;
        }

        let mut status = self.status.lock();
        status.state = "ready".to_string();
        status.warning = None;
        status.download_progress = None;
        status.downloaded_bytes = None;
        status.total_bytes = None;
        status.download_speed_bps = None;
        status.eta_seconds = None;
        status.model_path = Some(format!("ollama://{}", Self::effective_model_id(config)));
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
            "Summarize this text in concise bullet points. Preserve decisions and commitments.\\n\\n{}",
            text
        );
        self.inference(config, system, &prompt, max_tokens.or(Some(128)), true)
            .await
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
        self.inference(config, system, prompt, max_tokens.or(Some(160)), no_think)
            .await
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
            "Given this conversation context, produce up to {} short suggested next user prompts. Return one prompt per line with no numbering.\\n\\n{}",
            config.local_ai.max_suggestions.max(1),
            context
        );
        let raw = self.inference(config, system, &prompt, Some(96), true).await?;
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
        no_think: bool,
    ) -> Result<String, String> {
        if !matches!(self.status.lock().state.as_str(), "ready") {
            self.bootstrap(config).await;
        }

        let started = std::time::Instant::now();
        let mut combined_prompt = String::new();
        if no_think {
            combined_prompt.push_str("Respond with only the final answer. No reasoning.\\n\\n");
        }
        combined_prompt.push_str(prompt);

        let body = OllamaGenerateRequest {
            model: Self::effective_model_id(config),
            prompt: combined_prompt,
            system: Some(system.to_string()),
            stream: false,
            options: Some(OllamaGenerateOptions {
                temperature: Some(0.2),
                top_k: Some(40),
                top_p: Some(0.9),
                num_predict: max_tokens.map(|v| v as i32),
            }),
        };

        let response = self
            .http
            .post(format!("{OLLAMA_BASE_URL}/api/generate"))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("ollama request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("ollama request failed with status {}", response.status()));
        }

        let payload: OllamaGenerateResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama response parse failed: {e}"))?;

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let prompt_tps = payload
            .prompt_eval_count
            .zip(payload.prompt_eval_duration)
            .and_then(|(count, dur_ns)| ns_to_tps(count as f32, dur_ns));
        let gen_tps = payload
            .eval_count
            .zip(payload.eval_duration)
            .and_then(|(count, dur_ns)| ns_to_tps(count as f32, dur_ns));

        {
            let mut status = self.status.lock();
            status.state = "ready".to_string();
            status.last_latency_ms = Some(elapsed_ms);
            status.prompt_toks_per_sec = prompt_tps;
            status.gen_toks_per_sec = gen_tps;
            status.warning = None;
        }

        if payload.response.trim().is_empty() {
            Err("ollama returned empty content".to_string())
        } else {
            Ok(payload.response)
        }
    }

    async fn ensure_ollama_server(&self) -> Result<(), String> {
        if self.ollama_healthy().await {
            return Ok(());
        }

        if let Err(err) = tokio::process::Command::new("ollama")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
        {
            return Err(format!(
                "Ollama is not installed or not on PATH ({err}). Install Ollama to use local models."
            ));
        }

        let _ = tokio::process::Command::new("ollama")
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        for _ in 0..20 {
            if self.ollama_healthy().await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        Err("Ollama runtime is not reachable at http://127.0.0.1:11434. Start `ollama serve` and retry.".to_string())
    }

    async fn ollama_healthy(&self) -> bool {
        self.http
            .get(format!("{OLLAMA_BASE_URL}/api/tags"))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    async fn ensure_model_available(&self, config: &Config) -> Result<(), String> {
        let model_id = Self::effective_model_id(config);
        if self.has_model(&model_id).await? {
            return Ok(());
        }

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!(
                "Pulling model `{}` from Ollama library",
                model_id
            ));
            status.download_progress = Some(0.0);
            status.downloaded_bytes = Some(0);
            status.total_bytes = None;
            status.download_speed_bps = Some(0);
            status.eta_seconds = None;
        }

        let started_at = std::time::Instant::now();
        let response = self
            .http
            .post(format!("{OLLAMA_BASE_URL}/api/pull"))
            .json(&OllamaPullRequest {
                name: model_id.clone(),
                stream: true,
            })
            .send()
            .await
            .map_err(|e| format!("ollama pull request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("ollama pull failed with status {}", response.status()));
        }

        let mut stream = response.bytes_stream();
        let mut pending = String::new();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| format!("ollama pull stream error: {e}"))?;
            pending.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = pending.find('\n') {
                let line = pending[..pos].trim().to_string();
                pending = pending[pos + 1..].to_string();
                if line.is_empty() {
                    continue;
                }
                let event: OllamaPullEvent = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(err) = event.error {
                    return Err(format!("ollama pull error: {err}"));
                }

                let completed = event.completed.unwrap_or(0);
                let total = event.total;
                let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                let speed_bps = (completed as f64 / elapsed).round().max(0.0) as u64;
                let eta_seconds = total.and_then(|t| {
                    if completed >= t || speed_bps == 0 {
                        None
                    } else {
                        Some((t.saturating_sub(completed)) / speed_bps.max(1))
                    }
                });

                let mut status = self.status.lock();
                status.downloaded_bytes = Some(completed);
                status.total_bytes = total;
                status.download_speed_bps = Some(speed_bps);
                status.eta_seconds = eta_seconds;
                status.download_progress = total
                    .map(|t| (completed as f32 / t as f32).clamp(0.0, 1.0))
                    .or(Some(0.0));
            }
        }

        if !self.has_model(&model_id).await? {
            return Err(format!(
                "ollama pull finished but model `{}` was not found",
                model_id
            ));
        }

        Ok(())
    }

    async fn has_model(&self, model: &str) -> Result<bool, String> {
        let response = self
            .http
            .get(format!("{OLLAMA_BASE_URL}/api/tags"))
            .send()
            .await
            .map_err(|e| format!("ollama tags request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("ollama tags failed with status {}", response.status()));
        }
        let payload: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama tags parse failed: {e}"))?;

        let target = model.to_ascii_lowercase();
        Ok(payload.models.iter().any(|m| {
            let name = m.name.to_ascii_lowercase();
            name == target || name.starts_with(&(target.clone() + ":"))
        }))
    }
}

#[derive(Debug, Serialize)]
struct OllamaPullRequest {
    name: String,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaPullEvent {
    #[allow(dead_code)]
    status: Option<String>,
    total: Option<u64>,
    completed: Option<u64>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModelTag>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelTag {
    name: String,
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest {
    model: String,
    prompt: String,
    system: Option<String>,
    stream: bool,
    options: Option<OllamaGenerateOptions>,
}

#[derive(Debug, Serialize)]
struct OllamaGenerateOptions {
    temperature: Option<f32>,
    top_k: Option<u32>,
    top_p: Option<f32>,
    num_predict: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
    #[allow(dead_code)]
    done: Option<bool>,
    #[allow(dead_code)]
    total_duration: Option<u64>,
    prompt_eval_count: Option<u32>,
    prompt_eval_duration: Option<u64>,
    eval_count: Option<u32>,
    eval_duration: Option<u64>,
}

fn ns_to_tps(tokens: f32, duration_ns: u64) -> Option<f32> {
    if duration_ns == 0 || tokens <= 0.0 {
        return None;
    }
    let seconds = duration_ns as f32 / 1_000_000_000.0;
    if seconds <= 0.0 {
        None
    } else {
        Some(tokens / seconds)
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
        .join(LocalAiService::effective_model_id(config).replace(':', "-") + ".ollama")
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
