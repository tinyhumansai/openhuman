use super::*;

#[test]
fn inference_agent_chat_controllers_keep_their_wire_names_and_namespace() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), 2);
    assert_eq!(
        controllers[0].rpc_method_name(),
        crate::agent_runs::INFERENCE_AGENT_CHAT
    );
    assert_eq!(controllers[0].schema.namespace, "inference");
    assert_eq!(controllers[0].schema.function, "agent_chat");
    assert_eq!(controllers[1].schema.function, "agent_chat_simple");
}

#[test]
fn ordinary_agent_chat_still_ignores_inference_only_fields() {
    let params: AgentChatParams = serde_json::from_value(serde_json::json!({
        "message": "hello",
        "inference_url": 7,
        "api_key": {"unexpected": true},
        "inference_headers": "malformed",
        "agent_id": false
    }))
    .expect("unknown inference-only fields remain ignored on agent.chat");
    assert_eq!(params.message, "hello");
    assert!(params.cwd.is_none());
}

#[test]
fn common_agent_chat_params_preserve_success_and_invalid_message_errors() {
    let success = serde_json::json!({
        "message": "hello",
        "model_override": "gpt",
        "temperature": 0.2
    });
    let params: AgentChatParams =
        deserialize_params(success.as_object().unwrap().clone()).expect("valid common params");
    assert_eq!(params.message, "hello");
    assert_eq!(params.model_override.as_deref(), Some("gpt"));
    assert_eq!(params.temperature, Some(0.2));

    let invalid = serde_json::json!({ "message": true });
    let error = deserialize_params::<AgentChatParams>(invalid.as_object().unwrap().clone())
        .expect_err("message must remain a string");
    assert!(error.starts_with("invalid params:"));
}

#[test]
fn inference_agent_chat_agent_id_defaults_to_none() {
    let without: InferenceAgentChatParams =
        serde_json::from_value(serde_json::json!({ "message": "hello" })).expect("decodes");
    assert!(without.agent_id.is_none());
    let with: InferenceAgentChatParams =
        serde_json::from_value(serde_json::json!({ "message": "hello", "agent_id": "alpha" }))
            .expect("decodes with agent id");
    assert_eq!(with.agent_id.as_deref(), Some("alpha"));
}

#[test]
fn inference_agent_chat_parses_its_route_fields() {
    let params: InferenceAgentChatParams = serde_json::from_value(serde_json::json!({
        "message": "hello",
        "agent_id": "alpha",
        "inference_url": "https://example.test/v1",
        "api_key": "secret",
        "inference_headers": [["x-test", "value"]],
        "inference_provider_options": {"reasoning": {"effort": "high"}},
        "inference_reasoning_effort": "high"
    }))
    .expect("inference route params decode");
    assert_eq!(params.agent_id.as_deref(), Some("alpha"));
    assert_eq!(
        params.inference_url.as_deref(),
        Some("https://example.test/v1")
    );
    assert_eq!(
        params.inference_headers,
        vec![("x-test".into(), "value".into())]
    );
    assert_eq!(
        params.inference_provider_options,
        Some(serde_json::json!({"reasoning": {"effort": "high"}}))
    );
    assert_eq!(params.inference_reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn inference_agent_chat_keeps_the_optional_agent_id_schema_and_typed_route_fields() {
    let schema = schemas("agent_chat");
    let agent_id = schema
        .inputs
        .iter()
        .find(|field| field.name == "agent_id")
        .expect("agent_id is part of the inference chat contract");
    assert!(!agent_id.required);

    let invalid = serde_json::from_value::<InferenceAgentChatParams>(serde_json::json!({
        "message": "hello",
        "inference_url": 7
    }));
    assert!(
        invalid.is_err(),
        "the inference surface still validates route fields"
    );

    for name in ["inference_provider_options", "inference_reasoning_effort"] {
        assert!(
            schema.inputs.iter().any(|field| field.name == name),
            "the inference chat schema must preserve the {name} wire field"
        );
    }
}
