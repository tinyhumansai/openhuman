use crate::agent::goose::qwen::{
    build_protocol_correction_prompt, is_local_qwen_route, normalize_qwen_response,
    protocol_failure_terminal_answer,
};
use tinyinference::{
    message::{AssistantMessage, ContentBlock},
    model::{ModelResolutionSource, ModelResponse, ResolvedModel},
    tool::{ToolCall, ToolSchema},
};

fn search_schema() -> ToolSchema {
    ToolSchema::new(
        "search",
        "Search the web or documentation",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        }),
    )
}

fn destructive_schema() -> ToolSchema {
    ToolSchema::new(
        "write_file",
        "Overwrites destination file with provided content",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        }),
    )
}

fn make_response(content: Vec<ContentBlock>, tool_calls: Vec<ToolCall>) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: Some("assistant-msg".into()),
            content,
            tool_calls,
            usage: None,
        },
        usage: None,
        finish_reason: Some("stop".into()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    }
}

#[test]
fn test_result_exit_final_boundaries_prevent_later_calls() {
    let boundary_cases = [
        "<tool_result>result body</tool_result>",
        "<tool_results>results</tool_results>",
        "<tool_response>resp</tool_response>",
        "<result>res</result>",
        "<exit>",
        "<exit/>",
        "<final>",
        "<final/>",
        "<final_answer>",
        "<final_answer/>",
        "<|im_end|>",
    ];

    let tools = vec![search_schema()];

    for boundary in boundary_cases {
        let text = format!(
            "Some prose {boundary} later text <tool_call>{{\"name\":\"search\",\"arguments\":{{\"query\":\"boundary blocked\"}}}}</tool_call>"
        );
        let response = make_response(vec![ContentBlock::Text(text)], vec![]);
        let normalized = normalize_qwen_response(response, &tools);

        assert!(
            normalized.message.tool_calls.is_empty(),
            "Tool call after boundary '{boundary}' must not survive"
        );
    }
}

#[test]
fn test_recovered_ids_stable_and_native_id_survives_exactly() {
    let native_id = "provider_generated_id_abcdef";
    let native_call = ToolCall::new(
        native_id,
        "search",
        serde_json::json!({ "query": "native call" }),
    );
    let response_native = make_response(vec![], vec![native_call]);
    let tools = vec![search_schema()];

    let normalized_native = normalize_qwen_response(response_native, &tools);
    assert_eq!(normalized_native.message.tool_calls.len(), 1);
    assert_eq!(normalized_native.message.tool_calls[0].id, native_id);

    let tagged_text =
        "<tool_call>{\"name\":\"search\",\"arguments\":{\"query\":\"tagged call\"}}</tool_call>";
    let response_tagged_1 =
        make_response(vec![ContentBlock::Text(tagged_text.to_string())], vec![]);
    let normalized_tagged_1 = normalize_qwen_response(response_tagged_1, &tools);
    assert_eq!(normalized_tagged_1.message.tool_calls.len(), 1);
    assert_eq!(normalized_tagged_1.message.tool_calls[0].id, "call_0");

    let response_tagged_2 =
        make_response(vec![ContentBlock::Text(tagged_text.to_string())], vec![]);
    let normalized_tagged_2 = normalize_qwen_response(response_tagged_2, &tools);
    assert_eq!(normalized_tagged_2.message.tool_calls[0].id, "call_0");
}

