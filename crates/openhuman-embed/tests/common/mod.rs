//! Helpers shared by the embed crate's end-to-end tests.
//!
//! Each test file owns its process: a runtime claims a process-wide slot, so
//! the files split by scenario rather than by assertion.

#![allow(dead_code)]

use openhuman_core::config::Config;
use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};
use serde_json::json;
use wiremock::{MockServer, Request};

/// Requests that represent model inference, excluding the new `/models` discovery call.
pub async fn chat_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .expect("provider recorded requests")
        .into_iter()
        .filter(|request| request.url.path().ends_with("/chat/completions"))
        .collect()
}

/// An OpenAI-compatible chat completion carrying `content`.
pub fn chat_completion(content: &str) -> serde_json::Value {
    json!({
        "id": "chatcmpl-embed-test",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": "embed-test-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

/// A config that keeps the turn offline: no local runtimes and no
/// conversation memory. Mirrors `profile/src/bin/library_profile/harness.rs::fixture()` in openhuman-benchmarks,
/// which is the recipe already proven against real turns.
pub fn offline_config() -> Config {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    // Conversation memory ingest is intentionally fire-and-forget. It can
    // still be buffering after a turn returns, which is useful in the product
    // but unrelated to these tests' contracts and would race the final
    // ephemeral-workspace cleanup assertion.
    config.memory.conversations.enabled = false;
    // Session-store dual writes and shadow reads are also deliberately
    // fire-and-forget; leaving them on makes their detached filesystem work
    // race the synchronous ephemeral cleanup.
    config.agent.session_dual_write = false;
    config.agent.session_shadow_reads = false;
    config.default_temperature = 0.0;
    config
}

/// The tuned runtime the harness documents as the caller's responsibility.
///
/// A default 2 MiB worker stack overflows on a turn that delegates to a
/// sub-agent and aborts the whole process, so building it the documented way is
/// both what the test needs and a check that the documented way works.
pub fn runtime() -> tokio::runtime::Runtime {
    runtime_without_master_key()
}

/// A tuned async runtime for profile hosts which initialize keys after choosing their root.
pub fn runtime_without_master_key() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .build()
        .expect("tokio runtime")
}

/// A stub backend that answers every incidental non-inference call.
pub async fn stub_backend() -> wiremock::MockServer {
    let backend = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": { "id": "embed-test", "email": "local@openhuman.local" }
        })))
        .mount(&backend)
        .await;
    backend
}

