use super::*;
use tinyagents_registry::router::{FallbackDiagnostics, FallbackReason};

fn vision_fallback() -> FallbackDiagnostics {
    FallbackDiagnostics {
        role: "vision".into(),
        requested_route: None,
        fallback_route: "anthropic:claude-sonnet-4".into(),
        reason: FallbackReason::ImplicitCloudFallback,
        override_role: "vision".into(),
    }
}

#[test]
fn fallback_notice_renders_native_route_as_branded_actionable_copy() {
    let message = cloud_fallback_notice(&vision_fallback(), "ollama:gemma3:1b");
    assert!(message.contains("Vision (image input)"), "got: {message}");
    assert!(message.contains("ollama:gemma3:1b"), "got: {message}");
    assert!(
        message.contains("anthropic:claude-sonnet-4"),
        "got: {message}"
    );
    assert!(message.contains("vision_provider"), "got: {message}");
}

#[test]
fn missing_credentials_copy_uses_native_role_and_override_metadata() {
    let message = missing_provider_credentials_message(
        &vision_fallback(),
        "anthropic",
        Some("ollama:gemma3:1b"),
    );
    assert!(message.contains("anthropic"), "got: {message}");
    assert!(message.contains("ollama:gemma3:1b"), "got: {message}");
    assert!(message.contains("vision (image input)"), "got: {message}");
    assert!(message.contains("Connections"), "got: {message}");
    assert!(message.contains("vision_provider"), "got: {message}");
}

#[test]
fn missing_credentials_without_local_chat_does_not_claim_a_local_model() {
    let message = missing_provider_credentials_message(&vision_fallback(), "anthropic", None);
    assert!(message.contains("anthropic"), "got: {message}");
    assert!(message.contains("vision (image input)"), "got: {message}");
    assert!(
        !message.contains("Your chat model is local"),
        "got: {message}"
    );
}

#[test]
fn vision_unsupported_message_names_model_and_a_concrete_remedy() {
    let message = local_vision_unsupported_message("gemma3:1b");
    assert!(message.contains("gemma3:1b"), "got: {message}");
    assert!(message.contains("llava:7b"), "got: {message}");
    assert!(message.contains("vision_provider"), "got: {message}");
}
