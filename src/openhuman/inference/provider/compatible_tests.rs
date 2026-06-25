use super::*;
use sentry::test::TestTransport;
use std::sync::Arc;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn make_provider(name: &str, url: &str, key: Option<&str>) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(name, url, key, AuthStyle::Bearer)
}

/// Wrap a ResponseMessage in a minimal ApiChatResponse for tests.
fn wrap_message(message: ResponseMessage) -> ApiChatResponse {
    ApiChatResponse {
        choices: vec![Choice { message }],
        usage: None,
        openhuman: None,
    }
}

#[test]
fn creates_with_key() {
    let p = make_provider(
        "venice",
        "https://api.venice.ai",
        Some("venice-test-credential"),
    );
    assert_eq!(p.name, "venice");
    assert_eq!(p.base_url, "https://api.venice.ai");
    assert_eq!(p.credential.as_deref(), Some("venice-test-credential"));
}

#[test]
fn creates_without_key() {
    let p = make_provider("test", "https://example.com", None);
    assert!(p.credential.is_none());
}

#[test]
fn strips_trailing_slash() {
    let p = make_provider("test", "https://example.com/", None);
    assert_eq!(p.base_url, "https://example.com");
}

#[tokio::test]
async fn chat_fails_without_key() {
    let p = make_provider("Venice", "https://api.venice.ai", None);
    let result = p
        .chat_with_system(None, "hello", "llama-3.3-70b", 0.7)
        .await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Venice API key not set"));
}

#[test]
fn native_request_emits_thread_id_when_present() {
    let req = super::NativeChatRequest {
        model: "sonnet".to_string(),
        messages: Vec::new(),
        temperature: Some(0.7),
        stream: Some(false),
        tools: None,
        tool_choice: None,
        thread_id: Some("thread-abc".to_string()),
        stream_options: None,
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(
        json.get("thread_id").and_then(|v| v.as_str()),
        Some("thread-abc"),
        "thread_id must be forwarded so the backend can group InferenceLog + KV cache by chat thread"
    );

    let req_no_thread = super::NativeChatRequest {
        model: "sonnet".to_string(),
        messages: Vec::new(),
        temperature: Some(0.7),
        stream: Some(false),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: None,
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let json_no_thread = serde_json::to_value(&req_no_thread).unwrap();
    assert!(
        json_no_thread.get("thread_id").is_none(),
        "absent thread_id must not be serialized so non-OpenHuman backends don't reject the field"
    );
}

#[test]
fn native_request_serializes_frequency_penalty_only_when_set() {
    let base = super::NativeChatRequest {
        model: "kimi".to_string(),
        messages: Vec::new(),
        temperature: Some(0.7),
        stream: Some(false),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: None,
        options: None,
        frequency_penalty: Some(0.3),
        max_tokens: None,
    };
    let json = serde_json::to_value(&base).unwrap();
    assert_eq!(
        json.get("frequency_penalty")
            .and_then(serde_json::Value::as_f64),
        Some(0.3),
        "a set frequency_penalty must be forwarded to damp repetition loops"
    );

    let none = super::NativeChatRequest {
        frequency_penalty: None,
        ..base
    };
    let json_none = serde_json::to_value(&none).unwrap();
    assert!(
        json_none.get("frequency_penalty").is_none(),
        "absent frequency_penalty must be omitted so providers that reject it are unaffected"
    );
}

#[test]
fn native_request_serializes_max_tokens_only_when_set() {
    // A set cap must reach the wire as OpenAI `max_tokens` so a credit-metered
    // provider prices the request against a realistic output budget rather than
    // the model's full window (TAURI-RUST-C62).
    let with_cap = super::NativeChatRequest {
        model: "anthropic/claude-fable-5".to_string(),
        messages: Vec::new(),
        temperature: Some(0.0),
        stream: Some(false),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: None,
        options: None,
        frequency_penalty: None,
        max_tokens: Some(8192),
    };
    let json = serde_json::to_value(&with_cap).unwrap();
    assert_eq!(
        json.get("max_tokens").and_then(serde_json::Value::as_u64),
        Some(8192),
        "a set max_tokens must be forwarded so the provider's balance pre-flight is bounded"
    );

    let no_cap = super::NativeChatRequest {
        max_tokens: None,
        ..with_cap
    };
    let json_none = serde_json::to_value(&no_cap).unwrap();
    assert!(
        json_none.get("max_tokens").is_none(),
        "absent max_tokens must be omitted so open-ended generations are unaffected"
    );
}

#[test]
fn agent_turn_cap_reaches_the_wire() {
    // The agent turns now set `max_tokens: Some(AGENT_TURN_MAX_OUTPUT_TOKENS)`
    // (#4005); assert that exact cap serializes onto the wire so a careless edit
    // to the const — or a regression to `None` on the agent path — fails CI and
    // can't silently restore the full-window reservation that 402s low-balance
    // BYO users (TAURI-RUST-C62).
    let cap = crate::openhuman::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS;
    assert!(
        (8192..=32768).contains(&cap),
        "agent cap must stay well above realistic turns yet below the model's full output window; got {cap}"
    );
    let req = super::NativeChatRequest {
        model: "anthropic/claude-fable-5".to_string(),
        messages: Vec::new(),
        temperature: Some(0.0),
        stream: Some(false),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: None,
        options: None,
        frequency_penalty: None,
        max_tokens: Some(cap),
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(
        json.get("max_tokens").and_then(serde_json::Value::as_u64),
        Some(u64::from(cap)),
        "the agent-turn cap must be forwarded as OpenAI max_tokens"
    );
}

#[test]
fn responses_request_serializes_max_output_tokens_only_when_set() {
    // The Responses-API branch must carry the cap as `max_output_tokens` so a
    // capped request isn't silently uncapped when responses_api_primary is on
    // (TAURI-RUST-C62).
    let with_cap = super::compatible_types::ResponsesRequest {
        model: "gpt-x".to_string(),
        input: vec![],
        instructions: None,
        stream: Some(false),
        store: Some(false),
        max_output_tokens: Some(8192),
    };
    let json = serde_json::to_value(&with_cap).unwrap();
    assert_eq!(
        json.get("max_output_tokens")
            .and_then(serde_json::Value::as_u64),
        Some(8192),
        "a set cap must reach the Responses API as max_output_tokens"
    );

    let no_cap = super::compatible_types::ResponsesRequest {
        max_output_tokens: None,
        ..with_cap
    };
    let json_none = serde_json::to_value(&no_cap).unwrap();
    assert!(
        json_none.get("max_output_tokens").is_none(),
        "absent cap must be omitted"
    );
}

#[test]
fn detects_frequency_penalty_rejection_for_retry() {
    use super::OpenAiCompatibleProvider as P;
    // Strict providers that 400 on the field → retry without it.
    assert!(P::err_indicates_frequency_penalty_unsupported(
        "400 Bad Request: unknown parameter 'frequency_penalty'"
    ));
    assert!(P::err_indicates_frequency_penalty_unsupported(
        "frequency_penalty is not supported by this model"
    ));
    // Unrelated errors, or the field merely mentioned, must NOT trigger a retry.
    assert!(!P::err_indicates_frequency_penalty_unsupported(
        "rate limit exceeded"
    ));
    assert!(!P::err_indicates_frequency_penalty_unsupported(
        "applied frequency_penalty 0.3"
    ));
}

#[test]
fn endpoint_rejects_frequency_penalty_matches_google_gemini_host() {
    use super::compatible_request::endpoint_rejects_frequency_penalty as rejects;
    // The Google Gemini OpenAI-compat shim 400s on the field (TAURI-RUST-4PJ).
    assert!(rejects(
        "https://generativelanguage.googleapis.com/v1beta/openai"
    ));
    // Host match is case-insensitive and covers a registrable-domain suffix,
    // so a BYOK provider pointed at a regional/sub-host is covered too.
    assert!(rejects(
        "https://GenerativeLanguage.GoogleAPIs.com/v1beta/openai"
    ));
    assert!(rejects(
        "https://eu.generativelanguage.googleapis.com/v1beta/openai"
    ));
    // Every other provider keeps the penalty; an unparseable URL is a no-op.
    assert!(!rejects("https://api.openai.com/v1"));
    assert!(!rejects("https://api.venice.ai"));
    // A look-alike host must NOT match (suffix check is dot-anchored).
    assert!(!rejects(
        "https://notgenerativelanguage.googleapis.com.evil.test/v1"
    ));
    assert!(!rejects("not a url"));
}

#[test]
fn effective_frequency_penalty_omitted_for_google_kept_for_others() {
    // Google Gemini endpoint → field omitted at the source (no rejected
    // round-trip, no Sentry report).
    let google = OpenAiCompatibleProvider::new(
        "google",
        "https://generativelanguage.googleapis.com/v1beta/openai",
        None,
        AuthStyle::Bearer,
    );
    assert_eq!(
        google.effective_frequency_penalty(),
        None,
        "Gemini shim rejects frequency_penalty — it must be omitted up front"
    );

    // Any other OpenAI-compatible provider keeps the repetition-damping value.
    let other = make_provider("openai", "https://api.openai.com/v1", None);
    assert_eq!(
        other.effective_frequency_penalty(),
        Some(super::compatible_repeat::CHAT_FREQUENCY_PENALTY),
        "providers that accept the field must still receive it"
    );
}

#[tokio::test]
async fn streaming_chat_frequency_penalty_rejection_not_reported_to_sentry() {
    // Defense-in-depth (TAURI-RUST-4PJ): an unknown strict provider — one not
    // covered by the host allow-list, so prevention did not omit the field —
    // 400s on frequency_penalty. The caller retries without it and succeeds, so
    // this self-healed condition must NOT page Sentry, while still propagating
    // as an Err so the retry path fires.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"code":400,"message":"Invalid JSON payload received. Unknown name \"frequency_penalty\": Cannot find field.","status":"INVALID_ARGUMENT"}}"#,
        ))
        .mount(&mock_server)
        .await;

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    // Provider URL is the mock host (not the google allow-list host), so the
    // request DOES carry frequency_penalty and exercises the stream-error
    // classifier arm rather than the prevent-at-source omission.
    let provider =
        OpenAiCompatibleProvider::new("strict_byok", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "some-model".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("hello".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: Some(super::compatible_repeat::CHAT_FREQUENCY_PENALTY),
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(8);

    let err = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .expect_err(
            "400 frequency_penalty rejection must still propagate as Err to drive the retry",
        );
    assert!(
        err.to_string().contains("streaming API error"),
        "err: {err}"
    );
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "a self-healed frequency_penalty rejection must not be reported to Sentry"
    );
}

/// Verbatim TAURI-RUST-5MV body — ollama.com hosted-inference 500 envelope.
const OLLAMA_CLOUD_500_WIRE_BODY: &str =
    r#"{"error":"Internal Server Error (ref: df512dcb-d915-493b-8f2d-e8d3dfa640c1)"}"#;

/// TAURI-RUST-5MV: ollama.com hosted-inference 500 on the non-streaming native
/// path must be demoted (no Sentry event) and surface the actionable message,
/// while still propagating as `Err` so reliable-provider retry/fallback runs.
#[tokio::test]
async fn native_chat_ollama_cloud_500_demoted_and_actionable() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string(OLLAMA_CLOUD_500_WIRE_BODY))
        .mount(&mock_server)
        .await;

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    let provider =
        OpenAiCompatibleProvider::new("ollama", &mock_server.uri(), None, AuthStyle::None);
    let messages = vec![ChatMessage {
        id: None,
        role: "user".to_string(),
        content: "hello".to_string(),
        extra_metadata: None,
    }];
    let err = provider
        .chat(
            super::super::ChatRequest {
                messages: &messages,
                tools: None,
                stream: None,
                max_tokens: None,
            },
            "minimax-m3:cloud",
            0.7,
        )
        .await
        .expect_err("ollama cloud 500 must still propagate as Err to drive retry/fallback");

    let msg = err.to_string();
    assert!(
        msg.contains("Ollama cloud is temporarily unavailable") && msg.contains("minimax-m3:cloud"),
        "expected actionable message, got: {msg}"
    );
    assert!(
        !msg.contains("ref:"),
        "opaque ref body must be replaced: {msg}"
    );
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "ollama cloud 500 must be demoted, not reported to Sentry"
    );
}

/// TAURI-RUST-5MV: same demotion on the streaming native path.
#[tokio::test]
async fn streaming_chat_ollama_cloud_500_demoted_and_actionable() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string(OLLAMA_CLOUD_500_WIRE_BODY))
        .mount(&mock_server)
        .await;

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    let provider =
        OpenAiCompatibleProvider::new("ollama", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "minimax-m3:cloud".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("hello".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(8);

    let err = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .expect_err("ollama cloud 500 must still propagate as Err to drive retry/fallback");

    let msg = err.to_string();
    assert!(
        msg.contains("Ollama cloud is temporarily unavailable") && msg.contains("minimax-m3:cloud"),
        "expected actionable message, got: {msg}"
    );
    assert!(
        !msg.contains("ref:"),
        "opaque ref body must be replaced: {msg}"
    );
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "ollama cloud 500 must be demoted, not reported to Sentry"
    );
}

/// TAURI-RUST-5MV: the shared `api_error` helper demotes the same body and
/// returns the actionable message (covers `chat_with_system` / `chat_with_history`
/// callers that funnel through it).
#[tokio::test]
async fn api_error_ollama_cloud_500_demoted_and_actionable() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(OLLAMA_CLOUD_500_WIRE_BODY))
        .mount(&mock_server)
        .await;

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    let response = reqwest::Client::new()
        .post(mock_server.uri())
        .send()
        .await
        .expect("mock request");
    let err = super::super::api_error("ollama", response).await;

    let msg = err.to_string();
    assert!(
        msg.contains("Ollama cloud is temporarily unavailable"),
        "expected actionable message, got: {msg}"
    );
    assert!(
        !msg.contains("ref:"),
        "opaque ref body must be replaced: {msg}"
    );
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "ollama cloud 500 must be demoted, not reported to Sentry"
    );
}

