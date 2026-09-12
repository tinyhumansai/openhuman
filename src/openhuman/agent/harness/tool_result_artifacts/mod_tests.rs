use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
use crate::openhuman::tools::traits::Tool;
use crate::openhuman::tools::FileReadTool;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn threshold_persists_preview_and_readable_file() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session/one");
    let raw = format!(
        "{} {}",
        "x".repeat(4096),
        "ghp_abcdefghijklmnopqrstuvwxyz123456"
    );

    let (out, outcome) =
        apply_per_result_persistence(raw.clone(), Some(&store), "shell", Some("call-1"), 1024)
            .await;

    assert!(outcome.persisted);
    assert!(out.contains("artifact_path: artifacts/tool-results/session_one/shell/call-1.txt"));
    assert!(out.contains("original_bytes:"));
    assert!(out.contains("[preview]"));
    assert!(!out.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));

    let policy = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        action_dir: tmp.path().to_path_buf(),
        workspace_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });
    let reader = FileReadTool::new(policy);
    let read = reader
        .execute(json!({"path": "artifacts/tool-results/session_one/shell/call-1.txt"}))
        .await
        .unwrap();
    assert!(!read.is_error, "{}", read.output());
    assert!(read.output().contains("xxxx"));
    assert!(!read
        .output()
        .contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
}

#[tokio::test]
async fn fallback_truncates_when_store_missing() {
    let raw = "z".repeat(4096);
    let (out, outcome) = apply_per_result_persistence(raw, None, "shell", Some("call"), 512).await;
    assert!(!outcome.persisted);
    assert!(out.contains("truncated by tool_result_budget"));
    assert!(out.len() < 4096);
}

#[tokio::test]
async fn persisted_preview_is_bounded_for_small_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session");
    let raw = "x".repeat(800);

    let (out, outcome) =
        apply_per_result_persistence(raw, Some(&store), "shell", Some("call"), 320).await;

    assert!(outcome.persisted);
    assert!(outcome.final_bytes <= 320, "final={}", outcome.final_bytes);
    assert_eq!(out.len(), outcome.final_bytes);
    assert!(out.contains("[tool_result_preview]"));
    assert!(tmp
        .path()
        .join("artifacts/tool-results/session/shell/call.txt")
        .exists());
}

#[tokio::test]
async fn aggregate_spills_largest_until_under_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session");
    let mut results = vec![
        ToolExecutionResult {
            name: "small".into(),
            output: "a".repeat(100),
            success: true,
            tool_call_id: Some("small".into()),
        },
        ToolExecutionResult {
            name: "largest".into(),
            output: "b".repeat(2000),
            success: true,
            tool_call_id: Some("largest".into()),
        },
        ToolExecutionResult {
            name: "medium".into(),
            output: "c".repeat(900),
            success: true,
            tool_call_id: Some("medium".into()),
        },
    ];

    spill_aggregate_tool_results(&mut results, Some(&store), 1800).await;

    assert!(results[1].output.starts_with("[tool_result_preview]\n"));
    let total: usize = results.iter().map(|result| result.output.len()).sum();
    assert!(total <= 1800, "total={total}");
    assert!(!results[0].output.starts_with("[tool_result_preview]\n"));
    assert!(tmp
        .path()
        .join("artifacts/tool-results/session/largest/largest.txt")
        .exists());
}

#[tokio::test]
async fn aggregate_forces_budget_when_envelope_has_no_savings() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session");
    let mut results = vec![
        ToolExecutionResult {
            name: "one".into(),
            output: "a".repeat(350),
            success: true,
            tool_call_id: Some("one".into()),
        },
        ToolExecutionResult {
            name: "two".into(),
            output: "b".repeat(350),
            success: true,
            tool_call_id: Some("two".into()),
        },
        ToolExecutionResult {
            name: "three".into(),
            output: "c".repeat(350),
            success: true,
            tool_call_id: Some("three".into()),
        },
    ];

    spill_aggregate_tool_results(&mut results, Some(&store), 500).await;

    let total: usize = results.iter().map(|result| result.output.len()).sum();
    // #4469 item 6: the aggregate spill now floors each persisted envelope at
    // MIN_ENVELOPE_ALLOWANCE_BYTES so the `[tool_result_preview]` header +
    // `artifact_path` pointer always survives (previously an exhausted budget
    // could blank a result to ""). That is a documented trade — the total may
    // slightly overshoot the raw aggregate budget — so the invariant is now:
    // (a) no envelope is blanked, and (b) the total stays bounded by the
    // per-result floor rather than the raw budget.
    assert!(
        results.iter().all(|result| !result.output.is_empty()),
        "no persisted envelope may be blanked — the artifact pointer must survive"
    );
    assert!(
        total <= results.len() * MIN_ENVELOPE_ALLOWANCE_BYTES,
        "total={total} exceeds the per-result envelope floor bound"
    );
    assert!(tmp
        .path()
        .join("artifacts/tool-results/session/one/one.txt")
        .exists());
}
