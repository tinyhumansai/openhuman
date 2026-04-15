use crate::openhuman::config::Config;
use crate::openhuman::local_ai::device::DeviceProfile;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::presets::{self, VisionMode};
use crate::openhuman::local_ai::types::LocalAiStatus;

use super::LocalAiService;

impl LocalAiService {
    pub(crate) fn new(config: &Config) -> Self {
        let model_id = model_ids::effective_chat_model_id(config);
        let vision_model_id = model_ids::effective_vision_model_id(config);
        let embedding_model_id = model_ids::effective_embedding_model_id(config);
        let vision_mode = vision_mode_str(config);
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
                vision_state: initial_vision_state(config),
                vision_mode,
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
            whisper_load_lock: tokio::sync::Mutex::new(()),
            last_memory_summary_at: parking_lot::Mutex::new(None),
            http: reqwest::Client::builder()
                // Local models can take >30s on cold start and first-token generation.
                // Keep this generous so inline autocomplete and local chat stay reliable.
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|e| {
                    log::warn!("[local_ai] reqwest client build failed, falling back to default client: {e}");
                    reqwest::Client::new()
                }),
        }
    }

    pub fn status(&self) -> LocalAiStatus {
        self.status.lock().clone()
    }

    pub fn reset_to_idle(&self, config: &Config) {
        let model_id = model_ids::effective_chat_model_id(config);
        let vision_mode = vision_mode_str(config);
        let mut status = self.status.lock();
        status.state = "idle".to_string();
        status.model_id = model_id.clone();
        status.chat_model_id = model_id.clone();
        status.vision_model_id = model_ids::effective_vision_model_id(config);
        status.embedding_model_id = model_ids::effective_embedding_model_id(config);
        status.stt_model_id = model_ids::effective_stt_model_id(config);
        status.tts_voice_id = model_ids::effective_tts_voice_id(config);
        status.quantization = model_ids::effective_quantization(config);
        status.vision_state = initial_vision_state(config);
        status.vision_mode = vision_mode;
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
        let device = crate::openhuman::local_ai::device::detect_device_profile();
        let effective_config = config_with_recommended_tier_if_unselected(config, &device);

        if !effective_config.local_ai.enabled {
            *self.status.lock() = LocalAiStatus::disabled(&effective_config);
            return;
        }

        // Return early if already succeeded or previously degraded.
        // "degraded" means a prior bootstrap attempt already failed; further
        // automatic retries just spam Ollama pull requests.  An explicit retry
        // (local_ai_download with force=true) resets to "idle" first.
        if matches!(self.status.lock().state.as_str(), "ready" | "degraded") {
            return;
        }

        {
            let mut status = self.status.lock();
            status.model_id = model_ids::effective_chat_model_id(&effective_config);
            status.chat_model_id = model_ids::effective_chat_model_id(&effective_config);
            status.vision_model_id = model_ids::effective_vision_model_id(&effective_config);
            status.embedding_model_id = model_ids::effective_embedding_model_id(&effective_config);
            status.stt_model_id = model_ids::effective_stt_model_id(&effective_config);
            status.tts_voice_id = model_ids::effective_tts_voice_id(&effective_config);
            status.quantization = model_ids::effective_quantization(&effective_config);
            status.state = "loading".to_string();
            status.vision_mode = vision_mode_str(&effective_config);
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
                model_ids::effective_chat_model_id(&effective_config)
            ));
        }

        if let Err(first_err) = self.ensure_ollama_server(&effective_config).await {
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
            if let Err(err) = self.ensure_ollama_server_fresh(&effective_config).await {
                let mut status = self.status.lock();
                status.state = "degraded".to_string();
                let is_install_error = status.error_category.as_deref() == Some("install");
                if is_install_error {
                    status.warning = Some(err);
                } else {
                    status.error_category = Some("server".to_string());
                    status.warning = Some(format_degraded_warning(&err, &effective_config));
                }
                return;
            }
        }

        if let Err(err) = self.ensure_models_available(&effective_config).await {
            let mut status = self.status.lock();
            status.state = "degraded".to_string();
            status.error_category = Some("download".to_string());
            status.warning = Some(format_degraded_warning(&err, &effective_config));
            return;
        }

        // Attempt to load whisper model in-process if configured (blocking I/O).
        // Pass GPU info from the device profile so whisper can use hardware acceleration.
        if effective_config.local_ai.whisper_in_process {
            if let Ok(model_path) =
                crate::openhuman::local_ai::paths::resolve_stt_model_path(&effective_config)
            {
                let model = std::path::PathBuf::from(&model_path);
                let handle = self.whisper.clone();
                let gpu = device.has_gpu;
                let gpu_desc = device.gpu_description.clone();
                let load_result = tokio::task::spawn_blocking(move || {
                    super::whisper_engine::load_engine(&handle, &model, gpu, gpu_desc.as_deref())
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
        status.vision_state = match presets::vision_mode_for_config(&effective_config.local_ai) {
            VisionMode::Disabled => "disabled".to_string(),
            VisionMode::Bundled => "ready".to_string(),
            VisionMode::Ondemand => "idle".to_string(),
        };
        status.embedding_state = if effective_config.local_ai.preload_embedding_model {
            "ready".to_string()
        } else {
            "idle".to_string()
        };
        if !effective_config.local_ai.preload_stt_model {
            status.stt_state = "idle".to_string();
        }
        if !effective_config.local_ai.preload_tts_voice {
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
            model_ids::effective_chat_model_id(&effective_config)
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

fn config_with_recommended_tier_if_unselected(config: &Config, device: &DeviceProfile) -> Config {
    let selected_tier = config
        .local_ai
        .selected_tier
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let current_tier =
        crate::openhuman::local_ai::presets::current_tier_from_config(&config.local_ai);
    if selected_tier.is_some()
        || matches!(
            current_tier,
            crate::openhuman::local_ai::presets::ModelTier::Ram1Gb
                | crate::openhuman::local_ai::presets::ModelTier::Ram2To4Gb
                | crate::openhuman::local_ai::presets::ModelTier::Ram4To8Gb
                | crate::openhuman::local_ai::presets::ModelTier::Ram8To16Gb
                | crate::openhuman::local_ai::presets::ModelTier::Ram16PlusGb
        )
    {
        return config.clone();
    }

    let recommended = crate::openhuman::local_ai::presets::recommend_tier(device);
    let mut effective_config = config.clone();
    crate::openhuman::local_ai::presets::apply_preset_to_config(
        &mut effective_config.local_ai,
        recommended,
    );
    tracing::debug!(
        ?recommended,
        "[local_ai] bootstrap: no tier selected, using recommended preset"
    );
    effective_config
}

fn format_degraded_warning(err: &str, config: &Config) -> String {
    let current = crate::openhuman::local_ai::presets::current_tier_from_config(&config.local_ai);
    match current {
        crate::openhuman::local_ai::presets::ModelTier::Ram16PlusGb => {
            format!(
                "{err}. Hint: your device may not support the 16 GB+ tier model. \
                 Try switching to the 8-16 GB or 4-8 GB tier in Settings > Local AI Model."
            )
        }
        crate::openhuman::local_ai::presets::ModelTier::Ram8To16Gb => {
            format!(
                "{err}. Hint: your device may not support the 8-16 GB tier model. \
                 Try switching to the 4-8 GB or 2-4 GB tier in Settings > Local AI Model."
            )
        }
        crate::openhuman::local_ai::presets::ModelTier::Ram4To8Gb => format!(
            "{err}. Hint: your device may not support the 4-8 GB tier vision sidecar. \
             Try switching to the 2-4 GB tier for text-only local AI."
        ),
        _ => err.to_string(),
    }
}

fn initial_vision_state(config: &Config) -> String {
    match presets::vision_mode_for_config(&config.local_ai) {
        VisionMode::Disabled => "disabled".to_string(),
        VisionMode::Ondemand | VisionMode::Bundled => "idle".to_string(),
    }
}

fn vision_mode_str(config: &Config) -> String {
    format!("{:?}", presets::vision_mode_for_config(&config.local_ai)).to_ascii_lowercase()
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

    #[test]
    fn bootstrap_uses_recommended_tier_when_selection_missing() {
        let config = Config::default();
        let device = DeviceProfile {
            total_ram_bytes: 4 * 1024 * 1024 * 1024,
            cpu_count: 4,
            cpu_brand: String::new(),
            os_name: String::new(),
            os_version: String::new(),
            has_gpu: false,
            gpu_description: None,
        };

        let effective = config_with_recommended_tier_if_unselected(&config, &device);

        // If config already matches a built-in preset, preserve user defaults
        // and keep selected_tier unset.
        assert!(effective.local_ai.selected_tier.is_none());
        assert_eq!(
            effective.local_ai.chat_model_id,
            config.local_ai.chat_model_id
        );
    }

    #[test]
    fn bootstrap_keeps_existing_selected_tier() {
        let mut config = Config::default();
        config.local_ai.selected_tier = Some("high".to_string());
        let original_chat_model = config.local_ai.chat_model_id.clone();
        let device = DeviceProfile {
            total_ram_bytes: 4 * 1024 * 1024 * 1024,
            cpu_count: 4,
            cpu_brand: String::new(),
            os_name: String::new(),
            os_version: String::new(),
            has_gpu: false,
            gpu_description: None,
        };

        let effective = config_with_recommended_tier_if_unselected(&config, &device);

        assert_eq!(effective.local_ai.selected_tier.as_deref(), Some("high"));
        assert_eq!(effective.local_ai.chat_model_id, original_chat_model);
    }
}
