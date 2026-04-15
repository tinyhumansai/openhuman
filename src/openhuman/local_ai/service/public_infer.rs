use crate::openhuman::config::Config;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    ns_to_tps, ollama_base_url, OllamaGenerateOptions, OllamaGenerateRequest,
};
use crate::openhuman::local_ai::parse::{parse_suggestions, sanitize_inline_completion};
use crate::openhuman::local_ai::types::Suggestion;

use super::LocalAiService;

impl LocalAiService {
    pub async fn summarize(
        &self,
        config: &Config,
        text: &str,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let system = "You summarize internal assistant context. Keep concise bullet points.";
        let prompt = format!(
            "Summarize this text in concise bullet points. Preserve decisions and commitments.\\n\\n{}",
            text
        );
        self.inference(config, system, &prompt, max_tokens.or(Some(128)), true)
            .await
    }

    pub async fn prompt(
        &self,
        config: &Config,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }
        let system = if no_think {
            "You are a concise assistant. Return only the final answer. Do not include reasoning or chain-of-thought."
        } else {
            "You are a helpful assistant."
        };
        self.inference(config, system, prompt, max_tokens.or(Some(160)), no_think)
            .await
    }

    pub async fn suggest_questions(
        &self,
        config: &Config,
        context: &str,
    ) -> Result<Vec<Suggestion>, String> {
        if !config.local_ai.enabled {
            return Ok(Vec::new());
        }
        let system = "You create short suggested user prompts.";
        let prompt = format!(
            "Given this conversation context, produce up to {} short suggested next user prompts. Return one prompt per line with no numbering.\\n\\n{}",
            config.local_ai.max_suggestions.max(1),
            context
        );
        let raw = self
            .inference(config, system, &prompt, Some(96), true)
            .await?;
        Ok(parse_suggestions(
            &raw,
            config.local_ai.max_suggestions.max(1),
        ))
    }

    pub async fn inline_complete(
        &self,
        config: &Config,
        context: &str,
        style_preset: &str,
        style_instructions: Option<&str>,
        style_examples: &[String],
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Ok(String::new());
        }

        let mut prompt = String::from(
            "You are an inline autocomplete engine.\n\
     Predict the most likely continuation of the user's partial text.\n\
     Return only the exact continuation suffix.\n\
     Do not repeat or rewrite any part of the user's existing text.\n\
     Do not include any prefix labels like 'suffix:' or 'completion:'.\n\
     Return exactly one line with plain text and no quotes.\n\
     Do not add leading or trailing whitespace.\n\
     Do not add tabs or newlines.\n\
     Do not add non-breaking spaces or zero-width characters.\n\
     Preserve natural spacing inside the continuation only when required between words.\n\
     If the user is in the middle of a word, continue that word directly with no space.\n\
     If the continuation is uncertain, return an empty string.\n",
        );
        prompt.push_str(&format!("Style preset: {}\n", style_preset.trim()));
        if let Some(instructions) = style_instructions {
            if !instructions.trim().is_empty() {
                prompt.push_str(&format!("Style instructions: {}\n", instructions.trim()));
            }
        }
        if !style_examples.is_empty() {
            prompt.push_str("Style examples:\n");
            for example in style_examples.iter().take(8) {
                let trimmed = example.trim();
                if !trimmed.is_empty() {
                    prompt.push_str("- ");
                    prompt.push_str(trimmed);
                    prompt.push('\n');
                }
            }
        }
        let escaped_context = context.replace("</USER_TEXT>", "<\\/USER_TEXT>");
        prompt.push_str("\nUser text (verbatim):\n<USER_TEXT>\n");
        prompt.push_str(&escaped_context);
        prompt.push_str("\n</USER_TEXT>");

        let raw = self
            .inference_with_temperature_allow_empty(
                config,
                "You are a low-latency inline text completion assistant.",
                &prompt,
                max_tokens.or(Some(24)),
                true,
                0.05,
            )
            .await?;
        Ok(sanitize_inline_completion(&raw, context))
    }

    /// Multi-turn chat completion via Ollama /api/chat.
    /// Messages are `[{role: "user"|"assistant"|"system", content: "..."}]`.
    /// Returns the assistant reply string.
    pub(crate) async fn chat_with_history(
        &self,
        config: &Config,
        messages: Vec<crate::openhuman::local_ai::ollama_api::OllamaChatMessage>,
        max_tokens: Option<u32>,
    ) -> Result<String, String> {
        if !config.local_ai.enabled {
            return Err("local ai is disabled".to_string());
        }

        if !matches!(self.status.lock().state.as_str(), "ready") {
            self.bootstrap(config).await;
        }

        if messages.is_empty() {
            return Err("messages must not be empty".to_string());
        }

        tracing::debug!(
            message_count = messages.len(),
            model = %crate::openhuman::local_ai::model_ids::effective_chat_model_id(config),
            "[local_ai:chat] sending to ollama /api/chat"
        );

        let started = std::time::Instant::now();

        let body = crate::openhuman::local_ai::ollama_api::OllamaChatRequest {
            model: crate::openhuman::local_ai::model_ids::effective_chat_model_id(config),
            messages,
            stream: false,
            options: Some(
                crate::openhuman::local_ai::ollama_api::OllamaGenerateOptions {
                    temperature: Some(config.default_temperature as f32),
                    top_k: Some(40),
                    top_p: Some(0.9),
                    num_predict: max_tokens.map(|v| v as i32),
                },
            ),
        };

        let response = self
            .http
            .post(format!("{}/api/chat", ollama_base_url()))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("ollama chat request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "ollama chat failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }

        let payload: crate::openhuman::local_ai::ollama_api::OllamaChatResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama chat response parse failed: {e}"))?;

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let prompt_tps = payload
            .prompt_eval_count
            .zip(payload.prompt_eval_duration)
            .and_then(|(count, dur_ns)| ns_to_tps(count as f32, dur_ns));
        let gen_tps = payload
            .eval_count
            .zip(payload.eval_duration)
            .and_then(|(count, dur_ns)| ns_to_tps(count as f32, dur_ns));

        {
            let mut status = self.status.lock();
            status.state = "ready".to_string();
            status.last_latency_ms = Some(elapsed_ms);
            status.prompt_toks_per_sec = prompt_tps;
            status.gen_toks_per_sec = gen_tps;
            status.warning = None;
        }

        tracing::debug!(
            elapsed_ms,
            reply_len = payload.message.content.len(),
            "[local_ai:chat] ollama /api/chat done"
        );

        let reply = payload.message.content.trim().to_string();
        if reply.is_empty() {
            Err("ollama returned empty reply".to_string())
        } else {
            Ok(reply)
        }
    }

    pub(crate) async fn inference(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
    ) -> Result<String, String> {
        self.inference_with_temperature(config, system, prompt, max_tokens, no_think, 0.2)
            .await
    }

    pub(crate) async fn inference_with_temperature(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
        temperature: f32,
    ) -> Result<String, String> {
        self.inference_with_temperature_internal(
            config,
            system,
            prompt,
            max_tokens,
            no_think,
            temperature,
            false,
        )
        .await
    }

    async fn inference_with_temperature_allow_empty(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
        temperature: f32,
    ) -> Result<String, String> {
        self.inference_with_temperature_internal(
            config,
            system,
            prompt,
            max_tokens,
            no_think,
            temperature,
            true,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn inference_with_temperature_internal(
        &self,
        config: &Config,
        system: &str,
        prompt: &str,
        max_tokens: Option<u32>,
        no_think: bool,
        temperature: f32,
        allow_empty: bool,
    ) -> Result<String, String> {
        if !matches!(self.status.lock().state.as_str(), "ready") {
            self.bootstrap(config).await;
        }

        let started = std::time::Instant::now();
        let model_id = model_ids::effective_chat_model_id(config);

        // When `no_think` is set, append the instruction to the system
        // prompt so the model treats it as a directive rather than content
        // it might parrot back.
        let effective_system = if no_think {
            tracing::debug!(
                no_think = true,
                max_tokens = ?max_tokens,
                allow_empty = allow_empty,
                model = %model_id,
                "[local_ai:infer] selected no_think prompt branch"
            );
            format!("{system}\n\nRespond with only the final answer. No reasoning, no preamble.")
        } else {
            tracing::debug!(
                no_think = false,
                max_tokens = ?max_tokens,
                allow_empty = allow_empty,
                model = %model_id,
                "[local_ai:infer] selected standard prompt branch"
            );
            system.to_string()
        };

        let body = OllamaGenerateRequest {
            model: model_id,
            prompt: prompt.to_string(),
            system: Some(effective_system),
            images: None,
            stream: false,
            options: Some(OllamaGenerateOptions {
                temperature: Some(temperature),
                top_k: Some(40),
                top_p: Some(0.9),
                num_predict: max_tokens.map(|v| v as i32),
            }),
        };

        let response = self
            .http
            .post(format!("{}/api/generate", ollama_base_url()))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("ollama request failed: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "ollama request failed with status {}{}",
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
            .map_err(|e| format!("ollama response parse failed: {e}"))?;

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let prompt_tps = payload
            .prompt_eval_count
            .zip(payload.prompt_eval_duration)
            .and_then(|(count, dur_ns)| ns_to_tps(count as f32, dur_ns));
        let gen_tps = payload
            .eval_count
            .zip(payload.eval_duration)
            .and_then(|(count, dur_ns)| ns_to_tps(count as f32, dur_ns));

        {
            let mut status = self.status.lock();
            status.state = "ready".to_string();
            status.last_latency_ms = Some(elapsed_ms);
            status.prompt_toks_per_sec = prompt_tps;
            status.gen_toks_per_sec = gen_tps;
            status.warning = None;
        }

        if payload.response.trim().is_empty() {
            if allow_empty {
                Ok(String::new())
            } else {
                Err("ollama returned empty content".to_string())
            }
        } else {
            Ok(payload.response)
        }
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
        let mut config = Config::default();
        config.local_ai.enabled = true;
        config
    }

    /// Build a LocalAiService pre-seeded to `ready` so inference calls skip
    /// `bootstrap()` and hit the HTTP path directly.
    fn ready_service(config: &Config) -> LocalAiService {
        let service = LocalAiService::new(config);
        {
            let mut guard = service.status.lock();
            guard.state = "ready".to_string();
        }
        service
    }

    #[tokio::test]
    async fn inference_hits_ollama_generate_and_returns_response() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai test mutex");

        let app = Router::new().route(
            "/api/generate",
            post(|Json(_body): Json<serde_json::Value>| async move {
                Json(json!({
                    "model": "test",
                    "response": "hello from mock",
                    "done": true,
                    "total_duration": 1_000_000u64,
                    "prompt_eval_count": 5,
                    "prompt_eval_duration": 100_000u64,
                    "eval_count": 3,
                    "eval_duration": 500_000u64
                }))
            }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = enabled_config();
        let service = ready_service(&config);
        let reply = service
            .prompt(&config, "hi", Some(16), true)
            .await
            .expect("ollama prompt");
        assert_eq!(reply, "hello from mock");

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn inference_errors_on_non_success_status() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai test mutex");

        let app = Router::new().route(
            "/api/generate",
            post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = enabled_config();
        let service = ready_service(&config);
        let err = service.prompt(&config, "hi", None, true).await.unwrap_err();
        assert!(err.contains("500"));

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn inference_errors_on_empty_response_when_allow_empty_false() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai test mutex");

        let app = Router::new().route(
            "/api/generate",
            post(|| async {
                Json(json!({
                    "model": "test",
                    "response": "   ",
                    "done": true
                }))
            }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = enabled_config();
        let service = ready_service(&config);
        // `inference()` is the lower-level entry that hard-codes
        // allow_empty=false, so a whitespace-only mock response must
        // surface as the "empty content" error.
        let res = service.inference(&config, "", "hi", None, false).await;

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }

        let err = res.expect_err("whitespace response must be rejected when allow_empty=false");
        assert!(
            err.contains("empty"),
            "expected an empty-content error, got: {err}"
        );
    }

    #[tokio::test]
    async fn suggest_questions_parses_line_separated_output() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai test mutex");

        let app = Router::new().route(
            "/api/generate",
            post(|| async {
                Json(json!({
                    "model": "test",
                    "response": "What next?\nHow about this?\nTell me more.",
                    "done": true
                }))
            }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let mut config = enabled_config();
        config.local_ai.max_suggestions = 3;
        let service = ready_service(&config);
        let suggestions = service
            .suggest_questions(&config, "prior context")
            .await
            .expect("suggest_questions");
        assert!(!suggestions.is_empty());

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn summarize_disabled_returns_error() {
        // When local_ai is disabled the summarize fn should short-circuit.
        let mut config = Config::default();
        config.local_ai.enabled = false;
        let service = LocalAiService::new(&config);
        let err = service.summarize(&config, "text", None).await.unwrap_err();
        assert!(err.contains("local ai is disabled"));
    }

    #[tokio::test]
    async fn prompt_disabled_returns_error() {
        let mut config = Config::default();
        config.local_ai.enabled = false;
        let service = LocalAiService::new(&config);
        let err = service
            .prompt(&config, "text", None, false)
            .await
            .unwrap_err();
        assert!(err.contains("local ai is disabled"));
    }

    #[tokio::test]
    async fn suggest_questions_disabled_returns_empty() {
        let mut config = Config::default();
        config.local_ai.enabled = false;
        let service = LocalAiService::new(&config);
        let out = service.suggest_questions(&config, "ctx").await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn inline_complete_disabled_returns_empty_string() {
        let mut config = Config::default();
        config.local_ai.enabled = false;
        let service = LocalAiService::new(&config);
        let out = service
            .inline_complete(&config, "ctx", "casual", None, &[], None)
            .await
            .unwrap();
        assert!(out.is_empty());
    }
}
