use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::flows::store;
use crate::openhuman::flows::types::SuggestionStatus;
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Arc<Config> {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    Arc::new(config)
}

fn tool(config: Arc<Config>) -> SuggestWorkflowsTool {
    SuggestWorkflowsTool::new(config)
}

#[test]
fn declares_no_side_effect_permission() {
    let tmp = TempDir::new().unwrap();
    let t = tool(test_config(&tmp));
    assert_eq!(t.name(), "suggest_workflows");
    assert_eq!(t.permission_level(), PermissionLevel::None);
    assert!(!t.external_effect());
}

#[tokio::test]
async fn persists_valid_suggestions_and_returns_payload() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let t = tool(config.clone());

    let args = json!({
        "run_id": "run-123",
        "suggestions": [
            {
                "title": "Auto-file email receipts",
                "one_liner": "Add each Gmail receipt to your expenses sheet.",
                "rationale": "You forward receipts to yourself most weeks.",
                "trigger_hint": "app_event",
                "steps_outline": ["Watch Gmail", "Extract vendor + amount", "Append Sheet row"],
                "suggested_connections": ["composio:gmail:conn_1"],
                "suggested_slugs": ["GMAIL_NEW_GMAIL_MESSAGE"],
                "build_prompt": "Build a workflow that watches Gmail for receipts…",
                "confidence": 0.82
            }
        ]
    });

    let res = t.execute(args).await.unwrap();
    assert!(!res.is_error, "expected success, got: {res:?}");

    // Persisted and queryable.
    let stored = store::list_suggestions(&config, Some(SuggestionStatus::New), 50).unwrap();
    assert_eq!(stored.len(), 1);
    let s = &stored[0];
    assert_eq!(s.title, "Auto-file email receipts");
    assert_eq!(s.trigger_hint.as_deref(), Some("app_event"));
    assert_eq!(s.suggested_connections, vec!["composio:gmail:conn_1"]);
    assert_eq!(s.source_run_id.as_deref(), Some("run-123"));
    assert!((s.confidence - 0.82).abs() < 1e-9);
}

#[tokio::test]
async fn rejects_empty_suggestions() {
    let tmp = TempDir::new().unwrap();
    let t = tool(test_config(&tmp));
    let res = t.execute(json!({ "suggestions": [] })).await.unwrap();
    assert!(res.is_error);
}

#[tokio::test]
async fn rejects_suggestion_missing_required_field() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let t = tool(config.clone());
    // Missing `build_prompt`.
    let args = json!({
        "suggestions": [
            {
                "title": "Incomplete",
                "one_liner": "does a thing",
                "rationale": "because"
            }
        ]
    });
    let res = t.execute(args).await.unwrap();
    assert!(res.is_error);
    // Nothing persisted on a rejected batch.
    assert!(store::list_suggestions(&config, None, 50)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn dedupes_identical_titles_within_a_batch() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let t = tool(config.clone());
    let one = json!({
        "title": "Daily Digest",
        "one_liner": "a",
        "rationale": "b",
        "build_prompt": "c"
    });
    // Same title with different casing/spacing normalizes to the same id.
    let two = json!({
        "title": "  daily   digest ",
        "one_liner": "a2",
        "rationale": "b2",
        "build_prompt": "c2"
    });
    let res = t
        .execute(json!({ "suggestions": [one, two] }))
        .await
        .unwrap();
    assert!(!res.is_error);
    let stored = store::list_suggestions(&config, None, 50).unwrap();
    assert_eq!(
        stored.len(),
        1,
        "identical titles should collapse to one row"
    );
}

#[tokio::test]
async fn clamps_confidence_and_truncates_extra_items() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let t = tool(config.clone());

    // 10 suggestions with out-of-range confidence; expect cap at 8 stored and
    // confidence clamped into [0,1].
    let items: Vec<_> = (0..10)
        .map(|i| {
            json!({
                "title": format!("Idea {i}"),
                "one_liner": "x",
                "rationale": "y",
                "build_prompt": "z",
                "confidence": 5.0
            })
        })
        .collect();
    let res = t.execute(json!({ "suggestions": items })).await.unwrap();
    assert!(!res.is_error);
    let stored = store::list_suggestions(&config, None, 50).unwrap();
    assert_eq!(stored.len(), 8, "batch capped at MAX_SUGGESTIONS_PER_CALL");
    assert!(stored.iter().all(|s| s.confidence == 1.0));
}