/// Streaming responses arrive without `usage` unless the request asks
/// for `stream_options.include_usage = true` (OpenAI spec). Without it
/// the OpenHuman backend's `openhuman.billing` block also never lands,
/// so transcript headers for orchestrator sessions lose the
/// `- Charged: $…` line. The non-streaming path stays untouched.
#[test]
fn streaming_request_sets_stream_options_include_usage() {
    let req = super::NativeChatRequest {
        model: "sonnet".to_string(),
        messages: Vec::new(),
        temperature: Some(0.0),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(
        json.pointer("/stream_options/include_usage")
            .and_then(|v| v.as_bool()),
        Some(true),
        "streaming requests must opt into the final usage chunk"
    );
}

#[test]
fn non_streaming_request_omits_stream_options() {
    let req = super::NativeChatRequest {
        model: "sonnet".to_string(),
        messages: Vec::new(),
        temperature: Some(0.0),
        stream: Some(false),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: None,
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert!(
        json.get("stream_options").is_none(),
        "non-streaming requests must not emit stream_options (OpenAI rejects it on stream=false)"
    );
}

#[test]
fn ollama_options_num_ctx_serializes_correctly() {
    let req = super::NativeChatRequest {
        model: "qwen3:14b".to_string(),
        messages: Vec::new(),
        temperature: Some(0.7),
        stream: Some(false),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: None,
        options: Some(super::compatible_types::OllamaOptions {
            num_ctx: Some(32768),
        }),
        frequency_penalty: None,
        max_tokens: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(
        json.pointer("/options/num_ctx").and_then(|v| v.as_u64()),
        Some(32768),
        "Ollama num_ctx must appear at options.num_ctx in serialized body"
    );
}

#[test]
fn ollama_options_none_is_omitted() {
    let req = super::NativeChatRequest {
        model: "gpt-4o".to_string(),
        messages: Vec::new(),
        temperature: Some(0.7),
        stream: Some(false),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: None,
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert!(
        json.get("options").is_none(),
        "options field must be omitted when None (non-Ollama providers)"
    );
}

#[tokio::test]
async fn outbound_thread_id_is_gated_per_provider() {
    use crate::openhuman::inference::provider::thread_context::with_thread_id;

    let third_party = make_provider("Venice", "https://api.venice.ai", None);
    let openhuman =
        make_provider("OpenHuman", "https://api.openhuman.test", None).with_openhuman_thread_id();

    with_thread_id("thread-xyz", async {
        assert!(
            third_party.outbound_thread_id().is_none(),
            "third-party OpenAI-compatible providers must NOT see the OpenHuman thread_id extension \
             — unknown fields can trip strict input validation on Venice/Moonshot/Groq/etc."
        );
        assert_eq!(
            openhuman.outbound_thread_id().as_deref(),
            Some("thread-xyz"),
            "the OpenHuman backend provider opts in via with_openhuman_thread_id() and must \
             forward the ambient id so InferenceLog grouping + KV cache locality work"
        );
    })
    .await;
}

#[test]
fn request_serializes_correctly() {
    let req = ApiChatRequest {
        model: "llama-3.3-70b".to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: "You are OpenHuman".into(),
            },
            Message {
                role: "user".to_string(),
                content: "hello".into(),
            },
        ],
        temperature: Some(0.4),
        stream: Some(false),
        tools: None,
        tool_choice: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("llama-3.3-70b"));
    assert!(json.contains("system"));
    assert!(json.contains("user"));
    // tools/tool_choice should be omitted when None
    assert!(!json.contains("tools"));
    assert!(!json.contains("tool_choice"));
}

#[test]
fn response_deserializes() {
    let json = r#"{"choices":[{"message":{"content":"Hello from Venice!"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        resp.choices[0].message.content,
        Some("Hello from Venice!".to_string())
    );
}

#[test]
fn response_empty_choices() {
    let json = r#"{"choices":[]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    assert!(resp.choices.is_empty());
}

#[test]
fn parse_chat_response_body_reports_sanitized_snippet() {
    let body = r#"{"choices":"invalid","api_key":"sk-test-secret-value"}"#;
    let err = parse_chat_response_body("custom", body).expect_err("payload should fail");
    let msg = err.to_string();

    assert!(msg.contains("custom API returned an unexpected chat-completions payload"));
    assert!(msg.contains("body="));
    assert!(msg.contains("[REDACTED]"));
    assert!(!msg.contains("sk-test-secret-value"));
}

#[test]
fn parse_responses_response_body_reports_sanitized_snippet() {
    let body = r#"{"output_text":123,"api_key":"sk-another-secret"}"#;
    let err = parse_responses_response_body("custom", body).expect_err("payload should fail");
    let msg = err.to_string();

    assert!(msg.contains("custom Responses API returned an unexpected payload"));
    assert!(msg.contains("body="));
    assert!(msg.contains("[REDACTED]"));
    assert!(!msg.contains("sk-another-secret"));
}

// ── aggregate_responses_sse_body (#3201) ─────────────────────────────────────

/// Per-delta accumulation: the Codex/ChatGPT OAuth stream is a sequence of
/// `response.output_text.delta` events whose `delta` fields concatenate into
/// the final assistant text.
#[test]
fn aggregate_responses_sse_body_concatenates_text_deltas() {
    let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello \"}\n\n\
                data: {\"type\":\"response.output_text.delta\",\"delta\":\"world\"}\n\n\
                data: [DONE]\n";
    let text =
        super::compatible_parse::aggregate_responses_sse_body("custom", body).expect("aggregate");
    assert_eq!(text, "hello world");
}

/// Some providers (and the Codex endpoint when the model batches its
/// reply) skip per-token deltas and emit the full text in
/// `response.completed.response.output_text`. The aggregator must fall
/// back to that terminal field when no deltas accumulated.
#[test]
fn aggregate_responses_sse_body_prefers_terminal_output_text_when_present() {
    let body = "data: {\"type\":\"response.created\",\"response\":{}}\n\n\
                data: {\"type\":\"response.completed\",\"response\":{\"output_text\":\"batched final text\"}}\n\n\
                data: [DONE]\n";
    let text =
        super::compatible_parse::aggregate_responses_sse_body("custom", body).expect("aggregate");
    assert_eq!(text, "batched final text");
}

/// #3201 CodeRabbit nit: a whitespace-only terminal `output_text` must
/// behave like the field is absent, so accumulated deltas survive instead
/// of being silently collapsed into blank output. Mirrors
/// `extract_responses_text`'s `first_nonempty(...)` policy.
#[test]
fn aggregate_responses_sse_body_ignores_whitespace_only_terminal_output_text() {
    let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"good \"}\n\n\
                data: {\"type\":\"response.output_text.delta\",\"delta\":\"reply\"}\n\n\
                data: {\"type\":\"response.completed\",\"response\":{\"output_text\":\"   \\n\\t\"}}\n\n\
                data: [DONE]\n";
    let text =
        super::compatible_parse::aggregate_responses_sse_body("custom", body).expect("aggregate");
    assert_eq!(text, "good reply");
}

/// Carriage-return line endings (CRLF, common in HTTP/1.1 SSE) parse the
/// same as LF-only — the trimming is just `\r` stripping.
#[test]
fn aggregate_responses_sse_body_tolerates_crlf_line_endings() {
    let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"crlf\"}\r\n\r\n\
                data: [DONE]\r\n";
    let text =
        super::compatible_parse::aggregate_responses_sse_body("custom", body).expect("aggregate");
    assert_eq!(text, "crlf");
}

/// `response.failed` / `response.error` / `error` event shapes are
/// terminal failures — bubble them up so the caller surfaces the upstream
/// reason instead of returning empty text.
#[test]
fn aggregate_responses_sse_body_surfaces_failure_events() {
    let body = "data: {\"type\":\"response.created\",\"response\":{}}\n\n\
                data: {\"type\":\"response.failed\",\"error\":\"upstream model unavailable\"}\n\n";
    let err = super::compatible_parse::aggregate_responses_sse_body("custom", body)
        .expect_err("failure event should propagate");
    assert!(
        err.to_string()
            .contains("custom Responses API stream reported a failure event"),
        "unexpected error: {err}"
    );
}

/// A stream that produced no usable text events returns a sanitised
/// "no text events" error so the caller sees something actionable
/// instead of an empty string.
#[test]
fn aggregate_responses_sse_body_errors_when_no_text_events_present() {
    let body = "data: {\"type\":\"response.created\",\"response\":{}}\n\n\
                data: [DONE]\n";
    let err = super::compatible_parse::aggregate_responses_sse_body("custom", body)
        .expect_err("empty stream should fail");
    assert!(
        err.to_string()
            .contains("custom Responses API SSE stream produced no text events"),
        "unexpected error: {err}"
    );
}

/// Malformed individual events (provider keepalive comments, etc.) must
/// not abort the whole turn — they're skipped and the good deltas still
/// aggregate.
#[test]
fn aggregate_responses_sse_body_skips_unparseable_events() {
    let body = "data: {malformed-keepalive\n\n\
                data: {\"type\":\"response.output_text.delta\",\"delta\":\"good\"}\n\n\
                data: [DONE]\n";
    let text =
        super::compatible_parse::aggregate_responses_sse_body("custom", body).expect("aggregate");
    assert_eq!(text, "good");
}

#[test]
fn x_api_key_auth_style() {
    let p = OpenAiCompatibleProvider::new(
        "moonshot",
        "https://api.moonshot.cn",
        Some("ms-key"),
        AuthStyle::XApiKey,
    );
    assert!(matches!(p.auth_header, AuthStyle::XApiKey));
}

#[test]
fn custom_auth_style() {
    let p = OpenAiCompatibleProvider::new(
        "custom",
        "https://api.example.com",
        Some("key"),
        AuthStyle::Custom("X-Custom-Key".into()),
    );
    assert!(matches!(p.auth_header, AuthStyle::Custom(_)));
}

#[test]
fn no_auth_style_allows_missing_key() {
    let p =
        OpenAiCompatibleProvider::new("ollama", "http://localhost:11434/v1", None, AuthStyle::None);
    assert!(matches!(p.auth_header, AuthStyle::None));
    assert!(p.credential_for_request().unwrap().is_none());

    let req = p
        .apply_auth_header(
            p.http_client()
                .post("http://localhost:11434/v1/chat/completions"),
            None,
        )
        .build()
        .unwrap();
    assert!(req.headers().get("authorization").is_none());
    assert!(req.headers().get("x-api-key").is_none());
}

#[test]
fn extra_headers_are_applied_with_auth_header() {
    let p = OpenAiCompatibleProvider::new(
        "openai",
        "https://chatgpt.com/backend-api/codex",
        Some("oauth-access-token"),
        AuthStyle::Bearer,
    )
    .with_extra_header("ChatGPT-Account-ID", "acct_123")
    .with_extra_header("originator", "codex_cli_rs")
    .with_user_agent("codex_cli_rs/0.0.0 (OpenHuman test)");

    let req = p
        .apply_auth_header(
            p.http_client()
                .post("https://chatgpt.com/backend-api/codex/responses"),
            Some("oauth-access-token"),
        )
        .build()
        .unwrap();

    assert_eq!(
        req.headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer oauth-access-token")
    );
    assert_eq!(
        req.headers()
            .get("ChatGPT-Account-ID")
            .and_then(|value| value.to_str().ok()),
        Some("acct_123")
    );
    assert_eq!(
        req.headers()
            .get("originator")
            .and_then(|value| value.to_str().ok()),
        Some("codex_cli_rs")
    );
    assert_eq!(
        req.headers()
            .get(reqwest::header::USER_AGENT)
            .and_then(|value| value.to_str().ok()),
        Some("codex_cli_rs/0.0.0 (OpenHuman test)")
    );
}

#[test]
fn openrouter_requests_include_app_attribution_headers() {
    let p = OpenAiCompatibleProvider::new(
        "openrouter",
        "https://openrouter.ai/api/v1",
        Some("sk-or-test"),
        AuthStyle::Bearer,
    );

    let req = p
        .apply_auth_header(
            p.http_client()
                .post("https://openrouter.ai/api/v1/chat/completions"),
            Some("sk-or-test"),
        )
        .build()
        .unwrap();

    assert_eq!(
        req.headers()
            .get("HTTP-Referer")
            .and_then(|value| value.to_str().ok()),
        Some("https://openhuman.ai")
    );
    assert_eq!(
        req.headers()
            .get("X-OpenRouter-Title")
            .and_then(|value| value.to_str().ok()),
        Some("OpenHuman")
    );
}

#[test]
fn non_openrouter_requests_do_not_include_openrouter_attribution_headers() {
    let p = OpenAiCompatibleProvider::new(
        "custom",
        "https://api.example.com/v1",
        Some("test-key"),
        AuthStyle::Bearer,
    );

    let req = p
        .apply_auth_header(
            p.http_client()
                .post("https://api.example.com/v1/chat/completions"),
            Some("test-key"),
        )
        .build()
        .unwrap();

    assert!(req.headers().get("HTTP-Referer").is_none());
    assert!(req.headers().get("X-OpenRouter-Title").is_none());
}

#[test]
fn extra_query_params_are_applied_to_codex_urls() {
    let p = OpenAiCompatibleProvider::new(
        "openai",
        "https://chatgpt.com/backend-api/codex",
        Some("oauth-access-token"),
        AuthStyle::Bearer,
    )
    .with_extra_query_param("client_version", "0.54.17");

    let chat_url = reqwest::Url::parse(&p.chat_completions_url()).unwrap();
    assert_eq!(chat_url.path(), "/backend-api/codex/chat/completions");
    assert_eq!(
        chat_url
            .query_pairs()
            .find(|(key, _)| key == "client_version")
            .map(|(_, value)| value.into_owned()),
        Some("0.54.17".to_string())
    );

    let responses_url = reqwest::Url::parse(&p.responses_url()).unwrap();
    assert_eq!(responses_url.path(), "/backend-api/codex/responses");
    assert_eq!(
        responses_url
            .query_pairs()
            .find(|(key, _)| key == "client_version")
            .map(|(_, value)| value.into_owned()),
        Some("0.54.17".to_string())
    );
}

/// #3201: the Codex/ChatGPT OAuth Responses endpoint rejects
/// `stream: false` with `{"detail":"Stream must be set to true"}` and
/// only emits SSE bodies. The non-streaming `chat_via_responses` wrapper
/// must therefore (a) flip the `stream` flag for `/backend-api/codex`
/// URLs and (b) aggregate the SSE body back into the same `String`
/// the caller expects. PR #3192 fixed the sibling `store: false`
/// requirement; this test pins both wire-shape requirements together.
#[tokio::test]
async fn responses_api_primary_posts_directly_to_responses() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/backend-api/codex/responses"))
        .and(body_json(serde_json::json!({
            "model": "gpt-5.5",
            "input": [{
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }],
            "stream": true,
            "store": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello \"}\n\n\
             data: {\"type\":\"response.output_text.delta\",\"delta\":\"from \"}\n\n\
             data: {\"type\":\"response.output_text.delta\",\"delta\":\"responses\"}\n\n\
             data: {\"type\":\"response.completed\",\"response\":{\"output_text\":\"hello from responses\"}}\n\n\
             data: [DONE]\n\n",
        ))
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(
        "openai",
        &format!("{}/backend-api/codex", server.uri()),
        Some("oauth-access-token"),
        AuthStyle::Bearer,
    )
    .with_responses_api_primary();

    let text = provider
        .chat_with_history(&[ChatMessage::user("hello")], "gpt-5.5", 0.0)
        .await
        .unwrap();

    assert_eq!(text, "hello from responses");
}

/// TAURI-RUST-EWD: the Codex/ChatGPT OAuth Responses endpoint rejects
/// `max_output_tokens` (400 `Unsupported parameter: max_output_tokens`). The
/// memory-extraction cap (`ChatRequest::max_tokens`) must therefore be dropped
/// on the wire for `/backend-api/codex/responses`. The exact `body_json`
/// matcher omits the field, so a request that still carried it would fail to
/// match the mock and the call would error — i.e. a match positively proves the
/// omission.
#[tokio::test]
async fn codex_responses_omits_max_output_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/backend-api/codex/responses"))
        .and(body_json(serde_json::json!({
            "model": "gpt-5.5",
            "input": [{
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }],
            "stream": true,
            "store": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n\
             data: {\"type\":\"response.completed\",\"response\":{\"output_text\":\"ok\"}}\n\n\
             data: [DONE]\n\n",
        ))
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(
        "openai",
        &format!("{}/backend-api/codex", server.uri()),
        Some("oauth-access-token"),
        AuthStyle::Bearer,
    )
    .with_responses_api_primary();

    let messages = [ChatMessage::user("hello")];
    let request = crate::openhuman::inference::provider::traits::ChatRequest {
        messages: &messages,
        tools: None,
        stream: None,
        // A bounded cap (memory extraction) that the Codex endpoint rejects.
        max_tokens: Some(256),
    };
    let resp = provider.chat(request, "gpt-5.5", 0.0).await.unwrap();
    assert_eq!(resp.text.as_deref(), Some("ok"));
}

/// A non-Codex Responses backend (`/v1/responses`) must still receive the cap —
/// the omission is endpoint-specific, not a blanket drop (guards against
/// silently uncapping memory extraction on real `/v1/responses`, TAURI-RUST-C62).
#[tokio::test]
async fn standard_responses_keeps_max_output_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_json(serde_json::json!({
            "model": "gpt-5.4",
            "input": [{
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}]
            }],
            "stream": false,
            "store": false,
            "max_output_tokens": 256
        })))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"output_text":"ok","output":[]}"#),
        )
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(
        "openai",
        &format!("{}/v1", server.uri()),
        Some("sk-test"),
        AuthStyle::Bearer,
    )
    .with_responses_api_primary();

    let messages = [ChatMessage::user("hello")];
    let request = crate::openhuman::inference::provider::traits::ChatRequest {
        messages: &messages,
        tools: None,
        stream: None,
        max_tokens: Some(256),
    };
    let resp = provider.chat(request, "gpt-5.4", 0.0).await.unwrap();
    assert_eq!(resp.text.as_deref(), Some("ok"));
}

/// The reactive strip-and-retry defends Responses backends we don't match by
/// URL: a 400 whose body flags `max_output_tokens` as unsupported triggers one
/// retry with the field removed, which then succeeds. Exact-body matchers are
/// mutually exclusive on `max_output_tokens`, so the result is deterministic
/// regardless of mock precedence.
#[tokio::test]
async fn responses_strips_max_output_tokens_and_retries_on_400() {
    let server = MockServer::start().await;
    let input = serde_json::json!([{
        "role": "user",
        "content": [{"type": "input_text", "text": "hello"}]
    }]);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_json(serde_json::json!({
            "model": "gpt-5.5",
            "input": input.clone(),
            "stream": false,
            "store": false,
            "max_output_tokens": 256
        })))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"detail":"Unsupported parameter: max_output_tokens"}"#),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_json(serde_json::json!({
            "model": "gpt-5.5",
            "input": input.clone(),
            "stream": false,
            "store": false
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"output_text":"recovered","output":[]}"#),
        )
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(
        "strict-responses",
        &format!("{}/v1", server.uri()),
        Some("sk-test"),
        AuthStyle::Bearer,
    )
    .with_responses_api_primary();

    let messages = [ChatMessage::user("hello")];
    let request = crate::openhuman::inference::provider::traits::ChatRequest {
        messages: &messages,
        tools: None,
        stream: None,
        max_tokens: Some(256),
    };
    let resp = provider.chat(request, "gpt-5.5", 0.0).await.unwrap();
    assert_eq!(resp.text.as_deref(), Some("recovered"));
}

