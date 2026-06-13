//! Tests for the `save_preference` two-lane preference tool.

use super::*;

use crate::openhuman::embeddings::NoopEmbedding;
use crate::openhuman::memory_store::UnifiedMemory;
use crate::openhuman::security::SecurityPolicy;
use serde_json::json;
use tempfile::TempDir;

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::default())
}

fn test_mem() -> (TempDir, Arc<dyn Memory>) {
    let tmp = TempDir::new().unwrap();
    let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
    (tmp, Arc::new(mem))
}

async fn keys_in(mem: &Arc<dyn Memory>, namespace: &str) -> Vec<String> {
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
    let (_tmp, mem) = test_mem();
    let tool = SavePreferenceTool::new(mem, test_security());
    assert_eq!(tool.name(), "save_preference");
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}

#[test]
fn schema_has_required_fields() {
    let (_tmp, mem) = test_mem();
    let tool = SavePreferenceTool::new(mem, test_security());
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
    let (_tmp, mem) = test_mem();
    let tool = SavePreferenceTool::new(mem, test_security());
    let r = tool
        .execute(json!({"topic": "x", "value": "y", "category": "bogus"}))
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.output().contains("category"));
}

#[tokio::test]
async fn invalid_topic_chars_returns_error() {
    let (_tmp, mem) = test_mem();
    let tool = SavePreferenceTool::new(mem, test_security());
    let r = tool
        .execute(json!({"topic": "Bad Topic!", "value": "y", "category": "general"}))
        .await
        .unwrap();
    assert!(r.is_error);
}

#[tokio::test]
async fn empty_value_returns_error() {
    let (_tmp, mem) = test_mem();
    let tool = SavePreferenceTool::new(mem, test_security());
    let r = tool
        .execute(json!({"topic": "topic", "value": "   ", "category": "general"}))
        .await
        .unwrap();
    assert!(r.is_error);
}

#[tokio::test]
async fn secret_like_value_is_rejected_before_write() {
    let (_tmp, mem) = test_mem();
    let tool = SavePreferenceTool::new(mem.clone(), test_security());
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
async fn saves_general_pref_to_general_namespace() {
    let (_tmp, mem) = test_mem();
    let tool = SavePreferenceTool::new(mem.clone(), test_security());
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
async fn recategorising_moves_pref_between_namespaces() {
    let (_tmp, mem) = test_mem();
    let tool = SavePreferenceTool::new(mem.clone(), test_security());

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

use async_trait::async_trait;

/// Keyword-sensitive embedder so prefs about the same theme embed close together
/// (high cosine) and unrelated ones don't.
struct KwEmbedder;

#[async_trait]
impl crate::openhuman::embeddings::EmbeddingProvider for KwEmbedder {
    fn name(&self) -> &str {
        "kw"
    }
    fn model_id(&self) -> &str {
        "kw"
    }
    fn dimensions(&self) -> usize {
        2
    }
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let l = t.to_lowercase();
                vec![
                    if l.contains("terse") || l.contains("verbose") || l.contains("detail") {
                        1.0
                    } else {
                        0.0
                    },
                    if l.contains("rust") { 1.0 } else { 0.0 },
                ]
            })
            .collect())
    }
}

fn kw_mem() -> (TempDir, Arc<dyn Memory>) {
    let tmp = TempDir::new().unwrap();
    let mem = UnifiedMemory::new(tmp.path(), Arc::new(KwEmbedder), None).unwrap();
    (tmp, Arc::new(mem))
}

#[tokio::test]
async fn save_surfaces_related_preference_for_contradiction_check() {
    let (_tmp, mem) = kw_mem();
    let tool = SavePreferenceTool::new(mem.clone(), test_security());

    tool.execute(json!({"topic": "verbosity", "value": "always be terse", "category": "general"}))
        .await
        .unwrap();

    // A semantically-related pref under a different topic.
    let r = tool
        .execute(json!({
            "topic": "explanation_style",
            "value": "give detailed verbose explanations",
            "category": "general"
        }))
        .await
        .unwrap();
    assert!(!r.is_error);
    assert!(
        r.output().contains("verbosity") && r.output().contains("always be terse"),
        "expected the related pref to be surfaced for a contradiction check, got: {}",
        r.output()
    );
}

#[tokio::test]
async fn save_unrelated_preference_surfaces_nothing() {
    let (_tmp, mem) = kw_mem();
    let tool = SavePreferenceTool::new(mem.clone(), test_security());

    tool.execute(json!({"topic": "verbosity", "value": "always be terse", "category": "general"}))
        .await
        .unwrap();

    // An unrelated pref (rust) — no contradiction note.
    let r = tool
        .execute(json!({
            "topic": "rust_edition",
            "value": "use rust 2021 edition",
            "category": "situational"
        }))
        .await
        .unwrap();
    assert!(!r.is_error);
    assert!(
        !r.output().contains("check for contradictions"),
        "an unrelated pref should surface no related prefs, got: {}",
        r.output()
    );
}
