use futures_util::StreamExt;

use crate::openhuman::config::Config;
use crate::openhuman::inference::local::ollama::{
    ollama_base_url_from_config, OllamaPullEvent, OllamaPullProgress, OllamaPullRequest,
};
use crate::openhuman::inference::model_ids;
use crate::openhuman::inference::presets::{self, VisionMode};

use super::super::LocalAiService;
use super::util::interrupted_pull_settle_window_secs;

impl LocalAiService {
    pub(in crate::openhuman::inference::local::service) async fn ensure_models_available(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        let chat_model = model_ids::effective_chat_model_id(config);
        self.ensure_ollama_model_available(config, &chat_model, "chat")
            .await?;

        match presets::vision_mode_for_config(&config.local_ai) {
            VisionMode::Disabled => {
                self.status.lock().vision_state = "disabled".to_string();
            }
            VisionMode::Ondemand => {
                self.status.lock().vision_state = "idle".to_string();
            }
            VisionMode::Bundled => {
                let vision_model = model_ids::effective_vision_model_id(config);
                self.ensure_ollama_model_available(config, &vision_model, "vision")
                    .await?;
                self.status.lock().vision_state = "ready".to_string();
            }
        }

        let embedding_model = model_ids::effective_embedding_model_id(config);
        if config.local_ai.preload_embedding_model {
            self.ensure_ollama_model_available(config, &embedding_model, "embedding")
                .await?;
            self.status.lock().embedding_state = "ready".to_string();
        }

        if config.local_ai.preload_stt_model {
            self.ensure_stt_asset_available(config).await?;
        }

        if config.local_ai.preload_tts_voice {
            self.ensure_tts_asset_available(config).await?;
        }

        Ok(())
    }

