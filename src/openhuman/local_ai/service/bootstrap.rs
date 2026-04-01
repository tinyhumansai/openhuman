use crate::openhuman::config::Config;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::types::LocalAiStatus;

use super::LocalAiService;

impl LocalAiService {
    pub(crate) fn new(config: &Config) -> Self {
        let model_id = model_ids::effective_chat_model_id(config);
        let vision_model_id = model_ids::effective_vision_model_id(config);
        let embedding_model_id = model_ids::effective_embedding_model_id(config);
        Self {
            whisper: super::whisper_engine::new_handle(),
            status: parking_lot::Mutex::new(LocalAiStatus {
                state: "idle".to_string(),
                model_id: model_id.clone(),
                chat_model_id: model_id.clone(),
                vision_model_id: vision_model_id.clone(),
                embedding_model_id: embedding_model_id.clone(),
                stt_model_id: model_ids::effective_stt_model_id(config),
                tts_voice_id: model_ids::effective_tts_voice_id(config),
                quantization: model_ids::effective_quantization(config),
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
                error_detail: None,
                error_category: None,
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
        let model_id = model_ids::effective_chat_model_id(config);
        let mut status = self.status.lock();
        status.state = "idle".to_string();
        status.model_id = model_id.clone();
        status.chat_model_id = model_id.clone();
        status.vision_model_id = model_ids::effective_vision_model_id(config);
        status.embedding_model_id = model_ids::effective_embedding_model_id(config);
        status.stt_model_id = model_ids::effective_stt_model_id(config);
        status.tts_voice_id = model_ids::effective_tts_voice_id(config);
        status.quantization = model_ids::effective_quantization(config);
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
        status.error_detail = None;
        status.error_category = None;
        status.model_path = Some(format!("ollama://{}", model_id));
        status.active_backend = "ollama".to_string();
        status.backend_reason = None;
        status.last_latency_ms = None;
        status.prompt_toks_per_sec = None;
        status.gen_toks_per_sec = None;
    }

    pub fn mark_degraded(&self, warning: String) {
        let mut status = self.status.lock();
        status.state = "degraded".to_string();
        status.warning = Some(warning);
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
            status.model_id = model_ids::effective_chat_model_id(config);
            status.chat_model_id = model_ids::effective_chat_model_id(config);
            status.vision_model_id = model_ids::effective_vision_model_id(config);
            status.embedding_model_id = model_ids::effective_embedding_model_id(config);
            status.stt_model_id = model_ids::effective_stt_model_id(config);
            status.tts_voice_id = model_ids::effective_tts_voice_id(config);
            status.quantization = model_ids::effective_quantization(config);
            status.state = "loading".to_string();
            status.warning = Some("Connecting to local Ollama runtime".to_string());
            status.download_progress = None;
            status.downloaded_bytes = None;
            status.total_bytes = None;
            status.download_speed_bps = None;
            status.eta_seconds = None;
            status.error_detail = None;
            status.error_category = None;
            status.active_backend = "ollama".to_string();
            status.backend_reason = Some("Inference delegated to Ollama runtime".to_string());
            status.model_path = Some(format!(
                "ollama://{}",
                model_ids::effective_chat_model_id(config)
            ));
        }

        if let Err(first_err) = self.ensure_ollama_server(config).await {
            log::warn!(
                "[local_ai] ensure_ollama_server failed, retrying with fresh install: {first_err}"
            );
            // Force a fresh install attempt before giving up.
            {
                let mut status = self.status.lock();
                status.state = "installing".to_string();
                status.warning = Some("Retrying Ollama installation...".to_string());
                status.error_detail = None;
                status.error_category = None;
            }
            if let Err(err) = self.ensure_ollama_server_fresh(config).await {
                let mut status = self.status.lock();
                status.state = "degraded".to_string();
                let is_install_error = status.error_category.as_deref() == Some("install");
                if is_install_error {
                    status.warning = Some(err);
                } else {
                    status.error_category = Some("server".to_string());
                    status.warning = Some(format_degraded_warning(&err, config));
                }
                return;
            }
        }

        if let Err(err) = self.ensure_models_available(config).await {
            let mut status = self.status.lock();
            status.state = "degraded".to_string();
            status.error_category = Some("download".to_string());
            status.warning = Some(format_degraded_warning(&err, config));
            return;
        }

        // Attempt to load whisper model in-process if configured (blocking I/O).
        if config.local_ai.whisper_in_process {
            if let Ok(model_path) =
                crate::openhuman::local_ai::paths::resolve_stt_model_path(config)
            {
                let model = std::path::PathBuf::from(&model_path);
                let handle = self.whisper.clone();
                let load_result = tokio::task::spawn_blocking(move || {
                    super::whisper_engine::load_engine(&handle, &model)
                })
                .await;
                match load_result {
                    Ok(Ok(())) => {
                        log::info!("[local_ai] whisper engine loaded in-process: {model_path}");
                    }
                    Ok(Err(e)) => {
                        log::warn!(
                            "[local_ai] whisper in-process load failed, will fall back to CLI: {e}"
                        );
                    }
                    Err(e) => {
                        log::warn!("[local_ai] whisper load task panicked: {e}");
                    }
                }
            } else {
                log::debug!("[local_ai] STT model not found, whisper in-process not loaded");
            }
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
        if !config.local_ai.preload_stt_model {
            status.stt_state = "idle".to_string();
        }
        if !config.local_ai.preload_tts_voice {
            status.tts_state = "idle".to_string();
        }
        status.warning = None;
        status.error_detail = None;
        status.error_category = None;
        status.download_progress = None;
        status.downloaded_bytes = None;
        status.total_bytes = None;
        status.download_speed_bps = None;
        status.eta_seconds = None;
        status.model_path = Some(format!(
            "ollama://{}",
            model_ids::effective_chat_model_id(config)
        ));
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
}

/// Append a tier step-down hint when the current tier is Medium or High.
fn format_degraded_warning(err: &str, config: &Config) -> String {
    let current = crate::openhuman::local_ai::presets::current_tier_from_config(&config.local_ai);
    match current {
        crate::openhuman::local_ai::presets::ModelTier::High => {
            format!(
                "{err}. Hint: your device may not support the High tier model. \
                 Try switching to Medium or Low in Settings > Local AI Model."
            )
        }
        crate::openhuman::local_ai::presets::ModelTier::Medium => {
            format!(
                "{err}. Hint: your device may not support the Medium tier model. \
                 Try switching to Low in Settings > Local AI Model."
            )
        }
        _ => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autosummary_debounce_blocks_repeated_calls_inside_window() {
        let mut config = Config::default();
        config.local_ai.autosummary_debounce_ms = 60_000;
        let service = LocalAiService::new(&config);

        assert!(service.should_run_memory_autosummary(&config));
        assert!(!service.should_run_memory_autosummary(&config));
    }
}
