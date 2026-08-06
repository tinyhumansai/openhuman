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

        // Held until every other preload has run. `ensure_ollama_model_available`
        // writes `status.warning` for its own transient "Pulling …" progress, so
        // publishing the vision reason here would let a later embedding/STT/TTS
        // pull bury it and leave `vision_state = "missing"` with no explanation.
        let mut vision_warning: Option<String> = None;

        match presets::vision_mode_for_config(&config.local_ai) {
            VisionMode::Disabled => {
                self.status.lock().vision_state = "disabled".to_string();
            }
            VisionMode::Ondemand => {
                self.status.lock().vision_state = "idle".to_string();
            }
            VisionMode::Bundled => match model_ids::resolve_vision_model_id(config) {
                Ok(vision_model) => {
                    self.ensure_ollama_model_available(config, &vision_model, "vision")
                        .await?;
                    self.status.lock().vision_state = "ready".to_string();
                }
                Err(err) => {
                    // A vision model the user misconfigured must not take the
                    // whole local runtime down with it. `bootstrap()` returns
                    // on the first `ensure_models_available` error, so
                    // propagating here would leave the service `degraded` and
                    // skip the embedding/STT/TTS preloads and the ready state —
                    // punishing chat for a vision-only mistake. Record it and
                    // carry on; `resolve_vision_model_id` raises the same
                    // message again, actionably, at request time.
                    tracing::warn!(
                        vision_model_id = %config.local_ai.vision_model_id.trim(),
                        vision_state = "missing",
                        %err,
                        "[local_ai] bundled vision model is unusable; continuing without vision"
                    );
                    self.status.lock().vision_state = "missing".to_string();
                    vision_warning = Some(err);
                }
            },
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

        // Last write wins, which is the point: whatever the preloads left behind
        // is transient progress text, while this is a standing configuration
        // problem the user has to act on.
        if let Some(err) = vision_warning {
            self.status.lock().warning = Some(err);
        }

        Ok(())
    }

    pub(in crate::openhuman::inference::local::service) async fn ensure_ollama_model_available(
        &self,
        config: &Config,
        model_id: &str,
        label: &str,
    ) -> Result<(), String> {
        // #5146 P1: never pull a nameless model. `effective_*_model_id` returns
        // an empty string when there is no usable model for a role, and several
        // callers feed that straight in here. Without this guard that became a
        // `POST /api/pull` with a blank name, retried three times, ending in an
        // opaque error — and it is the same code path that silently pulled a
        // ~1.7 GB vision substitute the user never chose. Fail immediately and
        // say which role is unconfigured instead.
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(format!(
                "no {label} model is configured for the local runtime, so there is nothing to \
                 download. Set the {label} model in Settings → Local AI (or pick a provider for \
                 the {label} workload) before retrying."
            ));
        }

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