#[test]
fn err_indicates_max_output_tokens_unsupported_matches_codex_body() {
    assert!(
        OpenAiCompatibleProvider::err_indicates_max_output_tokens_unsupported(
            r#"{"detail":"Unsupported parameter: max_output_tokens"}"#
        )
    );
    assert!(
        !OpenAiCompatibleProvider::err_indicates_max_output_tokens_unsupported(
            r#"{"detail":"rate limit exceeded"}"#
        )
    );
    // A different unsupported parameter must not trip the max_output_tokens strip.
    assert!(
        !OpenAiCompatibleProvider::err_indicates_max_output_tokens_unsupported(
            r#"{"detail":"Unsupported parameter: temperature"}"#
        )
    );
    // An invalid-*value* error (cap out of the model's allowed range) must NOT
    // strip the cap — that would silently uncap the request. Surface it instead.
    assert!(
        !OpenAiCompatibleProvider::err_indicates_max_output_tokens_unsupported(
            r#"{"detail":"Invalid value for max_output_tokens: must be <= 4096"}"#
        )
    );
    assert!(
        !OpenAiCompatibleProvider::err_indicates_max_output_tokens_unsupported(
            r#"{"detail":"max_output_tokens exceeds the maximum allowed for this model"}"#
        )
    );
}

#[test]
fn blank_required_key_counts_as_missing() {
    let p = OpenAiCompatibleProvider::new(
        "custom",
        "https://api.example.com",
        Some("  "),
        AuthStyle::Bearer,
    );
    let err = p.credential_for_request().unwrap_err().to_string();
    assert!(err.contains("custom API key not set"), "err: {err}");
}

#[tokio::test]
async fn all_compatible_providers_fail_without_key() {
    let providers = vec![
        make_provider("Venice", "https://api.venice.ai", None),
        make_provider("Moonshot", "https://api.moonshot.cn", None),
        make_provider("GLM", "https://open.bigmodel.cn", None),
        make_provider("MiniMax", "https://api.minimaxi.com/v1", None),
        make_provider("Groq", "https://api.groq.com/openai", None),
        make_provider("Mistral", "https://api.mistral.ai", None),
        make_provider("xAI", "https://api.x.ai", None),
        make_provider("Astrai", "https://as-trai.com/v1", None),
    ];

    for p in providers {
        let result = p.chat_with_system(None, "test", "model", 0.7).await;
        assert!(result.is_err(), "{} should fail without key", p.name);
        assert!(
            result.unwrap_err().to_string().contains("API key not set"),
            "{} error should mention key",
            p.name
        );
    }
}

#[test]
fn responses_extracts_top_level_output_text() {
    let json = r#"{"output_text":"Hello from top-level","output":[]}"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        extract_responses_text(response).as_deref(),
        Some("Hello from top-level")
    );
}

#[test]
fn responses_extracts_nested_output_text() {
    let json = r#"{"output":[{"content":[{"type":"output_text","text":"Hello from nested"}]}]}"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        extract_responses_text(response).as_deref(),
        Some("Hello from nested")
    );
}

#[test]
fn responses_extracts_any_text_as_fallback() {
    let json = r#"{"output":[{"content":[{"type":"message","text":"Fallback text"}]}]}"#;
    let response: ResponsesResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        extract_responses_text(response).as_deref(),
        Some("Fallback text")
    );
}

#[test]
fn build_responses_prompt_preserves_multi_turn_history() {
    let messages = vec![
        ChatMessage::system("policy"),
        ChatMessage::user("step 1"),
        ChatMessage::assistant("ack 1"),
        ChatMessage::tool("{\"result\":\"ok\"}"),
        ChatMessage::user("step 2"),
    ];

    let (instructions, input) = build_responses_prompt(&messages);

    assert_eq!(instructions.as_deref(), Some("policy"));
    assert_eq!(input.len(), 4);
    assert_eq!(input[0].role, "user");
    assert_eq!(input[0].content[0].kind, "input_text");
    assert_eq!(input[0].content[0].text, "step 1");
    assert_eq!(input[1].role, "assistant");
    assert_eq!(input[1].content[0].kind, "output_text");
    assert_eq!(input[1].content[0].text, "ack 1");
    // A `tool` turn normalizes to the `assistant` role, so its content part
    // MUST be `output_text` — the Responses API rejects `input_text` on an
    // assistant item (Sentry TAURI-RUST-8FQ / GH #3624).
    assert_eq!(input[2].role, "assistant");
    assert_eq!(input[2].content[0].kind, "output_text");
    assert_eq!(input[2].content[0].text, "{\"result\":\"ok\"}");
    assert_eq!(input[3].role, "user");
    assert_eq!(input[3].content[0].kind, "input_text");
    assert_eq!(input[3].content[0].text, "step 2");
}

/// Regression for Sentry TAURI-RUST-8FQ / GH #3624: the Responses API only
/// accepts `output_text`/`refusal` for assistant items. `normalize_responses_role`
/// folds `tool` into `assistant`, so the content kind must follow the normalized
/// role — never the raw one. No assistant-role item may carry `input_text`.
#[test]
fn build_responses_prompt_tool_role_uses_output_text() {
    let messages = vec![
        ChatMessage::assistant("calling a tool"),
        ChatMessage::tool("{\"result\":\"ok\"}"),
        ChatMessage::user("thanks"),
    ];

    let (_instructions, input) = build_responses_prompt(&messages);

    // The tool turn folds to assistant and must carry output_text.
    assert_eq!(input[1].role, "assistant");
    assert_eq!(input[1].content[0].kind, "output_text");

    // Invariant: an assistant-role item never carries input_text.
    for item in &input {
        if item.role == "assistant" {
            assert_eq!(
                item.content[0].kind, "output_text",
                "assistant-role item must use output_text, got {}",
                item.content[0].kind
            );
        }
    }
}

#[tokio::test]
async fn chat_via_responses_requires_non_system_message() {
    let provider = make_provider("custom", "https://api.example.com", Some("test-key"));
    let err = provider
        .chat_via_responses(
            Some("test-key"),
            &[ChatMessage::system("policy")],
            "gpt-test",
            None,
        )
        .await
        .expect_err("system-only fallback payload should fail");

    assert!(err
        .to_string()
        .contains("requires at least one non-system message"));
}

#[tokio::test]
async fn streaming_chat_config_rejection_propagates_error_without_sentry_report() {
    // Representative guardrail for the new provider-config-rejection
    // suppression branches in compatible.rs: streaming_chat should still
    // return an error, but it must not call report_error/Sentry for this
    // deterministic user-config state.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string("invalid temperature: only 1 is allowed for this model"),
        )
        .mount(&mock_server)
        .await;

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    let provider =
        OpenAiCompatibleProvider::new("custom_openai", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "kimi-k2".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("hello".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(8);

    let err = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .expect_err("400 provider config-rejection must still propagate as Err");
    assert!(
        err.to_string().contains("streaming API error"),
        "err: {err}"
    );
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "provider config-rejection must not be reported to Sentry"
    );
}

#[tokio::test]
async fn streaming_chat_byo_auth_failure_propagates_error_without_sentry_report() {
    // Guardrail for #3671 (TAURI-RUST-DHM): a missing/invalid BYO API key on a
    // non-backend custom provider returns 401 with an auth-error body. The
    // error must still propagate to the caller, but it must NOT page Sentry —
    // memory-tree extraction + memory jobs retry through the broken credential
    // and previously flooded Sentry with thousands of identical events.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string(
            r#"{"error":{"message":"Invalid or missing API key","type":"authentication_error"}}"#,
        ))
        .mount(&mock_server)
        .await;

    let transport = TestTransport::new();
    let sentry_options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let sentry_hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(sentry_options.into())),
        Arc::new(Default::default()),
    ));
    let _sentry_guard = sentry::HubSwitchGuard::new(sentry_hub);

    // `kiro` is the exact (user-named) custom provider from the Sentry report.
    let provider = OpenAiCompatibleProvider::new("kiro", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "claude-sonnet-4.5".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("hello".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(8);

    let err = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .expect_err("401 BYO auth failure must still propagate as Err");
    assert!(
        err.to_string().contains("streaming API error"),
        "err: {err}"
    );
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "BYO provider auth failure must not be reported to Sentry"
    );
}

// ----------------------------------------------------------
// Custom endpoint path tests (Issue #114)
// ----------------------------------------------------------

#[test]
fn chat_completions_url_standard_openai() {
    // Standard OpenAI-compatible providers get /chat/completions appended
    let p = make_provider("openai", "https://api.openai.com/v1", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.openai.com/v1/chat/completions"
    );
}

#[test]
fn chat_completions_url_trailing_slash() {
    // Trailing slash is stripped, then /chat/completions appended
    let p = make_provider("test", "https://api.example.com/v1/", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.example.com/v1/chat/completions"
    );
}

#[test]
fn chat_completions_url_volcengine_ark() {
    // VolcEngine ARK uses custom path - should use as-is
    let p = make_provider(
        "volcengine",
        "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions",
        None,
    );
    assert_eq!(
        p.chat_completions_url(),
        "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions"
    );
}

#[test]
fn chat_completions_url_custom_full_endpoint() {
    // Custom provider with full endpoint path
    let p = make_provider(
        "custom",
        "https://my-api.example.com/v2/llm/chat/completions",
        None,
    );
    assert_eq!(
        p.chat_completions_url(),
        "https://my-api.example.com/v2/llm/chat/completions"
    );
}

#[test]
fn chat_completions_url_requires_exact_suffix_match() {
    let p = make_provider(
        "custom",
        "https://my-api.example.com/v2/llm/chat/completions-proxy",
        None,
    );
    assert_eq!(
        p.chat_completions_url(),
        "https://my-api.example.com/v2/llm/chat/completions-proxy/chat/completions"
    );
}

#[test]
fn responses_url_standard() {
    // Standard providers get /v1/responses appended
    let p = make_provider("test", "https://api.example.com", None);
    assert_eq!(p.responses_url(), "https://api.example.com/v1/responses");
}

#[test]
fn responses_url_custom_full_endpoint() {
    // Custom provider with full responses endpoint
    let p = make_provider(
        "custom",
        "https://my-api.example.com/api/v2/responses",
        None,
    );
    assert_eq!(
        p.responses_url(),
        "https://my-api.example.com/api/v2/responses"
    );
}

#[test]
fn responses_url_requires_exact_suffix_match() {
    let p = make_provider(
        "custom",
        "https://my-api.example.com/api/v2/responses-proxy",
        None,
    );
    assert_eq!(
        p.responses_url(),
        "https://my-api.example.com/api/v2/responses-proxy/responses"
    );
}

#[test]
fn responses_url_derives_from_chat_endpoint() {
    let p = make_provider(
        "custom",
        "https://my-api.example.com/api/v2/chat/completions",
        None,
    );
    assert_eq!(
        p.responses_url(),
        "https://my-api.example.com/api/v2/responses"
    );
}

#[test]
fn responses_url_base_with_v1_no_duplicate() {
    let p = make_provider("test", "https://api.example.com/v1", None);
    assert_eq!(p.responses_url(), "https://api.example.com/v1/responses");
}

#[test]
fn responses_url_non_v1_api_path_uses_raw_suffix() {
    let p = make_provider("test", "https://api.example.com/api/coding/v3", None);
    assert_eq!(
        p.responses_url(),
        "https://api.example.com/api/coding/v3/responses"
    );
}

#[test]
fn chat_completions_url_without_v1() {
    // Provider configured without /v1 in base URL
    let p = make_provider("test", "https://api.example.com", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.example.com/chat/completions"
    );
}

#[test]
fn chat_completions_url_base_with_v1() {
    // Provider configured with /v1 in base URL
    let p = make_provider("test", "https://api.example.com/v1", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.example.com/v1/chat/completions"
    );
}

// ----------------------------------------------------------
// Provider-specific endpoint tests (Issue #167)
// ----------------------------------------------------------

#[test]
fn chat_completions_url_zai() {
    // Z.AI uses /api/paas/v4 base path
    let p = make_provider("zai", "https://api.z.ai/api/paas/v4", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.z.ai/api/paas/v4/chat/completions"
    );
}

#[test]
fn chat_completions_url_minimax() {
    // MiniMax OpenAI-compatible endpoint requires /v1 base path.
    let p = make_provider("minimax", "https://api.minimaxi.com/v1", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://api.minimaxi.com/v1/chat/completions"
    );
}

#[test]
fn chat_completions_url_glm() {
    // GLM (BigModel) uses /api/paas/v4 base path
    let p = make_provider("glm", "https://open.bigmodel.cn/api/paas/v4", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://open.bigmodel.cn/api/paas/v4/chat/completions"
    );
}

#[test]
fn chat_completions_url_opencode() {
    // OpenCode Zen uses /zen/v1 base path
    let p = make_provider("opencode", "https://opencode.ai/zen/v1", None);
    assert_eq!(
        p.chat_completions_url(),
        "https://opencode.ai/zen/v1/chat/completions"
    );
}

#[test]
fn parse_native_response_preserves_tool_call_id() {
    let message = ResponseMessage {
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: Some("call_123".to_string()),
            kind: Some("function".to_string()),
            function: Some(Function {
                name: Some("shell".to_string()),
                arguments: Some(serde_json::Value::String(
                    r#"{"command":"pwd"}"#.to_string(),
                )),
            }),
            extra_content: None,
        }]),
        function_call: None,
        reasoning_content: None,
    };

    let parsed =
        OpenAiCompatibleProvider::parse_native_response(wrap_message(message), "test").unwrap();
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].id, "call_123");
    assert_eq!(parsed.tool_calls[0].name, "shell");
}

/// DeepSeek thinking mode emits the chain-of-thought in `reasoning_content`
/// alongside the tool call. `parse_native_response` must surface it so the
/// agent loop can replay it on the follow-up request (Sentry TAURI-RUST-4KB).
#[test]
fn parse_native_response_captures_reasoning_content() {
    let message = ResponseMessage {
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: Some("call_r".to_string()),
            kind: Some("function".to_string()),
            function: Some(Function {
                name: Some("shell".to_string()),
                arguments: Some(serde_json::Value::String("{}".to_string())),
            }),
            extra_content: None,
        }]),
        function_call: None,
        reasoning_content: Some("  weighing the options  ".to_string()),
    };

    let parsed =
        OpenAiCompatibleProvider::parse_native_response(wrap_message(message), "deepseek").unwrap();
    assert_eq!(
        parsed.reasoning_content.as_deref(),
        Some("weighing the options")
    );
}

/// Whitespace-only / empty reasoning is normalised to `None` so it never
/// produces a spurious `reasoning_content` key on the wire.
#[test]
fn parse_native_response_blank_reasoning_is_none() {
    let message = ResponseMessage {
        content: Some("hello".to_string()),
        tool_calls: None,
        function_call: None,
        reasoning_content: Some("   ".to_string()),
    };

    let parsed =
        OpenAiCompatibleProvider::parse_native_response(wrap_message(message), "deepseek").unwrap();
    assert!(parsed.reasoning_content.is_none());
}

#[test]
fn convert_messages_for_native_maps_tool_result_payload() {
    // A `tool` result must be opened by a preceding `assistant(tool_calls)`,
    // else the invariant sanitizer drops it as an orphan (see `tool_invariants_*`).
    // Pair it with its opener so this test exercises payload mapping only.
    let input = vec![
        ChatMessage::assistant(
            r#"{"content":"on it","tool_calls":[{"id":"call_abc","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_abc","content":"done"}"#),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);
    assert_eq!(converted.len(), 2);
    assert_eq!(converted[1].role, "tool");
    assert_eq!(converted[1].tool_call_id.as_deref(), Some("call_abc"));
    assert_eq!(
        serde_json::to_value(&converted[1].content).unwrap(),
        serde_json::json!("done")
    );
}

// ── TAURI-RUST-4PK: Gemini thought_signature round-trip ──────────────────────

/// The wire `ToolCall` must capture Gemini's `extra_content` from the response
/// and re-emit it verbatim on the request, so the thought_signature survives the
/// round-trip. A tool call without it omits the field entirely (non-Gemini
/// providers stay byte-identical on the wire).
#[test]
fn tool_call_wire_round_trips_extra_content() {
    let json = r#"{"id":"call_g","type":"function","function":{"name":"shell","arguments":"{}"},"extra_content":{"google":{"thought_signature":"SIG123"}}}"#;
    let tc: ToolCall = serde_json::from_str(json).unwrap();
    assert_eq!(
        tc.extra_content
            .as_ref()
            .and_then(|v| v.pointer("/google/thought_signature"))
            .and_then(|v| v.as_str()),
        Some("SIG123"),
        "extra_content must be captured from the Gemini response"
    );
    let reemitted = serde_json::to_value(&tc).unwrap();
    assert_eq!(
        reemitted
            .pointer("/extra_content/google/thought_signature")
            .and_then(|v| v.as_str()),
        Some("SIG123"),
        "extra_content must be echoed verbatim on the request body"
    );

    let bare: ToolCall = serde_json::from_str(
        r#"{"id":"c","type":"function","function":{"name":"x","arguments":"{}"}}"#,
    )
    .unwrap();
    assert!(bare.extra_content.is_none());
    assert!(
        serde_json::to_value(&bare)
            .unwrap()
            .get("extra_content")
            .is_none(),
        "providers that don't send extra_content keep a byte-identical wire body"
    );
}

/// `parse_native_response` lifts the tool-call `extra_content` onto the harness
/// ToolCall so it can be persisted and echoed (TAURI-RUST-4PK).
#[test]
fn parse_native_response_captures_tool_call_extra_content() {
    let message = ResponseMessage {
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: Some("call_g".to_string()),
            kind: Some("function".to_string()),
            function: Some(Function {
                name: Some("shell".to_string()),
                arguments: Some(serde_json::Value::String("{}".to_string())),
            }),
            extra_content: Some(serde_json::json!({"google":{"thought_signature":"SIG_RESP"}})),
        }]),
        function_call: None,
        reasoning_content: None,
    };
    let parsed =
        OpenAiCompatibleProvider::parse_native_response(wrap_message(message), "google").unwrap();
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(
        parsed.tool_calls[0]
            .extra_content
            .as_ref()
            .and_then(|v| v.pointer("/google/thought_signature"))
            .and_then(|v| v.as_str()),
        Some("SIG_RESP"),
        "the signature must land on the harness ToolCall"
    );
}

