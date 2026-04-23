use super::*;

#[test]
fn tool_metadata() {
    let tool = CompleteOnboardingTool::new();
    assert_eq!(tool.name(), "complete_onboarding");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert_eq!(tool.scope(), ToolScope::AgentOnly);
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["action"].is_object());
    assert_eq!(schema["required"], serde_json::json!(["action"]));
}

#[test]
fn build_status_snapshot_carries_new_fields() {
    // A default Config is "bare install" — no channels, no
    // integrations. This test locks in the JSON shape the welcome
    // agent's prompt.md depends on. Dropping or renaming a field
    // breaks this test loudly.
    let config = Config::default();
    let snapshot = build_status_snapshot(
        &config,
        "pending",
        0,
        false,
        "fewer_than_min_exchanges_and_no_skills_connected",
    );

    assert_eq!(snapshot["onboarding_status"], "pending");
    assert_eq!(snapshot["exchange_count"], 0);
    assert_eq!(snapshot["ready_to_complete"], false);
    assert_eq!(
        snapshot["ready_to_complete_reason"],
        "fewer_than_min_exchanges_and_no_skills_connected"
    );
    assert_eq!(snapshot["chat_onboarding_completed"], false);
    assert_eq!(snapshot["ui_onboarding_completed"], false);
    assert_eq!(snapshot["active_channel"], "web");
    assert_eq!(
        snapshot["channels_connected"]
            .as_array()
            .expect("channels_connected is an array")
            .len(),
        0,
        "default Config should report zero connected channels"
    );
    assert!(snapshot["integrations"].is_object());
    assert!(snapshot["memory"].is_object());
    for key in [
        "composio",
        "browser",
        "web_search",
        "http_request",
        "local_ai",
    ] {
        assert!(
            snapshot["integrations"][key].is_boolean(),
            "integrations.{key} must be a bool"
        );
    }
}

#[test]
fn build_status_snapshot_ready_to_complete_reflected() {
    let config = Config::default();
    let snapshot = build_status_snapshot(&config, "pending", 5, true, "criteria_met");
    assert_eq!(snapshot["ready_to_complete"], true);
    assert_eq!(snapshot["ready_to_complete_reason"], "criteria_met");
    assert_eq!(snapshot["exchange_count"], 5);
    assert_eq!(snapshot["onboarding_status"], "pending");
}

#[test]
fn build_status_snapshot_unauthenticated_reason_reflected() {
    let config = Config::default();
    let snapshot = build_status_snapshot(&config, "unauthenticated", 0, false, "unauthenticated");
    assert_eq!(snapshot["ready_to_complete"], false);
    assert_eq!(snapshot["ready_to_complete_reason"], "unauthenticated");
    assert_eq!(snapshot["onboarding_status"], "unauthenticated");
}

#[test]
fn detect_auth_on_default_config_is_unauthenticated() {
    let config = Config::default();
    let (is_auth, source) = detect_auth(&config);
    assert!(!is_auth);
    assert!(source.is_null());
}

// ── exchange counter ──────────────────────────────────────────────────────

#[test]
fn exchange_counter_increments_and_resets() {
    reset_welcome_exchange_count();
    assert_eq!(get_welcome_exchange_count(), 0);
    increment_welcome_exchange_count();
    assert_eq!(get_welcome_exchange_count(), 1);
    increment_welcome_exchange_count();
    increment_welcome_exchange_count();
    assert_eq!(get_welcome_exchange_count(), 3);
    reset_welcome_exchange_count();
    assert_eq!(get_welcome_exchange_count(), 0);
}

// ── description ───────────────────────────────────────────────────────────

#[test]
fn description_mentions_key_actions() {
    let tool = CompleteOnboardingTool::new();
    let desc = tool.description();
    assert!(!desc.is_empty());
    assert!(
        desc.contains("check_status"),
        "description should mention check_status"
    );
    assert!(
        desc.contains("complete"),
        "description should mention complete"
    );
    assert!(
        desc.contains("ready_to_complete"),
        "description should mention ready_to_complete"
    );
    assert!(
        desc.contains("ready_to_complete_reason"),
        "description should mention ready_to_complete_reason"
    );
}

