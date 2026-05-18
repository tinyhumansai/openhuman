//! OpenAI-compatible HTTP handlers for `/v1/chat/completions` and `/v1/models`.
//!
//! ## Mounting
//!
//! The router returned by [`router()`] is merged into the core axum server
//! in `src/core/jsonrpc.rs` via `.nest("/v1", inference::http::router())`.
//! It reuses the same bearer-token auth middleware that guards `/rpc`.
//!
//! ## Authentication
//!
//! All routes require `Authorization: Bearer <OPENHUMAN_CORE_TOKEN>` — the
//! same per-launch token used by the JSON-RPC endpoint. Missing or wrong
//! tokens get a `401 Unauthorized` from the shared middleware.
//!
//! ## Provider routing
//!
//! The `model` field in the request selects the provider:
//! - `"ollama:<model>"` or a bare model name → local Ollama
//! - `"<slug>:<model>"` → cloud provider entry by slug
//! - everything else → OpenHuman backend (session JWT)

use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{extract::State, Json, Router};
use futures_util::stream::{self, StreamExt};
use serde_json::json;
use tracing::{debug, error};

use crate::core::types::AppState;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider;
use crate::openhuman::inference::provider::traits::ChatMessage;

use super::types::{
    ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionDelta,
    ChatCompletionMessage, ChatCompletionRequest, ChatCompletionResponse, ChatCompletionUsage,
    ModelObject, ModelsResponse,
};

const LOG_PREFIX: &str = "[inference::http]";

/// Build the `/v1` axum sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/chat/completions", post(chat_completions_handler))
        .route("/models", get(models_handler))
}

