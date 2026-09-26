//! Helpers shared by the embed crate's end-to-end tests.
//!
//! Each test file owns its process: a runtime claims a process-wide slot, so
//! the files split by scenario rather than by assertion.

#![allow(dead_code)]

use openhuman_core::config::Config;
use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};
use serde_json::json;

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

/// A config that keeps the turn offline: no local runtimes, no spaCy, no
/// embeddings endpoint. Mirrors `crates/openhuman-core/src/bin/library_profile/harness.rs::fixture()`,
/// which is the recipe already proven against real turns.
pub fn offline_config() -> Config {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    config.runtime_python.enabled = false;
    config.memory_tree.spacy_enabled = false;
    config.memory_tree.embedding_endpoint = None;
    config.memory_tree.embedding_model = None;
    config.memory_tree.embedding_strict = false;
    // User-message memory autosave is intentionally fire-and-forget. It can
    // still be writing after a turn returns, which is useful in the product but
    // unrelated to these tests' contracts and would race the final
    // ephemeral-workspace cleanup assertion.
    config.memory.auto_save = false;
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