/// On rebuild, a persisted assistant tool-call message whose stored JSON carries
/// `extra_content` must re-emit it on the wire tool_calls, so Gemini sees the
/// signature on the follow-up turn (TAURI-RUST-4PK).
#[test]
fn convert_messages_for_native_echoes_tool_call_extra_content() {
    let input = vec![
        ChatMessage::assistant(
            r#"{"content":"on it","tool_calls":[{"id":"call_g","name":"shell","arguments":"{}","extra_content":{"google":{"thought_signature":"SIG_ECHO"}}}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_g","content":"done"}"#),
    ];
    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);
    let tool_calls = converted[0]
        .tool_calls
        .as_ref()
        .expect("assistant tool_calls present");
    assert_eq!(
        tool_calls[0]
            .extra_content
            .as_ref()
            .and_then(|v| v.pointer("/google/thought_signature"))
            .and_then(|v| v.as_str()),
        Some("SIG_ECHO"),
        "stored extra_content must be echoed back on the rebuilt request"
    );
    assert_eq!(
        serde_json::to_value(tool_calls)
            .unwrap()
            .pointer("/0/extra_content/google/thought_signature")
            .and_then(|v| v.as_str()),
        Some("SIG_ECHO"),
        "echoed signature must appear on the serialized wire body"
    );
}

/// A non-Gemini stored tool call (no `extra_content`) rebuilds with the field
/// omitted — every other provider's wire body stays byte-identical.
#[test]
fn convert_messages_for_native_tool_call_without_extra_content_stays_none() {
    let input = vec![
        ChatMessage::assistant(
            r#"{"content":"on it","tool_calls":[{"id":"call_x","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_x","content":"done"}"#),
    ];
    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);
    let tool_calls = converted[0]
        .tool_calls
        .as_ref()
        .expect("assistant tool_calls present");
    assert!(tool_calls[0].extra_content.is_none());
    assert!(serde_json::to_value(&tool_calls[0])
        .unwrap()
        .get("extra_content")
        .is_none());
}

/// INVARIANT (TAURI-RUST-4PK / 4PJ): a PARALLEL multi-`functionCall` assistant
/// turn reloaded from history must echo a non-empty `thought_signature` on
/// EVERY part of the rebuilt outbound payload — not just the first. The stored
/// JSON here is the exact shape `build_native_assistant_history` now emits (per
/// the writer-side test in `agent::harness::parse_tests`). Before the fix the
/// writer dropped `extra_content`, so a reloaded multi-call turn went out with
/// missing signatures and Gemini 400'd ("Function call is missing a
/// thought_signature in functionCall parts"). Covers both the non-stream and
/// streaming paths since both persist through the single native history writer.
#[test]
fn convert_messages_for_native_echoes_signature_on_every_parallel_call() {
    let stored = r#"{"content":"on it","tool_calls":[
        {"id":"call_a","name":"shell","arguments":"{}","extra_content":{"google":{"thought_signature":"SIG_A"}}},
        {"id":"call_b","name":"read","arguments":"{}","extra_content":{"google":{"thought_signature":"SIG_B"}}}
    ]}"#;
    let input = vec![
        ChatMessage::assistant(stored),
        ChatMessage::tool(r#"{"tool_call_id":"call_a","content":"done"}"#),
        ChatMessage::tool(r#"{"tool_call_id":"call_b","content":"done"}"#),
    ];
    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);
    let tool_calls = converted[0]
        .tool_calls
        .as_ref()
        .expect("assistant tool_calls survive the reload");
    assert_eq!(tool_calls.len(), 2, "both parallel calls survive");

    let wire = serde_json::to_value(tool_calls).unwrap();
    for (idx, expected) in ["SIG_A", "SIG_B"].iter().enumerate() {
        let sig = wire
            .pointer(&format!("/{idx}/extra_content/google/thought_signature"))
            .and_then(|v| v.as_str());
        assert_eq!(
            sig,
            Some(*expected),
            "functionCall part {idx} must echo its own thought_signature on the wire"
        );
        assert!(
            sig.is_some_and(|s| !s.is_empty()),
            "functionCall part {idx} thought_signature must be non-empty"
        );
    }
}

