use super::*;
use sentry::test::TestTransport;
use std::sync::Arc;
use wiremock::matchers::{method, path};
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
    };
    let json_no_thread = serde_json::to_value(&req_no_thread).unwrap();
    assert!(
        json_no_thread.get("thread_id").is_none(),
        "absent thread_id must not be serialized so non-OpenHuman backends don't reject the field"
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
    };
    let json = serde_json::to_value(&req).unwrap();
    assert!(
        json.get("stream_options").is_none(),
        "non-streaming requests must not emit stream_options (OpenAI rejects it on stream=false)"
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
                content: "You are OpenHuman".to_string(),
            },
            Message {
                role: "user".to_string(),
                content: "hello".to_string(),
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
    assert_eq!(input[0].content, "step 1");
    assert_eq!(input[1].role, "assistant");
    assert_eq!(input[1].content, "ack 1");
    assert_eq!(input[2].role, "assistant");
    assert_eq!(input[2].content, "{\"result\":\"ok\"}");
    assert_eq!(input[3].role, "user");
    assert_eq!(input[3].content, "step 2");
}

#[tokio::test]
async fn chat_via_responses_requires_non_system_message() {
    let provider = make_provider("custom", "https://api.example.com", Some("test-key"));
    let err = provider
        .chat_via_responses(
            Some("test-key"),
            &[ChatMessage::system("policy")],
            "gpt-test",
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
            content: Some("hello".to_string()),
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
    assert_eq!(converted[1].content.as_deref(), Some("done"));
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
    assert_eq!(converted[0].content.as_deref(), Some("on it"));
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
    let input = vec![ChatMessage::assistant(
        r#"{"content":null,"reasoning_content":"let me think about this","tool_calls":[{"id":"call_x","name":"shell","arguments":"{}"}]}"#,
    )];

    let converted = OpenAiCompatibleProvider::convert_messages_for_native(&input);

    assert_eq!(converted.len(), 1);
    assert_eq!(
        converted[0].reasoning_content.as_deref(),
        Some("let me think about this")
    );

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
            content: "What is the weather?".to_string(),
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
        content: Some("hello".to_string()),
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
        content: Some("hello".to_string()),
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