    pub(in crate::openhuman::inference::local::service) async fn ensure_ollama_model_available(
        &self,
        config: &Config,
        model_id: &str,
        label: &str,
    ) -> Result<(), String> {
        let base_url = ollama_base_url_from_config(config);
        if self.has_model_at(&base_url, model_id).await? {
            return Ok(());
        }

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!(
                "Pulling {} model `{}` from Ollama library",
                label, model_id
            ));
            match label {
                "vision" => status.vision_state = "downloading".to_string(),
                "embedding" => status.embedding_state = "downloading".to_string(),
                _ => {}
            }
            status.download_progress = Some(0.0);
            status.downloaded_bytes = Some(0);
            status.total_bytes = None;
            status.download_speed_bps = Some(0);
            status.eta_seconds = None;
        }

        const MAX_PULL_RETRIES: usize = 3;
        const PULL_RETRY_BACKOFF_MS: u64 = 1_500;
        const PULL_INTERRUPT_SETTLE_SECS: u64 = 20;
        let mut last_error: Option<String> = None;

        for attempt in 1..=MAX_PULL_RETRIES {
            if attempt > 1 {
                let retry_msg = format!(
                    "Ollama pull stream interrupted. Retrying {}/{}...",
                    attempt, MAX_PULL_RETRIES
                );
                {
                    let mut status = self.status.lock();
                    status.state = "downloading".to_string();
                    status.warning = Some(retry_msg.clone());
                }
                log::warn!(
                    "[local_ai] pull retry {}/{} for model `{}` after interruption",
                    attempt,
                    MAX_PULL_RETRIES,
                    model_id
                );
                tokio::time::sleep(std::time::Duration::from_millis(
                    PULL_RETRY_BACKOFF_MS * attempt as u64,
                ))
                .await;
            }

            let response = match self
                .http
                .post(format!("{base_url}/api/pull"))
                .json(&OllamaPullRequest {
                    name: model_id.to_string(),
                    stream: true,
                })
                // Model pulls are long-running streaming responses; the default 30s
                // client timeout can interrupt healthy downloads mid-stream.
                .timeout(std::time::Duration::from_secs(30 * 60))
                .send()
                .await
            {
                Ok(response) => response,
                Err(e) => {
                    let err = format!("ollama pull request failed: {e}");
                    last_error = Some(err.clone());
                    if attempt < MAX_PULL_RETRIES {
                        continue;
                    }
                    return Err(format!("{err} after {MAX_PULL_RETRIES} attempts"));
                }
            };
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let detail = body.trim();
                return Err(format!(
                    "ollama pull failed with status {}{}",
                    status,
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                ));
            }

            let mut stream = response.bytes_stream();
            let mut pending = String::new();
            let mut stream_error: Option<String> = None;
            let started_at = std::time::Instant::now();
            let mut progress = OllamaPullProgress::default();
            let mut observed_bytes = false;
            while let Some(item) = stream.next().await {
                let chunk = match item {
                    Ok(value) => value,
                    Err(e) => {
                        stream_error = Some(format!("ollama pull stream error: {e}"));
                        break;
                    }
                };
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

                    progress.observe(&event);
                    let completed = progress.aggregate_downloaded();
                    let total = progress.aggregate_total();
                    let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                    let speed_bps = (completed as f64 / elapsed).round().max(0.0) as u64;
                    let eta_seconds = total.and_then(|t| {
                        if completed >= t || speed_bps == 0 {
                            None
                        } else {
                            Some((t.saturating_sub(completed)) / speed_bps.max(1))
                        }
                    });
                    observed_bytes |= completed > 0;

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

            if let Some(err) = stream_error {
                last_error = Some(err.clone());
                let resumed = self
                    .wait_for_model_after_pull_interruption(
                        &base_url,
                        model_id,
                        attempt,
                        MAX_PULL_RETRIES,
                        observed_bytes,
                        PULL_INTERRUPT_SETTLE_SECS,
                    )
                    .await?;
                if resumed {
                    break;
                }
                if attempt < MAX_PULL_RETRIES {
                    continue;
                }
                return Err(format!("{err} after {MAX_PULL_RETRIES} attempts"));
            }

            if self.has_model_at(&base_url, model_id).await? {
                break;
            }

            last_error = Some(format!(
                "ollama pull finished but model `{}` was not found",
                model_id
            ));
            let resumed = self
                .wait_for_model_after_pull_interruption(
                    &base_url,
                    model_id,
                    attempt,
                    MAX_PULL_RETRIES,
                    observed_bytes,
                    PULL_INTERRUPT_SETTLE_SECS,
                )
                .await?;
            if resumed {
                break;
            }
            if attempt < MAX_PULL_RETRIES {
                continue;
            }
        }

        if !self.has_model_at(&base_url, model_id).await? {
            return Err(last_error.unwrap_or_else(|| {
                format!(
                    "ollama pull finished but model `{}` was not found",
                    model_id
                )
            }));
        }

        match label {
            "vision" => self.status.lock().vision_state = "ready".to_string(),
            "embedding" => self.status.lock().embedding_state = "ready".to_string(),
            _ => {}
        }

        Ok(())
    }

    async fn wait_for_model_after_pull_interruption(
        &self,
        base_url: &str,
        model_id: &str,
        attempt: usize,
        max_attempts: usize,
        observed_bytes: bool,
        settle_window_secs: u64,
    ) -> Result<bool, String> {
        let wait_secs = interrupted_pull_settle_window_secs(observed_bytes, settle_window_secs);
        if wait_secs == 0 {
            return Ok(false);
        }

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!(
                "Ollama pull stream disconnected. Waiting up to {wait_secs}s for ongoing download to resume before retry {}/{}.",
                attempt + 1,
                max_attempts
            ));
        }
        log::warn!(
            "[local_ai] pull stream interrupted for model `{}`; waiting up to {}s before retry {}/{}",
            model_id,
            wait_secs,
            attempt + 1,
            max_attempts
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_secs);
        while std::time::Instant::now() < deadline {
            if self.has_model_at(base_url, model_id).await? {
                log::info!(
                    "[local_ai] model `{}` became available after interrupted pull stream",
                    model_id
                );
                return Ok(true);
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        Ok(false)
    }
}
