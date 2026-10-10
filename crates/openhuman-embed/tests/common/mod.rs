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
    config.runtime_python.enabled = false;
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
    static KEYRING: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let directory = KEYRING.get_or_init(|| tempfile::tempdir().expect("scratch keyring workspace"));
    runtime_with_keyring(directory.path())
}

/// Build the tuned runtime with the fixture's intended encrypted keyring root.
/// SaaS fixtures supply their operator workspace, which their boot guard requires.
pub fn runtime_with_keyring(workspace: &std::path::Path) -> tokio::runtime::Runtime {
    // Core is a normal dependency here, so its unit-test keyring fallback is
    // absent. Supply a fresh headless key before any credential operation.
    static KEY: std::sync::Once = std::sync::Once::new();
    KEY.call_once(|| {
        let bytes: [u8; 32] = rand::random();
        let key: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "encrypted_file");
        std::env::remove_var("OPENHUMAN_KEYRING_MASTER_KEY_FILE");
        std::env::set_var("OPENHUMAN_KEYRING_MASTER_KEY", key);
    });
    openhuman_core::security::keyring::init_workspace(workspace);
    openhuman_core::security::keyring::init_master_key().expect("headless test master key");
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

/// A backend transport that sends managed inference to `base_url` (a mock)
/// and answers every control-plane call as "no backend". For suites that run
/// turns through the core's managed-inference path, such as SaaS profiles,
/// whose configs name no inference endpoint of their own.
pub struct PointedTransport {
    base_url: String,
    client: reqwest::Client,
}

impl PointedTransport {
    pub fn install(base_url: &str) {
        openhuman_embed::install_backend_transport(std::sync::Arc::new(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }));
    }
}

#[async_trait::async_trait]
impl openhuman_embed::BackendTransport for PointedTransport {
    async fn send_json(
        &self,
        req: openhuman_embed::BackendRequest<'_>,
    ) -> Result<serde_json::Value, openhuman_embed::BackendTransportError> {
        // Forward inference requests to the pointed mock endpoint.
        // This uses the same HTTP client as a real transport would.
        let url = format!("{}/{}", self.base_url, req.path.trim_start_matches('/'));
        let resp = self
            .client
            .post(&url)
            .json(&req.body)
            .send()
            .await
            .map_err(|_| openhuman_embed::BackendTransportError::Unavailable)?;
        resp.json()
            .await
            .map_err(|_| openhuman_embed::BackendTransportError::Unavailable)
    }

    async fn send_multipart(
        &self,
        _req: openhuman_embed::BackendRequest<'_>,
        _form: reqwest::multipart::Form,
    ) -> Result<serde_json::Value, openhuman_embed::BackendTransportError> {
        Err(openhuman_embed::BackendTransportError::Unavailable)
    }

    fn http_client(&self, _profile: openhuman_embed::TransportProfile) -> reqwest::Client {
        reqwest::Client::new()
    }

    fn base_url(
        &self,
        _configured: Option<&str>,
        _purpose: openhuman_embed::BaseUrlPurpose,
    ) -> String {
        self.base_url.clone()
    }

    fn product_identity(&self) -> String {
        "openhuman-embed-test".to_string()
    }

    fn attribution_headers(&self) -> reqwest::header::HeaderMap {
        reqwest::header::HeaderMap::new()
    }

    fn name(&self) -> &'static str {
        "embed-test-pointed"
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
