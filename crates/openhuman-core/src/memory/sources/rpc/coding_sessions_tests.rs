//! Coding-session ingestion prerequisite checks.
use super::*;

#[test]
fn cloud_opt_out_is_refused_before_provider_construction_and_stays_unchanged() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    config.memory_tree.cloud_summarization_opt_in = false;
    let before = config.memory_provider.clone();
    let error = validate_ingestion_provider(&config).unwrap_err();
    assert!(error.starts_with("cloud_processing_disabled:"));
    assert!(!config.memory_tree.cloud_summarization_opt_in);
    assert_eq!(config.memory_provider, before);
}

#[test]
fn local_processing_passes_preflight_without_cloud_opt_in() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = true;
    config.local_ai.chat_model_id = "offline-fixture".into();
    config.memory_tree.cloud_summarization_opt_in = false;
    assert!(validate_ingestion_provider(&config).is_ok());
    assert!(!config.memory_tree.cloud_summarization_opt_in);
}

#[test]
fn missing_provider_reports_the_step_without_changing_the_route() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    config.memory_tree.cloud_summarization_opt_in = true;
    config.memory_provider = Some("invalid-fixture-provider:model".into());
    let error = validate_ingestion_provider(&config).unwrap_err();
    assert!(error.starts_with("summarization_unavailable:"));
    assert_eq!(
        config.memory_provider.as_deref(),
        Some("invalid-fixture-provider:model")
    );
}