#[test]
fn premature_complete_error_mentions_skills_and_exchanges() {
    let msg = build_not_ready_to_complete_error(1);
    assert!(
        msg.contains("User hasn't connected any skills and minimum exchanges not reached"),
        "expected issue #596 wording in error message, got: {msg}"
    );
    assert!(
        msg.contains("currently 1; 2 more needed"),
        "expected dynamic exchange counters in error message, got: {msg}"
    );
}

// ── schema enum values ────────────────────────────────────────────────────

#[test]
fn schema_action_enum_has_both_values() {
    let tool = CompleteOnboardingTool::new();
    let schema = tool.parameters_schema();
    let enum_vals = schema["properties"]["action"]["enum"]
        .as_array()
        .expect("action enum should be an array");
    let names: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        names.contains(&"check_status"),
        "enum should contain check_status"
    );
    assert!(names.contains(&"complete"), "enum should contain complete");
}

// ── spec roundtrip ────────────────────────────────────────────────────────

#[test]
fn spec_roundtrip() {
    let tool = CompleteOnboardingTool::new();
    let spec = tool.spec();
    assert_eq!(spec.name, "complete_onboarding");
    assert!(spec.parameters.is_object());
}

// ── execute: unknown action ───────────────────────────────────────────────

#[tokio::test]
async fn execute_unknown_action_returns_error() {
    let tool = CompleteOnboardingTool::new();
    let result = tool
        .execute(serde_json::json!({"action": "unknown_action"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("Unknown action"),
        "error message should contain 'Unknown action', got: {}",
        result.output()
    );
}

// ── execute: missing action defaults to check_status ─────────────────────

#[tokio::test]
async fn execute_missing_action_defaults_to_check_status() {
    // When action is absent it defaults to "check_status", which calls
    // Config::load_or_init() — that may succeed or fail depending on env,
    // but it should not return the "Unknown action" error.
    let tool = CompleteOnboardingTool::new();
    let result = tool.execute(serde_json::json!({})).await;
    if let Ok(r) = result {
        assert!(
            !r.output().contains("Unknown action"),
            "missing action should default to check_status, not 'Unknown action'"
        );
    }
}

// ── guard: engagement_criteria_met ───────────────────────────────────────

/// Zero exchanges, no composio → criteria NOT met.
#[test]
fn criteria_not_met_zero_exchanges_no_composio() {
    assert!(!engagement_criteria_met(0, 0));
}

/// One exchange below threshold, no composio → criteria NOT met.
#[test]
fn criteria_not_met_below_threshold() {
    assert!(!engagement_criteria_met(MIN_EXCHANGES_TO_COMPLETE - 1, 0));
}

/// Exactly at the exchange threshold, no composio → criteria MET.
#[test]
fn criteria_met_at_exchange_threshold() {
    assert!(engagement_criteria_met(MIN_EXCHANGES_TO_COMPLETE, 0));
}

/// Above the exchange threshold → criteria MET.
#[test]
fn criteria_met_above_threshold() {
    assert!(engagement_criteria_met(MIN_EXCHANGES_TO_COMPLETE + 5, 0));
}

/// Zero exchanges but one composio connection → criteria MET
/// (composio is an OR shortcut, not AND).
#[test]
fn criteria_met_via_composio_zero_exchanges() {
    assert!(engagement_criteria_met(0, 1));
}

/// One exchange and one composio connection → criteria MET.
#[test]
fn criteria_met_via_composio_with_exchanges() {
    assert!(engagement_criteria_met(1, 1));
}

/// Exchange count at u32::MAX — no panic, criteria met.
#[test]
fn criteria_met_saturating_exchange_count() {
    assert!(engagement_criteria_met(u32::MAX, 0));
}
