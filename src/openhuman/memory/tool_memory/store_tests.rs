//! Tests for [`ToolMemoryStore`] — exercise the put/list/delete/prompt
//! surface against an in-memory mock backend.

use std::sync::Arc;

use super::*;
use crate::openhuman::memory::tool_memory::test_helpers::MockMemory;
use crate::openhuman::memory::tool_memory::types::{
    ToolMemoryPriority, ToolMemoryRule, ToolMemorySource,
};

fn fresh_store() -> ToolMemoryStore {
    ToolMemoryStore::new(Arc::new(MockMemory::default()))
}

#[tokio::test]
async fn put_rule_rejects_blank_tool_name() {
    let store = fresh_store();
    let rule = ToolMemoryRule::new(
        "  ",
        "body",
        ToolMemoryPriority::Normal,
        ToolMemorySource::Programmatic,
    );
    let err = store.put_rule(rule).await.unwrap_err();
    assert!(err.contains("tool_name"));
}

#[tokio::test]
async fn put_rule_rejects_blank_body() {
    let store = fresh_store();
    let rule = ToolMemoryRule::new(
        "email",
        "   ",
        ToolMemoryPriority::Normal,
        ToolMemorySource::Programmatic,
    );
    let err = store.put_rule(rule).await.unwrap_err();
    assert!(err.contains("body"));
}

#[tokio::test]
async fn put_then_get_round_trip_returns_same_rule() {
    let store = fresh_store();
    let rule = store
        .record(
            "email",
            "never email Sarah",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            vec!["safety".into()],
        )
        .await
        .unwrap();
    let fetched = store.get_rule("email", &rule.id).await.unwrap().unwrap();
    assert_eq!(fetched.tool_name, "email");
    assert_eq!(fetched.rule, "never email Sarah");
    assert_eq!(fetched.priority, ToolMemoryPriority::Critical);
    assert_eq!(fetched.source, ToolMemorySource::UserExplicit);
    assert_eq!(fetched.tags, vec!["safety".to_string()]);
}

#[tokio::test]
async fn put_rule_preserves_created_at_on_upsert() {
    let store = fresh_store();
    let mut rule = store
        .record(
            "email",
            "rule body",
            ToolMemoryPriority::Normal,
            ToolMemorySource::Programmatic,
            vec![],
        )
        .await
        .unwrap();
    let created_at = rule.created_at.clone();
    // Mutate and re-put under the same id.
    rule.rule = "updated rule body".into();
    // Sleep a tiny amount to give the timestamp string a chance to change.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let updated = store.put_rule(rule.clone()).await.unwrap();
    assert_eq!(updated.created_at, created_at);
    assert_ne!(updated.updated_at, created_at);
    assert_eq!(updated.rule, "updated rule body");
}

#[tokio::test]
async fn list_rules_sorts_critical_first_then_freshest() {
    let store = fresh_store();
    store
        .record(
            "email",
            "older normal",
            ToolMemoryPriority::Normal,
            ToolMemorySource::Programmatic,
            vec![],
        )
        .await
        .unwrap();
    // Tiny sleep to ensure different updated_at strings.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let crit = store
        .record(
            "email",
            "never email Sarah",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            vec![],
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let high = store
        .record(
            "email",
            "double-check the recipient",
            ToolMemoryPriority::High,
            ToolMemorySource::PostTurn,
            vec![],
        )
        .await
        .unwrap();
    let rules = store.list_rules("email").await.unwrap();
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[0].id, crit.id);
    assert_eq!(rules[1].id, high.id);
    assert_eq!(rules[2].rule, "older normal");
}

#[tokio::test]
async fn delete_rule_removes_only_target_rule() {
    let store = fresh_store();
    let rule = store
        .record(
            "shell",
            "never run sudo",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            vec![],
        )
        .await
        .unwrap();
    store
        .record(
            "shell",
            "prefer tmux for long-running commands",
            ToolMemoryPriority::Normal,
            ToolMemorySource::Programmatic,
            vec![],
        )
        .await
        .unwrap();
    let deleted = store.delete_rule("shell", &rule.id).await.unwrap();
    assert!(deleted);
    let remaining = store.list_rules("shell").await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_ne!(remaining[0].id, rule.id);
}

#[tokio::test]
async fn delete_rule_returns_false_when_missing() {
    let store = fresh_store();
    let deleted = store.delete_rule("shell", "does-not-exist").await.unwrap();
    assert!(!deleted);
}

