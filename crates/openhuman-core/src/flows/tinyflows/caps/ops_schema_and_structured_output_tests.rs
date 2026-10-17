use super::*;
use crate::inference::provider::is_raw_passthrough_model;
use tinyflows::nodes::integration::agent_prompt;

// ── resolve_completion_model raw/BYOK passthrough (issue #4598) ───────────
#[test]
fn resolve_completion_model_forwards_raw_byok_node_model_verbatim() {
    // A raw/BYOK id maps to the `chat` role, so the role resolves to the
    // default model — but the pinned id is what the user selected and must
    // be the model the completion runs on.
    assert_eq!(
        agent_prompt::resolve_completion_model(
            Some("claude-opus-4"),
            "chat-v1".to_string(),
            is_raw_passthrough_model
        ),
        "claude-opus-4"
    );
    assert_eq!(
        agent_prompt::resolve_completion_model(
            Some("deepseek-v4-pro"),
            "chat-v1".to_string(),
            is_raw_passthrough_model
        ),
        "deepseek-v4-pro"
    );
}

#[test]
fn resolve_completion_model_leaves_managed_tier_and_hint_node_models_untouched() {
    // Managed tiers and every `hint:*` alias keep the role-resolved model.
    assert_eq!(
        agent_prompt::resolve_completion_model(
            Some("chat-v1"),
            "chat-v1".to_string(),
            is_raw_passthrough_model
        ),
        "chat-v1"
    );
    assert_eq!(
        agent_prompt::resolve_completion_model(
            Some("hint:reasoning"),
            "reasoning-v1".to_string(),
            is_raw_passthrough_model
        ),
        "reasoning-v1"
    );
    assert_eq!(
        agent_prompt::resolve_completion_model(
            Some("hint:garbage"),
            "reasoning-v1".to_string(),
            is_raw_passthrough_model
        ),
        "reasoning-v1"
    );
    // No pinned model, or a whitespace-only pin, keeps the resolved default.
    assert_eq!(
        agent_prompt::resolve_completion_model(
            None,
            "chat-v1".to_string(),
            is_raw_passthrough_model
        ),
        "chat-v1"
    );
    assert_eq!(
        agent_prompt::resolve_completion_model(
            Some("   "),
            "chat-v1".to_string(),
            is_raw_passthrough_model
        ),
        "chat-v1"
    );
}

#[test]
fn crate_model_response_preserves_flow_completion_contract() {
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

// ── build_agent_result improvements (issue #5151) ────────────────────
