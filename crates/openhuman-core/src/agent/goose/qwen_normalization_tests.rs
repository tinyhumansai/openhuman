use crate::agent::goose::qwen::{normalize_qwen_response, QwenInvalidCall};
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

fn fetch_schema() -> ToolSchema {
    ToolSchema::new(
        "fetch",
        "Fetch URL content",
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" }
            },
            "required": ["url"]
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

fn extract_visible_text(response: &ModelResponse) -> String {
    response
        .message
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn extract_thinking_texts(response: &ModelResponse) -> Vec<String> {
    response
        .message
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Thinking { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn test_ordinary_final_text_unchanged() {
    let input_text = "I have completed the analysis. The answer is 42.";
    let response = make_response(vec![ContentBlock::Text(input_text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.invalid_call.is_none());
    assert!(normalized.message.tool_calls.is_empty());
    assert_eq!(extract_visible_text(&normalized), input_text);
    assert_eq!(normalized.finish_reason, Some("stop".to_string()));
    assert!(extract_thinking_texts(&normalized).is_empty());
}

#[test]
fn test_native_structured_call_precedence_and_preservation() {
    let native_id = "chatcmpl-call-native-999";
    let native_call = ToolCall::new(
        native_id,
        "search",
        serde_json::json!({ "query": "rust async executor" }),
    );
    let conflicting_text =
        "Searching now.\n<tool_call>{\"name\":\"fetch\",\"arguments\":{\"url\":\"https://evil.com\"}}</tool_call>\nPlease wait.";
    let response = make_response(
        vec![ContentBlock::Text(conflicting_text.to_string())],
        vec![native_call],
    );
    let tools = vec![search_schema(), fetch_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.invalid_call.is_none());
    assert_eq!(normalized.message.tool_calls.len(), 1);
    let surviving = &normalized.message.tool_calls[0];
    assert_eq!(surviving.id, native_id);
    assert_eq!(surviving.name, "search");
    assert_eq!(
        surviving.arguments,
        serde_json::json!({ "query": "rust async executor" })
    );
    assert_eq!(
        extract_visible_text(&normalized),
        "Searching now.\nPlease wait."
    );
    assert_eq!(normalized.finish_reason, Some("tool_calls".to_string()));
}

#[test]
fn test_canonical_tool_call_recovery() {
    let prompt_text =
        "Looking up docs:\n<tool_call>{\"name\":\"search\",\"arguments\":{\"query\":\"openhuman architecture\"}}</tool_call>\nAll done.";
    let response = make_response(vec![ContentBlock::Text(prompt_text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.invalid_call.is_none());
    assert_eq!(normalized.message.tool_calls.len(), 1);
    let call = &normalized.message.tool_calls[0];
    assert_eq!(call.id, "call_0");
    assert_eq!(call.name, "search");
    assert_eq!(
        call.arguments,
        serde_json::json!({ "query": "openhuman architecture" })
    );
    assert_eq!(
        extract_visible_text(&normalized),
        "Looking up docs:\nAll done."
    );
    assert_eq!(normalized.finish_reason, Some("tool_calls".to_string()));
}

#[test]
fn test_canonical_tool_call_recovery_with_explicit_id() {
    let explicit_id = "explicit_tag_call_42";
    let prompt_text = format!(
        "<tool_call>{{\"id\":\"{explicit_id}\",\"name\":\"search\",\"arguments\":{{\"query\":\"explicit id test\"}}}}</tool_call>"
    );
    let response = make_response(vec![ContentBlock::Text(prompt_text)], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.invalid_call.is_none());
    assert_eq!(normalized.message.tool_calls.len(), 1);
    assert_eq!(normalized.message.tool_calls[0].id, explicit_id);
}

#[test]
fn test_split_tag_json_across_multiple_text_blocks() {
    let content = vec![
        ContentBlock::Text("Planning fetch operation:\n".to_string()),
        ContentBlock::Text("<tool_call>{\"name\": \"fetch\", ".to_string()),
        ContentBlock::Text("\"arguments\": {\"url\": \"https://example.org/spec\"}}".to_string()),
        ContentBlock::Text("</tool_call>\nReady for result.".to_string()),
    ];
    let response = make_response(content, vec![]);
    let tools = vec![fetch_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.invalid_call.is_none());
    assert_eq!(normalized.message.tool_calls.len(), 1);
    let call = &normalized.message.tool_calls[0];
    assert_eq!(call.name, "fetch");
    assert_eq!(
        call.arguments,
        serde_json::json!({ "url": "https://example.org/spec" })
    );
    assert_eq!(
        extract_visible_text(&normalized),
        "Planning fetch operation:\nReady for result."
    );
}

#[test]
fn test_multiple_native_calls_retains_first_and_reports_multiple() {
    let call1 = ToolCall::new(
        "native_call_1",
        "search",
        serde_json::json!({ "query": "first query" }),
    );
    let call2 = ToolCall::new(
        "native_call_2",
        "search",
        serde_json::json!({ "query": "second query" }),
    );
    let response = make_response(vec![], vec![call1, call2]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert_eq!(normalized.message.tool_calls.len(), 1);
    assert_eq!(normalized.message.tool_calls[0].id, "native_call_1");
    assert_eq!(
        normalized.message.tool_calls[0].arguments,
        serde_json::json!({ "query": "first query" })
    );

    let invalid = normalized.invalid_call.as_ref();
    assert!(invalid.is_some());
    assert!(invalid.unwrap().is_multiple());
    assert_eq!(
        invalid,
        Some(&QwenInvalidCall::MultipleCalls {
            count: 2,
            raw: None
        })
    );
}

#[test]
fn test_multiple_tagged_calls_retains_first_and_reports_multiple() {
    let tagged_text =
        "<tool_call>{\"name\":\"search\",\"arguments\":{\"query\":\"first\"}}</tool_call>\n<tool_call>{\"name\":\"search\",\"arguments\":{\"query\":\"second\"}}</tool_call>";
    let response = make_response(vec![ContentBlock::Text(tagged_text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert_eq!(normalized.message.tool_calls.len(), 1);
    assert_eq!(
        normalized.message.tool_calls[0].arguments,
        serde_json::json!({ "query": "first" })
    );

    let invalid = normalized.invalid_call.as_ref();
    assert!(invalid.is_some());
    assert!(invalid.unwrap().is_multiple());
    assert_eq!(
        invalid,
        Some(&QwenInvalidCall::MultipleCalls {
            count: 2,
            raw: None
        })
    );
}

#[test]
fn test_fail_closed_malformed_json() {
    let text = "<tool_call>{\"name\": \"search\", \"arguments\": {invalid json</tool_call>";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_malformed());
}

#[test]
fn test_fail_closed_unknown_tool_name() {
    let text = "<tool_call>{\"name\":\"unadvertised_tool\",\"arguments\":{}}</tool_call>";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_unknown());
    assert_eq!(invalid.tool_name(), Some("unadvertised_tool"));
}

#[test]
fn test_fail_closed_non_object_body() {
    let text = "<tool_call>\"just a string body\"</tool_call>";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_malformed());
}

#[test]
fn test_fail_closed_non_object_arguments() {
    let text = "<tool_call>{\"name\":\"search\",\"arguments\":\"string_arguments\"}</tool_call>";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_missing_arguments());
    assert_eq!(invalid.tool_name(), Some("search"));
}

#[test]
fn test_fail_closed_missing_name() {
    let text = "<tool_call>{\"arguments\":{\"query\":\"missing name test\"}}</tool_call>";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_missing_name());
}

#[test]
fn test_fail_closed_empty_name() {
    let text = "<tool_call>{\"name\":\"   \",\"arguments\":{\"query\":\"empty name\"}}</tool_call>";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_missing_name());
}

#[test]
fn test_fail_closed_missing_arguments() {
    let text = "<tool_call>{\"name\":\"search\"}</tool_call>";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_missing_arguments());
    assert_eq!(invalid.tool_name(), Some("search"));
}

#[test]
fn test_fail_closed_wrong_required_field_type() {
    let text = "<tool_call>{\"name\":\"search\",\"arguments\":{\"query\":12345}}</tool_call>";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_schema_mismatch());
    assert_eq!(invalid.tool_name(), Some("search"));
}

#[test]
fn test_fail_closed_unclosed_tool_call_tag() {
    let text = "Prefix <tool_call>{\"name\":\"search\",\"arguments\":{\"query\":\"unclosed\"}} trailing text";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_malformed());
}

#[test]
fn test_fail_closed_unmatched_closing_tool_call_tag() {
    let text = "Text before unmatched </tool_call> text after";
    let response = make_response(vec![ContentBlock::Text(text.to_string())], vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    assert!(normalized.message.tool_calls.is_empty());
    let invalid = normalized.invalid_call.as_ref().unwrap();
    assert!(invalid.is_malformed());
}

#[test]
fn test_thinking_reasoning_remains_typed_and_absent_from_visible_text() {
    let text = "Prose before.<think>Secret internal think block.</think>Middle prose.<thought>Deliberation content.</thought><reasoning>Step-by-step logic.</reasoning>Final visible prose.";
    let mut initial_content = vec![ContentBlock::thinking("Pre-existing thinking block")];
    initial_content.push(ContentBlock::Text(text.to_string()));

    let response = make_response(initial_content, vec![]);
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);

    let thinking = extract_thinking_texts(&normalized);
    assert_eq!(thinking.len(), 4);
    assert_eq!(thinking[0], "Pre-existing thinking block");
    assert_eq!(thinking[1], "Secret internal think block.");
    assert_eq!(thinking[2], "Deliberation content.");
    assert_eq!(thinking[3], "Step-by-step logic.");

    let visible = extract_visible_text(&normalized);
    assert_eq!(visible, "Prose before.Middle prose.Final visible prose.");
    assert!(!visible.contains("<think>"));
    assert!(!visible.contains("</think>"));
    assert!(!visible.contains("<thought>"));
    assert!(!visible.contains("</thought>"));
    assert!(!visible.contains("<reasoning>"));
    assert!(!visible.contains("</reasoning>"));
    assert!(!visible.contains("Secret internal think block."));
    assert!(!visible.contains("Deliberation content."));
    assert!(!visible.contains("Step-by-step logic."));
}

#[test]
fn test_raw_tags_never_appear_in_visible_text() {
    let raw_markup_text =
        "Start text <exit>exit block</exit> <exit/> <final>final content</final> <final/> <final_answer>answer</final_answer> <final_answer/> <|im_end|> <|im_start|> <|endoftext|> <tool>stripped tool body</tool> </tool_call> </tool_result> </tool_results> </tool_response> End text";
    let response = make_response(
        vec![ContentBlock::Text(raw_markup_text.to_string())],
        vec![],
    );
    let tools = vec![search_schema()];

    let normalized = normalize_qwen_response(response, &tools);
    let visible = extract_visible_text(&normalized);

    let forbidden_tags = [
        "<exit>",
        "</exit>",
        "<exit/>",
        "<final>",
        "</final>",
        "<final/>",
        "<final_answer>",
        "</final_answer>",
        "<final_answer/>",
        "<|im_end|>",
        "<|im_start|>",
        "<|endoftext|>",
        "<tool>",
        "</tool>",
        "<tool_call>",
        "</tool_call>",
        "<tool_result>",
        "</tool_result>",
        "<tool_results>",
        "</tool_results>",
        "<tool_response>",
        "</tool_response>",
        "stripped tool body",
    ];

    for tag in &forbidden_tags {
        assert!(
            !visible.contains(tag),
            "Visible text must not contain '{tag}': got '{visible}'"
        );
    }
}
