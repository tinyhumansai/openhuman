use crate::openhuman::agent::multimodal;
use crate::openhuman::config::Config;
use crate::openhuman::inference::local::ollama::{
    ollama_base_url_from_config, redact_ollama_base_url, OllamaGenerateOptions,
    OllamaGenerateRequest,
};
use crate::openhuman::inference::model_ids;
use crate::openhuman::inference::presets::{self, VisionMode};
use crate::openhuman::inference::types::LocalAiEmbeddingResult;
use tinyagents::harness::embeddings::{
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
        c.local_ai.runtime_enabled = true;
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
        let _guard = crate::openhuman::inference::inference_test_guard();

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
        let _guard = crate::openhuman::inference::inference_test_guard();

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
        config.local_ai.runtime_enabled = false;
        let service = LocalAiService::new(&config);
        let err = service.embed(&config, &["x".into()]).await.unwrap_err();
        assert!(err.contains("local ai is disabled"));
    }

    #[test]
    fn embedding_dimensions_match_supported_legacy_models() {
        assert_eq!(embedding_dimensions("bge-m3"), Some(1024));
        assert_eq!(embedding_dimensions("all-minilm:latest"), Some(384));
        assert_eq!(embedding_dimensions("nomic-embed-text"), Some(768));
        assert_eq!(embedding_dimensions("user-managed-model"), None);
    }

    #[tokio::test]
    async fn vision_prompt_disabled_returns_error() {
        let mut config = Config::default();
        config.local_ai.runtime_enabled = false;
        let service = LocalAiService::new(&config);
        let err = service
            .vision_prompt(&config, "describe", &[], None)
            .await
            .unwrap_err();
        assert!(err.contains("local ai is disabled"));
    }

    // ── #5146 §Part 1: which model a vision request actually reaches ────────
    //
    // These drive the real `vision_prompt` path against a mock Ollama server.
    // `ready_service` marks the status "ready", which makes `bootstrap` return
    // early, so no process launch or network beyond the mock is involved.

    /// Mock Ollama exposing `/api/tags` with `installed` present, and an
    /// `/api/generate` that echoes back the `model` field it was sent. The
    /// echo is what lets a test assert *which* model the request targeted.
    fn mock_ollama_echoing_requested_model(installed: &'static str) -> Router {
        use axum::routing::get;
        Router::new()
            .route(
                "/api/tags",
                get(move || async move {
                    Json(json!({
                        "models": [
                            { "name": installed, "modified_at": "", "size": 0u64, "digest": "a" }
                        ]
                    }))
                }),
            )
            .route(
                "/api/generate",
                post(|Json(body): Json<serde_json::Value>| async move {
                    Json(json!({
                        "response": body["model"].as_str().unwrap_or("<no model field>"),
                        "done": true
                    }))
                }),
            )
    }

    /// A configured, genuinely vision-capable model must reach Ollama unchanged.
    ///
    /// Before #5146 the `MVP_ALLOWED_VISION_MODELS = &[""]` allowlist rewrote
    /// this to the empty string, so the request went out with `model: ""`.
    #[tokio::test]
    async fn vision_prompt_sends_the_configured_vision_capable_model() {
        let _guard = crate::openhuman::inference::inference_test_guard();

        let base = spawn_mock(mock_ollama_echoing_requested_model("llava:7b")).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let mut config = enabled_config();
        config.local_ai.vision_model_id = "llava:7b".to_string();
        let service = ready_service(&config);

        let result = service
            .vision_prompt(
                &config,
                "describe",
                &["data:image/png;base64,QUJD".to_string()],
                None,
            )
            .await;

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }

        assert_eq!(
            result.expect("vision prompt should succeed"),
            "llava:7b",
            "the configured vision model must reach Ollama unchanged"
        );
    }

    /// A configured-but-unpullable vision model must report a vision problem
    /// naming the model and the `ollama pull` that fixes it.
    #[tokio::test]
    async fn vision_prompt_reports_an_unavailable_vision_model() {
        use axum::routing::get;
        let _guard = crate::openhuman::inference::inference_test_guard();

        // Empty tag list, and a pull that refuses: nothing to fall back to.
        let app = Router::new()
            .route("/api/tags", get(|| async { Json(json!({ "models": [] })) }))
            .route(
                "/api/pull",
                post(|| async {
                    (
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "pull refused",
                    )
                }),
            );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let mut config = enabled_config();
        config.local_ai.vision_model_id = "llava:7b".to_string();
        let service = ready_service(&config);

        let err = service
            .vision_prompt(
                &config,
                "describe",
                &["data:image/png;base64,QUJD".to_string()],
                None,
            )
            .await
            .expect_err("an unpullable vision model must fail");

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }

        assert!(
            err.contains("llava:7b"),
            "error should name the model: {err}"
        );
        assert!(
            err.contains("ollama pull"),
            "error should say how to install it: {err}"
        );
        assert_eq!(service.status.lock().vision_state, "missing");
    }

    /// #5146 P1: a chat-only `vision_model_id` must fail *before* any network
    /// work, naming the configured model — no substitution, and above all no
    /// pull of a model the user never chose.
    ///
    /// The mock deliberately offers a working `/api/pull`. If the request ever
    /// reaches it, the substitution is back and this test fails on the assert
    /// that no pull was attempted rather than on a transport error.
    #[tokio::test]
    async fn chat_only_vision_model_errors_without_substituting_or_pulling() {
        use axum::routing::get;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let _guard = crate::openhuman::inference::inference_test_guard();

        let pulls = Arc::new(AtomicUsize::new(0));
        let pull_counter = pulls.clone();
        let app = Router::new()
            .route("/api/tags", get(|| async { Json(json!({ "models": [] })) }))
            .route(
                "/api/pull",
                post(move || {
                    let pulls = pull_counter.clone();
                    async move {
                        pulls.fetch_add(1, Ordering::SeqCst);
                        Json(json!({ "status": "success" }))
                    }
                }),
            );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let mut config = enabled_config();
        config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();
        let service = ready_service(&config);

        let err = service
            .vision_prompt(
                &config,
                "describe",
                &["data:image/png;base64,QUJD".to_string()],
                None,
            )
            .await
            .expect_err("a chat-only vision model must fail");

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }

        assert!(
            err.contains("gemma3n:e4b-it-q8_0"),
            "error must name the model the user actually configured: {err}"
        );
        assert!(
            err.contains("not vision-capable"),
            "error must explain what is wrong with it: {err}"
        );
        // The suggestion list legitimately contains `DEFAULT_LOW_VISION_MODEL`,
        // so its mere presence proves nothing. What must not happen is the
        // message presenting it as the model that *ran*; the `pulls` and
        // `vision_state` assertions below pin that nothing was fetched behind
        // the user's back.
        assert!(
            err.contains("for example"),
            "a vision-capable model must be offered as an example to choose, never as a \
             substitute that was already applied: {err}"
        );
        assert_eq!(
            pulls.load(Ordering::SeqCst),
            0,
            "no model may be downloaded for a vision request the user misconfigured"
        );
        assert_eq!(service.status.lock().vision_state, "missing");
    }

    /// #5146 P6: a reference that is not base64 (a filesystem path is the
    /// common case) must produce a message about what `image_refs` accepts,
    /// not Ollama's `illegal base64 data at input byte 19`.
    #[tokio::test]
    async fn non_base64_image_reference_is_rejected_with_guidance() {
        use axum::routing::get;
        let _guard = crate::openhuman::inference::inference_test_guard();

        // The payload check runs *after* model availability, so the model must
        // read as already installed or this would dial a real Ollama.
        let app = Router::new().route(
            "/api/tags",
            get(|| async {
                Json(json!({
                    "models": [
                        { "name": "llava:7b", "modified_at": "", "size": 0u64, "digest": "a" }
                    ]
                }))
            }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let mut config = enabled_config();
        config.local_ai.vision_model_id = "llava:7b".to_string();
        let service = ready_service(&config);

        let err = service
            .vision_prompt(
                &config,
                "describe",
                &["/tmp/vision-test.png".to_string()],
                None,
            )
            .await
            .expect_err("a filesystem path is not an image payload");

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }

        assert!(
            err.contains("base64"),
            "error must say what the parameter accepts: {err}"
        );
        assert!(
            err.contains("filesystem path"),
            "error must name the mistake the caller actually made: {err}"
        );
    }
}
