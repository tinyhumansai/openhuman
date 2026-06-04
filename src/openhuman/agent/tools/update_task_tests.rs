//! Tests for the `update_task` tool.

use super::*;
use crate::openhuman::agent::task_board::TaskCardStatus;
use crate::openhuman::tools::traits::Tool;
use serde_json::json;

// ── build_patch ──────────────────────────────────────────────────────────────

#[test]
fn build_patch_maps_status_and_all_fields() {
    let patch = build_patch(&json!({
        "status": "in_progress",
        "objective": "ship it",
        "notes": "halfway",
        "blocker": "",
        "evidence": ["https://x/1", "  ", "note"],
        "plan": ["a", "b"],
        "acceptanceCriteria": ["covered"]
    }))
    .expect("valid patch");
    assert_eq!(patch.status, Some(TaskCardStatus::InProgress));
    assert_eq!(patch.objective.as_deref(), Some("ship it"));
    assert_eq!(patch.notes.as_deref(), Some("halfway"));
    // empty/blank array entries are dropped; blanks elsewhere become None via edit.
    assert_eq!(
        patch.evidence,
        Some(vec!["https://x/1".to_string(), "note".to_string()])
    );
    assert_eq!(patch.plan, Some(vec!["a".to_string(), "b".to_string()]));
    assert_eq!(patch.acceptance_criteria, Some(vec!["covered".to_string()]));
}

#[test]
fn build_patch_empty_args_is_empty() {
    let patch = build_patch(&json!({})).expect("ok");
    assert!(patch_is_empty(&patch));
}

#[test]
fn build_patch_with_only_status_is_not_empty() {
    let patch = build_patch(&json!({ "status": "done" })).expect("ok");
    assert!(!patch_is_empty(&patch));
    assert_eq!(patch.status, Some(TaskCardStatus::Done));
}

#[test]
fn build_patch_rejects_unknown_status() {
    assert!(build_patch(&json!({ "status": "nonsense" })).is_err());
}

#[test]
fn build_patch_rejects_non_string_array_item() {
    assert!(build_patch(&json!({ "evidence": [1, 2] })).is_err());
}

// ── execute() guard rails (no board/workspace needed) ────────────────────────

#[tokio::test]
async fn execute_without_id_is_an_error() {
    let res = UpdateTaskTool::new()
        .execute(json!({ "status": "done" }))
        .await
        .unwrap();
    assert!(res.is_error, "missing id must be a tool error");
}

#[tokio::test]
async fn execute_with_id_but_no_changes_is_an_error() {
    // id present but nothing to change → rejected before any board write.
    let res = UpdateTaskTool::new()
        .execute(json!({ "id": "task-1" }))
        .await
        .unwrap();
    assert!(res.is_error, "empty update must be a tool error");
}

// ── the move + update applied through ops::edit (the tool's real effect) ─────

#[test]
fn apply_moves_and_updates_a_card_and_returns_success() {
    let dir = tempfile::tempdir().unwrap();
    let location = BoardLocation::Thread {
        workspace_dir: dir.path().to_path_buf(),
        thread_id: TASK_SOURCES_THREAD_ID.to_string(),
    };
    let id = ops::add(&location, "Review PR #5", CardPatch::default())
        .unwrap()
        .cards[0]
        .id
        .clone();

    // Move to done + attach evidence — the exact shape the tool produces.
    let patch = build_patch(&json!({
        "status": "done",
        "evidence": ["posted review on PR #5"]
    }))
    .unwrap();
    let res = apply(&location, &id, patch);
    assert!(!res.is_error, "successful move/update must not be an error");

    // The board reflects the move + evidence.
    let card = ops::list(&location)
        .unwrap()
        .cards
        .into_iter()
        .find(|c| c.id == id)
        .unwrap();
    assert_eq!(card.status, TaskCardStatus::Done);
    assert!(card.evidence.iter().any(|e| e.contains("posted review")));
}

#[test]
fn apply_on_unknown_id_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let location = BoardLocation::Thread {
        workspace_dir: dir.path().to_path_buf(),
        thread_id: TASK_SOURCES_THREAD_ID.to_string(),
    };
    let patch = build_patch(&json!({ "status": "blocked", "blocker": "no channel" })).unwrap();
    let res = apply(&location, "task-does-not-exist", patch);
    assert!(res.is_error, "missing card must surface as a tool error");
}