/// Streaming: Gemini sends the thought_signature in the tool-call delta's
/// `extra_content` on the first chunk. The accumulator must preserve it onto the
/// aggregated tool call so it reaches history (TAURI-RUST-4PK).
#[tokio::test]
async fn streaming_tool_call_captures_extra_content() {
    let mock_server = MockServer::start().await;
    let body = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_g\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"{}\"},\"extra_content\":{\"google\":{\"thought_signature\":\"SIG_STREAM\"}}}]}}]}\n\n\
                data: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&mock_server)
        .await;

    let provider =
        OpenAiCompatibleProvider::new("google", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "models/gemini-3.5-flash".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("hi".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(64);
    let resp = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .unwrap();
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(
        resp.tool_calls[0]
            .extra_content
            .as_ref()
            .and_then(|v| v.pointer("/google/thought_signature"))
            .and_then(|v| v.as_str()),
        Some("SIG_STREAM"),
        "streaming must preserve the thought_signature onto the aggregated tool call"
    );
}

/// Regression: some providers (DashScope/Qwen, GMI) emit the tool-call `id`
/// ONLY on the first delta for an index, then send `"id": ""` (empty string,
/// not omitted) on every argument-continuation delta. The streaming accumulator
/// must not let those empty continuation ids clobber the resolved id down to
/// `""` — an empty `tool_call_id` is rejected by the upstream tool-message
/// ordering check on the next turn (400), dead-ending the conversation.
#[tokio::test]
async fn streaming_empty_continuation_id_does_not_clobber_tool_call_id() {
    let mock_server = MockServer::start().await;
    // Delta 1 carries the real id + name; deltas 2-3 are arg continuations that
    // repeat index 0 with an EMPTY id (the DashScope/GMI wire shape).
    let body = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_real\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{}\"}}]}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"\",\"function\":{\"arguments\":\"\"}}]}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"\"}]}}]}\n\n\
                data: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&mock_server)
        .await;

    let provider =
        OpenAiCompatibleProvider::new("dashscope", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "qwen3.7-plus".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("weather in paris?".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(64);
    let resp = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .unwrap();
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(
        resp.tool_calls[0].id.as_str(),
        "call_real",
        "empty-string id on continuation deltas must not clobber the resolved tool_call id"
    );
}

/// Regression: a single turn can emit MULTIPLE parallel tool calls. The
/// per-`index` accumulator must keep each call's first-delta id even when the
/// empty-id continuation deltas for both indices arrive together — neither may
/// clobber the other (or itself) to `""`.
#[tokio::test]
async fn streaming_parallel_tool_calls_preserve_ids_against_empty_continuations() {
    let mock_server = MockServer::start().await;
    // Two parallel calls (index 0 + 1), then one continuation delta carrying
    // BOTH indices with empty ids.
    let body = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{}\"}}]}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_b\",\"type\":\"function\",\"function\":{\"name\":\"get_time\",\"arguments\":\"{}\"}}]}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"\"},{\"index\":1,\"id\":\"\"}]}}]}\n\n\
                data: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&mock_server)
        .await;

    let provider =
        OpenAiCompatibleProvider::new("dashscope", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "qwen3.7-plus".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("weather and time?".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(64);
    let resp = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .unwrap();
    assert_eq!(resp.tool_calls.len(), 2, "both parallel tool calls survive");
    // Order-independent: each id must be preserved AND mapped to the right tool
    // (no cross-index contamination).
    let by_id: std::collections::HashMap<&str, (&str, &str)> = resp
        .tool_calls
        .iter()
        .map(|t| (t.id.as_str(), (t.name.as_str(), t.arguments.as_str())))
        .collect();
    assert_eq!(by_id.get("call_a"), Some(&("get_weather", "{}")));
    assert_eq!(by_id.get("call_b"), Some(&("get_time", "{}")));
}

/// Counterpart to the DashScope cases: DeepSeek OMITS the `id` key on
/// argument-continuation deltas (rather than sending `""`). That deserializes
/// to `None`, so the accumulator already leaves the resolved id alone — assert
/// the contract holds for the key-absent shape too, and that args still
/// accumulate across the continuation.
#[tokio::test]
async fn streaming_omitted_continuation_id_preserves_tool_call_id() {
    let mock_server = MockServer::start().await;
    // Delta 2 has NO `id` key at all (DeepSeek shape) and carries the rest of
    // the arguments.
    let body = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_ds\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\"}}]}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"}\"}}]}}]}\n\n\
                data: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&mock_server)
        .await;

    let provider =
        OpenAiCompatibleProvider::new("deepseek", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "deepseek-v4-flash".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("weather?".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(64);
    let resp = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .unwrap();
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id.as_str(), "call_ds");
    assert_eq!(resp.tool_calls[0].name.as_str(), "get_weather");
    assert_eq!(
        resp.tool_calls[0].arguments.as_str(),
        "{}",
        "arguments must accumulate across the id-omitted continuation delta"
    );
}

/// Helper: roles in serialized order.
fn roles(messages: &[NativeMessage]) -> Vec<&str> {
    messages.iter().map(|m| m.role.as_str()).collect()
}

/// Mechanism (A): history tail-trimming dropped an `assistant(tool_calls)` but
/// kept its `tool` result, orphaning the result at the head of the window. The
/// sanitizer must drop the orphan so the wire array never starts a tool block
/// without a preceding `tool_calls`.
#[test]
fn tool_invariants_drop_orphaned_tool_result_from_trim(/* A */) {
    let input = vec![
        ChatMessage::system("system prompt"),
        // assistant(tool_calls=call_orphan) was sliced off by trim_history;
        // only its result survived as the first non-system message.
        ChatMessage::tool(r#"{"tool_call_id":"call_orphan","content":"stale result"}"#),
        ChatMessage::user("and then?"),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(roles(&converted), vec!["system", "user"]);
    assert!(
        converted.iter().all(|m| m.role != "tool"),
        "orphaned tool result must be dropped"
    );
}

/// Mechanism (B): a persisted assistant tool-call message whose `content` no
/// longer parses as `{tool_calls: [...]}` is emitted as plain assistant text
/// with its `tool_calls` stripped, orphaning the following `tool` result. The
/// assistant message stays; the now-orphaned tool result is dropped.
#[test]
fn tool_invariants_drop_tool_after_unparseable_assistant_call(/* B */) {
    let input = vec![
        // Plain text, not the JSON tool-call shape -> tool_calls stripped on convert.
        ChatMessage::assistant("let me check that for you"),
        ChatMessage::tool(r#"{"tool_call_id":"call_b","content":"tool ran"}"#),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(roles(&converted), vec!["assistant"]);
    assert!(converted[0].tool_calls.is_none());
    assert!(
        converted.iter().all(|m| m.role != "tool"),
        "tool result with no opening tool_calls must be dropped"
    );
}

/// Mechanism (C): an `assistant(tool_calls=[answered, missing])` whose second
/// call never received a `tool` response (aborted / max-iteration turn, or a
/// partially-answered multi-call cycle). The sanitizer prunes the dangling
/// tool-call entry while keeping the answered one and its result.
#[test]
fn tool_invariants_prune_unanswered_tool_call(/* C */) {
    let input = vec![
        ChatMessage::assistant(
            r#"{"content":"on it","tool_calls":[{"id":"call_done","name":"shell","arguments":"{}"},{"id":"call_missing","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_done","content":"finished"}"#),
        // call_missing never gets a tool response.
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    let assistant = converted
        .iter()
        .find(|m| m.role == "assistant")
        .expect("assistant message present");
    let calls = assistant
        .tool_calls
        .as_ref()
        .expect("answered tool_call retained");
    assert_eq!(calls.len(), 1, "dangling tool_call must be pruned");
    assert_eq!(calls[0].id.as_deref(), Some("call_done"));
    assert!(
        converted
            .iter()
            .any(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some("call_done")),
        "answered tool result must survive"
    );
}

/// (C) extreme: an `assistant(tool_calls)` with NO response at all collapses to
/// a plain assistant message (tool_calls dropped) rather than a dangling block.
#[test]
fn tool_invariants_collapse_fully_unanswered_assistant_call() {
    let input = vec![
        ChatMessage::assistant(
            r#"{"content":"on it","tool_calls":[{"id":"call_x","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::assistant("never mind, here's the answer"),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(roles(&converted), vec!["assistant", "assistant"]);
    assert!(
        converted[0].tool_calls.is_none(),
        "fully-unanswered tool_calls must be dropped"
    );
    assert_eq!(
        serde_json::to_value(&converted[0].content).unwrap(),
        serde_json::json!("on it")
    );
}

/// Regression guard: a well-formed tool cycle is passed through untouched —
/// the sanitizer must not strip or reorder valid messages.
#[test]
fn tool_invariants_preserve_well_formed_cycle() {
    let input = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("run it"),
        ChatMessage::assistant(
            r#"{"content":"on it","tool_calls":[{"id":"call_ok","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_ok","content":"done"}"#),
        ChatMessage::assistant("all set"),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(
        roles(&converted),
        vec!["system", "user", "assistant", "tool", "assistant"]
    );
    assert_eq!(converted[2].tool_calls.as_ref().unwrap().len(), 1);
    assert_eq!(
        converted[2].tool_calls.as_ref().unwrap()[0].id.as_deref(),
        Some("call_ok")
    );
    assert_eq!(converted[3].tool_call_id.as_deref(), Some("call_ok"));
}

/// Sequential tool cycles — successive agent iterations, each its own
/// `assistant(tool_calls)` → `tool` block. Distinct ids, opened then immediately
/// consumed. All survive untouched.
#[test]
fn tool_invariants_preserve_sequential_cycles() {
    let input = vec![
        ChatMessage::user("go"),
        ChatMessage::assistant(
            r#"{"content":"step 1","tool_calls":[{"id":"call_a","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_a","content":"a done"}"#),
        ChatMessage::assistant(
            r#"{"content":"step 2","tool_calls":[{"id":"call_b","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_b","content":"b done"}"#),
        ChatMessage::assistant(
            r#"{"content":"step 3","tool_calls":[{"id":"call_c","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_c","content":"c done"}"#),
        ChatMessage::assistant("all done"),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(
        roles(&converted),
        vec![
            "user",
            "assistant",
            "tool",
            "assistant",
            "tool",
            "assistant",
            "tool",
            "assistant"
        ]
    );
    for idx in [1usize, 3, 5] {
        assert_eq!(
            converted[idx].tool_calls.as_ref().unwrap().len(),
            1,
            "cycle at index {idx} must keep its call"
        );
    }
}

/// Parallel tool calls — one `assistant` issuing N calls, answered by N `tool`
/// messages arriving out of order. All survive; pairing is by membership, not
/// position, so order does not matter.
#[test]
fn tool_invariants_preserve_parallel_calls() {
    let input = vec![
        ChatMessage::assistant(
            r#"{"content":"fanning out","tool_calls":[{"id":"call_x","name":"shell","arguments":"{}"},{"id":"call_y","name":"shell","arguments":"{}"},{"id":"call_z","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_y","content":"y"}"#),
        ChatMessage::tool(r#"{"tool_call_id":"call_z","content":"z"}"#),
        ChatMessage::tool(r#"{"tool_call_id":"call_x","content":"x"}"#),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(roles(&converted), vec!["assistant", "tool", "tool", "tool"]);
    assert_eq!(converted[0].tool_calls.as_ref().unwrap().len(), 3);
}

/// Trim bisecting a sequence: the window opens inside cycle A (its assistant was
/// sliced off), followed by an intact cycle B. The orphaned A result is dropped;
/// cycle B survives — proving adjacency-pairing localizes the damage.
#[test]
fn tool_invariants_drop_orphan_but_keep_following_cycle() {
    let input = vec![
        // assistant(call_a) was sliced off by trim; only its result remains.
        ChatMessage::tool(r#"{"tool_call_id":"call_a","content":"orphaned"}"#),
        ChatMessage::assistant(
            r#"{"content":"step 2","tool_calls":[{"id":"call_b","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_b","content":"b done"}"#),
        ChatMessage::assistant("done"),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(roles(&converted), vec!["assistant", "tool", "assistant"]);
    assert_eq!(converted[0].tool_calls.as_ref().unwrap().len(), 1);
    assert_eq!(converted[1].tool_call_id.as_deref(), Some("call_b"));
}

/// DeepSeek thinking mode (Sentry TAURI-RUST-4KB): an `assistant` turn that
/// carries `tool_calls` must replay its `reasoning_content` on the follow-up
/// request, otherwise DeepSeek returns
/// `400 The reasoning_content in the thinking mode must be passed back to the
/// API.` The history JSON written by `build_native_assistant_history` carries
/// `reasoning_content`; `convert_messages_for_native` must lift it back onto
/// the wire message.
#[test]
fn convert_preserves_reasoning_content_on_tool_call_turn() {
    let input = vec![
        ChatMessage::assistant(
            r#"{"content":null,"reasoning_content":"let me think about this","tool_calls":[{"id":"call_x","name":"shell","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_x","content":"result"}"#),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    // First message is the assistant with tool_calls + reasoning.
    assert_eq!(
        converted[0].reasoning_content.as_deref(),
        Some("let me think about this")
    );
    assert!(converted[0].tool_calls.is_some());

    // The wire payload must actually carry the field for DeepSeek to accept it.
    let wire = serde_json::to_value(&converted[0]).unwrap();
    assert_eq!(wire["reasoning_content"], "let me think about this");
}

/// Assistant tool-call turns from non-reasoning models carry no
/// `reasoning_content`; it must never appear on the wire for them (most
/// OpenAI-compatible providers don't recognise the field).
#[test]
fn convert_omits_reasoning_content_when_absent() {
    let input = vec![ChatMessage::assistant(
        r#"{"content":"sure","tool_calls":[{"id":"call_y","name":"shell","arguments":"{}"}]}"#,
    )];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(converted.len(), 1);
    assert!(converted[0].reasoning_content.is_none());

    let wire = serde_json::to_value(&converted[0]).unwrap();
    assert!(
        wire.get("reasoning_content").is_none(),
        "reasoning_content must be omitted from the wire when absent"
    );
}

/// Tool-call assistant messages with no narrative text must emit `"content":""`
/// on the wire (not omit the key) so providers that validate the presence of a
/// content field alongside reasoning_content don't reject the request.
#[test]
fn convert_tool_call_turn_emits_content_key_even_when_empty() {
    let input = vec![
        ChatMessage::assistant(
            r#"{"content":null,"reasoning_content":"thinking","tool_calls":[{"id":"call_a","name":"web_fetch","arguments":"{}"}]}"#,
        ),
        ChatMessage::tool(r#"{"tool_call_id":"call_a","content":"fetched"}"#),
    ];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);
    let wire = serde_json::to_value(&converted[0]).unwrap();

    assert!(
        wire.get("content").is_some(),
        "content key must be present on the wire even when the model emitted null/empty content"
    );
    assert_eq!(wire["content"], "");
    assert_eq!(wire["reasoning_content"], "thinking");
}

/// When `enforce_tool_message_invariants` collapses an assistant tool-call
/// message to plain text (all tool_calls pruned because no responses matched),
/// it must also clear `reasoning_content` — leaving stale reasoning on a
/// non-tool assistant message is a malformed shape for thinking-mode providers.
#[test]
fn enforce_invariants_clears_reasoning_when_assistant_collapses_to_text() {
    let messages = vec![
        NativeMessage {
            role: "assistant".to_string(),
            content: Some("partial thought".into()),
            tool_call_id: None,
            tool_calls: Some(vec![ToolCall {
                id: Some("orphan_call".to_string()),
                kind: Some("function".to_string()),
                function: Some(Function {
                    name: Some("web_fetch".to_string()),
                    arguments: Some(serde_json::Value::String("{}".to_string())),
                }),
                extra_content: None,
            }]),
            reasoning_content: Some("deep reasoning".to_string()),
        },
        // No tool result follows — the tool_calls are orphaned.
        NativeMessage {
            role: "user".to_string(),
            content: Some("next question".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        },
    ];

    let sanitized = OpenAiCompatibleProvider::enforce_tool_message_invariants(messages);

    // The assistant message should have been collapsed: tool_calls pruned.
    let assistant = &sanitized[0];
    assert!(assistant.tool_calls.is_none());
    // reasoning_content must also be cleared on collapse.
    assert!(
        assistant.reasoning_content.is_none(),
        "reasoning_content must be stripped when tool_calls are pruned to avoid malformed shape"
    );
}

#[test]
fn chat_message_identity_metadata_is_not_provider_wire_payload() {
    let message = ChatMessage {
        id: Some("msg_123".to_string()),
        role: "user".to_string(),
        content: "hello".to_string(),
        extra_metadata: Some(serde_json::json!({"citation": "mem-1"})),
    };

    let serialized = serde_json::to_value(&message).unwrap();

    assert_eq!(
        serialized.get("role").and_then(|v| v.as_str()),
        Some("user")
    );
    assert_eq!(
        serialized.get("content").and_then(|v| v.as_str()),
        Some("hello")
    );
    assert!(
        serialized.get("id").is_none(),
        "provider ChatMessage serialization must not leak UI message ids"
    );
    assert!(
        serialized.get("extra_metadata").is_none(),
        "provider ChatMessage serialization must not leak UI metadata"
    );
}

#[test]
fn flatten_system_messages_merges_into_first_user() {
    let input = vec![
        ChatMessage::system("core policy"),
        ChatMessage::assistant("ack"),
        ChatMessage::system("delivery rules"),
        ChatMessage::user("hello"),
        ChatMessage::assistant("post-user"),
    ];

    let output = OpenAiCompatibleProvider::flatten_system_messages(&input);
    assert_eq!(output.len(), 3);
    assert_eq!(output[0].role, "assistant");
    assert_eq!(output[0].content, "ack");
    assert_eq!(output[1].role, "user");
    assert_eq!(output[1].content, "core policy\n\ndelivery rules\n\nhello");
    assert_eq!(output[2].role, "assistant");
    assert_eq!(output[2].content, "post-user");
    assert!(output.iter().all(|m| m.role != "system"));
}

#[test]
fn flatten_system_messages_inserts_user_when_missing() {
    let input = vec![
        ChatMessage::system("core policy"),
        ChatMessage::assistant("ack"),
    ];

    let output = OpenAiCompatibleProvider::flatten_system_messages(&input);
    assert_eq!(output.len(), 2);
    assert_eq!(output[0].role, "user");
    assert_eq!(output[0].content, "core policy");
    assert_eq!(output[1].role, "assistant");
    assert_eq!(output[1].content, "ack");
}

#[test]
fn strip_think_tags_drops_unclosed_block_suffix() {
    let input = "visible<think>hidden";
    assert_eq!(strip_think_tags(input), "visible");
}

#[test]
fn native_tool_schema_unsupported_detection_is_precise() {
    assert!(OpenAiCompatibleProvider::is_native_tool_schema_unsupported(
        reqwest::StatusCode::BAD_REQUEST,
        "unknown parameter: tools"
    ));
    assert!(
        !OpenAiCompatibleProvider::is_native_tool_schema_unsupported(
            reqwest::StatusCode::UNAUTHORIZED,
            "unknown parameter: tools"
        )
    );
}

#[test]
fn prompt_guided_tool_fallback_injects_system_instruction() {
    let input = vec![ChatMessage::user("check status")];
    let tools = vec![crate::openhuman::tools::ToolSpec {
        name: "shell_exec".to_string(),
        description: "Execute shell command".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" }
            },
            "required": ["command"]
        }),
    }];

    let output =
        OpenAiCompatibleProvider::with_prompt_guided_tool_instructions(&input, Some(&tools));
    assert!(!output.is_empty());
    assert_eq!(output[0].role, "system");
    assert!(output[0].content.contains("Available Tools"));
    assert!(output[0].content.contains("shell_exec"));
}

#[tokio::test]
async fn warmup_without_key_is_noop() {
    let provider = make_provider("test", "https://example.com", None);
    let result = provider.warmup().await;
    assert!(result.is_ok());
}

// ══════════════════════════════════════════════════════════
// Native tool calling tests
// ══════════════════════════════════════════════════════════

#[test]
fn capabilities_reports_native_tool_calling() {
    let p = make_provider("test", "https://example.com", None);
    let caps = <OpenAiCompatibleProvider as Provider>::capabilities(&p);
    assert!(caps.native_tool_calling);
    assert!(caps.vision);
}

// Sub-issue 3 of #3098: Ollama's OpenAI-compat endpoint silently rejects the
// `tools` parameter for many models, so we must let the factory opt the
// Ollama provider out of native tool calling. The agent harness then falls
// back to prompt-guided tool specs (embedded in the system prompt) which
// any chat model can follow. The builder defaults to enabled so cloud
// providers (OpenAI, BYOK slugs, OpenHuman backend) are unaffected.

#[test]
fn with_native_tool_calling_false_disables_capability() {
    let p = make_provider("test", "https://example.com", None).with_native_tool_calling(false);
    let caps = <OpenAiCompatibleProvider as Provider>::capabilities(&p);
    assert!(
        !caps.native_tool_calling,
        "capabilities() must mirror the builder override; this is the gate the agent harness uses to decide between native vs prompt-guided tool specs"
    );
}

#[test]
fn with_native_tool_calling_true_preserves_default() {
    let p = make_provider("test", "https://example.com", None).with_native_tool_calling(true);
    let caps = <OpenAiCompatibleProvider as Provider>::capabilities(&p);
    assert!(caps.native_tool_calling);
}

#[test]
fn with_native_tool_calling_is_idempotent() {
    let p = make_provider("test", "https://example.com", None)
        .with_native_tool_calling(false)
        .with_native_tool_calling(false);
    let caps = <OpenAiCompatibleProvider as Provider>::capabilities(&p);
    assert!(!caps.native_tool_calling);
}

#[test]
fn with_vision_false_disables_capability() {
    let p = make_provider("test", "https://example.com", None).with_vision(false);
    let caps = <OpenAiCompatibleProvider as Provider>::capabilities(&p);
    assert!(!caps.vision);
    assert!(!p.supports_vision());
}

/// `supports_native_tools()` is the gate the agent harness reads
/// (`traits.rs:415`) when deciding whether to send tools natively or
/// inject them into the prompt. It MUST agree with
/// `capabilities().native_tool_calling`; otherwise
/// `with_native_tool_calling(false)` silently fails to switch to
/// prompt-guided and Ollama still receives a `tools` array (the exact
/// regression sub-issue 3 of #3098 was meant to fix).
#[test]
fn supports_native_tools_mirrors_capabilities_flag() {
    let default = make_provider("test", "https://example.com", None);
    assert_eq!(
        default.supports_native_tools(),
        <OpenAiCompatibleProvider as Provider>::capabilities(&default).native_tool_calling,
        "default provider: the two capability signals must match"
    );
    assert!(default.supports_native_tools(), "default must remain true");

    let opted_out =
        make_provider("test", "https://example.com", None).with_native_tool_calling(false);
    assert_eq!(
        opted_out.supports_native_tools(),
        <OpenAiCompatibleProvider as Provider>::capabilities(&opted_out).native_tool_calling,
        "after with_native_tool_calling(false): the two capability signals must match"
    );
    assert!(
        !opted_out.supports_native_tools(),
        "after with_native_tool_calling(false), supports_native_tools must report false so the harness picks the prompt-guided fallback"
    );
}

#[test]
fn tool_specs_convert_to_openai_format() {
    let specs = vec![crate::openhuman::tools::ToolSpec {
        name: "shell".to_string(),
        description: "Run shell command".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"]
        }),
    }];

    let tools = OpenAiCompatibleProvider::tool_specs_to_openai_format(&specs);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "shell");
    assert_eq!(tools[0]["function"]["description"], "Run shell command");
    assert_eq!(tools[0]["function"]["parameters"]["required"][0], "command");
}

#[test]
fn request_serializes_with_tools() {
    let tools = vec![serde_json::json!({
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get weather for a location",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                }
            }
        }
    })];

    let req = ApiChatRequest {
        model: "test-model".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "What is the weather?".into(),
        }],
        temperature: Some(0.7),
        stream: Some(false),
        tools: Some(tools),
        tool_choice: Some("auto".to_string()),
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"tools\""));
    assert!(json.contains("get_weather"));
    assert!(json.contains("\"tool_choice\":\"auto\""));
}

#[test]
fn response_with_tool_calls_deserializes() {
    let json = r#"{
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"location\":\"London\"}"
                    }
                }]
            }
        }]
    }"#;

    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert!(msg.content.is_none());
    let tool_calls = msg.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(
        tool_calls[0].function.as_ref().unwrap().name.as_deref(),
        Some("get_weather")
    );
    assert_eq!(
        tool_calls[0].function.as_ref().unwrap().arguments.as_ref(),
        Some(&serde_json::Value::String(
            "{\"location\":\"London\"}".to_string()
        ))
    );
}

#[test]
fn response_with_tool_call_object_arguments_deserializes() {
    let json = r#"{
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_456",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": {"location":"London","unit":"c"}
                    }
                }]
            }
        }]
    }"#;

    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    let tool_calls = msg.tool_calls.as_ref().unwrap();
    assert_eq!(
        tool_calls[0].function.as_ref().unwrap().arguments.as_ref(),
        Some(&serde_json::json!({"location":"London","unit":"c"}))
    );

    let parsed = OpenAiCompatibleProvider::parse_native_response(
        wrap_message(ResponseMessage {
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: Some("call_456".to_string()),
                kind: Some("function".to_string()),
                function: Some(Function {
                    name: Some("get_weather".to_string()),
                    arguments: Some(serde_json::json!({"location":"London","unit":"c"})),
                }),
                extra_content: None,
            }]),
            function_call: None,
        }),
        "test",
    )
    .unwrap();
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].id, "call_456");
    assert_eq!(
        parsed.tool_calls[0].arguments,
        r#"{"location":"London","unit":"c"}"#
    );
}

#[test]
fn parse_native_response_recovers_tool_calls_from_json_content() {
    let content = r#"{"content":"Checking files...","tool_calls":[{"id":"call_json_1","function":{"name":"shell","arguments":"{\"command\":\"ls -la\"}"}}]}"#;
    let parsed = OpenAiCompatibleProvider::parse_native_response(
        wrap_message(ResponseMessage {
            content: Some(content.to_string()),
            reasoning_content: None,
            tool_calls: None,
            function_call: None,
        }),
        "test",
    )
    .unwrap();

    assert_eq!(parsed.text.as_deref(), Some("Checking files..."));
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].id, "call_json_1");
    assert_eq!(parsed.tool_calls[0].name, "shell");
    assert_eq!(parsed.tool_calls[0].arguments, r#"{"command":"ls -la"}"#);
}

#[test]
fn parse_native_response_supports_legacy_function_call() {
    let parsed = OpenAiCompatibleProvider::parse_native_response(
        wrap_message(ResponseMessage {
            content: Some("Let me check".to_string()),
            reasoning_content: None,
            tool_calls: None,
            function_call: Some(Function {
                name: Some("shell".to_string()),
                arguments: Some(serde_json::Value::String(
                    r#"{"command":"pwd"}"#.to_string(),
                )),
            }),
        }),
        "test",
    )
    .unwrap();

    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].name, "shell");
    assert_eq!(parsed.tool_calls[0].arguments, r#"{"command":"pwd"}"#);
}

#[test]
fn response_with_multiple_tool_calls() {
    let json = r#"{
        "choices": [{
            "message": {
                "content": "I'll check both.",
                "tool_calls": [
                    {
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"London\"}"
                        }
                    },
                    {
                        "type": "function",
                        "function": {
                            "name": "get_time",
                            "arguments": "{\"timezone\":\"UTC\"}"
                        }
                    }
                ]
            }
        }]
    }"#;

    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.content.as_deref(), Some("I'll check both."));
    let tool_calls = msg.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(
        tool_calls[0].function.as_ref().unwrap().name.as_deref(),
        Some("get_weather")
    );
    assert_eq!(
        tool_calls[1].function.as_ref().unwrap().name.as_deref(),
        Some("get_time")
    );
}

#[tokio::test]
async fn chat_with_tools_fails_without_key() {
    let p = make_provider("TestProvider", "https://example.com", None);
    let messages = vec![ChatMessage {
        id: None,
        role: "user".to_string(),
        content: "hello".to_string(),
        extra_metadata: None,
    }];
    let tools = vec![serde_json::json!({
        "type": "function",
        "function": {
            "name": "test_tool",
            "description": "A test tool",
            "parameters": {}
        }
    })];

    let result = p.chat_with_tools(&messages, &tools, "model", 0.7).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("TestProvider API key not set"));
}

#[test]
fn response_with_no_tool_calls_has_empty_vec() {
    let json = r#"{"choices":[{"message":{"content":"Just text, no tools."}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.content.as_deref(), Some("Just text, no tools."));
    assert!(msg.tool_calls.is_none());
}

