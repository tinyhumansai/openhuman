//! Gap G1: a standalone `invoke` must stay usage-faithful — token
//! breakdowns ride the crate `Usage`, and the two host fields with no crate
//! home (charged USD + context window) ride `ModelResponse.raw` and
//! reconstruct exactly via [`usage_info_from_response`].
use super::*;
use tinyinference_llm::usage::Usage;

fn empty_registry() -> tinytools_agent::PFormatRegistry {
    tinytools_agent::PFormatRegistry::default()
}

#[test]
fn usage_round_trips_charged_usd_and_all_token_breakdowns() {
    let chat = ChatResponse {
        text: Some("hi".to_string()),
        tool_calls: Vec::new(),
        usage: Some(
            BilledUsage::from_counts(100, 20)
                .with_context_window(128_000)
                .with_cached_input_tokens(40)
                .with_cache_creation_tokens(10)
                .with_reasoning_tokens(7)
                .with_charged_usd(0.0123),
        ),
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
    assert_eq!(
        usage.charged_amount.map(|amount| amount.micros),
        Some(12_300)
    );
    assert_eq!(usage.context_window_tokens, Some(128_000));

    // Charged USD + context window ride raw and reconstruct exactly.
    let recovered = usage_info_from_response(&model_response).expect("usage info");
    assert_eq!(recovered.input_tokens, 100);
    assert_eq!(recovered.output_tokens, 20);
    assert_eq!(recovered.context_window(), 128_000);
    assert_eq!(recovered.cached_input_tokens(), 40);
    assert_eq!(recovered.cache_creation_tokens, 10);
    assert_eq!(recovered.reasoning_tokens, 7);
    assert!((recovered.charged_amount_usd - 0.0123).abs() < 1e-9);
}

#[test]
fn no_billing_metadata_leaves_raw_clean() {
    let chat = ChatResponse {
        text: Some("hi".to_string()),
        tool_calls: Vec::new(),
        usage: Some(BilledUsage::from_counts(5, 3)),
        reasoning_content: None,
    };
    let model_response = response_to_model_response(&chat, &empty_registry(), false);
    assert!(
        model_response.raw.is_none(),
        "no charged USD / window ⇒ raw stays None"
    );
    let recovered = usage_info_from_response(&model_response).expect("usage info");
    assert_eq!(recovered.charged_amount_usd, 0.0);
    assert_eq!(recovered.context_window(), 0);
    assert_eq!(recovered.input_tokens, 5);
}

#[test]
fn provider_neutral_total_cost_metadata_is_recovered() {
    let model_response = ModelResponse {
        usage: Some(Usage::new(8, 3)),
        raw: Some(serde_json::json!({"total_cost_usd": 0.0042})),
        ..ModelResponse::assistant("done")
    };

    let recovered = usage_info_from_response(&model_response).expect("usage info");
    assert!((recovered.charged_amount_usd - 0.0042).abs() < 1e-9);
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
        tools: vec![tinyinference_llm::tool::ToolSchema::new(
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

/// Wire shape of the billing/context metadata stashed in `ModelResponse.raw`:
/// a literal-JSON pin so the key and field names cannot drift, and a response
/// written by the current release (with the old key set) still reconstructs.
#[test]
fn usage_meta_raw_wire_shape_is_stable_and_old_payloads_load() {
    let chat = ChatResponse {
        text: Some("hi".to_string()),
        tool_calls: Vec::new(),
        usage: Some(
            BilledUsage::from_counts(100, 20)
                .with_context_window(128_000)
                .with_charged_usd(0.0123),
        ),
        reasoning_content: None,
    };
    let written = response_to_model_response(&chat, &empty_registry(), false);
    assert_eq!(
        written.raw,
        Some(serde_json::json!({
            "openhuman_usage_meta": {"charged_amount_usd": 0.0123, "context_window": 128000}
        }))
    );

    let mut old = response_to_model_response(
        &ChatResponse {
            text: Some("x".into()),
            usage: Some(BilledUsage::from_counts(7, 2)),
            ..Default::default()
        },
        &empty_registry(),
        false,
    );
    old.raw = Some(serde_json::json!({
        "openhuman_usage_meta": {"charged_amount_usd": 0.5, "context_window": 32000}
    }));
    let recovered = usage_info_from_response(&old).expect("usage");
    assert_eq!(recovered.charged_amount_usd, 0.5);
    assert_eq!(recovered.context_window(), 32_000);
    assert_eq!(recovered.input_tokens, 7);
}