#[test]
fn test_destructive_schema_omitted_fields_fail_closed_and_no_call_survives() {
    let tools = vec![destructive_schema()];

    // 1. Tagged call omitting 'path'
    let omit_path =
        "<tool_call>{\"name\":\"write_file\",\"arguments\":{\"content\":\"hello\"}}</tool_call>";
    let resp = make_response(vec![ContentBlock::Text(omit_path.to_string())], vec![]);
    let norm = normalize_qwen_response(resp, &tools);
    assert!(norm.message.tool_calls.is_empty());
    let inv = norm.invalid_call.as_ref().unwrap();
    assert!(inv.is_schema_mismatch());
    assert_eq!(inv.tool_name(), Some("write_file"));

    // 2. Tagged call omitting 'content'
    let omit_content = "<tool_call>{\"name\":\"write_file\",\"arguments\":{\"path\":\"/tmp/file.txt\"}}</tool_call>";
    let resp = make_response(vec![ContentBlock::Text(omit_content.to_string())], vec![]);
    let norm = normalize_qwen_response(resp, &tools);
    assert!(norm.message.tool_calls.is_empty());
    let inv = norm.invalid_call.as_ref().unwrap();
    assert!(inv.is_schema_mismatch());
    assert_eq!(inv.tool_name(), Some("write_file"));

    // 3. Tagged call omitting both fields
    let omit_both = "<tool_call>{\"name\":\"write_file\",\"arguments\":{}}</tool_call>";
    let resp = make_response(vec![ContentBlock::Text(omit_both.to_string())], vec![]);
    let norm = normalize_qwen_response(resp, &tools);
    assert!(norm.message.tool_calls.is_empty());
    let inv = norm.invalid_call.as_ref().unwrap();
    assert!(inv.is_schema_mismatch());

    // 4. Tagged call with explicit null for 'path'
    let null_path = "<tool_call>{\"name\":\"write_file\",\"arguments\":{\"path\":null,\"content\":\"data\"}}</tool_call>";
    let resp = make_response(vec![ContentBlock::Text(null_path.to_string())], vec![]);
    let norm = normalize_qwen_response(resp, &tools);
    assert!(norm.message.tool_calls.is_empty());
    let inv = norm.invalid_call.as_ref().unwrap();
    assert!(inv.is_schema_mismatch());

    // 5. Tagged call with wrong type for 'path'
    let wrong_type_path = "<tool_call>{\"name\":\"write_file\",\"arguments\":{\"path\":12345,\"content\":\"data\"}}</tool_call>";
    let resp = make_response(
        vec![ContentBlock::Text(wrong_type_path.to_string())],
        vec![],
    );
    let norm = normalize_qwen_response(resp, &tools);
    assert!(norm.message.tool_calls.is_empty());
    let inv = norm.invalid_call.as_ref().unwrap();
    assert!(inv.is_schema_mismatch());

    // 6. Native call omitting 'path'
    let native_omit_path = ToolCall::new(
        "native_destr_1",
        "write_file",
        serde_json::json!({ "content": "hello" }),
    );
    let resp = make_response(vec![], vec![native_omit_path]);
    let norm = normalize_qwen_response(resp, &tools);
    assert!(norm.message.tool_calls.is_empty());
    let inv = norm.invalid_call.as_ref().unwrap();
    assert!(inv.is_schema_mismatch());

    // 7. Native call omitting 'content'
    let native_omit_content = ToolCall::new(
        "native_destr_2",
        "write_file",
        serde_json::json!({ "path": "/etc/passwd" }),
    );
    let resp = make_response(vec![], vec![native_omit_content]);
    let norm = normalize_qwen_response(resp, &tools);
    assert!(norm.message.tool_calls.is_empty());
    let inv = norm.invalid_call.as_ref().unwrap();
    assert!(inv.is_schema_mismatch());

    // 8. Valid call with both required fields survives
    let valid_call = "<tool_call>{\"name\":\"write_file\",\"arguments\":{\"path\":\"safe.txt\",\"content\":\"all good\"}}</tool_call>";
    let resp = make_response(vec![ContentBlock::Text(valid_call.to_string())], vec![]);
    let norm = normalize_qwen_response(resp, &tools);
    assert_eq!(norm.message.tool_calls.len(), 1);
    assert_eq!(norm.message.tool_calls[0].name, "write_file");
    assert_eq!(
        norm.message.tool_calls[0].arguments,
        serde_json::json!({
            "path": "safe.txt",
            "content": "all good"
        })
    );
    assert!(norm.invalid_call.is_none());
}

#[test]
fn test_response_metadata_and_usage_preserved() {
    let mut response = make_response(
        vec![ContentBlock::Text("Finished task.".to_string())],
        vec![],
    );
    response.message.id = Some("assistant-msg-custom-id-789".to_string());
    response.resolved_model = Some(ResolvedModel {
        name: "qwen-2.5-coder-32b-instruct".to_string(),
        requested: None,
        source: ModelResolutionSource::RegistryDefault,
    });
    response.raw = Some(serde_json::json!({
        "provider": "vllm",
        "custom_metric": 42
    }));
    response.finish_reason = Some("stop".to_string());

    let tools = vec![search_schema()];
    let normalized = normalize_qwen_response(response.clone(), &tools);

    assert_eq!(normalized.message.id, response.message.id);
    assert_eq!(normalized.resolved_model, response.resolved_model);
    assert_eq!(normalized.raw, response.raw);
    assert_eq!(normalized.continue_turn, response.continue_turn);
    assert_eq!(normalized.served_from_cache, response.served_from_cache);
    assert_eq!(normalized.usage, response.usage);
    assert_eq!(normalized.message.usage, response.message.usage);
    assert_eq!(normalized.finish_reason, Some("stop".to_string()));

    // When tool calls are present, finish_reason updates to tool_calls
    let call = ToolCall::new(
        "call_1",
        "search",
        serde_json::json!({ "query": "test query" }),
    );
    let response_with_call = make_response(vec![], vec![call]);
    let normalized_call = normalize_qwen_response(response_with_call, &tools);
    assert_eq!(
        normalized_call.finish_reason,
        Some("tool_calls".to_string())
    );
}

