use crate::openhuman::agent::multimodal;
use crate::openhuman::config::Config;
use crate::openhuman::inference::local::ollama::{
    ollama_base_url_from_config, redact_ollama_base_url, OllamaGenerateOptions,
    OllamaGenerateRequest,
};
use crate::openhuman::inference::model_ids;
use crate::openhuman::inference::presets::{self, VisionMode};
use crate::openhuman::inference::types::LocalAiEmbeddingResult;
use tinyinference::embeddings::{
    EmbeddingModel, OllamaEmbeddingModel, DEFAULT_OLLAMA_DIMENSIONS,
    RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
};

use super::LocalAiService;

fn embedding_dimensions(model_id: &str) -> Option<usize> {
    let normalized = model_id.trim().to_ascii_lowercase();
    if normalized.starts_with("all-minilm") {
        Some(384)
    } else if normalized.contains("bge-m3") || normalized.starts_with("mxbai-embed-large") {
        Some(DEFAULT_OLLAMA_DIMENSIONS)
    } else if normalized.starts_with("nomic-embed-text") {
        Some(768)
    } else {
        None
    }
}

impl LocalAiService {
    pub async fn vision_prompt(
        &self,
        config: &Config,
        prompt: &str,
        image_refs: &[String],
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.runtime_enabled {
            return Err("local ai is disabled".to_string());
        }
        if image_refs.is_empty() {
            return Err("vision prompt requires at least one image reference".to_string());
        }
        if matches!(
            presets::vision_mode_for_config(&config.local_ai),
            VisionMode::Disabled
        ) {
            self.status.lock().vision_state = "disabled".to_string();
            return Err(
                "vision summaries are unavailable for this RAM tier. Use OCR-only summarization or switch to a higher local AI tier."
                    .to_string(),
            );
        }
        self.bootstrap(config).await;

        // Resolve through `resolve_vision_model_id` rather than
        // `effective_vision_model_id`: the latter returns an empty string when
        // there is no usable vision model, which used to be handed straight to
        // `ensure_ollama_model_available` and became a nameless `POST
        // /api/pull` retried three times before failing opaquely (#5146).
        // The resolver guarantees a non-empty, vision-capable id or a message
        // that says what to configure.
        //
        // Since #5146 P1 it also refuses a *chat-only* configured model instead
        // of substituting one. That arm IS reachable: `vision_mode_for_config`
        // only checks the tier, so a user on a vision-enabled tier who points
        // `vision_model_id` at their chat model reaches here and now gets told
        // exactly that, rather than having a 1.7 GB substitute pulled behind
        // their back.
        let vision_model = match model_ids::resolve_vision_model_id(config) {
            Ok(model) => model,
            Err(error) => {
                self.status.lock().vision_state = "missing".to_string();
                tracing::warn!(
                    target: "local_ai::vision",
                    %error,
                    "[local_ai:vision] no vision-capable model resolved; refusing request"
                );
                return Err(error);
            }
        };
        tracing::debug!(
            target: "local_ai::vision",
            model = %vision_model,
            "[local_ai:vision] resolved vision-capable model"
        );

        // A model that is configured but not pulled (and cannot be pulled)
        // must also read as a vision problem, not a generic pull failure.
        if let Err(error) = self
            .ensure_ollama_model_available(config, &vision_model, "vision")
            .await
        {
            self.status.lock().vision_state = "missing".to_string();
            tracing::warn!(
                target: "local_ai::vision",
                model = %vision_model,
                %error,
                "[local_ai:vision] vision model unavailable"
            );
            // `vision_model` is now always the model the user configured, so
            // "pull it" can no longer name a model they never chose — the
            // substitution note this used to carry has no case left to cover.
            return Err(format!(
                "local vision model `{vision_model}` is not available: {error}. \
                 Pull it with `ollama pull {vision_model}`, or route the vision \
                 workload to a cloud provider with `vision_provider`."
            ));
        }

        let images: Vec<String> = image_refs
            .iter()
            .filter_map(|reference| multimodal::extract_ollama_image_payload(reference))
            .collect();
        if images.is_empty() {
            // #5146 P6: the most common cause is a caller passing a filesystem
            // path. Say what this parameter actually takes rather than leaving
            // the caller to discover it from Ollama's "illegal base64 data".
            return Err(format!(
                "none of the {} supplied image reference(s) carried a usable image payload. \
                 `image_refs` takes a `data:image/...;base64,<data>` URI or a bare base64 \
                 string — a filesystem path is not read from disk here.",
                image_refs.len()
            ));
        }

        // Vision generation is background LLM-bound work; gate it through
        // the scheduler's global LLM permit.
        let _gate_permit = crate::openhuman::cron::scheduler_gate::wait_for_capacity().await;

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

        let base = ollama_base_url_from_config(config);
        let url = format!("{base}/api/generate");
        let body_bytes = serde_json::to_vec(&body).map(|v| v.len()).unwrap_or(0);
        tracing::debug!(
            target: "local_ai::vision",
            %base,
            %url,
            model = %body.model,
            prompt_chars = body.prompt.chars().count(),
            images = body.images.as_ref().map(|v| v.len()).unwrap_or(0),
            body_bytes,
            "[local_ai:vision] sending generate request"
        );

        let response = self.http.post(&url).json(&body).send().await.map_err(|e| {
            tracing::warn!(
                target: "local_ai::vision",
                %url,
                error = %e,
                "[local_ai:vision] request send failed"
            );
            format!("ollama vision request failed: {e}")
        })?;

        let status = response.status();
        tracing::debug!(
            target: "local_ai::vision",
            %url,
            %status,
            "[local_ai:vision] received response"
        );

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            tracing::warn!(
                target: "local_ai::vision",
                %url,
                %status,
                body = %detail,
                "[local_ai:vision] non-success response"
            );
            return Err(format!(
                "ollama vision request failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let payload: crate::openhuman::inference::local::ollama::OllamaGenerateResponse = response
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
        if !config.local_ai.runtime_enabled {
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
        let embedding_model = model_ids::effective_embedding_model_id(config);
        self.ensure_ollama_model_available(config, &embedding_model, "embedding")
            .await?;

        // Embeds are bge-m3 calls (8K context, ~1.3 GB resident) — the
        // single concurrent embed that has historically crashed the
        // user's laptop when stacked with other Ollama work. Gate it.
        let _gate_permit = crate::openhuman::cron::scheduler_gate::wait_for_capacity().await;

        let embed_base = ollama_base_url_from_config(config);
        let dimensions = embedding_dimensions(&embedding_model);
        log::debug!(
            "[local_ai:embed] embed: using model={} dimensions={} base_url={}",
            embedding_model,
            dimensions
                .map(|value| value.to_string())
                .unwrap_or_else(|| "dynamic".to_string()),
            redact_ollama_base_url(&embed_base)
        );
        let (dims, vectors) = if let Some(dimensions) = dimensions {
            let model = OllamaEmbeddingModel::try_new(&embed_base, &embedding_model, dimensions)
                .map_err(|error| format!("invalid local embedding RPC configuration: {error}"))?
                .with_client(self.http.clone())
                .with_context_options(
                    RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
                    RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
                );
            let vectors = model
                .embed(&items)
                .await
                .map_err(|error| format!("local embedding RPC failed: {error}"))?;
            (model.dimensions(), vectors)
        } else {
            OllamaEmbeddingModel::embed_discovering_dimensions(
                &embed_base,
                &embedding_model,
                self.http.clone(),
                &items,
                RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
                RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
            )
            .await
            .map_err(|error| format!("local embedding RPC failed: {error}"))?
        };
        self.status.lock().embedding_state = "ready".to_string();
        Ok(LocalAiEmbeddingResult {
            model_id: embedding_model,
            dimensions: dims,
            vectors,
        })
    }
}

#[cfg(test)]
#[path = "vision_embed_tests.rs"]
mod tests;
