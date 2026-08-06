use crate::openhuman::config::Config;
use crate::openhuman::inference::local::lm_studio::{
    apply_lm_studio_auth, lm_studio_base_url, ollama_tags_fallback_url, LmStudioModelsResponse,
};
use crate::openhuman::inference::local::ollama::{OllamaModelTag, OllamaTagsResponse};

use super::LocalAiService;

fn diagnostic_body_snippet(body: &str) -> String {
    const MAX_CHARS: usize = 512;
    let mut snippet: String = body.chars().take(MAX_CHARS).collect();
    if body.chars().count() > MAX_CHARS {
        snippet.push_str("...");
    }
    snippet
}

impl LocalAiService {
    pub(in crate::openhuman::inference::local::service) async fn ensure_lm_studio_available(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        // Probe connectivity only — the server must be reachable. Whether any
        // models are loaded is a separate concern surfaced via diagnostics and
        // the asset-status warning, so bootstrap can succeed and the UI can
        // show an actionable "load a model in LM Studio" CTA instead of a
        // hard error.
        self.list_lm_studio_models(config).await?;
        Ok(())
    }

    pub(in crate::openhuman::inference::local::service) async fn list_lm_studio_models(
        &self,
        config: &Config,
    ) -> Result<Vec<OllamaModelTag>, String> {
        let base = lm_studio_base_url(config);
        let url = format!("{base}/models");
        // GH #5055: log the *resolved* discovery URL so a wrong base URL is
        // diagnosable from app logs alone, without reproducing against the
        // runtime's own request log.
        tracing::debug!(
            target: "local_ai::lm_studio",
            %base,
            discovery_url = %url,
            api = "openai_v1_models",
            "[local_ai:lm_studio] list_models: resolved discovery URL — sending GET"
        );

        let request = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5));
        let response = apply_lm_studio_auth(request, config)
            .send()
            .await
            .map_err(|e| {
                tracing::debug!(
                    target: "local_ai::lm_studio",
                    %url,
                    error = %e,
                    "[local_ai:lm_studio] list_models: request failed"
                );
                format!("lm studio models request failed: {e}")
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            tracing::debug!(
                target: "local_ai::lm_studio",
                %url,
                %status,
                body = %diagnostic_body_snippet(&body),
                "[local_ai:lm_studio] list_models: non-success response"
            );

            // GH #5055: a 404 on `/v1/models` means this host is reachable but
            // does not serve the OpenAI catalog. Try the Ollama-native
            // `/api/tags` exactly once before giving up, so a runtime that only
            // speaks the Ollama listing still discovers its models. Discovery is
            // still selected by provider *type* first (`model_discovery_api`);
            // this is a recovery path, never a probe order, and it never runs
            // for any status other than 404.
            if status == reqwest::StatusCode::NOT_FOUND {
                if let Some(models) = self.list_ollama_tags_fallback(config, &base).await {
                    return Ok(models);
                }
            }

            return Err(format!(
                "lm studio models failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let body = response.text().await.map_err(|e| {
            tracing::debug!(
                target: "local_ai::lm_studio",
                %url,
                error = %e,
                "[local_ai:lm_studio] list_models: body read failed"
            );
            format!("lm studio models body read failed: {e}")
        })?;
        let payload: LmStudioModelsResponse = serde_json::from_str(&body).map_err(|e| {
            tracing::debug!(
                target: "local_ai::lm_studio",
                %url,
                error = %e,
                body = %diagnostic_body_snippet(&body),
                "[local_ai:lm_studio] list_models: parse failed"
            );
            format!("lm studio models parse failed: {e}")
        })?;

        Ok(payload
            .data
            .into_iter()
            .map(|model| OllamaModelTag {
                name: model.id,
                size: None,
                modified_at: None,
            })
            .collect())
    }

    /// One-shot Ollama-native `/api/tags` fallback for a `/v1/models` 404.
    ///
    /// Returns `Some(models)` only when the fallback actually produced a
    /// catalog; every failure returns `None` so the caller surfaces the original
    /// `/v1/models` error rather than a confusing second one. Logs at WARN on
    /// entry because taking this path means the configured base URL and the
    /// provider type disagree — the user should fix the configuration even
    /// though discovery recovered (GH #5055).
    async fn list_ollama_tags_fallback(
        &self,
        config: &Config,
        base: &str,
    ) -> Option<Vec<OllamaModelTag>> {
        let fallback_url = ollama_tags_fallback_url(base);
        tracing::warn!(
            target: "local_ai::lm_studio",
            %base,
            discovery_url = %fallback_url,
            api = "ollama_api_tags",
            "[local_ai:lm_studio] list_models: /v1/models returned 404 — retrying once against \
             the Ollama-native /api/tags. Check the configured base URL: an OpenAI-compatible \
             runtime should serve /v1/models."
        );

        let request = self
            .http
            .get(&fallback_url)
            .timeout(std::time::Duration::from_secs(5));
        let response = match apply_lm_studio_auth(request, config).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(
                    target: "local_ai::lm_studio",
                    url = %fallback_url,
                    error = %e,
                    "[local_ai:lm_studio] /api/tags fallback request failed"
                );
                return None;
            }
        };

        let status = response.status();
        if !status.is_success() {
            tracing::debug!(
                target: "local_ai::lm_studio",
                url = %fallback_url,
                %status,
                "[local_ai:lm_studio] /api/tags fallback returned non-success"
            );
            return None;
        }

        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(
                    target: "local_ai::lm_studio",
                    url = %fallback_url,
                    error = %e,
                    "[local_ai:lm_studio] /api/tags fallback body read failed"
                );
                return None;
            }
        };
        let payload: OllamaTagsResponse = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(
                    target: "local_ai::lm_studio",
                    url = %fallback_url,
                    error = %e,
                    body = %diagnostic_body_snippet(&body),
                    "[local_ai:lm_studio] /api/tags fallback parse failed"
                );
                return None;
            }
        };

        // Reject on an explicit error envelope, NOT on emptiness. LM Studio
        // answers unknown paths with `200 {"error": …}` and no models
        // (GH #5053) — that is the case that must fall through to the original
        // /v1/models error. A fresh Ollama with nothing pulled yet legitimately
        // returns `{"models":[]}`, and treating that as a failure hid a
        // reachable runtime behind a 404, so the UI could not offer the
        // model-download action.
        if let Some(error) = payload
            .error
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            tracing::debug!(
                target: "local_ai::lm_studio",
                url = %fallback_url,
                error = %error,
                "[local_ai:lm_studio] /api/tags fallback returned an error envelope — not a recovery"
            );
            return None;
        }
        if payload.models.is_empty() {
            tracing::info!(
                target: "local_ai::lm_studio",
                url = %fallback_url,
                "[local_ai:lm_studio] /api/tags fallback reached a reachable runtime with an empty catalog — recovering as zero models"
            );
        }

        tracing::info!(
            target: "local_ai::lm_studio",
            url = %fallback_url,
            model_count = payload.models.len(),
            "[local_ai:lm_studio] recovered model discovery via the Ollama /api/tags fallback"
        );
        Some(payload.models)
    }

    pub(in crate::openhuman::inference::local::service) async fn has_lm_studio_model(
        &self,
        config: &Config,
        model: &str,
    ) -> Result<bool, String> {
        let target = model.trim().to_ascii_lowercase();
        Ok(self
            .list_lm_studio_models(config)
            .await?
            .into_iter()
            .any(|m| m.name.to_ascii_lowercase() == target))
    }
}