#[test]
fn flatten_system_messages_merges_into_first_user_and_removes_system_roles() {
    let messages = vec![
        ChatMessage::system("System A"),
        ChatMessage::assistant("Earlier assistant turn"),
        ChatMessage::system("System B"),
        ChatMessage::user("User turn"),
        ChatMessage::tool(r#"{"ok":true}"#),
    ];

    let flattened = OpenAiCompatibleProvider::flatten_system_messages(&messages);
    assert_eq!(flattened.len(), 3);
    assert_eq!(flattened[0].role, "assistant");
    assert_eq!(
        flattened[1].content,
        "System A\n\nSystem B\n\nUser turn".to_string()
    );
    assert_eq!(flattened[1].role, "user");
    assert_eq!(flattened[2].role, "tool");
    assert!(!flattened.iter().any(|m| m.role == "system"));
}

#[test]
fn flatten_system_messages_inserts_synthetic_user_when_no_user_exists() {
    let messages = vec![
        ChatMessage::assistant("Assistant only"),
        ChatMessage::system("Synthetic system"),
    ];

    let flattened = OpenAiCompatibleProvider::flatten_system_messages(&messages);
    assert_eq!(flattened.len(), 2);
    assert_eq!(flattened[0].role, "user");
    assert_eq!(flattened[0].content, "Synthetic system");
    assert_eq!(flattened[1].role, "assistant");
}

#[test]
fn strip_think_tags_removes_multiple_blocks_with_surrounding_text() {
    let input = "Answer A <think>hidden 1</think> and B <think>hidden 2</think> done";
    let output = strip_think_tags(input);
    assert_eq!(output, "Answer A  and B  done");
}

#[test]
fn strip_think_tags_drops_tail_for_unclosed_block() {
    let input = "Visible<think>hidden tail";
    let output = strip_think_tags(input);
    assert_eq!(output, "Visible");
}

// ----------------------------------------------------------
// Reasoning model fallback tests (reasoning_content)
// ----------------------------------------------------------

#[test]
fn reasoning_content_fallback_when_content_empty() {
    // Reasoning models (Qwen3, GLM-4) return content: "" with reasoning_content populated
    let json =
        r#"{"choices":[{"message":{"content":"","reasoning_content":"Thinking output here"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.effective_content(), "Thinking output here");
}

#[test]
fn reasoning_content_fallback_when_content_null() {
    // Some models may return content: null with reasoning_content
    let json = r#"{"choices":[{"message":{"content":null,"reasoning_content":"Fallback text"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.effective_content(), "Fallback text");
}

#[test]
fn reasoning_content_fallback_when_content_missing() {
    // content field absent entirely, reasoning_content present
    let json = r#"{"choices":[{"message":{"reasoning_content":"Only reasoning"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.effective_content(), "Only reasoning");
}

#[test]
fn reasoning_content_not_used_when_content_present() {
    // Normal model: content populated, reasoning_content should be ignored
    let json = r#"{"choices":[{"message":{"content":"Normal response","reasoning_content":"Should be ignored"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.effective_content(), "Normal response");
}

#[test]
fn reasoning_content_used_when_content_only_think_tags() {
    let json = r#"{"choices":[{"message":{"content":"<think>secret</think>","reasoning_content":"Fallback text"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.effective_content(), "Fallback text");
    assert_eq!(
        msg.effective_content_optional().as_deref(),
        Some("Fallback text")
    );
}

#[test]
fn reasoning_content_both_absent_returns_empty() {
    // Neither content nor reasoning_content - returns empty string
    let json = r#"{"choices":[{"message":{}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.effective_content(), "");
}

#[test]
fn reasoning_content_ignored_by_normal_models() {
    // Standard response without reasoning_content still works
    let json = r#"{"choices":[{"message":{"content":"Hello from Venice!"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert!(msg.reasoning_content.is_none());
    assert_eq!(msg.effective_content(), "Hello from Venice!");
}

// ----------------------------------------------------------
// `reasoning` field-name alias (issue #3094)
//
// DeepSeek/Qwen3/GLM-4 emit chain-of-thought as `reasoning_content`, but
// OpenRouter and vLLM/SGLang-backed OpenAI-compatible proxies emit it as
// `reasoning`. If we only deserialize `reasoning_content`, a third-party
// thinking-mode provider that uses `reasoning` is captured as `None`, so the
// CoT is never replayed on the follow-up tool-call turn and the provider
// rejects the request with `400 The reasoning_content in the thinking mode
// must be passed back to the API`. The `#[serde(alias = "reasoning")]` makes
// both field names map to the same captured value.
// ----------------------------------------------------------

#[test]
fn reasoning_alias_captured_from_response_message() {
    let json = r#"{"choices":[{"message":{"content":null,"reasoning":"weighing the options"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(
        msg.reasoning_content.as_deref(),
        Some("weighing the options")
    );
}

#[test]
fn reasoning_content_canonical_field_still_wins_over_alias_absence() {
    // The canonical `reasoning_content` field keeps working unchanged.
    let json = r#"{"choices":[{"message":{"content":null,"reasoning_content":"canonical cot"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let msg = &resp.choices[0].message;
    assert_eq!(msg.reasoning_content.as_deref(), Some("canonical cot"));
}

#[test]
fn reasoning_alias_captured_in_stream_delta() {
    let json = r#"{"choices":[{"delta":{"reasoning":"streamed cot"},"finish_reason":null}]}"#;
    let chunk: StreamChunkResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        chunk.choices[0].delta.reasoning_content.as_deref(),
        Some("streamed cot")
    );
}

/// Regression for Sentry TAURI-RUST-A5N: a provider that emits BOTH `reasoning`
/// and `reasoning_content` in the same message object must not fail with
/// `duplicate field \`reasoning_content\``. Both keys deserialize and fold into
/// the canonical field, which wins when both are present.
#[test]
fn reasoning_and_reasoning_content_both_present_does_not_error() {
    let json = r#"{"choices":[{"message":{"content":null,"reasoning":"alias cot","reasoning_content":"canonical cot"}}]}"#;
    let resp: ApiChatResponse = serde_json::from_str(json)
        .expect("both reasoning keys must parse without a duplicate-field error");
    assert_eq!(
        resp.choices[0].message.reasoning_content.as_deref(),
        Some("canonical cot"),
        "canonical reasoning_content wins when both keys are present"
    );
}

/// Same regression on the streaming delta path (TAURI-RUST-A5N also hits the
/// native stream parser at `compatible_stream_native.rs`).
#[test]
fn reasoning_and_reasoning_content_both_present_in_stream_delta_does_not_error() {
    let json = r#"{"choices":[{"delta":{"reasoning":"alias cot","reasoning_content":"canonical cot"},"finish_reason":null}]}"#;
    let chunk: StreamChunkResponse = serde_json::from_str(json)
        .expect("both reasoning keys must parse without a duplicate-field error");
    assert_eq!(
        chunk.choices[0].delta.reasoning_content.as_deref(),
        Some("canonical cot"),
        "canonical reasoning_content wins when both keys are present"
    );
}

/// End-to-end: a tool-call turn whose reasoning arrived under the `reasoning`
/// alias must still be surfaced by `parse_native_response` so the agent loop
/// can replay it on the follow-up request (the issue #3094 failure path).
#[test]
fn parse_native_response_captures_reasoning_from_alias() {
    let json = r#"{
        "choices":[{"message":{
            "content":null,
            "reasoning":"  let me think about this  ",
            "tool_calls":[{"id":"call_z","type":"function","function":{"name":"web_fetch","arguments":"{}"}}]
        }}]
    }"#;
    let resp: ApiChatResponse = serde_json::from_str(json).unwrap();
    let parsed = OpenAiCompatibleProvider::parse_native_response(resp, "deepseek").unwrap();
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(
        parsed.reasoning_content.as_deref(),
        Some("let me think about this"),
        "reasoning captured via the `reasoning` alias must be available to replay"
    );
}

// ----------------------------------------------------------
// SSE streaming reasoning_content fallback tests
// ----------------------------------------------------------

#[test]
fn parse_sse_line_with_content() {
    let line = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
    let result = parse_sse_line(line).unwrap();
    assert_eq!(result, Some("hello".to_string()));
}

#[test]
fn parse_sse_line_with_reasoning_content() {
    let line = r#"data: {"choices":[{"delta":{"reasoning_content":"thinking..."}}]}"#;
    let result = parse_sse_line(line).unwrap();
    assert_eq!(result, Some("thinking...".to_string()));
}

#[test]
fn parse_sse_line_with_both_prefers_content() {
    let line = r#"data: {"choices":[{"delta":{"content":"real answer","reasoning_content":"thinking..."}}]}"#;
    let result = parse_sse_line(line).unwrap();
    assert_eq!(result, Some("real answer".to_string()));
}

#[test]
fn parse_sse_line_with_empty_content_falls_back_to_reasoning_content() {
    let line = r#"data: {"choices":[{"delta":{"content":"","reasoning_content":"thinking..."}}]}"#;
    let result = parse_sse_line(line).unwrap();
    assert_eq!(result, Some("thinking...".to_string()));
}

#[test]
fn parse_sse_line_done_sentinel() {
    let line = "data: [DONE]";
    let result = parse_sse_line(line).unwrap();
    assert_eq!(result, None);
}