#[test]
fn test_is_local_qwen_route_exact_case_sensitive_boundary() {
    // True only for exact case-sensitive "lmstudio" + "qwen38-openhuman"
    assert!(is_local_qwen_route("lmstudio", "qwen38-openhuman"));

    // Case variations must be false
    assert!(!is_local_qwen_route("LMStudio", "qwen38-openhuman"));
    assert!(!is_local_qwen_route("Lmstudio", "qwen38-openhuman"));
    assert!(!is_local_qwen_route("lmstudio", "Qwen38-openhuman"));
    assert!(!is_local_qwen_route("lmstudio", "QWEN38-OPENHUMAN"));
    assert!(!is_local_qwen_route("LMSTUDIO", "qwen38-openhuman"));
    assert!(!is_local_qwen_route("LMSTUDIO", "QWEN38-OPENHUMAN"));

    // Near/similar pairs must be false
    assert!(!is_local_qwen_route("lm_studio", "qwen38-openhuman"));
    assert!(!is_local_qwen_route("lmstudio", "qwen38_openhuman"));
    assert!(!is_local_qwen_route("lmstudio", "qwen38-openhuman-v2"));
    assert!(!is_local_qwen_route("lmstudio", "qwen-38-openhuman"));
    assert!(!is_local_qwen_route("lmstudio", "qwen38"));
    assert!(!is_local_qwen_route("lmstudio", "qwen-2.5-coder-32b"));
    assert!(!is_local_qwen_route("lmstudio ", "qwen38-openhuman"));
    assert!(!is_local_qwen_route("lmstudio", " qwen38-openhuman"));
    assert!(!is_local_qwen_route("lmstudio", "qwen38-openhuman "));

    // Other providers/models must be false
    assert!(!is_local_qwen_route("ollama", "qwen38-openhuman"));
    assert!(!is_local_qwen_route("openai", "gpt-4o"));
    assert!(!is_local_qwen_route("anthropic", "claude-3-5-sonnet"));
    assert!(!is_local_qwen_route("", ""));
    assert!(!is_local_qwen_route("lmstudio", ""));
    assert!(!is_local_qwen_route("", "qwen38-openhuman"));
}

#[test]
fn test_build_protocol_correction_prompt_boundary_and_payload_isolation() {
    let tools = vec![search_schema(), destructive_schema()];
    let prompt = build_protocol_correction_prompt(&tools);

    // Contains each advertised exact tool name
    assert!(prompt.contains("\"search\""));
    assert!(prompt.contains("\"write_file\""));

    // Contains schema requirements for each tool
    assert!(prompt.contains("\"query\""));
    assert!(prompt.contains("\"path\""));
    assert!(prompt.contains("\"content\""));
    assert!(prompt.contains("\"required\""));

    // Contains canonical single-call name/arguments rule
    assert!(prompt.contains(
        "exactly one canonical tool-call object containing exact \"name\" and object \"arguments\""
    ));
    assert!(prompt.contains("{\"name\": \"<exact_tool_name>\", \"arguments\": { ... }}"));

    // Contains prohibition on invented missing values
    assert!(prompt.contains("Do not invent missing tool names, arguments, or required values."));

    // Cannot contain an arbitrary malformed/raw payload because none is accepted
    let arbitrary_malformed_payloads = [
        "{\"bad\": \"syntax error\"",
        "<tool_call>{\"name\":\"arbitrary_injected\"}</tool_call>",
        "rm -rf /",
        "drop table users;",
        "malformed_payload_marker_12345",
    ];
    for payload in arbitrary_malformed_payloads {
        assert!(
            !prompt.contains(payload),
            "Prompt must not contain arbitrary payload '{payload}'"
        );
    }

    // When empty, instructs direct final text without tool listings
    let empty_prompt = build_protocol_correction_prompt(&[]);
    assert!(empty_prompt
        .contains("No tools are currently available. Please provide direct final text."));
    assert!(!empty_prompt.contains("\"search\""));
    assert!(!empty_prompt.contains("\"write_file\""));
}

#[test]
fn test_protocol_failure_terminal_answer_safety_and_clarity() {
    let answer = protocol_failure_terminal_answer();

    // Must be clear and non-empty
    assert!(!answer.trim().is_empty());

    // Must mention no action taken
    assert!(answer.contains("no action"));

    // Must contain none of '<', '>', or 'tool_call'
    assert!(!answer.contains('<'));
    assert!(!answer.contains('>'));
    assert!(!answer.to_lowercase().contains("tool_call"));

    // Must contain no invalid or malformed payload markers
    assert!(!answer.contains('{'));
    assert!(!answer.contains('}'));
    assert!(!answer.to_lowercase().contains("payload"));
    assert!(!answer.to_lowercase().contains("malformed"));
    assert!(!answer.to_lowercase().contains("syntax"));
}