#[tokio::test]
async fn list_tool_names_returns_only_tool_prefixed_namespaces() {
    // Direct-write a global memory entry so we can verify the filter.
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    memory
        .store(
            "global",
            "noise",
            "{}",
            MemoryCategory::Custom("misc".into()),
            None,
        )
        .await
        .unwrap();
    let store = ToolMemoryStore::new(memory);
    store
        .record(
            "email",
            "rule a",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            vec![],
        )
        .await
        .unwrap();
    store
        .record(
            "shell",
            "rule b",
            ToolMemoryPriority::High,
            ToolMemorySource::PostTurn,
            vec![],
        )
        .await
        .unwrap();
    let mut tools = store.list_tool_names().await.unwrap();
    tools.sort();
    assert_eq!(tools, vec!["email".to_string(), "shell".to_string()]);
}

#[tokio::test]
async fn rules_for_prompt_returns_only_eager_priorities() {
    let store = fresh_store();
    store
        .record(
            "email",
            "never email Sarah",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            vec![],
        )
        .await
        .unwrap();
    store
        .record(
            "email",
            "double-check recipient",
            ToolMemoryPriority::High,
            ToolMemorySource::PostTurn,
            vec![],
        )
        .await
        .unwrap();
    store
        .record(
            "email",
            "we use BCC for newsletters",
            ToolMemoryPriority::Normal,
            ToolMemorySource::Programmatic,
            vec![],
        )
        .await
        .unwrap();

    let rendered = store
        .rules_for_prompt(&["email".to_string()])
        .await
        .unwrap();
    let email_rules = rendered.get("email").expect("email rules present");
    assert_eq!(email_rules.len(), 2, "Normal rule must not be eager");
    assert!(email_rules
        .iter()
        .any(|r| r.priority == ToolMemoryPriority::Critical));
    assert!(email_rules
        .iter()
        .any(|r| r.priority == ToolMemoryPriority::High));
    assert!(email_rules
        .iter()
        .all(|r| r.priority != ToolMemoryPriority::Normal));
}

#[tokio::test]
async fn rules_for_prompt_scans_all_namespaces_when_caller_passes_empty_slice() {
    let store = fresh_store();
    store
        .record(
            "email",
            "never email Sarah",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            vec![],
        )
        .await
        .unwrap();
    store
        .record(
            "shell",
            "never run sudo",
            ToolMemoryPriority::Critical,
            ToolMemorySource::UserExplicit,
            vec![],
        )
        .await
        .unwrap();
    let rendered = store.rules_for_prompt(&[]).await.unwrap();
    assert!(rendered.contains_key("email"));
    assert!(rendered.contains_key("shell"));
}

#[tokio::test]
async fn rules_for_prompt_caps_results() {
    let store = fresh_store();
    for idx in 0..(TOOL_MEMORY_PROMPT_CAP + 5) {
        store
            .record(
                "email",
                &format!("high rule {idx}"),
                ToolMemoryPriority::High,
                ToolMemorySource::Programmatic,
                vec![],
            )
            .await
            .unwrap();
    }
    let rendered = store
        .rules_for_prompt(&["email".to_string()])
        .await
        .unwrap();
    let count: usize = rendered.values().map(|v| v.len()).sum();
    assert_eq!(count, TOOL_MEMORY_PROMPT_CAP);
}

#[tokio::test]
async fn list_rules_skips_malformed_entries() {
    let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
    // Manually write a malformed entry under the tool namespace and a
    // valid one alongside it, then confirm the bad entry is dropped.
    memory
        .store(
            "tool-email",
            "rule/bad",
            "{not-valid-json",
            MemoryCategory::Custom("tool_memory".into()),
            None,
        )
        .await
        .unwrap();
    let store = ToolMemoryStore::new(memory);
    store
        .record(
            "email",
            "valid",
            ToolMemoryPriority::High,
            ToolMemorySource::Programmatic,
            vec![],
        )
        .await
        .unwrap();
    let rules = store.list_rules("email").await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].rule, "valid");
}

#[tokio::test]
async fn list_rules_json_serializes_payload_for_rpc_envelopes() {
    let store = fresh_store();
    store
        .record(
            "email",
            "rule body",
            ToolMemoryPriority::High,
            ToolMemorySource::Programmatic,
            vec![],
        )
        .await
        .unwrap();
    let json = store.list_rules_json("email").await.unwrap();
    let arr = json.as_array().expect("expected an array");
    assert_eq!(arr.len(), 1);
    assert!(arr[0]
        .get("priority")
        .and_then(|v| v.as_str())
        .map(|s| s == "high")
        .unwrap_or(false));
}

#[tokio::test]
async fn put_rule_assigns_id_when_blank() {
    let store = fresh_store();
    let mut rule = ToolMemoryRule::new(
        "email",
        "rule body",
        ToolMemoryPriority::Normal,
        ToolMemorySource::Programmatic,
    );
    rule.id = String::new();
    let stored = store.put_rule(rule).await.unwrap();
    assert!(!stored.id.is_empty());
}
