use crate::openhuman::config::Config;
use crate::openhuman::multimodal;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_OLLAMA_MODEL: &str = "qwen2.5:1.5b";
const DEFAULT_OLLAMA_VISION_MODEL: &str = "qwen2.5vl:3b";
const DEFAULT_OLLAMA_EMBED_MODEL: &str = "nomic-embed-text:latest";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiStatus {
    pub state: String,
    pub model_id: String,
    pub chat_model_id: String,
    pub vision_model_id: String,
    pub embedding_model_id: String,
    pub stt_model_id: String,
    pub tts_voice_id: String,
    pub quantization: String,
    pub vision_state: String,
    pub embedding_state: String,
    pub stt_state: String,
    pub tts_state: String,
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
            model_id: LocalAiService::effective_chat_model_id(config),
            chat_model_id: LocalAiService::effective_chat_model_id(config),
            vision_model_id: LocalAiService::effective_vision_model_id(config),
            embedding_model_id: LocalAiService::effective_embedding_model_id(config),
            stt_model_id: LocalAiService::effective_stt_model_id(config),
            tts_voice_id: LocalAiService::effective_tts_voice_id(config),
            quantization: LocalAiService::effective_quantization(config),
            vision_state: "disabled".to_string(),
            embedding_state: "disabled".to_string(),
            stt_state: "disabled".to_string(),
            tts_state: "disabled".to_string(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiAssetStatus {
    pub state: String,
    pub id: String,
    pub provider: String,
    pub path: Option<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiAssetsStatus {
    pub chat: LocalAiAssetStatus,
    pub vision: LocalAiAssetStatus,
    pub embedding: LocalAiAssetStatus,
    pub stt: LocalAiAssetStatus,
    pub tts: LocalAiAssetStatus,
    pub quantization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiEmbeddingResult {
    pub model_id: String,
    pub dimensions: usize,
    pub vectors: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiSpeechResult {
    pub text: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAiTtsResult {
    pub output_path: String,
    pub voice_id: String,
}

pub struct LocalAiService {
    status: parking_lot::Mutex<LocalAiStatus>,
    bootstrap_lock: tokio::sync::Mutex<()>,
    last_memory_summary_at: parking_lot::Mutex<Option<std::time::Instant>>,
    http: reqwest::Client,
}

impl LocalAiService {
    fn effective_chat_model_id(config: &Config) -> String {
        let raw = if !config.local_ai.chat_model_id.trim().is_empty() {
            config.local_ai.chat_model_id.trim()
        } else {
            config.local_ai.model_id.trim()
        };
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

    fn effective_vision_model_id(config: &Config) -> String {
        let raw = config.local_ai.vision_model_id.trim();
        if raw.is_empty() {
            return DEFAULT_OLLAMA_VISION_MODEL.to_string();
        }
        let lower = raw.to_ascii_lowercase();
        if lower == "qwen3-vl:2b" || lower == "qwen3-vl-2b" {
            return DEFAULT_OLLAMA_VISION_MODEL.to_string();
        }
        raw.to_string()
    }

    fn effective_embedding_model_id(config: &Config) -> String {
        let raw = config.local_ai.embedding_model_id.trim();
        if raw.is_empty() {
            return DEFAULT_OLLAMA_EMBED_MODEL.to_string();
        }
        raw.to_string()
    }

    fn effective_stt_model_id(config: &Config) -> String {
        let raw = config.local_ai.stt_model_id.trim();
        if raw.is_empty() {
            "ggml-tiny-q5_1.bin".to_string()
        } else {
            raw.to_string()
        }
    }

    fn effective_tts_voice_id(config: &Config) -> String {
        let raw = config.local_ai.tts_voice_id.trim();
        if raw.is_empty() {
            "en_US-lessac-medium".to_string()
        } else {
            raw.to_string()
        }
    }

    fn effective_quantization(config: &Config) -> String {
        let raw = config.local_ai.quantization.trim();
        if raw.is_empty() {
            "q4".to_string()
        } else {
            raw.to_ascii_lowercase()
        }
    }

    fn new(config: &Config) -> Self {
        let model_id = Self::effective_chat_model_id(config);
        let vision_model_id = Self::effective_vision_model_id(config);
        let embedding_model_id = Self::effective_embedding_model_id(config);
        Self {
            status: parking_lot::Mutex::new(LocalAiStatus {
                state: "idle".to_string(),
                model_id: model_id.clone(),
                chat_model_id: model_id.clone(),
                vision_model_id: vision_model_id.clone(),
                embedding_model_id: embedding_model_id.clone(),
                stt_model_id: Self::effective_stt_model_id(config),
                tts_voice_id: Self::effective_tts_voice_id(config),
                quantization: Self::effective_quantization(config),
                vision_state: "idle".to_string(),
                embedding_state: "idle".to_string(),
                stt_state: "idle".to_string(),
                tts_state: "idle".to_string(),
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
        let model_id = Self::effective_chat_model_id(config);
        let mut status = self.status.lock();
        status.state = "idle".to_string();
        status.model_id = model_id.clone();
        status.chat_model_id = model_id.clone();
        status.vision_model_id = Self::effective_vision_model_id(config);
        status.embedding_model_id = Self::effective_embedding_model_id(config);
        status.stt_model_id = Self::effective_stt_model_id(config);
        status.tts_voice_id = Self::effective_tts_voice_id(config);
        status.quantization = Self::effective_quantization(config);
        status.vision_state = "idle".to_string();
        status.embedding_state = "idle".to_string();
        status.stt_state = "idle".to_string();
        status.tts_state = "idle".to_string();
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
            status.model_id = Self::effective_chat_model_id(config);
            status.chat_model_id = Self::effective_chat_model_id(config);
            status.vision_model_id = Self::effective_vision_model_id(config);
            status.embedding_model_id = Self::effective_embedding_model_id(config);
            status.stt_model_id = Self::effective_stt_model_id(config);
            status.tts_voice_id = Self::effective_tts_voice_id(config);
            status.quantization = Self::effective_quantization(config);
            status.state = "loading".to_string();
            status.warning = Some("Connecting to local Ollama runtime".to_string());
            status.download_progress = None;
            status.downloaded_bytes = None;
            status.total_bytes = None;
            status.download_speed_bps = None;
            status.eta_seconds = None;
            status.active_backend = "ollama".to_string();
            status.backend_reason = Some("Inference delegated to Ollama runtime".to_string());
            status.model_path = Some(format!(
                "ollama://{}",
                Self::effective_chat_model_id(config)
            ));
        }

        if let Err(err) = self.ensure_ollama_server(config).await {
            let mut status = self.status.lock();
            status.state = "degraded".to_string();
            status.warning = Some(err);
            return;
        }

        if let Err(err) = self.ensure_models_available(config).await {
            let mut status = self.status.lock();
            status.state = "degraded".to_string();
            status.warning = Some(err);
            return;
        }

        let mut status = self.status.lock();
        status.state = "ready".to_string();
        status.vision_state = if config.local_ai.preload_vision_model {
            "ready".to_string()
        } else {
            "idle".to_string()
        };
        status.embedding_state = if config.local_ai.preload_embedding_model {
            "ready".to_string()
        } else {
            "idle".to_string()
        };
        status.stt_state = if config.local_ai.preload_stt_model {
            "loading".to_string()
        } else {
            "idle".to_string()
        };
        status.tts_state = if config.local_ai.preload_tts_voice {
            "loading".to_string()
        } else {
            "idle".to_string()
        };
        status.warning = None;
        status.download_progress = None;
        status.downloaded_bytes = None;
        status.total_bytes = None;
        status.download_speed_bps = None;
        status.eta_seconds = None;
        status.model_path = Some(format!(
            "ollama://{}",
            Self::effective_chat_model_id(config)
        ));
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

    pub async fn vision_prompt(
        &self,
        config: &Config,
        prompt: &str,
        image_refs: &[String],
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        if image_refs.is_empty() {
            return Err("vision prompt requires at least one image reference".to_string());
        }
        self.bootstrap(config).await;
        let vision_model = Self::effective_vision_model_id(config);
        self.ensure_ollama_model_available(&vision_model, "vision").await?;

        let images: Vec<String> = image_refs
            .iter()
            .filter_map(|reference| multimodal::extract_ollama_image_payload(reference))
            .collect();
        if images.is_empty() {
            return Err("no valid image payloads were provided".to_string());
        }

        let body = OllamaGenerateRequest {
            model: vision_model,
            prompt: prompt.trim().to_string(),
            system: Some("You are a vision model. Answer directly and concisely.".to_string()),
            images: Some(images),
            stream: false,
            options: Some(OllamaGenerateOptions {
                temperature: Some(0.2),
                top_k: Some(30),
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
            .map_err(|e| format!("ollama vision request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "ollama vision request failed with status {}",
                response.status()
            ));
        }

        let payload: OllamaGenerateResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama vision response parse failed: {e}"))?;
        if payload.response.trim().is_empty() {
            return Err("ollama vision returned empty content".to_string());
        }

        self.status.lock().vision_state = "ready".to_string();
        Ok(payload.response)
    }

    pub async fn embed(
        &self,
        config: &Config,
        inputs: &[String],
    ) -> Result<LocalAiEmbeddingResult, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let items: Vec<String> = inputs
            .iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        if items.is_empty() {
            return Err("embed requires at least one non-empty input".to_string());
        }
        self.bootstrap(config).await;
        let embedding_model = Self::effective_embedding_model_id(config);
        self.ensure_ollama_model_available(&embedding_model, "embedding")
            .await?;

        let response = self
            .http
            .post(format!("{OLLAMA_BASE_URL}/api/embed"))
            .json(&OllamaEmbedRequest {
                model: embedding_model.clone(),
                input: items.clone(),
            })
            .send()
            .await
            .map_err(|e| format!("ollama embed request failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "ollama embed request failed with status {}",
                response.status()
            ));
        }

        let payload: OllamaEmbedResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama embed parse failed: {e}"))?;
        if payload.embeddings.is_empty() {
            return Err("ollama embed returned no embeddings".to_string());
        }

        let dims = payload.embeddings.first().map(|v| v.len()).unwrap_or(0);
        self.status.lock().embedding_state = "ready".to_string();
        Ok(LocalAiEmbeddingResult {
            model_id: embedding_model,
            dimensions: dims,
            vectors: payload.embeddings,
        })
    }

    pub async fn transcribe(
        &self,
        config: &Config,
        audio_path: &str,
    ) -> Result<LocalAiSpeechResult, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let whisper_bin = resolve_whisper_binary()
            .ok_or_else(|| "whisper.cpp binary not found. Set WHISPER_BIN or install whisper-cli.".to_string())?;
        let model_path = resolve_stt_model_path(config)?;
        let output = tokio::process::Command::new(whisper_bin)
            .args(["-m", &model_path, "-f", audio_path])
            .output()
            .await
            .map_err(|e| format!("failed to run whisper.cpp: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "whisper.cpp failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            return Err("whisper.cpp returned empty transcript".to_string());
        }
        self.status.lock().stt_state = "ready".to_string();
        Ok(LocalAiSpeechResult {
            text,
            model_id: Self::effective_stt_model_id(config),
        })
    }

    pub async fn tts(
        &self,
        config: &Config,
        text: &str,
        output_path: Option<&str>,
    ) -> Result<LocalAiTtsResult, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let piper_bin = resolve_piper_binary()
            .ok_or_else(|| "piper binary not found. Set PIPER_BIN or install piper.".to_string())?;
        let model_path = resolve_tts_voice_path(config)?;
        let out_path = output_path
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| {
                config_root_dir(config)
                    .join("models")
                    .join("local-ai")
                    .join("tts-output.wav")
                    .display()
                    .to_string()
            });
        let parent = PathBuf::from(&out_path)
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| "invalid output_path".to_string())?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create TTS output directory: {e}"))?;

        let mut child = tokio::process::Command::new(piper_bin)
            .args(["--model", &model_path, "--output_file", &out_path])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to launch piper: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(text.as_bytes())
                .await
                .map_err(|e| format!("failed to write text to piper stdin: {e}"))?;
        }
        let output = child
            .wait_with_output()
            .await
            .map_err(|e| format!("failed to wait for piper: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "piper failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        self.status.lock().tts_state = "ready".to_string();
        Ok(LocalAiTtsResult {
            output_path: out_path,
            voice_id: Self::effective_tts_voice_id(config),
        })
    }

    pub async fn assets_status(&self, config: &Config) -> Result<LocalAiAssetsStatus, String> {
        let chat_model = Self::effective_chat_model_id(config);
        let vision_model = Self::effective_vision_model_id(config);
        let embedding_model = Self::effective_embedding_model_id(config);
        let stt_model = Self::effective_stt_model_id(config);
        let tts_voice = Self::effective_tts_voice_id(config);

        let chat_ready = self.has_model(&chat_model).await.unwrap_or(false);
        let vision_ready = self.has_model(&vision_model).await.unwrap_or(false);
        let embedding_ready = self.has_model(&embedding_model).await.unwrap_or(false);
        let stt_path = resolve_stt_model_path(config).ok();
        let tts_path = resolve_tts_voice_path(config).ok();

        Ok(LocalAiAssetsStatus {
            chat: LocalAiAssetStatus {
                state: if chat_ready { "ready" } else { "missing" }.to_string(),
                id: chat_model,
                provider: "ollama".to_string(),
                path: None,
                warning: None,
            },
            vision: LocalAiAssetStatus {
                state: if vision_ready { "ready" } else { "missing" }.to_string(),
                id: vision_model,
                provider: "ollama".to_string(),
                path: None,
                warning: None,
            },
            embedding: LocalAiAssetStatus {
                state: if embedding_ready { "ready" } else { "missing" }.to_string(),
                id: embedding_model,
                provider: "ollama".to_string(),
                path: None,
                warning: None,
            },
            stt: LocalAiAssetStatus {
                state: if stt_path.is_some() { "ready" } else { "missing" }.to_string(),
                id: stt_model,
                provider: "whisper.cpp".to_string(),
                path: stt_path,
                warning: None,
            },
            tts: LocalAiAssetStatus {
                state: if tts_path.is_some() { "ready" } else { "missing" }.to_string(),
                id: tts_voice,
                provider: "piper".to_string(),
                path: tts_path,
                warning: None,
            },
            quantization: Self::effective_quantization(config),
        })
    }

    pub async fn download_asset(
        &self,
        config: &Config,
        capability: &str,
    ) -> Result<LocalAiAssetsStatus, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }

        let capability = capability.trim().to_ascii_lowercase();
        match capability.as_str() {
            "chat" => {
                self.ensure_ollama_server(config).await?;
                let model = Self::effective_chat_model_id(config);
                self.ensure_ollama_model_available(&model, "chat").await?;
            }
            "vision" => {
                self.ensure_ollama_server(config).await?;
                let model = Self::effective_vision_model_id(config);
                self.ensure_ollama_model_available(&model, "vision").await?;
            }
            "embedding" | "embeddings" => {
                self.ensure_ollama_server(config).await?;
                let model = Self::effective_embedding_model_id(config);
                self.ensure_ollama_model_available(&model, "embedding")
                    .await?;
            }
            "stt" => {
                return Err(
                    "Automatic STT model download is not implemented yet. Place your whisper model in models/local-ai/stt or set a full path in local_ai.stt_model_id."
                        .to_string(),
                );
            }
            "tts" => {
                return Err(
                    "Automatic TTS voice download is not implemented yet. Place your Piper ONNX voice in models/local-ai/tts or set a full path in local_ai.tts_voice_id."
                        .to_string(),
                );
            }
            _ => {
                return Err(
                    "Unknown capability. Use one of: chat, vision, embedding, stt, tts."
                        .to_string(),
                )
            }
        }

        self.assets_status(config).await
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
            model: Self::effective_chat_model_id(config),
            prompt: combined_prompt,
            system: Some(system.to_string()),
            images: None,
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

    async fn ensure_ollama_server(&self, config: &Config) -> Result<(), String> {
        if self.ollama_healthy().await {
            return Ok(());
        }

        let ollama_cmd = self.resolve_or_install_ollama_binary(config).await?;

        if let Err(err) = tokio::process::Command::new(&ollama_cmd)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
        {
            return Err(format!(
                "Ollama binary not available ({}; error: {err}).",
                ollama_cmd.display()
            ));
        }

        let _ = tokio::process::Command::new(&ollama_cmd)
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

    async fn resolve_or_install_ollama_binary(&self, config: &Config) -> Result<PathBuf, String> {
        if let Some(from_env) = std::env::var("OLLAMA_BIN")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            let path = PathBuf::from(from_env);
            if path.exists() {
                return Ok(path);
            }
        }

        let workspace_bin = workspace_ollama_binary(config);
        if workspace_bin.is_file() {
            return Ok(workspace_bin);
        }

        if self.command_works(Path::new("ollama")).await {
            return Ok(PathBuf::from("ollama"));
        }

        self.download_and_install_ollama(config).await?;
        let installed = workspace_ollama_binary(config);
        if installed.is_file() {
            Ok(installed)
        } else {
            Err("Ollama download completed but executable is missing.".to_string())
        }
    }

    async fn command_works(&self, command: &Path) -> bool {
        tokio::process::Command::new(command)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn download_and_install_ollama(&self, config: &Config) -> Result<(), String> {
        let install_dir = workspace_ollama_dir(config);
        tokio::fs::create_dir_all(&install_dir)
            .await
            .map_err(|e| format!("failed to create Ollama install directory: {e}"))?;

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some("Installing Ollama runtime (first run)".to_string());
            status.download_progress = None;
            status.downloaded_bytes = None;
            status.total_bytes = None;
            status.download_speed_bps = None;
            status.eta_seconds = None;
        }

        let install_status = run_ollama_install_script().await?;
        if !install_status.success() {
            return Err("Ollama install script failed".to_string());
        }

        let installed = find_system_ollama_binary()
            .ok_or_else(|| "Ollama installer finished but binary was not found".to_string())?;
        let dest = workspace_ollama_binary(config);
        tokio::fs::copy(&installed, &dest)
            .await
            .map_err(|e| format!("failed to copy Ollama binary into workspace: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("failed to set Ollama binary permissions: {e}"))?;
        }

        {
            let mut status = self.status.lock();
            status.warning = Some("Ollama runtime installed".to_string());
            status.download_progress = Some(1.0);
        }
        Ok(())
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

    async fn ensure_models_available(&self, config: &Config) -> Result<(), String> {
        let chat_model = Self::effective_chat_model_id(config);
        self.ensure_ollama_model_available(&chat_model, "chat").await?;

        let vision_model = Self::effective_vision_model_id(config);
        if config.local_ai.preload_vision_model {
            self.ensure_ollama_model_available(&vision_model, "vision").await?;
            self.status.lock().vision_state = "ready".to_string();
        }

        let embedding_model = Self::effective_embedding_model_id(config);
        if config.local_ai.preload_embedding_model {
            self.ensure_ollama_model_available(&embedding_model, "embedding")
                .await?;
            self.status.lock().embedding_state = "ready".to_string();
        }

        if config.local_ai.preload_stt_model {
            self.status.lock().stt_state = if resolve_stt_model_path(config).is_ok() {
                "ready".to_string()
            } else {
                "degraded".to_string()
            };
        }

        if config.local_ai.preload_tts_voice {
            self.status.lock().tts_state = if resolve_tts_voice_path(config).is_ok() {
                "ready".to_string()
            } else {
                "degraded".to_string()
            };
        }

        Ok(())
    }

    async fn ensure_ollama_model_available(
        &self,
        model_id: &str,
        label: &str,
    ) -> Result<(), String> {
        if self.has_model(model_id).await? {
            return Ok(());
        }

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!(
                "Pulling {} model `{}` from Ollama library",
                label, model_id
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
                name: model_id.to_string(),
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
                if let Some(status_text) = event.status.as_deref() {
                    status.warning = Some(format!("Ollama pull: {status_text}"));
                    if status_text.eq_ignore_ascii_case("success") {
                        status.download_progress = Some(1.0);
                    }
                }
                status.downloaded_bytes = Some(completed);
                status.total_bytes = total;
                status.download_speed_bps = Some(speed_bps);
                status.eta_seconds = eta_seconds;
                status.download_progress = total
                    .map(|t| (completed as f32 / t as f32).clamp(0.0, 1.0))
                    .or(Some(0.0));
            }
        }

        if !self.has_model(model_id).await? {
            return Err(format!(
                "ollama pull finished but model `{}` was not found",
                model_id
            ));
        }

        match label {
            "vision" => self.status.lock().vision_state = "ready".to_string(),
            "embedding" => self.status.lock().embedding_state = "ready".to_string(),
            _ => {}
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

fn config_root_dir(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config.workspace_dir.clone())
}

fn workspace_ollama_dir(config: &Config) -> PathBuf {
    config_root_dir(config).join("bin").join("ollama")
}

fn workspace_ollama_binary(config: &Config) -> PathBuf {
    let name = if cfg!(windows) { "ollama.exe" } else { "ollama" };
    workspace_ollama_dir(config).join(name)
}

fn workspace_local_models_dir(config: &Config) -> PathBuf {
    config_root_dir(config).join("models").join("local-ai")
}

fn resolve_whisper_binary() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var("WHISPER_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }

    let bin_name = if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    };
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var)
            .map(|entry| entry.join(bin_name))
            .find(|candidate| candidate.is_file())
    })
}

fn resolve_piper_binary() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var("PIPER_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }

    let bin_name = if cfg!(windows) { "piper.exe" } else { "piper" };
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var)
            .map(|entry| entry.join(bin_name))
            .find(|candidate| candidate.is_file())
    })
}

fn resolve_stt_model_path(config: &Config) -> Result<String, String> {
    let id = LocalAiService::effective_stt_model_id(config);
    let path = PathBuf::from(&id);
    if path.is_file() {
        return Ok(path.display().to_string());
    }
    let candidate = workspace_local_models_dir(config).join("stt").join(&id);
    if candidate.is_file() {
        Ok(candidate.display().to_string())
    } else {
        Err(format!(
            "STT model not found. Expected '{}' or '{}'",
            path.display(),
            candidate.display()
        ))
    }
}

fn resolve_tts_voice_path(config: &Config) -> Result<String, String> {
    let voice_id = LocalAiService::effective_tts_voice_id(config);
    let path = PathBuf::from(&voice_id);
    if path.is_file() {
        return Ok(path.display().to_string());
    }
    let filename = if voice_id.ends_with(".onnx") {
        voice_id
    } else {
        format!("{voice_id}.onnx")
    };
    let candidate = workspace_local_models_dir(config).join("tts").join(filename);
    if candidate.is_file() {
        Ok(candidate.display().to_string())
    } else {
        Err(format!(
            "TTS voice model not found. Expected '{}' or '{}'",
            path.display(),
            candidate.display()
        ))
    }
}

async fn run_ollama_install_script() -> Result<std::process::ExitStatus, String> {
    #[cfg(target_os = "windows")]
    {
        return tokio::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "irm https://ollama.com/install.ps1 | iex",
            ])
            .status()
            .await
            .map_err(|e| format!("failed to execute Ollama PowerShell installer: {e}"));
    }

    #[cfg(target_os = "macos")]
    {
        // User-provided mac installer flow: curl ... | sh -mac
        return tokio::process::Command::new("sh")
            .arg("-lc")
            .arg("curl -fsSL https://ollama.com/install.sh | sh -mac")
            .status()
            .await
            .map_err(|e| format!("failed to execute Ollama macOS installer: {e}"));
    }

    #[cfg(target_os = "linux")]
    {
        return tokio::process::Command::new("sh")
            .arg("-lc")
            .arg("curl -fsSL https://ollama.com/install.sh | sh")
            .status()
            .await
            .map_err(|e| format!("failed to execute Ollama Linux installer: {e}"));
    }

    #[allow(unreachable_code)]
    Err(format!(
        "Unsupported platform for automatic Ollama install: {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

fn find_system_ollama_binary() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var("OLLAMA_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }

    let binary_name = if cfg!(windows) { "ollama.exe" } else { "ollama" };
    if let Some(path_var) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path_var) {
            let candidate = entry.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    if cfg!(target_os = "macos") {
        let common = [
            PathBuf::from("/usr/local/bin/ollama"),
            PathBuf::from("/opt/homebrew/bin/ollama"),
        ];
        for candidate in common {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    if cfg!(target_os = "linux") {
        let common = [
            PathBuf::from("/usr/local/bin/ollama"),
            PathBuf::from("/usr/bin/ollama"),
        ];
        for candidate in common {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
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
    images: Option<Vec<String>>,
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

#[derive(Debug, Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
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
        .join(LocalAiService::effective_chat_model_id(config).replace(':', "-") + ".ollama")
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
