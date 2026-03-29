use std::path::Path;

use futures_util::TryStreamExt;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::paths::{
    resolve_stt_model_path, resolve_tts_voice_path, stt_model_target_path, tts_model_target_path,
};
use crate::openhuman::local_ai::types::{LocalAiAssetStatus, LocalAiAssetsStatus};

use super::LocalAiService;

impl LocalAiService {
    pub async fn assets_status(&self, config: &Config) -> Result<LocalAiAssetsStatus, String> {
        let chat_model = model_ids::effective_chat_model_id(config);
        let vision_model = model_ids::effective_vision_model_id(config);
        let embedding_model = model_ids::effective_embedding_model_id(config);
        let stt_model = model_ids::effective_stt_model_id(config);
        let tts_voice = model_ids::effective_tts_voice_id(config);

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
                state: if stt_path.is_some() {
                    "ready"
                } else {
                    "missing"
                }
                .to_string(),
                id: stt_model,
                provider: "whisper.cpp".to_string(),
                path: stt_path,
                warning: None,
            },
            tts: LocalAiAssetStatus {
                state: if tts_path.is_some() {
                    "ready"
                } else {
                    "missing"
                }
                .to_string(),
                id: tts_voice,
                provider: "piper".to_string(),
                path: tts_path,
                warning: None,
            },
            quantization: model_ids::effective_quantization(config),
        })
    }

    pub async fn download_all_models(&self, config: &Config) -> Result<(), String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let _guard = self.bootstrap_lock.lock().await;

        self.ensure_ollama_server(config).await?;

        let steps = vec![
            ("chat", model_ids::effective_chat_model_id(config)),
            ("vision", model_ids::effective_vision_model_id(config)),
            ("embedding", model_ids::effective_embedding_model_id(config)),
        ];

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
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let _guard = self.bootstrap_lock.lock().await;

        let capability = capability.trim().to_ascii_lowercase();
        match capability.as_str() {
            "chat" => {
                self.ensure_ollama_server(config).await?;
                let model = model_ids::effective_chat_model_id(config);
                self.ensure_ollama_model_available(&model, "chat").await?;
            }
            "vision" => {
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
            .send()
            .await
            .map_err(|e| format!("failed to start {label} download: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "failed to download {label} asset, status {}",
                response.status()
            ));
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
