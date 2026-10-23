use super::*;
use serde_json::json;

#[test]
fn summarize_action_pulls_safe_fields() {
    let args = json!({
        "action": "execute",
        "tool_slug": "SLACK_SEND",
        "params": { "body": "hi" }
    });
    let summary = summarize_action("composio", &args);
    assert!(summary.contains("composio"));
    assert!(summary.contains("action=execute"));
    assert!(summary.contains("tool_slug=SLACK_SEND"));
    assert!(!summary.contains("hi"));
}

#[test]
fn summarize_action_falls_back_to_size_only() {
    let args = json!({});
    let summary = summarize_action("pushover", &args);
    assert!(summary.contains("pushover"));
    assert!(summary.contains("bytes"));
}

#[test]
fn summarize_action_skill_install_is_human_readable() {
    let args = json!({ "entry_id": "notion" });
    let summary = summarize_action("skill_registry_install", &args);
    // Friendly sentence, not a key=value/byte dump (#3993).
    assert_eq!(
        summary,
        "Install the \"notion\" skill to complete your task"
    );
    assert!(!summary.contains("bytes"));
}

#[test]
fn summarize_action_skill_install_without_entry_id_falls_back() {
    let args = json!({});
    let summary = summarize_action("skill_registry_install", &args);
    // Missing slug → generic fallback so we never panic or mislabel.
    assert!(summary.contains("skill_registry_install"));
    assert!(summary.contains("bytes"));
}
