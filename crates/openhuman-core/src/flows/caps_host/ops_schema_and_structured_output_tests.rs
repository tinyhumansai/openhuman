use super::*;
use serde_json::json;

#[test]
fn unowned_model_ids_override_the_role_default() {
    for raw in ["claude-opus-4", "deepseek-v4-pro"] {
        assert!(crate::inference::provider_model::is_raw_model(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
            raw
        ));
        assert_eq!(resolve_host_model(Some(raw), "chat-v1".to_owned()), raw);
    }
}

#[test]
fn registered_provider_aliases_and_hints_keep_the_resolved_model() {
    crate::agent::session_host::test_support::install_model_fixture();
    for tier in ["fixture-reasoning", "hint:reasoning", "hint:garbage"] {
        assert_eq!(
            resolve_host_model(Some(tier), "reasoning-v1".to_owned()),
            "reasoning-v1"
        );
    }
    assert_eq!(resolve_host_model(None, "chat-v1".to_owned()), "chat-v1");
    assert_eq!(
        resolve_host_model(Some("   "), "chat-v1".to_owned()),
        "chat-v1"
    );
}

#[test]
fn provider_response_preserves_the_flow_completion_contract() {
    use tinyinference_llm::message::{AssistantMessage, ContentBlock};
    use tinyinference_llm::model::ModelResponse;
    use tinyinference_llm::tool::ToolCall;
    use tinyinference_llm::usage::Usage;

    let usage = Usage::new(11, 7);
    let response = ModelResponse {
        message: AssistantMessage {
            id: Some("msg-1".to_string()),
            content: vec![
                ContentBlock::Text("done".to_string()),
                ContentBlock::thinking("private chain"),
            ],
            tool_calls: vec![ToolCall {
                id: "call-1".to_string(),
                name: "lookup".to_string(),
                arguments: json!({"query": "weather"}),
                invalid: None,
            }],
            usage: Some(usage),
            origin: None,
        },
        usage: Some(usage),
        finish_reason: Some("tool_calls".to_string()),
        raw: tinyinference_llm::usage::merge_usage_meta(None, Some(0.125), 128_000),
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
        correlation: None,
        resolved_route: None,
    };

    let value = model_response_to_completion_value(&response);
    assert_eq!(value["text"], "done");
    assert_eq!(value["tool_calls"][0]["id"], "call-1");
    assert_eq!(value["tool_calls"][0]["name"], "lookup");
    assert_eq!(
        value["tool_calls"][0]["arguments"],
        r#"{"query":"weather"}"#
    );
    assert_eq!(value["usage"]["input_tokens"], 11);
    assert_eq!(value["usage"]["output_tokens"], 7);
    assert_eq!(value["usage"]["context_window"], 128_000);
    assert_eq!(value["usage"]["charged_amount_usd"], 0.125);
    assert_eq!(value["reasoning_content"], "private chain");
}