#[test]
fn normalize_function_arguments_valid_json_string_preserved() {
    let v = Some(serde_json::Value::String(r#"{"path":"/tmp"}"#.to_string()));
    assert_eq!(normalize_function_arguments(v), r#"{"path":"/tmp"}"#);
}

#[test]
fn normalize_function_arguments_invalid_json_string_falls_back_to_empty_object() {
    // OPENHUMAN-TAURI-6F: model emitted malformed JSON in `function.arguments`.
    // Forwarding the raw string back upstream causes a 400 from the backend's
    // `json.loads`. Substitute `{}` instead.
    for raw in ["{a:1}", "{'k':'v'}", "{\n", "{,}"] {
        let v = Some(serde_json::Value::String(raw.to_string()));
        assert_eq!(normalize_function_arguments(v), "{}", "raw = {raw:?}");
    }
}

#[test]
fn normalize_function_arguments_empty_or_null_becomes_empty_object() {
    assert_eq!(
        normalize_function_arguments(Some(serde_json::Value::String("   ".to_string()))),
        "{}"
    );
    assert_eq!(
        normalize_function_arguments(Some(serde_json::Value::Null)),
        "{}"
    );
    assert_eq!(normalize_function_arguments(None), "{}");
}

#[test]
fn normalize_function_arguments_object_value_serializes() {
    let v = Some(serde_json::json!({"path": "/tmp"}));
    assert_eq!(normalize_function_arguments(v), r#"{"path":"/tmp"}"#);
}

#[test]
fn parse_provider_tool_call_from_value_guards_malformed_arguments() {
    // OPENHUMAN-TAURI-6F: the early-return path in
    // `parse_provider_tool_call_from_value` previously bypassed
    // `normalize_function_arguments`, forwarding malformed JSON strings
    // directly. Verify the guard now applies on both code paths.
    let value = serde_json::json!({
        "id": "call_bad",
        "name": "shell",
        "arguments": "{a:1}"
    });
    let result = parse_provider_tool_call_from_value(&value);
    let call = result.expect("should produce a ToolCall");
    assert_eq!(
        call.arguments, "{}",
        "malformed arguments string must be normalised to {{}} via the first-path guard"
    );
}

#[test]
fn custom_openai_provider_has_no_responses_fallback() {
    let p = OpenAiCompatibleProvider::new_no_responses_fallback(
        "custom_openai",
        "http://localhost:11434/v1",
        Some("sk-test"),
        AuthStyle::Bearer,
    );
    assert!(
        !p.supports_responses_fallback,
        "custom_openai must not attempt the /v1/responses fallback"
    );
}

#[test]
fn responses_404_disables_fallback_for_endpoint() {
    // TAURI-RUST-FJZ: a custom slug (factory can't classify it, so the static
    // fallback flag is ON) whose endpoint 404s on `/responses` must stop
    // attempting that fallback once the route is known-missing — routing to
    // chat-completions only. Use a unique base_url; the cache is process-global.
    let base_url = "https://responses-404-test.example.com/v1";
    let p =
        OpenAiCompatibleProvider::new("nous-portal", base_url, Some("sk-test"), AuthStyle::Bearer);
    assert!(
        p.responses_fallback_active(),
        "a fresh custom slug starts with the fallback enabled"
    );

    super::mark_responses_api_unsupported(base_url);

    assert!(
        super::responses_api_known_unsupported(base_url),
        "the endpoint is recorded as Responses-incapable after a 404"
    );
    assert!(
        !p.responses_fallback_active(),
        "the fallback is disabled once `/responses` has 404'd for this endpoint"
    );

    // A provider for a different endpoint is unaffected.
    let other = OpenAiCompatibleProvider::new(
        "nous-portal",
        "https://responses-404-test.example.com/v2",
        Some("sk-test"),
        AuthStyle::Bearer,
    );
    assert!(
        other.responses_fallback_active(),
        "the cache is keyed per-endpoint, not globally"
    );
}

#[test]
fn responses_404_route_vs_model_disambiguation() {
    // A generic "route missing" 404 → this endpoint has no Responses API.
    assert!(OpenAiCompatibleProvider::responses_404_indicates_missing_route("404 Not Found"));
    assert!(
        OpenAiCompatibleProvider::responses_404_indicates_missing_route(
            "<html><body>404 page not found</body></html>"
        )
    );
    // A model/deployment-specific 404 → the route exists; keep the fallback so
    // we don't poison the cache for other models on a Responses-capable endpoint.
    assert!(
        !OpenAiCompatibleProvider::responses_404_indicates_missing_route(
            r#"{"error":{"message":"model 'gpt-x' not found","code":"model_not_found"}}"#
        )
    );
    assert!(
        !OpenAiCompatibleProvider::responses_404_indicates_missing_route(
            r#"{"error":{"message":"The API deployment for this resource does not exist"}}"#
        )
    );
}

#[test]
fn enrich_404_message_adds_hint_when_no_fallback() {
    let p = OpenAiCompatibleProvider::new_no_responses_fallback(
        "custom_openai",
        "http://localhost:11434/v1",
        Some("sk-test"),
        AuthStyle::Bearer,
    );
    let base = "custom_openai API error (404 Not Found): model not found".to_string();
    let result = p.enrich_404_message(base.clone(), reqwest::StatusCode::NOT_FOUND);
    assert!(
        result.starts_with(&base),
        "must preserve original error prefix: {result}"
    );
    assert!(
        result.contains("check that your endpoint URL is correct"),
        "must contain user-actionable hint: {result}"
    );

    // Non-404 status should NOT add the hint
    let result_200 = p.enrich_404_message(
        "custom_openai API error (503 Service Unavailable): overloaded".to_string(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
    );
    assert!(
        !result_200.contains("check that your endpoint URL"),
        "must not add hint for non-404: {result_200}"
    );

    // Provider with fallback enabled should NOT add the hint even on 404
    let p2 = OpenAiCompatibleProvider::new(
        "openai",
        "https://api.openai.com/v1",
        Some("sk-real"),
        AuthStyle::Bearer,
    );
    let result_with_fallback = p2.enrich_404_message(
        "openai API error (404 Not Found): model not found".to_string(),
        reqwest::StatusCode::NOT_FOUND,
    );
    assert_eq!(
        result_with_fallback, "openai API error (404 Not Found): model not found",
        "must not add hint when fallback is enabled: {result_with_fallback}"
    );
}

// ── reasoning_content round-trip tests (issue #2800 / Sentry TAURI-RUST-4WC) ─

/// `parse_native_response` must capture `reasoning_content` from a non-streaming
/// `ApiChatResponse` and surface it on `ChatResponse`.
#[test]
fn parse_native_response_captures_reasoning_content_from_api_response() {
    let api_resp = ApiChatResponse {
        choices: vec![Choice {
            message: ResponseMessage {
                content: Some("Here is my answer.".into()),
                reasoning_content: Some("I thought about it carefully.".into()),
                tool_calls: None,
                function_call: None,
            },
        }],
        usage: None,
        openhuman: None,
    };
    let result = OpenAiCompatibleProvider::parse_native_response(api_resp, "deepseek").unwrap();
    assert_eq!(
        result.reasoning_content.as_deref(),
        Some("I thought about it carefully."),
        "reasoning_content must be propagated to ChatResponse"
    );
    assert_eq!(result.text.as_deref(), Some("Here is my answer."));
}

/// When a response has no `reasoning_content`, `ChatResponse.reasoning_content`
/// must be `None` (no spurious field emitted on the next turn).
#[test]
fn parse_native_response_no_reasoning_content_stays_none() {
    let api_resp = ApiChatResponse {
        choices: vec![Choice {
            message: ResponseMessage {
                content: Some("Just a plain answer.".into()),
                reasoning_content: None,
                tool_calls: None,
                function_call: None,
            },
        }],
        usage: None,
        openhuman: None,
    };
    let result = OpenAiCompatibleProvider::parse_native_response(api_resp, "gpt-4o").unwrap();
    assert!(
        result.reasoning_content.is_none(),
        "reasoning_content must be None when the provider did not return it"
    );
}

/// `convert_messages_for_native` must echo `reasoning_content` back in the
/// `NativeMessage` for assistant turns that have it stored in `extra_metadata`.
/// This is the load-bearing contract: without it the API returns HTTP 400.
#[test]
fn convert_messages_for_native_echoes_reasoning_content_from_extra_metadata() {
    let mut assistant_msg = ChatMessage::assistant("Here is my answer.");
    assistant_msg.extra_metadata =
        Some(serde_json::json!({ "reasoning_content": "I thought carefully." }));

    let messages = vec![
        ChatMessage::user("What is 2+2?"),
        assistant_msg,
        ChatMessage::user("Are you sure?"),
    ];

    let native = OpenAiCompatibleProvider::convert_messages_for_native(&messages);

    // User messages must not carry reasoning_content.
    assert!(
        native[0].reasoning_content.is_none(),
        "user message must not have reasoning_content"
    );
    // The assistant message with extra_metadata must have reasoning_content echoed.
    assert_eq!(
        native[1].reasoning_content.as_deref(),
        Some("I thought carefully."),
        "assistant message must echo reasoning_content from extra_metadata"
    );
    // Second user message must not carry reasoning_content.
    assert!(
        native[2].reasoning_content.is_none(),
        "second user message must not have reasoning_content"
    );
}

/// Assistant messages without `extra_metadata` (or without a `reasoning_content`
/// key) must produce a `NativeMessage` with `reasoning_content = None` — the
/// `skip_serializing_if` attribute then omits the field from the JSON body so
/// standard providers don't reject the request.
#[test]
fn convert_messages_for_native_no_reasoning_content_stays_none() {
    let messages = vec![ChatMessage::user("hello"), ChatMessage::assistant("world")];

    let native = OpenAiCompatibleProvider::convert_messages_for_native(&messages);
    assert!(
        native[1].reasoning_content.is_none(),
        "assistant without extra_metadata must produce reasoning_content = None"
    );
}

/// The `reasoning_content` field must be omitted from the JSON serialized wire
/// payload when it is `None`, so standard providers that do not understand the
/// field are not broken.
#[test]
fn native_message_reasoning_content_omitted_when_none() {
    let msg = NativeMessage {
        role: "assistant".to_string(),
        content: Some("hello".into()),
        tool_call_id: None,
        tool_calls: None,
        reasoning_content: None,
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert!(
        json.get("reasoning_content").is_none(),
        "reasoning_content must be absent from the wire payload when None"
    );
}

/// When `reasoning_content` is present it must appear in the serialized payload
/// so thinking-model providers receive it.
#[test]
fn native_message_reasoning_content_present_when_some() {
    let msg = NativeMessage {
        role: "assistant".to_string(),
        content: Some("hello".into()),
        tool_call_id: None,
        tool_calls: None,
        reasoning_content: Some("I thought carefully.".to_string()),
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(
        json.get("reasoning_content").and_then(|v| v.as_str()),
        Some("I thought carefully."),
        "reasoning_content must be present in the wire payload when Some"
    );
}

// ── convert_tool_specs — TAURI-RUST-2E wire-boundary dedup ─────────────

fn spec(name: &str) -> crate::openhuman::tools::ToolSpec {
    crate::openhuman::tools::ToolSpec {
        name: name.to_string(),
        description: format!("{name} desc"),
        parameters: serde_json::json!({"type": "object"}),
    }
}

#[test]
fn convert_tool_specs_none_input_returns_none() {
    assert!(OpenAiCompatibleProvider::convert_tool_specs(None).is_none());
}

#[test]
fn convert_tool_specs_empty_slice_returns_empty_vec() {
    let out = OpenAiCompatibleProvider::convert_tool_specs(Some(&[])).unwrap();
    assert!(out.is_empty());
}

#[test]
fn convert_tool_specs_passes_through_unique_names() {
    let specs = vec![spec("alpha"), spec("beta"), spec("gamma")];
    let out = OpenAiCompatibleProvider::convert_tool_specs(Some(&specs)).unwrap();
    assert_eq!(out.len(), 3);
    let names: Vec<&str> = out
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
}

#[test]
fn convert_tool_specs_dedups_duplicate_names_first_wins() {
    // First occurrence of `alpha` (with description "alpha desc") survives;
    // the second is dropped wholesale even though its `parameters` differ.
    let mut second_alpha = spec("alpha");
    second_alpha.description = "should be dropped".to_string();
    second_alpha.parameters = serde_json::json!({"different": true});
    let specs = vec![spec("alpha"), spec("beta"), second_alpha, spec("gamma")];

    let out = OpenAiCompatibleProvider::convert_tool_specs(Some(&specs)).unwrap();
    assert_eq!(
        out.len(),
        3,
        "duplicate `alpha` must be dropped from wire payload"
    );
    let names: Vec<&str> = out
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    assert_eq!(
        out[0]["function"]["description"].as_str().unwrap(),
        "alpha desc",
        "first occurrence's description must survive (first-wins)"
    );
}

#[test]
fn convert_tool_specs_dedups_many_duplicates() {
    let specs = vec![
        spec("x"),
        spec("x"),
        spec("x"),
        spec("y"),
        spec("y"),
        spec("z"),
    ];
    let out = OpenAiCompatibleProvider::convert_tool_specs(Some(&specs)).unwrap();
    assert_eq!(out.len(), 3);
    let names: Vec<&str> = out
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["x", "y", "z"]);
}

// ── #3193: completion-only model 404 detection + actionable message ──────────

#[test]
fn completion_only_model_404_detected_from_openai_signature() {
    // The exact body OpenAI returns when a completion-only/base model is sent
    // to /v1/chat/completions.
    let body = "This is not a chat model and thus not supported in the \
                v1/chat/completions endpoint. Did you mean to use v1/completions?";
    assert!(OpenAiCompatibleProvider::is_completion_only_model_404(
        reqwest::StatusCode::NOT_FOUND,
        body
    ));
}

#[test]
fn completion_only_model_404_ignores_ordinary_not_found() {
    // A "model does not exist" 404 must NOT be misclassified — it should keep
    // its existing fallback / enrich behaviour, not get the completion-only
    // message.
    let body = "The model `gpt-9o` does not exist or you do not have access to it.";
    assert!(!OpenAiCompatibleProvider::is_completion_only_model_404(
        reqwest::StatusCode::NOT_FOUND,
        body
    ));
}

#[test]
fn completion_only_model_404_requires_404_status() {
    // Same phrasing under a non-404 status is not the completion-only case.
    let body = "not a chat model";
    assert!(!OpenAiCompatibleProvider::is_completion_only_model_404(
        reqwest::StatusCode::BAD_REQUEST,
        body
    ));
}

#[test]
fn completion_only_message_names_model_and_remediation() {
    let p = make_provider("openhuman", "https://api.example.com/v1", Some("k"));
    let msg = p.completion_only_model_message(
        "davinci-002",
        "This is not a chat model ... Did you mean to use v1/completions?",
    );
    assert!(
        msg.contains("davinci-002"),
        "names the offending model: {msg}"
    );
    assert!(
        msg.contains("completion-only") && msg.contains("chat-completions"),
        "explains the capability mismatch: {msg}"
    );
    assert!(
        msg.contains("chat-capable model"),
        "states the remediation: {msg}"
    );
}

#[test]
fn completion_only_404_guard_fires_only_on_signature() {
    let p = make_provider("openhuman", "https://api.example.com/v1", Some("k"));
    // Matches → Some(actionable error).
    let hit = p.completion_only_404_guard(
        reqwest::StatusCode::NOT_FOUND,
        "This is not a chat model. Did you mean to use v1/completions?",
        "davinci-002",
    );
    let err = hit.expect("guard should fire on the completion-only signature");
    assert!(err.to_string().contains("davinci-002"));
    // Ordinary not-found → None (normal fallback/enrich path is preserved).
    assert!(p
        .completion_only_404_guard(
            reqwest::StatusCode::NOT_FOUND,
            "The model `gpt-9o` does not exist.",
            "gpt-9o"
        )
        .is_none());
}

#[tokio::test]
async fn completion_only_404_fails_fast_without_responses_fallback() {
    // End-to-end over the wire: a completion-only 404 must short-circuit with
    // the actionable message and NOT attempt /v1/responses (not mounted here —
    // if the guard regressed, the error would instead read "responses fallback
    // failed"). Provider has the fallback ENABLED (default `new`), proving the
    // guard pre-empts it. #3193.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": {
                "message": "This is not a chat model and thus not supported in the \
                            v1/chat/completions endpoint. Did you mean to use v1/completions?",
                "type": "invalid_request_error"
            }
        })))
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(
        "openhuman",
        &format!("{}/v1", server.uri()),
        Some("key"),
        AuthStyle::Bearer,
    );

    let err = provider
        .chat_with_history(&[ChatMessage::user("write a file")], "davinci-002", 0.0)
        .await
        .expect_err("completion-only model must error");
    let msg = err.to_string();
    assert!(
        msg.contains("davinci-002") && msg.contains("chat-capable model"),
        "expected actionable completion-only message, got: {msg}"
    );
    assert!(
        !msg.contains("responses fallback failed"),
        "guard must pre-empt the responses fallback, got: {msg}"
    );
}

// ── TAURI-RUST-4P6: embedding model picked as chat model → 400 ───────────────

#[test]
fn not_chat_capable_detected_from_ollama_400() {
    // Verbatim Ollama wire body when an embedding model (bge-m3) is used as
    // the chat model. Sentry issue 5338.
    let body = r#"{"error":{"message":"\"bge-m3:latest\" does not support chat","type":"invalid_request_error","param":null,"code":null}}"#;
    assert!(OpenAiCompatibleProvider::is_not_chat_capable_model(
        reqwest::StatusCode::BAD_REQUEST,
        body
    ));
    // Some compatible backends use 422 for the same class.
    assert!(OpenAiCompatibleProvider::is_not_chat_capable_model(
        reqwest::StatusCode::UNPROCESSABLE_ENTITY,
        body
    ));
}

#[test]
fn not_chat_capable_requires_4xx_status() {
    // The exact phrase under a non-4xx status is not this case — let other
    // handling deal with 404/5xx so we don't shadow real failures.
    let body = "\"bge-m3:latest\" does not support chat";
    assert!(!OpenAiCompatibleProvider::is_not_chat_capable_model(
        reqwest::StatusCode::NOT_FOUND,
        body
    ));
    assert!(!OpenAiCompatibleProvider::is_not_chat_capable_model(
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        body
    ));
}

#[test]
fn not_chat_capable_ignores_unrelated_400() {
    // An ordinary 400 with no "does not support chat" phrase must keep its
    // normal enrich/handling path.
    assert!(!OpenAiCompatibleProvider::is_not_chat_capable_model(
        reqwest::StatusCode::BAD_REQUEST,
        "invalid temperature: only 1 is allowed for this model"
    ));
}

#[test]
fn not_chat_capable_message_names_model_remediation_and_keeps_phrase() {
    let p = make_provider("ollama", "http://127.0.0.1:11434/v1", None);
    let msg = p.not_chat_capable_model_message(
        "bge-m3:latest",
        r#"{"error":{"message":"\"bge-m3:latest\" does not support chat"}}"#,
    );
    assert!(msg.contains("bge-m3:latest"), "names the model: {msg}");
    assert!(
        msg.contains("chat-capable model"),
        "states the remediation: {msg}"
    );
    // CRITICAL: the actionable rewrite must still carry the upstream phrase so
    // the re-reported error stays demoted by the config-rejection classifier
    // (otherwise the 36.6k Sentry events come back). See TAURI-RUST-4P6.
    assert!(
        msg.to_lowercase().contains("does not support chat"),
        "must preserve the classifier anchor phrase: {msg}"
    );
    assert!(
        super::super::is_provider_config_rejection_message(&msg),
        "enriched message must classify as a provider config-rejection: {msg}"
    );
}

#[test]
fn not_chat_capable_guard_fires_only_on_signature() {
    let p = make_provider("ollama", "http://127.0.0.1:11434/v1", None);
    let hit = p.not_chat_capable_guard(
        reqwest::StatusCode::BAD_REQUEST,
        "\"bge-m3:latest\" does not support chat",
        "bge-m3:latest",
    );
    assert!(hit
        .expect("guard should fire on the does-not-support-chat 400")
        .to_string()
        .contains("bge-m3:latest"));
    // Unrelated 400 → None (normal handling preserved).
    assert!(p
        .not_chat_capable_guard(
            reqwest::StatusCode::BAD_REQUEST,
            "rate limit exceeded",
            "bge-m3:latest"
        )
        .is_none());
}

#[tokio::test]
async fn not_chat_capable_400_fails_fast_with_actionable_message() {
    // End-to-end over the wire: an embedding model used as chat 400s, and the
    // guard must short-circuit with the actionable message rather than the
    // opaque upstream JSON. TAURI-RUST-4P6.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {
                "message": "\"bge-m3:latest\" does not support chat",
                "type": "invalid_request_error",
                "param": null,
                "code": null
            }
        })))
        .mount(&server)
        .await;

    let provider = OpenAiCompatibleProvider::new(
        "ollama",
        &format!("{}/v1", server.uri()),
        // Dummy key only to clear the pre-flight credential gate; the mock
        // ignores auth. Real Ollama is keyless, but the provider requires a
        // non-empty credential before dispatching.
        Some("x"),
        AuthStyle::Bearer,
    );

    let err = provider
        .chat_with_history(&[ChatMessage::user("hello")], "bge-m3:latest", 0.0)
        .await
        .expect_err("embedding-model-as-chat must error");
    let msg = err.to_string();
    assert!(
        msg.contains("bge-m3:latest") && msg.contains("chat-capable model"),
        "expected actionable does-not-support-chat message, got: {msg}"
    );
    // And it must remain demotable — Sentry suppression depends on it.
    assert!(
        super::super::is_provider_config_rejection_message(&msg),
        "bubbled error must classify as config-rejection, got: {msg}"
    );
}

// ── #3205: multimodal [IMAGE:] markers → OpenAI image_url content parts ─────────

const TEST_PNG_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

/// Text with no markers stays the plain-string `content` arm — byte-identical
/// to the legacy wire shape so every non-attachment turn is unaffected.
#[test]
fn message_content_text_only_serializes_as_string() {
    let content = MessageContent::from_chat_text("just a normal message");
    let json = serde_json::to_value(&content).unwrap();
    assert_eq!(json, serde_json::json!("just a normal message"));
}

/// A user message carrying one `[IMAGE:data-uri]` marker is promoted to the
/// OpenAI `content` array: a `text` part followed by an `image_url` part.
#[test]
fn message_content_text_plus_image_serializes_as_parts() {
    let raw = format!("what is in this picture? [IMAGE:{TEST_PNG_DATA_URI}]");
    let json = serde_json::to_value(MessageContent::from_chat_text(&raw)).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            { "type": "text", "text": "what is in this picture?" },
            { "type": "image_url", "image_url": { "url": TEST_PNG_DATA_URI } },
        ])
    );
}

/// An image with no accompanying text emits only the `image_url` part.
#[test]
fn message_content_image_only_omits_empty_text_part() {
    let raw = format!("[IMAGE:{TEST_PNG_DATA_URI}]");
    let json = serde_json::to_value(MessageContent::from_chat_text(&raw)).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            { "type": "image_url", "image_url": { "url": TEST_PNG_DATA_URI } },
        ])
    );
}

/// Multiple markers become multiple `image_url` parts, with the text between
/// them preserved in authored order (not collapsed before the images).
#[test]
fn message_content_multiple_images_serialize_in_order() {
    let raw = format!("compare [IMAGE:{TEST_PNG_DATA_URI}] and [IMAGE:https://example.com/b.jpg]");
    let json = serde_json::to_value(MessageContent::from_chat_text(&raw)).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            { "type": "text", "text": "compare" },
            { "type": "image_url", "image_url": { "url": TEST_PNG_DATA_URI } },
            { "type": "text", "text": "and" },
            { "type": "image_url", "image_url": { "url": "https://example.com/b.jpg" } },
        ])
    );
}

/// Interleaved order is preserved exactly — an image-first prompt keeps the
/// image before the trailing text (CodeRabbit #3268).
#[test]
fn message_content_preserves_image_first_then_text_order() {
    let raw = format!("[IMAGE:{TEST_PNG_DATA_URI}] then explain");
    let json = serde_json::to_value(MessageContent::from_chat_text(&raw)).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            { "type": "image_url", "image_url": { "url": TEST_PNG_DATA_URI } },
            { "type": "text", "text": "then explain" },
        ])
    );
}

/// Request-level: a chat history with an image-bearing user turn serialises the
/// full body with a string `system` content and an array `user` content.
#[test]
fn api_chat_request_mixes_string_and_array_content() {
    let req = ApiChatRequest {
        model: "gpt-4o".to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: "You are helpful.".into(),
            },
            Message {
                role: "user".to_string(),
                content: MessageContent::from_chat_text(&format!(
                    "describe this [IMAGE:{TEST_PNG_DATA_URI}]"
                )),
            },
        ],
        temperature: None,
        stream: None,
        tools: None,
        tool_choice: None,
    };
    let json = serde_json::to_value(&req).unwrap();
    assert_eq!(
        json["messages"][0]["content"],
        serde_json::json!("You are helpful.")
    );
    assert_eq!(
        json["messages"][1]["content"],
        serde_json::json!([
            { "type": "text", "text": "describe this" },
            { "type": "image_url", "image_url": { "url": TEST_PNG_DATA_URI } },
        ])
    );
}