/// An OpenAI-compatible chat completion asking the harness to call `tool`.
pub fn tool_call_completion(tool: &str, arguments: &str) -> serde_json::Value {
    json!({
        "id": "chatcmpl-embed-test-tool",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": "embed-test-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_embed_test_1",
                    "type": "function",
                    "function": { "name": tool, "arguments": arguments }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
}

/// Prefix of the user message that carries tool results in the prompt-guided
/// (text) tool-call dialect, which the harness uses for models it does not
/// know to support native tool calling.
const PROMPT_TOOL_RESULTS_PREFIX: &str = "[Tool results]";

/// The concatenated tool results of a recorded request, in either dialect:
/// `tool`-role messages (native tool calling) or the `[Tool results]` user
/// message (prompt-guided tool calling).
pub fn tool_results(request: &wiremock::Request) -> String {
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
    body.get("messages")
        .and_then(|m| m.as_array())
        .map(|messages| {
            messages
                .iter()
                .filter_map(|m| {
                    let content = match m.get("content")? {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    match m.get("role").and_then(|r| r.as_str()) {
                        Some("tool") => Some(content),
                        // The harness may prepend continuation guidance and
                        // the active user request before the tool-result block.
                        Some("user") if content.contains(PROMPT_TOOL_RESULTS_PREFIX) => Some(
                            content
                                .split_once(PROMPT_TOOL_RESULTS_PREFIX)?
                                .1
                                .to_string(),
                        ),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// An OpenAI-compatible provider answering `/v1/chat/completions` with `reply`.
pub async fn provider(reply: &str) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(chat_completion(reply)))
        .mount(&server)
        .await;
    server
}

/// The tool names a recorded chat-completion request advertised.
pub fn tool_names(request: &wiremock::Request) -> Vec<String> {
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
    body.get("tools")
        .and_then(|t| t.as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| {
                    t.pointer("/function/name")
                        .or_else(|| t.get("name"))
                        .and_then(|n| n.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A provider that answers its first chat requests with `steps`, in order,
/// then every later one with `fallback`.
pub async fn scripted_provider(
    steps: Vec<serde_json::Value>,
    fallback: &str,
) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    for (index, step) in steps.into_iter().enumerate() {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(step))
            .up_to_n_times(1)
            .with_priority(u8::try_from(index + 1).unwrap_or(u8::MAX))
            .mount(&server)
            .await;
    }
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(chat_completion(fallback)))
        .with_priority(u8::MAX)
        .mount(&server)
        .await;
    server
}

/// A routed provider on `server` with a fixed model.
pub fn route(server: &wiremock::MockServer, model: &str) -> openhuman_embed::Provider {
    openhuman_embed::Provider::openai_compatible(format!("{}/v1", server.uri()), "sk-test")
        .model(model)
}

/// Polls `check` until it yields `Some` or about ten seconds pass.
pub async fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    for _ in 0..1000 {
        if let Some(value) = check() {
            return value;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Registers a managed chat-model provider that sends inference to `base_url`
/// (a mock). For suites that run turns through the core's managed-inference
/// path, such as SaaS profiles, whose configs name no inference endpoint of
/// their own. The core carries no hosted provider, so the suite supplies one
/// through the generic chat-model registry.
pub struct PointedInference;

impl PointedInference {
    pub fn install(base_url: &str) {
        use openhuman_embed::__host::inference_registry::{
            register_chat_model, ChatModelEntry, ChatModelRequest, PROVIDER_OPENHUMAN,
        };
        use tinyinference_llm::providers::openai::OpenAiModel;

        let base_url = base_url.trim_end_matches('/').to_string();
        let build = move |request: &ChatModelRequest<'_>| {
            let model = request
                .model_override
                .or(request.default_model)
                .unwrap_or("embed-test-model")
                .to_string();
            // Like the hosted provider: the acting profile's own credential
            // rides as the bearer.
            let token = openhuman_embed::__host::security::credentials::session_support::resolve_backend_credential(request.config)
                .map(|credential| credential.secret().to_string())
                .map_err(anyhow::Error::msg)?;
            let wire =
                OpenAiModel::compatible_provider("EmbedTest", token, base_url.clone(), &model)
                    .with_native_tool_calling(request.native_tool_calling);
            let wire: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>> =
                std::sync::Arc::new(wire);
            Ok((wire, model))
        };
        register_chat_model(
            PROVIDER_OPENHUMAN,
            ChatModelEntry {
                build: std::sync::Arc::new(build),
                probe: None,
            },
        );
    }
}

/// A mock OpenAI-compatible endpoint that answers every chat completion with
/// `echo: <the last user message>`, streamed when the request asks for it,
/// and every other request with an empty success.
pub async fn echo_inference() -> MockServer {
    let server = MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::path_regex(r"chat/completions$"))
        .respond_with(EchoCompletion)
        .with_priority(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({ "success": true, "data": [] })),
        )
        .with_priority(10)
        .mount(&server)
        .await;
    server
}

/// The last user message of a chat-completion request body.
pub fn last_user_message(body: &serde_json::Value) -> String {
    body.get("messages")
        .and_then(|m| m.as_array())
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        })
        .and_then(|m| match m.get("content")? {
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        })
        .unwrap_or_default()
}

struct EchoCompletion;

impl wiremock::Respond for EchoCompletion {
    fn respond(&self, request: &Request) -> wiremock::ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let reply = format!("echo: {}", last_user_message(&body));
        if body.get("stream").and_then(|s| s.as_bool()) == Some(true) {
            let chunk = json!({
                "id": "chatcmpl-embed-test",
                "object": "chat.completion.chunk",
                "created": 1_700_000_000_u64,
                "model": "embed-test-model",
                "choices": [{ "index": 0, "delta": { "role": "assistant", "content": reply }, "finish_reason": null }]
            });
            let done = json!({
                "id": "chatcmpl-embed-test",
                "object": "chat.completion.chunk",
                "created": 1_700_000_000_u64,
                "model": "embed-test-model",
                "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
                "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
            });
            wiremock::ResponseTemplate::new(200).set_body_raw(
                format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                "text/event-stream",
            )
        } else {
            wiremock::ResponseTemplate::new(200).set_body_json(chat_completion(&reply))
        }
    }
}
