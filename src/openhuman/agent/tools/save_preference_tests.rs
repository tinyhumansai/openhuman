//! Tests for the `save_preference` two-lane preference tool.

use super::*;

use crate::openhuman::memory::guard::MemoryGuard;
use crate::openhuman::memory::ops::{shared_memory_test_workspace, GLOBAL_MEMORY_TEST_LOCK};
use crate::openhuman::security::SecurityPolicy;
use serde_json::json;
use std::sync::Arc;

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::default())
}

/// Bind the shared test workspace and hand back its guard, with both preference
/// namespaces emptied first.
///
/// The tool resolves the ambient guarded driver per call, so there is no
/// per-test store to isolate into any more — every test in this file writes to
/// the one process-wide binding. Callers hold [`GLOBAL_MEMORY_TEST_LOCK`] for
/// the duration, and this clears the two lanes so a leftover row from an
/// earlier test cannot satisfy (or break) an assertion here.
async fn fresh_guard() -> Arc<MemoryGuard> {
    shared_memory_test_workspace();
    let guard = crate::openhuman::memory::ops::guard::active_memory_guard()
        .await
        .expect("guard resolves");
    for ns in [USER_PREF_GENERAL_NAMESPACE, USER_PREF_SITUATIONAL_NAMESPACE] {
        for key in keys_in(&guard, ns).await {
            let _ = guard.forget(ns, &key).await;
        }
    }
    guard
}

async fn keys_in(mem: &Arc<MemoryGuard>, namespace: &str) -> Vec<String> {
    mem.list(Some(namespace), None, None)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.key)
        .collect()
}

// ── PrefScope ────────────────────────────────────────────────────────────────

#[test]
fn pref_scope_parse_case_insensitive() {
    assert_eq!(PrefScope::parse("general"), Some(PrefScope::General));
    assert_eq!(
        PrefScope::parse("Situational"),
        Some(PrefScope::Situational)
    );
    assert_eq!(
        PrefScope::parse("SITUATIONAL"),
        Some(PrefScope::Situational)
    );
    assert_eq!(PrefScope::parse("bogus"), None);
    assert_eq!(PrefScope::parse(""), None);
}

#[test]
fn pref_scope_namespace_mapping() {
    assert_eq!(PrefScope::General.namespace(), USER_PREF_GENERAL_NAMESPACE);
    assert_eq!(
        PrefScope::Situational.namespace(),
        USER_PREF_SITUATIONAL_NAMESPACE
    );
    assert_eq!(
        PrefScope::General.other_namespace(),
        USER_PREF_SITUATIONAL_NAMESPACE
    );
    assert_eq!(
        PrefScope::Situational.other_namespace(),
        USER_PREF_GENERAL_NAMESPACE
    );
}

// ── Tool metadata ─────────────────────────────────────────────────────────────

#[test]
fn tool_name_and_permission() {
    let tool = SavePreferenceTool::new(test_security());
    assert_eq!(tool.name(), "save_preference");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}

#[test]
fn schema_has_required_fields() {
    let tool = SavePreferenceTool::new(test_security());
    let schema = tool.parameters_schema();
    let required: Vec<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(required.contains(&"topic"));
    assert!(required.contains(&"value"));
    assert!(required.contains(&"category"));
}

// ── Argument validation ─────────────────────────────────────────────────────────

#[tokio::test]
async fn invalid_category_returns_error() {
    let tool = SavePreferenceTool::new(test_security());
    let r = tool
        .execute(json!({"topic": "x", "value": "y", "category": "bogus"}))
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.output().contains("category"));
}

#[tokio::test]
async fn invalid_topic_chars_returns_error() {
    let tool = SavePreferenceTool::new(test_security());
    let r = tool
        .execute(json!({"topic": "Bad Topic!", "value": "y", "category": "general"}))
        .await
        .unwrap();
    assert!(r.is_error);
}

#[tokio::test]
async fn empty_value_returns_error() {
    let tool = SavePreferenceTool::new(test_security());
    let r = tool
        .execute(json!({"topic": "topic", "value": "   ", "category": "general"}))
        .await
        .unwrap();
    assert!(r.is_error);
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn secret_like_value_is_rejected_before_write() {
    let _serial = GLOBAL_MEMORY_TEST_LOCK.lock().await;
    let mem = fresh_guard().await;
    let tool = SavePreferenceTool::new(test_security());
    let r = tool
        .execute(json!({
            "topic": "api",
            "value": "api_key=sk-123456789012345678901234567890",
            "category": "general",
        }))
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.output().contains("looks like a secret"));
    // Nothing persisted in either lane.
    assert!(keys_in(&mem, USER_PREF_GENERAL_NAMESPACE).await.is_empty());
    assert!(keys_in(&mem, USER_PREF_SITUATIONAL_NAMESPACE)
        .await
        .is_empty());
}

// ── Storage behaviour ─────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn saves_general_pref_to_general_namespace() {
    let _serial = GLOBAL_MEMORY_TEST_LOCK.lock().await;
    let mem = fresh_guard().await;
    let tool = SavePreferenceTool::new(test_security());
    let r = tool
        .execute(json!({
            "topic": "reply_language",
            "value": "Reply in British English.",
            "category": "general"
        }))
        .await
        .unwrap();
    assert!(!r.is_error, "expected success, got: {}", r.output());

    assert!(keys_in(&mem, USER_PREF_GENERAL_NAMESPACE)
        .await
        .contains(&"reply_language".to_string()));
    assert!(keys_in(&mem, USER_PREF_SITUATIONAL_NAMESPACE)
        .await
        .is_empty());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn recategorising_moves_pref_between_namespaces() {
    let _serial = GLOBAL_MEMORY_TEST_LOCK.lock().await;
    let mem = fresh_guard().await;
    let tool = SavePreferenceTool::new(test_security());

    // Save as general.
    tool.execute(json!({"topic": "tone", "value": "be terse", "category": "general"}))
        .await
        .unwrap();
    assert!(keys_in(&mem, USER_PREF_GENERAL_NAMESPACE)
        .await
        .contains(&"tone".to_string()));

    // Re-save the same topic as situational → moves namespaces, no stale copy.
    tool.execute(
        json!({"topic": "tone", "value": "be terse in code reviews", "category": "situational"}),
    )
    .await
    .unwrap();
    assert!(keys_in(&mem, USER_PREF_SITUATIONAL_NAMESPACE)
        .await
        .contains(&"tone".to_string()));
    assert!(
        !keys_in(&mem, USER_PREF_GENERAL_NAMESPACE)
            .await
            .contains(&"tone".to_string()),
        "the general-scope copy must be cleared when re-categorised"
    );
}

// ── Contradiction surfacing (chat-affirmed) ──────────────────────────────────
//
// These two tests used to live here, over a bespoke `KwEmbedder` and a private
// `UnifiedMemory` built so vector similarity would move at all. Both the logic
// and the coverage moved with `recall_related_preferences` into
// `memory::preferences` — `related_preferences_exclude_the_just_saved_topic`
// and `situational_recall_filters_on_the_vector_component_not_the_final_score`
// script the score breakdown directly, so they pin the similarity gate the
// embedder was only ever an indirect way of reaching.
//
// The tool-side half of that behaviour — that the message threads the related
// preferences back to the model — is covered by the success path above.