/// The agent streaming path runs history through `convert_messages_for_native`;
/// an image marker must survive into the `NativeMessage` array content while
/// plain turns stay strings.
#[test]
fn convert_messages_for_native_promotes_image_marker() {
    let history = vec![
        ChatMessage::system("be brief"),
        ChatMessage::user(&format!("look [IMAGE:{TEST_PNG_DATA_URI}]")),
    ];
    let native = OpenAiCompatibleProvider::convert_messages_for_native(&history);
    assert_eq!(
        serde_json::to_value(&native[0]).unwrap()["content"],
        serde_json::json!("be brief")
    );
    assert_eq!(
        serde_json::to_value(&native[1]).unwrap()["content"],
        serde_json::json!([
            { "type": "text", "text": "look" },
            { "type": "image_url", "image_url": { "url": TEST_PNG_DATA_URI } },
        ])
    );
}

#[test]
fn stream_repeat_detector_trips_at_threshold() {
    let mut d = StreamRepeatDetector::new();
    // The degenerate pattern: one substantial sentence emitted with blank
    // separators, over and over (exactly what we observed, 234×).
    let chunk =
        "Now I have a complete understanding. Let me also check the llm.rs extraction logic.\n\n";
    let mut tripped_at = 0;
    for i in 1..=(STREAM_REPEAT_THRESHOLD + 3) {
        if d.observe(chunk) {
            tripped_at = i;
            break;
        }
    }
    assert_eq!(
        tripped_at, STREAM_REPEAT_THRESHOLD,
        "should trip exactly at the threshold, ignoring blank separators"
    );
}

#[test]
fn stream_repeat_detector_ignores_varied_and_short_lines() {
    let mut d = StreamRepeatDetector::new();
    // Distinct substantial lines never trip (real, progressing output).
    for i in 0..20 {
        assert!(
            !d.observe(&format!(
                "This is distinct analysis step number {i} of the task.\n"
            )),
            "varied lines must not trip"
        );
    }
    // Short identical lines (e.g. code braces) are below the min length → no trip.
    for _ in 0..20 {
        assert!(!d.observe("}\n"), "short repeated lines must not trip");
    }
}

// ── effective_context_window (#3550 / Sentry TAURI-RUST-6V0) ───────────────

#[tokio::test]
async fn effective_context_window_cloud_uses_static_table() {
    // No local provider kind → static trained-max table, unchanged behavior.
    let p = make_provider("openai", "https://api.openai.com/v1", Some("k"));
    assert_eq!(p.effective_context_window("gpt-4o").await, Some(128_000));
    // Unknown cloud model → None (skip trimming), as before.
    assert_eq!(p.effective_context_window("totally-unknown").await, None);
}

#[tokio::test]
async fn effective_context_window_lmstudio_uses_loaded_window() {
    use crate::openhuman::inference::local::profile::LocalProviderKind;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"data":[{"id":"qwen2.5-7b","loaded_context_length":4096,"max_context_length":32768}]}"#,
            "application/json",
        ))
        .mount(&server)
        .await;
    let p = OpenAiCompatibleProvider::new("lmstudio", &server.uri(), None, AuthStyle::None)
        .with_local_provider_kind(LocalProviderKind::LmStudio);
    // Trim to the runtime-loaded n_ctx (4096), NOT the model's trained max.
    assert_eq!(p.effective_context_window("qwen2.5-7b").await, Some(4096));
}

#[tokio::test]
async fn effective_context_window_lmstudio_falls_back_when_native_unavailable() {
    use crate::openhuman::inference::local::profile::LocalProviderKind;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let p = OpenAiCompatibleProvider::new("lmstudio", &server.uri(), None, AuthStyle::None)
        .with_local_provider_kind(LocalProviderKind::LmStudio);
    // Native probe fails → fall back to the LM Studio profile default (8192),
    // so unknown local models still get trimmed instead of skipped.
    assert_eq!(
        p.effective_context_window("unknown-local-model").await,
        Some(8_192)
    );
}

/// Verbatim TAURI-RUST-4QF DeepSeek 402 body. The demote arms key on the
/// "insufficient balance" phrase via `body_indicates_insufficient_credits`, so
/// pinning the exact wire shape makes a provider wording drift fail CI rather
/// than silently re-flood Sentry (23,970 events from 17 BYO users).
const DEEPSEEK_4QF_402_BODY: &str = r#"{"error":{"message":"Insufficient Balance","type":"unknown_error","param":null,"code":"invalid_request_error"}}"#;

/// Build a Sentry hub backed by a `TestTransport` and install it for the
/// current scope, returning the transport so the test can assert no events
/// were captured. Mirrors the inline setup used by the other
/// `*_not_reported_to_sentry` guards above.
fn install_test_sentry_hub() -> (Arc<TestTransport>, sentry::HubSwitchGuard) {
    let transport = TestTransport::new();
    let options = sentry::ClientOptions {
        dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
        transport: Some(Arc::new(transport.clone())),
        ..Default::default()
    };
    let hub = Arc::new(sentry::Hub::new(
        Some(Arc::new(options.into())),
        Arc::new(Default::default()),
    ));
    let guard = sentry::HubSwitchGuard::new(hub);
    (transport, guard)
}

#[tokio::test]
async fn chat_with_system_insufficient_balance_402_not_reported_to_sentry() {
    // TAURI-RUST-4QF: a BYO DeepSeek account out of balance 402s on every agent
    // turn. The non-streaming `chat_with_system` path (operation=chat_completions
    // in the Sentry tags) must still propagate the error, but must NOT page
    // Sentry — the user's own provider balance is exhausted, no local lever.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(402).set_body_string(DEEPSEEK_4QF_402_BODY))
        .mount(&mock_server)
        .await;

    let (transport, _guard) = install_test_sentry_hub();

    let provider = make_provider("deepseek", &mock_server.uri(), Some("byo-key"));
    let err = provider
        .chat_with_system(None, "hello", "deepseek-v4-flash", 0.7)
        .await
        .expect_err("a 402 insufficient-balance must still propagate as Err");
    assert!(err.to_string().contains("402"), "err: {err}");
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "an exhausted BYO balance 402 must not be reported to Sentry"
    );
}

#[tokio::test]
async fn chat_with_history_insufficient_balance_402_not_reported_to_sentry() {
    // Same 402 user-state reached through `chat_with_history`, which routes its
    // non-2xx through the shared `api_error` helper. Guards the api_error arm.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(402).set_body_string(DEEPSEEK_4QF_402_BODY))
        .mount(&mock_server)
        .await;

    let (transport, _guard) = install_test_sentry_hub();

    let provider = make_provider("deepseek", &mock_server.uri(), Some("byo-key"));
    let err = provider
        .chat_with_history(&[ChatMessage::user("hello")], "deepseek-v4-flash", 0.0)
        .await
        .expect_err("a 402 insufficient-balance must still propagate as Err");
    assert!(err.to_string().contains("402"), "err: {err}");
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "an exhausted BYO balance 402 (api_error path) must not be reported to Sentry"
    );
}

#[tokio::test]
async fn streaming_chat_insufficient_balance_402_not_reported_to_sentry() {
    // The streaming entry (`stream_native_chat`, operation=streaming_chat) has
    // its own non-2xx cascade. A 402 insufficient-balance there must demote too.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(402).set_body_string(DEEPSEEK_4QF_402_BODY))
        .mount(&mock_server)
        .await;

    let (transport, _guard) = install_test_sentry_hub();

    let provider =
        OpenAiCompatibleProvider::new("deepseek", &mock_server.uri(), None, AuthStyle::None);
    let request = NativeChatRequest {
        model: "deepseek-v4-flash".to_string(),
        messages: vec![NativeMessage {
            role: "user".to_string(),
            content: Some("hello".into()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }],
        temperature: Some(0.7),
        stream: Some(true),
        tools: None,
        tool_choice: None,
        thread_id: None,
        stream_options: Some(super::compatible_types::OpenAiStreamOptions {
            include_usage: true,
        }),
        options: None,
        frequency_penalty: None,
        max_tokens: None,
    };
    let (delta_tx, _delta_rx) = tokio::sync::mpsc::channel(8);

    let err = provider
        .stream_native_chat(None, &request, &delta_tx, 0)
        .await
        .expect_err("a 402 insufficient-balance must still propagate as Err");
    assert!(
        err.to_string().contains("streaming API error"),
        "err: {err}"
    );
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "an exhausted BYO balance 402 (streaming path) must not be reported to Sentry"
    );
}

#[tokio::test]
async fn responses_api_insufficient_balance_402_not_reported_to_sentry() {
    // The responses-API path (`chat_via_responses`, operation=responses_api) is
    // reached when the provider is configured responses-primary. A 402
    // insufficient-balance there must demote, not page Sentry. This path is
    // awaited inline, so the TestTransport assertion is authoritative.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(402).set_body_string(DEEPSEEK_4QF_402_BODY))
        .mount(&mock_server)
        .await;

    let (transport, _guard) = install_test_sentry_hub();

    let provider = OpenAiCompatibleProvider::new(
        "deepseek",
        &mock_server.uri(),
        Some("byo-key"),
        AuthStyle::Bearer,
    )
    .with_responses_api_primary();
    let err = provider
        .chat_with_history(&[ChatMessage::user("hello")], "deepseek-v4-flash", 0.0)
        .await
        .expect_err("a 402 insufficient-balance must still propagate as Err");
    // The responses-API error message is `<provider> Responses API error: <body>`
    // — note it carries NO "(402" status token, so the message-keyed before_send
    // net (#3913, `is_insufficient_credits_message`) would MISS it. Only the
    // status-based source arm here catches it, which the empty-transport
    // assertion below proves.
    assert!(
        err.to_string().contains("Insufficient Balance"),
        "err: {err}"
    );
    assert!(
        transport.fetch_and_clear_events().is_empty(),
        "an exhausted BYO balance 402 (responses_api path) must not be reported to Sentry"
    );
}

#[tokio::test]
async fn stream_chat_with_system_insufficient_balance_402_propagates_error() {
    // Exercises the `stream_chat_with_system` (operation=stream_chat) 402
    // insufficient-balance demote arm. The streaming work runs in a detached
    // tokio task whose Sentry hub is NOT the test hub, so a TestTransport
    // assertion would be vacuous here; instead assert the error still
    // propagates as a terminal Provider chunk (the demote arm and the report
    // fallback bail the same message, so this guards line execution + error
    // shape). The Sentry-suppression behavior for this exact body is proven by
    // the inline-awaited native/api tests above and the
    // `detects_insufficient_balance_402_family` predicate test.
    use crate::openhuman::inference::provider::traits::{StreamError, StreamOptions};
    use futures_util::StreamExt;

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(402).set_body_string(DEEPSEEK_4QF_402_BODY))
        .mount(&mock_server)
        .await;

    let provider =
        OpenAiCompatibleProvider::new("deepseek", &mock_server.uri(), None, AuthStyle::None);
    let mut stream = provider.stream_chat_with_system(
        None,
        "hello",
        "deepseek-v4-flash",
        0.7,
        StreamOptions::new(true),
    );
    let mut terminal: Option<String> = None;
    while let Some(item) = stream.next().await {
        if let Err(StreamError::Provider(msg)) = item {
            terminal = Some(msg);
        }
    }
    let msg = terminal.expect("a 402 must yield a terminal Provider error chunk");
    assert!(
        msg.contains("402") && msg.contains("Insufficient Balance"),
        "msg: {msg}"
    );
}

#[tokio::test]
async fn stream_chat_with_history_insufficient_balance_402_propagates_error() {
    // Sibling of the above for `stream_chat_with_history`
    // (operation=stream_chat_history). Same detached-task caveat — asserts the
    // 402 path executes and propagates as a terminal Provider error chunk.
    use crate::openhuman::inference::provider::traits::{StreamError, StreamOptions};
    use futures_util::StreamExt;

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(402).set_body_string(DEEPSEEK_4QF_402_BODY))
        .mount(&mock_server)
        .await;

    let provider =
        OpenAiCompatibleProvider::new("deepseek", &mock_server.uri(), None, AuthStyle::None);
    let mut stream = provider.stream_chat_with_history(
        &[ChatMessage::user("hello")],
        "deepseek-v4-flash",
        0.7,
        StreamOptions::new(true),
    );
    let mut terminal: Option<String> = None;
    while let Some(item) = stream.next().await {
        if let Err(StreamError::Provider(msg)) = item {
            terminal = Some(msg);
        }
    }
    let msg = terminal.expect("a 402 must yield a terminal Provider error chunk");
    assert!(
        msg.contains("402") && msg.contains("Insufficient Balance"),
        "msg: {msg}"
    );
}

// ----------------------------------------------------------
// Prompt-cache capability model (#3939)
// ----------------------------------------------------------

#[test]
fn prompt_cache_caps_openai_style_for_known_slugs() {
    for slug in ["openai", "openrouter", "gmi"] {
        let caps = super::prompt_cache_for_compatible_slug(slug);
        assert!(
            caps.automatic_prefix_cache,
            "{slug} should advertise automatic prefix cache"
        );
        assert!(
            caps.usage_reports_cached_input,
            "{slug} should report cached input tokens"
        );
        assert!(
            !caps.explicit_cache_control,
            "OpenAI-compatible chat API has no cache-control field"
        );
        assert!(
            !caps.cache_key_grouping,
            "thread/session grouping is OpenHuman-backend-only"
        );
    }
}

#[test]
fn prompt_cache_caps_match_slug_family_variants() {
    // Case-insensitive, leading-segment family match so renamed/suffixed slugs
    // still resolve to the verified family.
    for slug in ["OpenAI", "openai:gpt-5.1", "openai/responses", "openai-eu"] {
        let caps = super::prompt_cache_for_compatible_slug(slug);
        assert!(
            caps.automatic_prefix_cache && caps.usage_reports_cached_input,
            "{slug} should resolve to the openai family"
        );
    }
}

#[test]
fn prompt_cache_caps_conservative_for_unknown_or_custom_slugs() {
    // Custom / local / unverified providers must not advertise caching — they
    // get the all-false default so we never send or assume unsupported behaviour.
    let conservative =
        crate::openhuman::inference::provider::traits::PromptCacheCapabilities::default();
    for slug in ["custom_openai", "lmstudio", "deepseek", "mystery-proxy", ""] {
        assert_eq!(
            super::prompt_cache_for_compatible_slug(slug),
            conservative,
            "{slug} must stay conservative"
        );
    }
}

#[test]
fn compatible_provider_declares_prompt_cache_from_its_slug() {
    let conservative =
        crate::openhuman::inference::provider::traits::PromptCacheCapabilities::default();

    let openai = make_provider("openai", "https://api.openai.com", Some("k"));
    let caps = openai.prompt_cache_capabilities();
    assert!(
        caps.automatic_prefix_cache && caps.usage_reports_cached_input,
        "openai provider must advertise OpenAI-style caching"
    );

    let custom = make_provider("custom_openai", "https://proxy.example", Some("k"));
    assert_eq!(
        custom.prompt_cache_capabilities(),
        conservative,
        "unknown custom provider must stay conservative"
    );
}

#[test]
fn extract_usage_normalizes_openai_cached_prompt_tokens() {
    // Regression: an OpenAI-compatible usage block carrying cached prefix tokens
    // (`prompt_tokens_details.cached_tokens`) must normalize into
    // `UsageInfo.cached_input_tokens` so cached-prefix cost accounting is exact.
    let json = r#"{
        "choices":[{"message":{"role":"assistant","content":"hi"}}],
        "usage":{"prompt_tokens":1000,"completion_tokens":20,"total_tokens":1020,
                 "prompt_tokens_details":{"cached_tokens":768}}
    }"#;
    let resp: ApiChatResponse = serde_json::from_str(json).expect("parse api response");
    let usage = OpenAiCompatibleProvider::extract_usage(&resp).expect("usage present");
    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 20);
    assert_eq!(
        usage.cached_input_tokens, 768,
        "cached prefix tokens must be normalized into cached_input_tokens"
    );
}

#[test]
fn extract_usage_defaults_cached_tokens_to_zero_when_absent() {
    // A provider that omits cache details must yield cached_input_tokens = 0,
    // keeping cost accounting coherent (full prompt charged at the input rate).
    let json = r#"{
        "choices":[{"message":{"role":"assistant","content":"hi"}}],
        "usage":{"prompt_tokens":500,"completion_tokens":10,"total_tokens":510}
    }"#;
    let resp: ApiChatResponse = serde_json::from_str(json).expect("parse api response");
    let usage = OpenAiCompatibleProvider::extract_usage(&resp).expect("usage present");
    assert_eq!(usage.cached_input_tokens, 0);
    assert_eq!(usage.input_tokens, 500);
}
