use std::path::Path;

use futures_util::TryStreamExt;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::model_ids;
use tracing::{debug, trace};

use crate::openhuman::local_ai::paths::{
    resolve_stt_model_path, resolve_tts_voice_path, stt_model_target_path, tts_model_target_path,
};
use crate::openhuman::local_ai::presets::{self, VisionMode};
use crate::openhuman::local_ai::provider::{provider_from_config, LocalAiProvider};
use crate::openhuman::local_ai::types::{
    LocalAiAssetStatus, LocalAiAssetsStatus, LocalAiDownloadProgressItem, LocalAiDownloadsProgress,
};

use super::LocalAiService;

impl LocalAiService {
    pub async fn assets_status(&self, config: &Config) -> Result<LocalAiAssetsStatus, String> {
        let chat_model = model_ids::effective_chat_model_id(config);
        let vision_model = model_ids::effective_vision_model_id(config);
        let embedding_model = model_ids::effective_embedding_model_id(config);
        let stt_model = model_ids::effective_stt_model_id(config);
        let tts_voice = model_ids::effective_tts_voice_id(config);

        let provider = provider_from_config(config);
        let correlation_id = uuid::Uuid::new_v4().to_string();
        trace!(
            target: "local_ai::assets",
            %correlation_id,
            provider = %provider.as_str(),
            chat_model = %chat_model,
            vision_model = %vision_model,
            embedding_model = %embedding_model,
            "[local_ai:assets:provider_routing] entry"
        );

        // Pre-flight precondition: if no Ollama binary exists anywhere
        // discoverable, every Ollama-backed `has_model` call will fail (or
        // time out). LM Studio still delegates embeddings to Ollama in this
        // first provider slice, so it needs the same pre-flight for the
        // embedding branch.
        let uses_ollama_assets = matches!(
            provider,
            LocalAiProvider::Ollama | LocalAiProvider::LmStudio
        );
        let ollama_available = if uses_ollama_assets {
            let present = self.ollama_binary_present(config);
            debug!(
                target: "local_ai::assets",
                %correlation_id,
                provider = %provider.as_str(),
                ollama_available = present,
                "[local_ai:assets:provider_routing] ollama binary check"
            );
            present
        } else {
            true
        };
        let (chat_ready, vision_ready, embedding_ready) = if provider == LocalAiProvider::LmStudio {
            trace!(
                target: "local_ai::assets",
                %correlation_id,
                branch = "lm_studio",
                "[local_ai:assets:provider_routing] selected provider branch"
            );
            let chat_ready = match self.has_lm_studio_model(config, &chat_model).await {
                Ok(ready) => {
                    debug!(
                        target: "local_ai::assets",
                        %correlation_id,
                        provider = "lm_studio",
                        model = %chat_model,
                        ready,
                        "[local_ai:assets:provider_routing] lm studio chat model check"
                    );
                    ready
                }
                Err(err) => {
                    debug!(
                        target: "local_ai::assets",
                        %correlation_id,
                        provider = "lm_studio",
                        model = %chat_model,
                        error = %err,
                        "[local_ai:assets:provider_routing] lm studio chat model check failed"
                    );
                    false
                }
            };
            let embedding_ready = if ollama_available {
                match self.has_model(&embedding_model).await {
                    Ok(ready) => {
                        debug!(
                            target: "local_ai::assets",
                            %correlation_id,
                            provider = "ollama",
                            model = %embedding_model,
                            ready,
                            "[local_ai:assets:provider_routing] lm studio embedding ollama model check"
                        );
                        ready
                    }
                    Err(err) => {
                        debug!(
                            target: "local_ai::assets",
                            %correlation_id,
                            provider = "ollama",
                            model = %embedding_model,
                            error = %err,
                            "[local_ai:assets:provider_routing] lm studio embedding ollama model check failed"
                        );
                        false
                    }
                }
            } else {
                debug!(
                    target: "local_ai::assets",
                    %correlation_id,
                    provider = "ollama",
                    model = %embedding_model,
                    "[local_ai:assets:provider_routing] lm studio embedding check skipped; ollama binary missing"
                );
                false
            };
            (chat_ready, false, embedding_ready)
        } else if ollama_available {
            trace!(
                target: "local_ai::assets",
                %correlation_id,
                branch = "ollama",
                "[local_ai:assets:provider_routing] selected provider branch"
            );
            let chat_ready = match self.has_model(&chat_model).await {
                Ok(ready) => {
                    debug!(
                        target: "local_ai::assets",
                        %correlation_id,
                        provider = "ollama",
                        capability = "chat",
                        model = %chat_model,
                        ready,
                        "[local_ai:assets:provider_routing] ollama model check"
                    );
                    ready
                }
                Err(err) => {
                    debug!(
                        target: "local_ai::assets",
                        %correlation_id,
                        provider = "ollama",
                        capability = "chat",
                        model = %chat_model,
                        error = %err,
                        "[local_ai:assets:provider_routing] ollama model check failed"
                    );
                    false
                }
            };
            let vision_ready = match self.has_model(&vision_model).await {
                Ok(ready) => {
                    debug!(
                        target: "local_ai::assets",
                        %correlation_id,
                        provider = "ollama",
                        capability = "vision",
                        model = %vision_model,
                        ready,
                        "[local_ai:assets:provider_routing] ollama model check"
                    );
                    ready
                }
                Err(err) => {
                    debug!(
                        target: "local_ai::assets",
                        %correlation_id,
                        provider = "ollama",
                        capability = "vision",
                        model = %vision_model,
                        error = %err,
                        "[local_ai:assets:provider_routing] ollama model check failed"
                    );
                    false
                }
            };
            let embedding_ready = match self.has_model(&embedding_model).await {
                Ok(ready) => {
                    debug!(
                        target: "local_ai::assets",
                        %correlation_id,
                        provider = "ollama",
                        capability = "embedding",
                        model = %embedding_model,
                        ready,
                        "[local_ai:assets:provider_routing] ollama model check"
                    );
                    ready
                }
                Err(err) => {
                    debug!(
                        target: "local_ai::assets",
                        %correlation_id,
                        provider = "ollama",
                        capability = "embedding",
                        model = %embedding_model,
                        error = %err,
                        "[local_ai:assets:provider_routing] ollama model check failed"
                    );
                    false
                }
            };
            (chat_ready, vision_ready, embedding_ready)
        } else {
            trace!(
                target: "local_ai::assets",
                %correlation_id,
                branch = "ollama_missing_binary",
                "[local_ai:assets:provider_routing] selected provider branch"
            );
            (false, false, false)
        };
        trace!(
            target: "local_ai::assets",
            %correlation_id,
            chat_ready,
            vision_ready,
            embedding_ready,
            ollama_available,
            "[local_ai:assets:provider_routing] exit"
        );
        let stt_resolve = resolve_stt_model_path(config);
        let tts_resolve = resolve_tts_voice_path(config);

        let stt_path = stt_resolve.as_ref().ok().cloned();
        let tts_path = tts_resolve.as_ref().ok().cloned();

        // STT and TTS are downloaded on-demand (first transcription / first
        // synthesis).  When the model file is not yet on disk but a download
        // URL is configured, report "ondemand" instead of "missing" so the
        // UI can treat the capability as non-blocking.
        let has_stt_url = config
            .local_ai
            .stt_download_url
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty());
        let has_tts_url = config
            .local_ai
            .tts_download_url
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty());

        let stt_state = if stt_path.is_some() {
            "ready"
        } else if has_stt_url {
            "ondemand"
        } else {
            "missing"
        };
        let tts_state = if tts_path.is_some() {
            "ready"
        } else if has_tts_url {
            "ondemand"
        } else {
            "missing"
        };

        if let Err(ref err) = stt_resolve {
            debug!("[local_ai::assets_status] STT resolve failed (state={stt_state}): {err}");
        }
        if let Err(ref err) = tts_resolve {
            debug!("[local_ai::assets_status] TTS resolve failed (state={tts_state}): {err}");
        }

        let stt_warning = match stt_state {
            "ondemand" => {
                Some("STT model will download on first transcription request.".to_string())
            }
            _ => None,
        };
        let tts_warning = match tts_state {
            "ondemand" => Some("TTS voice will download on first synthesis request.".to_string()),
            _ => None,
        };

        let vision_mode = presets::vision_mode_for_config(&config.local_ai);
        let embedding_path = Some(format!("ollama://{embedding_model}"));
        Ok(LocalAiAssetsStatus {
            chat: LocalAiAssetStatus {
                state: if chat_ready { "ready" } else { "missing" }.to_string(),
                id: chat_model,
                provider: provider.as_str().to_string(),
                path: None,
                warning: (provider == LocalAiProvider::LmStudio && !chat_ready).then(|| {
                    "Load this model in LM Studio or update local_ai.chat_model_id.".to_string()
                }),
            },
            vision: LocalAiAssetStatus {
                state: if provider == LocalAiProvider::LmStudio {
                    "disabled".to_string()
                } else {
                    match vision_mode {
                        VisionMode::Disabled => "disabled",
                        VisionMode::Ondemand if vision_ready => "ready",
                        VisionMode::Ondemand => "ondemand",
                        VisionMode::Bundled if vision_ready => "ready",
                        VisionMode::Bundled => "missing",
                    }
                    .to_string()
                },
                id: vision_model,
                provider: provider.as_str().to_string(),
                path: None,
                warning: if provider == LocalAiProvider::LmStudio {
                    Some("Vision is not part of the first LM Studio provider slice.".to_string())
                } else {
                    match vision_mode {
                        VisionMode::Disabled => {
                            Some("Vision is disabled for this RAM tier.".to_string())
                        }
                        VisionMode::Ondemand if !vision_ready => {
                            Some("Vision model will download on first vision request.".to_string())
                        }
                        _ => None,
                    }
                },
            },
            embedding: LocalAiAssetStatus {
                state: if embedding_ready { "ready" } else { "missing" }.to_string(),
                id: embedding_model,
                provider: if provider == LocalAiProvider::LmStudio {
                    "ollama".to_string()
                } else {
                    provider.as_str().to_string()
                },
                path: embedding_path,
                warning: (provider == LocalAiProvider::LmStudio).then(|| {
                    "Embeddings still use the existing Ollama path in this first LM Studio slice."
                        .to_string()
                }),
            },
            stt: LocalAiAssetStatus {
                state: stt_state.to_string(),
                id: stt_model,
                provider: "whisper.cpp".to_string(),
                path: stt_path,
                warning: stt_warning,
            },
            tts: LocalAiAssetStatus {
                state: tts_state.to_string(),
                id: tts_voice,
                provider: "piper".to_string(),
                path: tts_path,
                warning: tts_warning,
            },
            quantization: model_ids::effective_quantization(config),
            ollama_available,
        })
    }

    pub async fn downloads_progress(
        &self,
        config: &Config,
    ) -> Result<LocalAiDownloadsProgress, String> {
        let assets = self.assets_status(config).await?;
        let status = self.status();

        let mut chat = LocalAiDownloadProgressItem {
            id: assets.chat.id,
            provider: assets.chat.provider,
            state: assets.chat.state,
            progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            speed_bps: None,
            eta_seconds: None,
            warning: assets.chat.warning,
            path: assets.chat.path,
        };
        let mut vision = LocalAiDownloadProgressItem {
            id: assets.vision.id,
            provider: assets.vision.provider,
            state: assets.vision.state,
            progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            speed_bps: None,
            eta_seconds: None,
            warning: assets.vision.warning,
            path: assets.vision.path,
        };
        let mut embedding = LocalAiDownloadProgressItem {
            id: assets.embedding.id,
            provider: assets.embedding.provider,
            state: assets.embedding.state,
            progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            speed_bps: None,
            eta_seconds: None,
            warning: assets.embedding.warning,
            path: assets.embedding.path,
        };
        let mut stt = LocalAiDownloadProgressItem {
            id: assets.stt.id,
            provider: assets.stt.provider,
            state: assets.stt.state,
            progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            speed_bps: None,
            eta_seconds: None,
            warning: assets.stt.warning,
            path: assets.stt.path,
        };
        let mut tts = LocalAiDownloadProgressItem {
            id: assets.tts.id,
            provider: assets.tts.provider,
            state: assets.tts.state,
            progress: None,
            downloaded_bytes: None,
            total_bytes: None,
            speed_bps: None,
            eta_seconds: None,
            warning: assets.tts.warning,
            path: assets.tts.path,
        };

        if status.state == "downloading" {
            let active = if status.stt_state == "downloading" {
                "stt"
            } else if status.tts_state == "downloading" {
                "tts"
            } else if status.vision_state == "downloading" {
                "vision"
            } else if status.embedding_state == "downloading" {
                "embedding"
            } else {
                "chat"
            };

            let apply = |item: &mut LocalAiDownloadProgressItem| {
                item.state = "downloading".to_string();
                item.progress = status.download_progress;
                item.downloaded_bytes = status.downloaded_bytes;
                item.total_bytes = status.total_bytes;
                item.speed_bps = status.download_speed_bps;
                item.eta_seconds = status.eta_seconds;
                item.warning = status.warning.clone();
            };

            match active {
                "stt" => apply(&mut stt),
                "tts" => apply(&mut tts),
                "vision" => apply(&mut vision),
                "embedding" => apply(&mut embedding),
                _ => apply(&mut chat),
            }
        }

        Ok(LocalAiDownloadsProgress {
            state: status.state,
            warning: status.warning,
            progress: status.download_progress,
            downloaded_bytes: status.downloaded_bytes,
            total_bytes: status.total_bytes,
            speed_bps: status.download_speed_bps,
            eta_seconds: status.eta_seconds,
            chat,
            vision,
            embedding,
            stt,
            tts,
            ollama_available: assets.ollama_available,
        })
    }

    fn finalize_lm_studio_download_status(
        &self,
        config: &Config,
        embedding_state: Option<&'static str>,
        stt_state: Option<&'static str>,
        tts_state: Option<&'static str>,
        warning: Option<String>,
    ) {
        let mut status = self.status.lock();
        status.state = "ready".to_string();
        status.vision_state = "disabled".to_string();
        if let Some(state) = embedding_state {
            status.embedding_state = state.to_string();
        } else if !config.local_ai.preload_embedding_model {
            status.embedding_state = "idle".to_string();
        } else if status.embedding_state != "ready" {
            status.embedding_state = "missing".to_string();
        }
        if let Some(state) = stt_state {
            status.stt_state = state.to_string();
        } else if !config.local_ai.preload_stt_model {
            status.stt_state = "idle".to_string();
        }
        if let Some(state) = tts_state {
            status.tts_state = state.to_string();
        } else if !config.local_ai.preload_tts_voice {
            status.tts_state = "idle".to_string();
        }
        status.warning = warning;
        status.error_detail = None;
        status.error_category = None;
        status.download_progress = None;
        status.downloaded_bytes = None;
        status.total_bytes = None;
        status.download_speed_bps = None;
        status.eta_seconds = None;
    }

    pub async fn download_all_models(&self, config: &Config) -> Result<(), String> {
        if !config.local_ai.runtime_enabled {
            return Err("local ai is disabled".to_string());
        }
        let _guard = self.bootstrap_lock.lock().await;

        if provider_from_config(config) == LocalAiProvider::LmStudio {
            self.ensure_lm_studio_available(config).await?;
            let mut embedding_state = None;
            if config.local_ai.preload_embedding_model {
                let model_id = model_ids::effective_embedding_model_id(config);
                {
                    let mut status = self.status.lock();
                    status.state = "downloading".to_string();
                    status.embedding_state = "downloading".to_string();
                    status.warning = Some(format!(
                        "Downloading embedding model via Ollama: `{model_id}`"
                    ));
                }
                if let Err(err) = async {
                    self.ensure_ollama_server(config).await?;
                    self.ensure_ollama_model_available(&model_id, "embedding")
                        .await
                }
                .await
                {
                    log::warn!(
                        "[local_ai] LM Studio download_all_models embedding preload failed: {err}"
                    );
                    self.finalize_lm_studio_download_status(
                        config,
                        Some("missing"),
                        None,
                        None,
                        None,
                    );
                    return Err(err);
                }
                embedding_state = Some("ready");
            }
            let mut stt_warning = None;
            let mut stt_state = None;
            if config.local_ai.preload_stt_model {
                if let Err(err) = self.ensure_stt_asset_available(config).await {
                    log::warn!(
                        "[local_ai] LM Studio download_all_models STT preload failed: {err}"
                    );
                    stt_state = Some("missing");
                    stt_warning = Some(err);
                } else {
                    stt_state = Some("ready");
                }
            }
            let mut tts_warning = None;
            let mut tts_state = None;
            if config.local_ai.preload_tts_voice {
                if let Err(err) = self.ensure_tts_asset_available(config).await {
                    log::warn!(
                        "[local_ai] LM Studio download_all_models TTS preload failed: {err}"
                    );
                    tts_state = Some("missing");
                    tts_warning = Some(err);
                } else {
                    tts_state = Some("ready");
                }
            }
            let warning = match (stt_warning, tts_warning) {
                (Some(a), Some(b)) => Some(format!("{a}; {b}")),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            self.finalize_lm_studio_download_status(
                config,
                embedding_state,
                stt_state,
                tts_state,
                warning,
            );
            return Ok(());
        }

        self.ensure_ollama_server(config).await?;

        let mut steps = vec![
            ("chat", model_ids::effective_chat_model_id(config)),
            ("embedding", model_ids::effective_embedding_model_id(config)),
        ];
        if matches!(
            presets::vision_mode_for_config(&config.local_ai),
            VisionMode::Bundled
        ) {
            steps.insert(1, ("vision", model_ids::effective_vision_model_id(config)));
        }

        let total = steps.len();
        for (index, (label, model_id)) in steps.into_iter().enumerate() {
            {
                let mut status = self.status.lock();
                status.state = "downloading".to_string();
                status.warning = Some(format!(
                    "Downloading {} model {}/{}: `{}`",
                    label,
                    index + 1,
                    total,
                    model_id
                ));
                match label {
                    "vision" => status.vision_state = "downloading".to_string(),
                    "embedding" => status.embedding_state = "downloading".to_string(),
                    _ => {}
                }
            }
            self.ensure_ollama_model_available(&model_id, label).await?;
        }

        let mut stt_warning = None;
        if let Err(err) = self.ensure_stt_asset_available(config).await {
            self.status.lock().stt_state = "missing".to_string();
            stt_warning = Some(err);
        }

        let mut tts_warning = None;
        if let Err(err) = self.ensure_tts_asset_available(config).await {
            self.status.lock().tts_state = "missing".to_string();
            tts_warning = Some(err);
        }

        {
            let mut status = self.status.lock();
            status.state = "ready".to_string();
            status.vision_state = match presets::vision_mode_for_config(&config.local_ai) {
                VisionMode::Disabled => "disabled".to_string(),
                VisionMode::Ondemand => "idle".to_string(),
                VisionMode::Bundled => "ready".to_string(),
            };
            status.download_progress = Some(1.0);
            status.downloaded_bytes = None;
            status.total_bytes = None;
            status.download_speed_bps = None;
            status.eta_seconds = None;
            status.warning = match (stt_warning, tts_warning) {
                (Some(a), Some(b)) => Some(format!("{a}; {b}")),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
        }

        Ok(())
    }

    pub async fn download_asset(
        &self,
        config: &Config,
        capability: &str,
    ) -> Result<LocalAiAssetsStatus, String> {
        if !config.local_ai.runtime_enabled {
            return Err("local ai is disabled".to_string());
        }
        let _guard = self.bootstrap_lock.lock().await;

        let capability = capability.trim().to_ascii_lowercase();
        if provider_from_config(config) == LocalAiProvider::LmStudio
            && matches!(capability.as_str(), "chat" | "vision")
        {
            return Err(
                "LM Studio manages chat and vision model downloads. Load the model in LM Studio, then retry."
                    .to_string(),
            );
        }
        match capability.as_str() {
            "chat" => {
                self.ensure_ollama_server(config).await?;
                let model = model_ids::effective_chat_model_id(config);
                self.ensure_ollama_model_available(&model, "chat").await?;
            }
            "vision" => {
                if matches!(
                    presets::vision_mode_for_config(&config.local_ai),
                    VisionMode::Disabled
                ) {
                    return Err(
                        "Vision is disabled for this RAM tier. Switch to the 4-8 GB tier or above to enable it."
                            .to_string(),
                    );
                }
                self.ensure_ollama_server(config).await?;
                let model = model_ids::effective_vision_model_id(config);
                self.ensure_ollama_model_available(&model, "vision").await?;
            }
            "embedding" | "embeddings" => {
                self.ensure_ollama_server(config).await?;
                let model = model_ids::effective_embedding_model_id(config);
                self.ensure_ollama_model_available(&model, "embedding")
                    .await?;
            }
            "stt" => {
                self.ensure_stt_asset_available(config).await?;
            }
            "tts" => {
                self.ensure_tts_asset_available(config).await?;
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

    pub(in crate::openhuman::local_ai::service) async fn ensure_stt_asset_available(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        if resolve_stt_model_path(config).is_ok() {
            self.status.lock().stt_state = "ready".to_string();
            return Ok(());
        }

        let url = config
            .local_ai
            .stt_download_url
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                "STT model missing and no local_ai.stt_download_url configured".to_string()
            })?;
        let dest = stt_model_target_path(config);
        self.download_file_with_progress(url, &dest, "stt").await?;
        self.status.lock().stt_state = "ready".to_string();
        Ok(())
    }

    pub(in crate::openhuman::local_ai::service) async fn ensure_tts_asset_available(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        if resolve_tts_voice_path(config).is_ok() {
            self.status.lock().tts_state = "ready".to_string();
            return Ok(());
        }

        let url = config
            .local_ai
            .tts_download_url
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                "TTS voice missing and no local_ai.tts_download_url configured".to_string()
            })?;
        let dest = tts_model_target_path(config);
        self.download_file_with_progress(url, &dest, "tts").await?;

        if let Some(config_url) = config
            .local_ai
            .tts_config_download_url
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            let config_dest = std::path::PathBuf::from(format!("{}.json", dest.display()));
            let _ = self
                .download_file_with_progress(config_url, &config_dest, "tts-config")
                .await;
        }

        self.status.lock().tts_state = "ready".to_string();
        Ok(())
    }

    async fn download_file_with_progress(
        &self,
        url: &str,
        dest: &Path,
        label: &str,
    ) -> Result<(), String> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("failed to create destination directory: {e}"))?;
        }

        let response = self
            .http
            .get(url)
            // Large model assets (STT/TTS) can take minutes on slower links.
            // Avoid inheriting the short default client timeout for these streams.
            .timeout(std::time::Duration::from_secs(30 * 60))
            .send()
            .await
            .map_err(|e| format!("failed to start {label} download: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "failed to download {label} asset, status {}",
                response.status()
            ));
        }

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!("Downloading {label} asset"));
            match label {
                "stt" => status.stt_state = "downloading".to_string(),
                "tts" | "tts-config" => status.tts_state = "downloading".to_string(),
                _ => {}
            }
            status.download_progress = Some(0.0);
            status.downloaded_bytes = Some(0);
            status.total_bytes = response.content_length();
            status.download_speed_bps = Some(0);
            status.eta_seconds = None;
        }

        let total = response.content_length();
        let mut downloaded: u64 = 0;
        let started_at = std::time::Instant::now();
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| format!("failed to create destination file: {e}"))?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|e| format!("download stream error for {label}: {e}"))?
        {
            use tokio::io::AsyncWriteExt;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("failed writing {label} file: {e}"))?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
            let speed_bps = (downloaded as f64 / elapsed).round().max(0.0) as u64;
            let eta_seconds = total.and_then(|t| {
                if downloaded >= t || speed_bps == 0 {
                    None
                } else {
                    Some((t.saturating_sub(downloaded)) / speed_bps.max(1))
                }
            });

            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!("Downloading {label} asset"));
            match label {
                "stt" => status.stt_state = "downloading".to_string(),
                "tts" | "tts-config" => status.tts_state = "downloading".to_string(),
                _ => {}
            }
            status.downloaded_bytes = Some(downloaded);
            status.total_bytes = total;
            status.download_speed_bps = Some(speed_bps);
            status.eta_seconds = eta_seconds;
            status.download_progress = total
                .map(|t| (downloaded as f32 / t as f32).clamp(0.0, 1.0))
                .or(Some(0.0));
        }

        Ok(())
    }
}
