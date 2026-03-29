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
            status.active_backend = "ollama".to_string();
            status.backend_reason = Some("Inference delegated to Ollama runtime".to_string());
            status.model_path = Some(format!(
                "ollama://{}",
                model_ids::effective_chat_model_id(config)
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
        if !config.local_ai.preload_stt_model {
            status.stt_state = "idle".to_string();
        }
        if !config.local_ai.preload_tts_voice {
            status.tts_state = "idle".to_string();
        }
        status.warning = None;
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