/// `POST /v1/chat/completions`
///
/// Accepts an OpenAI-compatible request body. Routes through the unified
/// `Provider` trait — local (Ollama) for `ollama:*` model names, cloud otherwise.
async fn chat_completions_handler(
    State(_state): State<AppState>,
    Json(req): Json<ChatCompletionRequest>,
) -> Response {
    debug!(
        model = %req.model,
        stream = req.stream,
        message_count = req.messages.len(),
        "{LOG_PREFIX} chat_completions: start"
    );

    let config = match Config::load_or_init().await {
        Ok(c) => c,
        Err(e) => {
            error!("{LOG_PREFIX} chat_completions: config load failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": format!("config load failed: {e}"), "type": "internal_error" }})),
            )
                .into_response();
        }
    };

    // Build provider string from model name.
    // If the model already looks like a provider string, use it directly.
    // Otherwise treat a bare model name as an Ollama model.
    let provider_string = if req.model.starts_with("ollama:")
        || req.model.contains(':')
        || req.model == "openhuman"
    {
        req.model.clone()
    } else {
        // Bare model name (no colon) — route to Ollama local runtime.
        format!("ollama:{}", req.model)
    };

    let (provider_box, model_id) = match provider::factory::create_chat_provider_from_string(
        "agentic",
        &provider_string,
        &config,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            error!("{LOG_PREFIX} chat_completions: provider build failed: {e}");
            return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": { "message": format!("provider error: {e}"), "type": "invalid_request_error" }})),
                )
                    .into_response();
        }
    };

    // Map request messages to provider ChatMessage type.
    let messages: Vec<ChatMessage> = req
        .messages
        .iter()
        .map(|m| ChatMessage {
            id: None,
            role: m.role.clone(),
            content: m.content.clone(),
            extra_metadata: None,
        })
        .collect();

    // If the caller supplied a temperature but the model is on the unsupported
    // list, log a warning and drop it — sending temperature to o1/o3/o4/gpt-5
    // reasoning models causes an API error. The provider layer applies the same
    // check on the outbound body, so this is belt-and-suspenders for logging.
    let temperature = {
        let raw = req.temperature.unwrap_or(config.default_temperature);
        let suppressed = crate::openhuman::inference::provider::temperature::temperature_for_model(
            &model_id, raw, &config,
        );
        if suppressed.is_none() && req.temperature.is_some() {
            tracing::warn!(
                model = %model_id,
                requested_temperature = req.temperature.unwrap_or(0.0),
                "{LOG_PREFIX} dropping caller-supplied temperature — model is on temperature_unsupported_models list"
            );
        }
        raw // the Provider layer handles omission; we pass the value through
    };
    let completion_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
    let created = chrono::Utc::now().timestamp();
    let model_name = req.model.clone();

    if req.stream {
        // Streaming response via SSE
        let options = provider::traits::StreamOptions::new(true);
        let stream =
            provider_box.stream_chat_with_history(&messages, &model_id, temperature, options);

        let cid = completion_id.clone();
        let model_clone = model_name.clone();
        let event_stream = stream
            .enumerate()
            .map(move |(i, chunk_result)| {
                let cid = cid.clone();
                let model_clone = model_clone.clone();
                match chunk_result {
                    Ok(chunk) => {
                        let finish_reason = if chunk.is_final { Some("stop") } else { None };
                        let content = if chunk.delta.is_empty() && chunk.is_final {
                            None
                        } else {
                            Some(chunk.delta)
                        };
                        let sse_chunk = ChatCompletionChunk {
                            id: cid,
                            object: "chat.completion.chunk",
                            created,
                            model: model_clone,
                            choices: vec![ChatCompletionChunkChoice {
                                index: 0,
                                delta: ChatCompletionDelta {
                                    role: if i == 0 {
                                        Some("assistant".to_string())
                                    } else {
                                        None
                                    },
                                    content,
                                },
                                finish_reason,
                            }],
                        };
                        let data =
                            serde_json::to_string(&sse_chunk).unwrap_or_else(|_| "{}".to_string());
                        Ok::<Event, std::convert::Infallible>(Event::default().data(data))
                    }
                    Err(e) => {
                        let err_event = json!({
                            "error": { "message": e.to_string(), "type": "stream_error" }
                        });
                        Ok(Event::default()
                            .data(serde_json::to_string(&err_event).unwrap_or_default()))
                    }
                }
            })
            .chain(stream::once(async {
                Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]"))
            }));

        debug!("{LOG_PREFIX} chat_completions: streaming response started");
        return Sse::new(event_stream)
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    // Non-streaming: call chat_with_history
    match provider_box
        .chat_with_history(&messages, &model_id, temperature)
        .await
    {
        Ok(content) => {
            debug!("{LOG_PREFIX} chat_completions: non-streaming ok");
            let response = ChatCompletionResponse {
                id: completion_id,
                object: "chat.completion",
                created,
                model: model_name,
                choices: vec![ChatCompletionChoice {
                    index: 0,
                    message: ChatCompletionMessage {
                        role: "assistant".to_string(),
                        content,
                    },
                    finish_reason: "stop",
                }],
                usage: ChatCompletionUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                },
            };
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            error!("{LOG_PREFIX} chat_completions: inference failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": format!("inference error: {e}"), "type": "internal_error" }})),
            )
                .into_response()
        }
    }
}

/// `GET /v1/models`
///
/// Lists all configured models (local Ollama + cloud providers).
async fn models_handler(State(_state): State<AppState>) -> Response {
    debug!("{LOG_PREFIX} models: start");

    let config = match Config::load_or_init().await {
        Ok(c) => c,
        Err(e) => {
            error!("{LOG_PREFIX} models: config load failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "message": format!("config load failed: {e}") }})),
            )
                .into_response();
        }
    };

    let created = chrono::Utc::now().timestamp();
    let mut data: Vec<ModelObject> = Vec::new();

    // Cloud provider default models
    for cp in &config.cloud_providers {
        if let Some(ref model) = cp.default_model {
            data.push(ModelObject {
                id: format!("{}:{}", cp.slug, model),
                object: "model",
                created,
                owned_by: cp.slug.clone(),
            });
        }
    }

    // Configured local chat model (Ollama)
    if !config.local_ai.chat_model_id.is_empty() {
        data.push(ModelObject {
            id: format!("ollama:{}", config.local_ai.chat_model_id),
            object: "model",
            created,
            owned_by: "ollama".to_string(),
        });
    }

    debug!(model_count = data.len(), "{LOG_PREFIX} models: ok");
    (
        StatusCode::OK,
        Json(ModelsResponse {
            object: "list",
            data,
        }),
    )
        .into_response()
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
