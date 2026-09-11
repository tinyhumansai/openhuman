use super::*;

#[test]
fn chat_tier_roles_do_not_fall_back_to_cloud() {
    // These inherit a BYOK slug instead; including them would attach the
    // "your local model can't serve this" explanation to the very roles the
    // user did configure locally.
    for role in ["chat", "reasoning", "coding"] {
        assert!(
            !role_falls_back_to_cloud(role),
            "{role} must not be treated as a cloud-fallback role"
        );
    }
}

#[test]
fn background_roles_fall_back_to_cloud() {
    for role in [
        "vision",
        "embeddings",
        "memory",
        "summarization",
        "heartbeat",
        "learning",
        "subconscious",
        "agentic",
        "burst",
    ] {
        assert!(
            role_falls_back_to_cloud(role),
            "{role} must be treated as a cloud-fallback role"
        );
    }
}

#[test]
fn unknown_role_does_not_fall_back() {
    assert!(!role_falls_back_to_cloud("not-a-role"));
    assert!(!role_falls_back_to_cloud(""));
}

#[test]
fn role_falls_back_to_cloud_ignores_surrounding_whitespace() {
    assert!(role_falls_back_to_cloud("  vision  "));
}

#[test]
fn burst_points_at_the_agentic_knob_that_actually_exists() {
    // There is no `burst_provider`; telling the user to set one would be a
    // dead end.
    assert_eq!(override_knob_for_role("burst"), "agentic");
    assert_eq!(override_knob_for_role("summarization"), "memory");
    assert_eq!(override_knob_for_role("vision"), "vision");
}

#[test]
fn fallback_notice_names_capability_local_model_and_override() {
    let msg = cloud_fallback_notice("vision", "ollama:gemma3:1b", "anthropic:claude-sonnet-4");
    assert!(msg.contains("Vision (image input)"), "got: {msg}");
    assert!(msg.contains("ollama:gemma3:1b"), "got: {msg}");
    assert!(msg.contains("anthropic:claude-sonnet-4"), "got: {msg}");
    assert!(msg.contains("vision_provider"), "got: {msg}");
}

#[test]
fn fallback_notice_for_role_without_capability_label_uses_role_name() {
    let msg = cloud_fallback_notice("heartbeat", "ollama:gemma3:1b", "openhuman");
    assert!(msg.contains("Heartbeat"), "got: {msg}");
    assert!(msg.contains("heartbeat_provider"), "got: {msg}");
}

#[test]
fn missing_credentials_message_explains_the_local_fallback() {
    let msg = missing_provider_credentials_message("vision", "anthropic", Some("ollama:gemma3:1b"));
    // The whole point of #5146 §2.1: the user must learn *why* a provider
    // they never configured is being asked for a key.
    assert!(msg.contains("anthropic"), "got: {msg}");
    assert!(msg.contains("ollama:gemma3:1b"), "got: {msg}");
    assert!(msg.contains("vision (image input)"), "got: {msg}");
    assert!(msg.contains("Connections"), "got: {msg}");
    assert!(msg.contains("vision_provider"), "got: {msg}");
}

#[test]
fn missing_credentials_message_without_local_chat_omits_the_local_clause() {
    let msg = missing_provider_credentials_message("embeddings", "openai", None);
    assert!(msg.contains("openai"), "got: {msg}");
    assert!(msg.contains("embeddings"), "got: {msg}");
    assert!(
        !msg.contains("Your chat model is local"),
        "must not claim a local chat model when there is none: {msg}"
    );
}

#[test]
fn missing_credentials_message_treats_blank_local_provider_as_absent() {
    let msg = missing_provider_credentials_message("embeddings", "openai", Some("   "));
    assert!(
        !msg.contains("Your chat model is local"),
        "blank provider string is not a local model: {msg}"
    );
}

#[test]
fn vision_unsupported_message_names_model_and_a_concrete_remedy() {
    let msg = local_vision_unsupported_message("gemma3:1b");
    assert!(msg.contains("gemma3:1b"), "got: {msg}");
    assert!(msg.contains("llava:7b"), "got: {msg}");
    assert!(msg.contains("vision_provider"), "got: {msg}");
}

#[test]
fn vision_preflight_allows_a_vision_capable_model() {
    use crate::openhuman::config::schema::ModelRegistryEntry;
    let mut config = crate::openhuman::config::Config::default();
    config.model_registry = vec![ModelRegistryEntry {
        id: "llava:7b".into(),
        provider: "ollama".into(),
        cost_per_1m_output: 0.0,
        vision: true,
        ..Default::default()
    }];
    assert!(vision_preflight("llava:7b", &config).is_ok());
}

#[test]
fn vision_preflight_rejects_a_text_only_model_with_an_actionable_message() {
    let config = crate::openhuman::config::Config::default();
    let err = vision_preflight("gemma3:1b", &config)
        .expect_err("a text-only model must fail the vision pre-flight");
    assert!(err.contains("gemma3:1b"), "got: {err}");
    assert!(err.contains("llava:7b"), "got: {err}");
}

#[test]
fn vision_preflight_allows_the_managed_vision_tier() {
    // `vision-v1` is multimodal per `oh_tier_supports_vision`; the
    // pre-flight must not fire for the managed vision sub-agent.
    let config = crate::openhuman::config::Config::default();
    assert!(vision_preflight("vision-v1", &config).is_ok());
}
