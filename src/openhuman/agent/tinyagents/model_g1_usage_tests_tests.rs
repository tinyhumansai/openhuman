//! Gap G1: a standalone `invoke` must stay usage-faithful — token
//! breakdowns ride the crate `Usage`, and the two host fields with no crate
//! home (charged USD + context window) ride `ModelResponse.raw` and
//! reconstruct exactly via [`usage_info_from_response`].
use super::*;

fn empty_registry() -> crate::openhuman::agent::pformat::PFormatRegistry {
    crate::openhuman::agent::pformat::PFormatRegistry::default()
}

#[test]
fn usage_round_trips_charged_usd_and_all_token_breakdowns() {
    let chat = ChatResponse {
        text: Some("hi".to_string()),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 100,
            output_tokens: 20,
            context_window: 128_000,
            cached_input_tokens: 40,
            cache_creation_tokens: 10,
            reasoning_tokens: 7,
            charged_amount_usd: 0.0123,
        }),
        reasoning_content: None,
    };
    let model_response = response_to_model_response(&chat, &empty_registry(), false);

    // Crate Usage carries every token breakdown natively.
    let usage = model_response.usage.expect("usage present");
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 20);
    assert_eq!(usage.cache_read_tokens, 40);
    assert_eq!(usage.cache_creation_tokens, 10);
    assert_eq!(usage.reasoning_tokens, 7);

    // Charged USD + context window ride raw and reconstruct exactly.
    let recovered = usage_info_from_response(&model_response).expect("usage info");
    assert_eq!(recovered.input_tokens, 100);
    assert_eq!(recovered.output_tokens, 20);
    assert_eq!(recovered.context_window, 128_000);
    assert_eq!(recovered.cached_input_tokens, 40);
    assert_eq!(recovered.cache_creation_tokens, 10);
    assert_eq!(recovered.reasoning_tokens, 7);
    assert!((recovered.charged_amount_usd - 0.0123).abs() < 1e-9);
}

#[test]
fn no_billing_metadata_leaves_raw_clean() {
    let chat = ChatResponse {
        text: Some("hi".to_string()),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 5,
            output_tokens: 3,
            ..Default::default()
        }),
        reasoning_content: None,
    };
    let model_response = response_to_model_response(&chat, &empty_registry(), false);
    assert!(
        model_response.raw.is_none(),
        "no charged USD / window ⇒ raw stays None"
    );
    let recovered = usage_info_from_response(&model_response).expect("usage info");
    assert_eq!(recovered.charged_amount_usd, 0.0);
    assert_eq!(recovered.context_window, 0);
    assert_eq!(recovered.input_tokens, 5);
}

#[test]
fn no_usage_reconstructs_to_none() {
    let chat = ChatResponse {
        text: Some("hi".to_string()),
        tool_calls: Vec::new(),
        usage: None,
        reasoning_content: None,
    };
    let model_response = response_to_model_response(&chat, &empty_registry(), false);
    assert!(usage_info_from_response(&model_response).is_none());
}

#[test]
fn tool_less_response_preserves_literal_tool_call_markup() {
    let text = r#"Example: <tool_call>{"name":"lookup","arguments":{}}</tool_call>"#;
    let chat = ChatResponse {
        text: Some(text.to_string()),
        ..Default::default()
    };

    let response = response_to_model_response(&chat, &empty_registry(), false);

    assert_eq!(response.text(), text);
    assert!(response.message.tool_calls.is_empty());
}

#[test]
fn tool_enabled_response_still_extracts_tool_call_markup() {
    let chat = ChatResponse {
        text: Some(r#"<tool_call>{"name":"lookup","arguments":{}}</tool_call>"#.to_string()),
        ..Default::default()
    };

    let response = response_to_model_response(&chat, &empty_registry(), true);

    assert_eq!(response.text(), "");
    assert_eq!(response.message.tool_calls.len(), 1);
    assert_eq!(response.message.tool_calls[0].name, "lookup");
}

fn tool_request() -> ModelRequest {
    ModelRequest {
        tools: vec![tinyinference::tool::ToolSchema::new(
            "lookup",
            "looks up a record",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "query": { "type": "string" }
                }
            }),
        )],
        ..Default::default()
    }
}

#[test]
fn prompt_guided_response_uses_tinyagents_xml_parser() {
    let response = prompt_guided_text_response(
        r#"Checking.<tool_call>{"name":"lookup","arguments":{"id":7}}</tool_call>"#.to_string(),
        &tool_request(),
    );

    assert_eq!(response.text(), "Checking.");
    assert_eq!(response.message.tool_calls.len(), 1);
    assert!(
        !response.message.tool_calls[0].id.is_empty(),
        "the upstream parser assigns the tool-call ID"
    );
    assert_eq!(response.message.tool_calls[0].name, "lookup");
    assert_eq!(
        response.message.tool_calls[0].arguments,
        serde_json::json!({"id": 7})
    );
}

#[test]
fn prompt_guided_response_keeps_legacy_pformat_fallback() {
    let response = prompt_guided_text_response(
        "<tool_call>lookup[7|needle]</tool_call>".to_string(),
        &tool_request(),
    );

    assert_eq!(response.text(), "");
    assert_eq!(response.message.tool_calls.len(), 1);
    assert_eq!(response.message.tool_calls[0].name, "lookup");
    assert_eq!(
        response.message.tool_calls[0].arguments,
        serde_json::json!({"id": 7, "query": "needle"})
    );
}
