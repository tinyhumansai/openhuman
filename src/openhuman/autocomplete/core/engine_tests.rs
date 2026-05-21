use super::detect_tab_artifact_suffix;
use super::is_low_quality_suggestion;
use super::{AutocompleteEngine, AutocompleteStatus};
use crate::openhuman::config::Config;

#[test]
fn low_quality_rejects_too_short() {
    assert!(is_low_quality_suggestion("", ""));
    assert!(is_low_quality_suggestion("a", "hello "));
}

#[test]
fn low_quality_rejects_pure_punct() {
    assert!(is_low_quality_suggestion("...", "hello"));
    assert!(is_low_quality_suggestion("  -- ", "hello"));
}

#[test]
fn low_quality_rejects_echo_of_tail() {
    assert!(is_low_quality_suggestion("world", "hello world"));
}

#[test]
fn low_quality_accepts_new_content() {
    assert!(!is_low_quality_suggestion(" world", "hello"));
    assert!(!is_low_quality_suggestion("tomorrow", "see you "));
}

#[test]
fn detects_literal_tab_suffix() {
    assert_eq!(
        detect_tab_artifact_suffix("hello world", "hello world\t"),
        1
    );
}

#[test]
fn detects_space_indentation_suffix() {
    assert_eq!(
        detect_tab_artifact_suffix("hello world", "hello world    "),
        4
    );
}

#[test]
fn returns_zero_when_context_does_not_match_expected_tail() {
    assert_eq!(
        detect_tab_artifact_suffix("hello world", "different    "),
        0
    );
}

#[test]
fn returns_zero_when_no_tab_like_suffix_present() {
    assert_eq!(detect_tab_artifact_suffix("hello world", "hello worldx"), 0);
}

#[tokio::test]
async fn status_with_config_returns_valid_status_without_disk_load() {
    let engine = AutocompleteEngine::new();
    let config = Config::default();

    let status: AutocompleteStatus = engine.status_with_config(&config).await;

    assert_eq!(status.enabled, config.autocomplete.enabled);
    assert!(!status.running, "fresh engine should not be running");
    assert_eq!(status.phase, "idle");
    assert_eq!(status.model_id, config.local_ai.chat_model_id);
    assert!(status.last_error.is_none());
    assert!(status.suggestion.is_none());
}

#[tokio::test]
async fn status_with_config_reflects_provided_config_not_disk() {
    let engine = AutocompleteEngine::new();
    let mut config = Config::default();
    config.autocomplete.enabled = false;
    config.local_ai.chat_model_id = "test-model-xyz".to_string();

    let status = engine.status_with_config(&config).await;

    assert!(
        !status.enabled,
        "should reflect the passed-in config, not disk state"
    );
    assert_eq!(status.model_id, "test-model-xyz");
}
