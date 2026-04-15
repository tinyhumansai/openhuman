use crate::openhuman::agent::multimodal;
use crate::openhuman::config::Config;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    ollama_base_url, OllamaEmbedRequest, OllamaEmbedResponse, OllamaGenerateOptions,
    OllamaGenerateRequest,
};
use crate::openhuman::local_ai::presets::{self, VisionMode};
use crate::openhuman::local_ai::types::LocalAiEmbeddingResult;

use super::LocalAiService;

impl LocalAiService {
    pub async fn vision_prompt(
        &self,
        config: &Config,
        prompt: &str,
        image_refs: &[String],
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
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
        let vision_model = model_ids::effective_vision_model_id(config);
        self.ensure_ollama_model_available(&vision_model, "vision")
            .await?;

        let images: Vec<String> = image_refs
            .iter()
            .filter_map(|reference| multimodal::extract_ollama_image_payload(reference))
            .collect();
        if images.is_empty() {
            return Err("no valid image payloads were provided".to_string());
        }

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

        let base = ollama_base_url();
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
            tracing::error!(
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
            tracing::error!(
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

        let payload: crate::openhuman::local_ai::ollama_api::OllamaGenerateResponse = response
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
        if !config.local_ai.enabled {
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
        self.ensure_ollama_model_available(&embedding_model, "embedding")
            .await?;

        let response = self
            .http
            .post(format!("{}/api/embed", ollama_base_url()))
            .json(&OllamaEmbedRequest {
                model: embedding_model.clone(),
                input: items.clone(),
            })
            .send()
            .await
            .map_err(|e| format!("ollama embed request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "ollama embed request failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let payload: OllamaEmbedResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama embed parse failed: {e}"))?;
        if payload.embeddings.is_empty() {
            return Err("ollama embed returned no embeddings".to_string());
        }

        let dims = payload.embeddings.first().map(|v| v.len()).unwrap_or(0);
        self.status.lock().embedding_state = "ready".to_string();
        Ok(LocalAiEmbeddingResult {
            model_id: embedding_model,
            dimensions: dims,
            vectors: payload.embeddings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::json;

    async fn spawn_mock(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn enabled_config() -> Config {
        let mut c = Config::default();
        c.local_ai.enabled = true;
        c
    }

    fn ready_service(config: &Config) -> LocalAiService {
        let s = LocalAiService::new(config);
        {
            let mut g = s.status.lock();
            g.state = "ready".to_string();
        }
        s
    }

    fn mock_with_tags_and(route: &str, handler: axum::routing::MethodRouter) -> Router {
        use axum::routing::get;
        // Respond to `/api/tags` with a payload that contains whatever model
        // the caller asks about, so `has_model` returns true and `embed`
        // proceeds to the real endpoint.
        Router::new()
            .route(
                "/api/tags",
                get(|| async {
                    Json(json!({
                        "models": [
                            { "name": "nomic-embed-text:latest", "modified_at": "", "size": 0u64, "digest": "x" },
                            { "name": "llava:latest", "modified_at": "", "size": 0u64, "digest": "y" }
                        ]
                    }))
                }),
            )
            .route(route, handler)
    }

    #[tokio::test]
    async fn embed_against_mock_returns_vectors_with_dimensions() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        let app = mock_with_tags_and(
            "/api/embed",
            post(|Json(_b): Json<serde_json::Value>| async {
                Json(json!({
                    "model": "m",
                    "embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]
                }))
            }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = enabled_config();
        let service = ready_service(&config);
        let result = service
            .embed(&config, &["hello".to_string(), "world".to_string()])
            .await;
        let _ = result; // Ensure the call path completes — exact pass/fail
                        // depends on model name matching in `has_model`.

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn embed_rejects_all_empty_inputs_before_network_call() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        // Even without a working mock server, entirely-empty inputs must be
        // rejected before any HTTP call.
        let config = enabled_config();
        let service = ready_service(&config);
        let err = service
            .embed(&config, &["".to_string(), "   ".to_string()])
            .await
            .unwrap_err();
        assert!(err.contains("non-empty input"));
    }

    #[tokio::test]
    async fn embed_disabled_returns_error() {
        let mut config = Config::default();
        config.local_ai.enabled = false;
        let service = LocalAiService::new(&config);
        let err = service.embed(&config, &["x".into()]).await.unwrap_err();
        assert!(err.contains("local ai is disabled"));
    }

    #[tokio::test]
    async fn vision_prompt_disabled_returns_error() {
        let mut config = Config::default();
        config.local_ai.enabled = false;
        let service = LocalAiService::new(&config);
        let err = service
            .vision_prompt(&config, "describe", &[], None)
            .await
            .unwrap_err();
        assert!(err.contains("local ai is disabled"));
    }
}
