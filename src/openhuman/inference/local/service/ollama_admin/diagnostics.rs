use crate::openhuman::config::Config;
use crate::openhuman::inference::local::lm_studio::lm_studio_base_url;
use crate::openhuman::inference::local::model_requirements::{
    evaluate_context, ContextEligibility, MIN_CONTEXT_TOKENS,
};
use crate::openhuman::inference::local::ollama::{
    ollama_base_url_from_config, OllamaModelShow, OllamaModelTag, OllamaShowRequest,
    OllamaShowResponse, OllamaTagsResponse,
};
use crate::openhuman::inference::local::provider::{provider_from_config, LocalAiProvider};
use crate::openhuman::inference::model_ids;
use crate::openhuman::inference::presets::{self, VisionMode};

use super::super::LocalAiService;
use super::util::lm_studio_models_error_means_unreachable;

impl LocalAiService {
    /// Run full diagnostics: check Ollama server health, list installed models,
    /// and verify expected models are present. Returns a JSON-serializable report.
    pub async fn diagnostics(&self, config: &Config) -> Result<serde_json::Value, String> {
        if provider_from_config(config) == LocalAiProvider::LmStudio {
            return self.lm_studio_diagnostics(config).await;
        }

        let base_url = ollama_base_url_from_config(config);
        let healthy = self.ollama_healthy_at(&base_url).await;
        let runner_ok = if healthy {
            self.ollama_runner_ok_at(&base_url).await
        } else {
            false
        };

        log::debug!(
            "[local_ai] diagnostics: entry base_url={} healthy={}",
            base_url,
            healthy
        );

        let (models, tags_error) = if healthy {
            match self.list_models_at(&base_url).await {
                Ok(models) => (models, None),
                Err(e) => (vec![], Some(e)),
            }
        } else {
            (vec![], None)
        };

        let expected_chat = model_ids::effective_chat_model_id(config);
        let expected_embedding = model_ids::effective_embedding_model_id(config);
        let expected_vision = model_ids::effective_vision_model_id(config);

        let model_names: Vec<String> = models.iter().map(|m| m.name.to_ascii_lowercase()).collect();
        let has = |target: &str| -> bool {
            let t = target.to_ascii_lowercase();
            model_names
                .iter()
                .any(|n| *n == t || n.starts_with(&(t.clone() + ":")))
        };

        let chat_found = has(&expected_chat);
        let embedding_found = has(&expected_embedding);
        let vision_found = has(&expected_vision);

        // Per-model native context window (vs the memory-layer minimum) and
        // chat-capability. `/api/show` is one bounded round-trip per installed
        // model, fetched concurrently and only on this diagnostics path; the
        // single call yields both signals.
        let model_shows: Vec<OllamaModelShow> = if healthy {
            futures_util::future::join_all(
                models
                    .iter()
                    .map(|m| self.fetch_model_show_at(&base_url, &m.name)),
            )
            .await
        } else {
            Vec::new()
        };
        let model_eligibilities: Vec<ContextEligibility> = model_shows
            .iter()
            .map(|s| evaluate_context(s.context_length))
            .collect();

        let installed_models: Vec<serde_json::Value> = models
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let eligibility = model_eligibilities.get(i).cloned();
                let context_length = match eligibility.as_ref() {
                    Some(ContextEligibility::Ok { context_length })
                    | Some(ContextEligibility::BelowMinimum { context_length, .. }) => {
                        Some(*context_length)
                    }
                    _ => None,
                };
                // `chat_capable: false` → embedding-only model the chat picker
                // must hide; `null`/`true` → keep visible (fail-open).
                // TAURI-RUST-4P6.
                let chat_capable = model_shows.get(i).and_then(|s| s.chat_capable);
                serde_json::json!({
                    "name": m.name,
                    "size": m.size,
                    "modified_at": m.modified_at,
                    "context_length": context_length,
                    "eligibility": eligibility,
                    "chat_capable": chat_capable,
                })
            })
            .collect();

        // Resolve the eligibility of an expected (active) model by tag prefix.
        let eligibility_for = |target: &str| -> Option<ContextEligibility> {
            let t = target.to_ascii_lowercase();
            models
                .iter()
                .zip(model_eligibilities.iter())
                .find(|(m, _)| {
                    let n = m.name.to_ascii_lowercase();
                    n == t || n.starts_with(&(t.clone() + ":"))
                })
                .map(|(_, e)| e.clone())
        };
        let chat_eligibility = eligibility_for(&expected_chat);
        let embedding_eligibility = eligibility_for(&expected_embedding);

        let binary_path = self.resolve_binary_path(config);

        let mut issues: Vec<String> = Vec::new();
        let repair_actions: Vec<serde_json::Value> = Vec::new();

        if !healthy {
            issues.push(format!(
                "Ollama server is not running or not reachable at {}",
                base_url
            ));
        }
        if healthy && !runner_ok {
            issues.push(
                "Configured Ollama runtime is reachable but cannot execute models. Restart the external runtime and retry."
                    .to_string(),
            );
        }
        if healthy && !chat_found {
            issues.push(format!("Chat model `{}` is not installed", expected_chat));
        }
        if healthy && config.local_ai.preload_embedding_model && !embedding_found {
            issues.push(format!(
                "Embedding model `{}` is not installed",
                expected_embedding
            ));
        }
        if healthy
            && matches!(
                presets::vision_mode_for_config(&config.local_ai),
                VisionMode::Bundled
            )
            && !vision_found
        {
            issues.push(format!(
                "Vision model `{}` is not installed",
                expected_vision
            ));
        }
        if let Some(ref e) = tags_error {
            issues.push(format!("Failed to list models: {e}"));
        }
        // Reject installed-but-too-small active models: a context window
        // below the memory-layer minimum silently truncates chunks /
        // summaries and corrupts recall.
        if let Some(ContextEligibility::BelowMinimum {
            context_length,
            required,
        }) = embedding_eligibility.as_ref()
        {
            issues.push(format!(
                "Embedding model `{}` has a {}-token context window; the memory layer \
                 requires at least {}. Choose an embedding model with a larger context \
                 (e.g. bge-m3).",
                expected_embedding, context_length, required
            ));
        }
        if let Some(ContextEligibility::BelowMinimum {
            context_length,
            required,
        }) = chat_eligibility.as_ref()
        {
            issues.push(format!(
                "Chat model `{}` has a {}-token context window; the memory layer \
                 requires at least {}.",
                expected_chat, context_length, required
            ));
        }

        log::debug!(
            "[local_ai] diagnostics: healthy={} models={} issues={} repair_actions={}",
            healthy,
            models.len(),
            issues.len(),
            repair_actions.len(),
        );

        Ok(serde_json::json!({
            "ollama_running": healthy,
            "ollama_runner_ok": runner_ok,
            "ollama_base_url": base_url,
            "ollama_binary_path": binary_path,
            "installed_models": installed_models,
            "context_requirement": {
                "min_context_tokens": MIN_CONTEXT_TOKENS,
            },
            "vision_mode": presets::vision_mode_for_config(&config.local_ai),
            "expected": {
                "chat_model": expected_chat,
                "chat_found": chat_found,
                "chat_eligibility": chat_eligibility,
                "embedding_model": expected_embedding,
                "embedding_found": embedding_found,
                "embedding_eligibility": embedding_eligibility,
                "vision_model": expected_vision,
                "vision_found": vision_found,
            },
            "issues": issues,
            "repair_actions": repair_actions,
            "ok": issues.is_empty(),
        }))
    }

    pub(in crate::openhuman::inference::local::service) async fn list_models_at(
        &self,
        base: &str,
    ) -> Result<Vec<OllamaModelTag>, String> {
        let url = format!("{base}/api/tags");
        tracing::debug!(
            target: "local_ai::ollama_admin",
            %base,
            %url,
            "[local_ai:ollama_admin] list_models: sending GET"
        );

        let response = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| {
                tracing::error!(
                    target: "local_ai::ollama_admin",
                    %url,
                    error = %e,
                    "[local_ai:ollama_admin] list_models: request send failed"
                );
                format!("ollama tags request failed: {e}")
            })?;

        let status = response.status();
        tracing::debug!(
            target: "local_ai::ollama_admin",
            %url,
            %status,
            "[local_ai:ollama_admin] list_models: received response"
        );

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            // Defense-in-depth (TAURI-RUST-A3T): the diagnostics caller already
            // tolerates this failure (degrades to empty models + surfaces the
            // error to the UI as `tags_error`). A non-2xx from the configured
            // endpoint is an external/server condition, not an openhuman code
            // defect, so log at `warn!` (breadcrumb) instead of `error!` to
            // avoid flooding Sentry on every diagnostics poll. The root-cause
            // fix for the common case lives in `validate_ollama_url` /
            // `reject_registry_host_or_default` (rejecting the ollama.com
            // public-website host, which returns 429).
            tracing::warn!(
                target: "local_ai::ollama_admin",
                %url,
                %status,
                body = %body,
                "[local_ai:ollama_admin] list_models: non-success response"
            );
            return Err(format!(
                "ollama tags failed with status {}: {}",
                status,
                body.trim()
            ));
        }

        // Read the body as text first so we can log it if JSON parsing fails.
        let body = response.text().await.map_err(|e| {
            tracing::error!(
                target: "local_ai::ollama_admin",
                %url,
                error = %e,
                "[local_ai:ollama_admin] list_models: failed to read response body"
            );
            format!("ollama tags body read failed: {e}")
        })?;

        let payload: OllamaTagsResponse = serde_json::from_str(&body).map_err(|e| {
            tracing::error!(
                target: "local_ai::ollama_admin",
                %url,
                body = %body,
                error = %e,
                "[local_ai:ollama_admin] list_models: JSON parse failed"
            );
            format!("ollama tags parse failed: {e}")
        })?;

        tracing::debug!(
            target: "local_ai::ollama_admin",
            %url,
            models = payload.models.len(),
            "[local_ai:ollama_admin] list_models: parsed successfully"
        );

        Ok(payload.models)
    }

    /// Fetch a model's native context window and chat-capability via Ollama
    /// `POST /api/show`.
    ///
    /// Both fields default to `None` on any failure (unreachable, non-2xx,
    /// parse error, or the metadata key is absent) — the caller maps a `None`
    /// context to an `Unknown` eligibility verdict, and a `None` chat-capable
    /// to "keep visible" (fail-open). One bounded HTTP round-trip per model;
    /// only ever invoked from the diagnostics path. The single round-trip
    /// yields both signals (context for the memory-layer gate, capability for
    /// the chat-picker filter — TAURI-RUST-4P6).
    pub(in crate::openhuman::inference::local::service) async fn fetch_model_show_at(
        &self,
        base_url: &str,
        model: &str,
    ) -> OllamaModelShow {
        let url = format!("{}/api/show", base_url.trim_end_matches('/'));
        let resp = match self
            .http
            .post(&url)
            .json(&OllamaShowRequest {
                model: model.to_string(),
            })
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::debug!(
                    target: "local_ai::ollama_admin",
                    %url, model, error = %e,
                    "[local_ai:ollama_admin] fetch_model_show: request failed"
                );
                return OllamaModelShow::default();
            }
        };
        let status = resp.status();
        if !status.is_success() {
            tracing::debug!(
                target: "local_ai::ollama_admin",
                %url, model, %status,
                "[local_ai:ollama_admin] fetch_model_show: non-success response"
            );
            return OllamaModelShow::default();
        }
        let parsed: OllamaShowResponse = match resp.json().await {
            Ok(parsed) => parsed,
            Err(e) => {
                tracing::debug!(
                    target: "local_ai::ollama_admin",
                    %url, model, error = %e,
                    "[local_ai:ollama_admin] fetch_model_show: JSON parse failed"
                );
                return OllamaModelShow::default();
            }
        };
        let show = OllamaModelShow {
            context_length: parsed.context_length(),
            chat_capable: parsed.chat_capability(),
        };
        tracing::debug!(
            target: "local_ai::ollama_admin",
            model,
            context_length = ?show.context_length,
            chat_capable = ?show.chat_capable,
            "[local_ai:ollama_admin] fetch_model_show: resolved"
        );
        show
    }

    async fn lm_studio_diagnostics(&self, config: &Config) -> Result<serde_json::Value, String> {
        let base_url = lm_studio_base_url(config);
        let models_result = self.list_lm_studio_models(config).await;
        let (models, models_error, healthy) = match models_result {
            Ok(models) => (models, None, true),
            Err(err) => {
                let reachable = !lm_studio_models_error_means_unreachable(&err);
                (vec![], Some(err), reachable)
            }
        };

        let expected_chat = model_ids::effective_chat_model_id(config);
        let model_names: Vec<String> = models.iter().map(|m| m.name.to_ascii_lowercase()).collect();
        let chat_found = model_names
            .iter()
            .any(|name| name == &expected_chat.to_ascii_lowercase());

        let mut issues: Vec<String> = Vec::new();
        let repair_actions: Vec<serde_json::Value> = Vec::new();

        if !healthy {
            let detail = models_error
                .as_deref()
                .map(|err| format!(": {err}"))
                .unwrap_or_default();
            issues.push(format!(
                "LM Studio server is not running or not reachable at {}{}",
                base_url, detail
            ));
        }
        if healthy && models_error.is_none() && models.is_empty() {
            issues.push("LM Studio is reachable but no models are loaded".to_string());
        } else if healthy && models_error.is_none() && !chat_found {
            issues.push(format!(
                "Chat model `{}` is not loaded in LM Studio",
                expected_chat
            ));
        }
        if healthy {
            if let Some(ref err) = models_error {
                issues.push(format!("Failed to list LM Studio models: {err}"));
            }
        }

        tracing::debug!(
            provider = "lm_studio",
            %base_url,
            healthy,
            models = models.len(),
            issues = issues.len(),
            "[local_ai] diagnostics"
        );

        Ok(serde_json::json!({
            "provider": "lm_studio",
            "lm_studio_running": healthy,
            "lm_studio_base_url": base_url,
            "ollama_running": false,
            "ollama_base_url": serde_json::Value::Null,
            "ollama_binary_path": serde_json::Value::Null,
            "installed_models": models,
            "vision_mode": "disabled",
            "expected": {
                "chat_model": expected_chat,
                "chat_found": chat_found,
                "embedding_model": model_ids::effective_embedding_model_id(config),
                "embedding_found": false,
                "vision_model": model_ids::effective_vision_model_id(config),
                "vision_found": false,
            },
            "issues": issues,
            "repair_actions": repair_actions,
            "ok": issues.is_empty(),
        }))
    }
}
