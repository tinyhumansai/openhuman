//! Native turn-model source coverage.

use std::sync::Arc;

use super::*;

#[test]
fn crate_native_turn_source_retains_only_role_and_config() {
    let source = TurnModelSource::new_crate_native(
        "chat",
        Arc::new(crate::openhuman::config::Config::default()),
    );

    assert!(source.direct_model.is_none());
    assert!(source.crate_native.is_some());
}

#[test]
fn crate_native_text_mode_is_recorded_without_resolving_a_model() {
    let source = TurnModelSource::new_crate_native(
        "chat",
        Arc::new(crate::openhuman::config::Config::default()),
    )
    .with_text_mode();

    assert!(source
        .crate_native
        .as_ref()
        .is_some_and(|native| native.force_text_mode));
}

#[test]
fn crate_native_text_mode_disables_native_tools_on_workload_fallbacks() {
    use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let _guard = crate::openhuman::inference::inference_test_guard();
    let provider = "deepseek:deepseek-chat".to_string();
    let mut config = crate::openhuman::config::Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_deepseek".to_string(),
        slug: "deepseek".to_string(),
        label: "DeepSeek".to_string(),
        endpoint: "https://api.deepseek.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("deepseek-chat".to_string()),
        ..Default::default()
    });
    config.chat_provider = Some(provider.clone());
    config.reasoning_provider = Some(provider.clone());
    config.agentic_provider = Some(provider.clone());
    config.coding_provider = Some(provider.clone());
    config.vision_provider = Some(provider.clone());
    config.memory_provider = Some(provider);

    let models = TurnModelSource::new_crate_native("chat", Arc::new(config))
        .with_text_mode()
        .build("chat-v1", 0.0, Some(32_000))
        .expect("text-mode turn models build");

    assert!(
        !models.routes.is_empty(),
        "expected workload fallback models"
    );
    assert!(
        models
            .routes
            .iter()
            .all(|(_, model)| { model.profile().is_some_and(|profile| !profile.tool_calling) }),
        "every workload fallback must preserve prompt-guided text mode"
    );
}

#[test]
fn direct_model_turn_source_builds_without_provider_adapter() {
    let model: Arc<dyn tinyagents::harness::model::ChatModel<()>> =
        Arc::new(tinyagents::harness::testkit::ScriptedModel::replies(vec![
            "done",
        ]));
    let source = TurnModelSource::from_model(model);

    assert!(source.crate_native.is_none());
    assert!(source.direct_model.is_some());

    let models = source
        .build("mock-model", 0.0, Some(32_000))
        .expect("direct model source builds");
    assert_eq!(models.provider_id(), "injected");
    assert_eq!(models.context_window(), Some(32_000));
    assert!(!models.native_tools());
}

#[test]
fn run_policy_for_makes_invalid_tool_arguments_recoverable() {
    let policy = run_policy_for(10, false);
    assert_eq!(
        policy.invalid_args,
        InvalidArgsPolicy::ReturnToolError,
        "schema-invalid calls must return a corrective tool result instead of aborting the turn"
    );
}
