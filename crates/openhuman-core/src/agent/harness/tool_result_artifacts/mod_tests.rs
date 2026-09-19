use super::*;
use crate::security::{AutonomyLevel, SecurityPolicy};
use crate::tools::FileReadTool;
use serde_json::json;
use std::sync::Arc;
use tinytools::Tool;

#[tokio::test]
async fn threshold_persists_preview_and_readable_file() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session/one");
    let raw = format!(
        "{} {}",
        "x".repeat(4096),
        "ghp_abcdefghijklmnopqrstuvwxyz123456"
    );

    let (out, outcome) = apply_per_result_persistence(
        raw.clone(),
        None,
        Some(&store),
        "shell",
        Some("call-1"),
        1024,
    )
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
    let (out, outcome) =
        apply_per_result_persistence(raw, None, None, "shell", Some("call"), 512).await;
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
        apply_per_result_persistence(raw, None, Some(&store), "shell", Some("call"), 320).await;

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
        ToolOutcome {
            name: "small".into(),
            output: "a".repeat(100),
            success: true,
            tool_call_id: Some("small".into()),
            trusted_verbatim: false,
        },
        ToolOutcome {
            name: "largest".into(),
            output: "b".repeat(2000),
            success: true,
            tool_call_id: Some("largest".into()),
            trusted_verbatim: false,
        },
        ToolOutcome {
            name: "medium".into(),
            output: "c".repeat(900),
            success: true,
            tool_call_id: Some("medium".into()),
            trusted_verbatim: false,
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
        ToolOutcome {
            name: "one".into(),
            output: "a".repeat(350),
            success: true,
            tool_call_id: Some("one".into()),
            trusted_verbatim: false,
        },
        ToolOutcome {
            name: "two".into(),
            output: "b".repeat(350),
            success: true,
            tool_call_id: Some("two".into()),
            trusted_verbatim: false,
        },
        ToolOutcome {
            name: "three".into(),
            output: "c".repeat(350),
            success: true,
            tool_call_id: Some("three".into()),
            trusted_verbatim: false,
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

#[test]
fn artifact_read_target_finds_a_path_nested_in_a_wrapper_call() {
    let args = json!({
        "skill": "files",
        "tool": "file_read",
        "args": {"path": "artifacts/tool-results/s/use_skill/c.txt", "offset": 42}
    });
    assert_eq!(
        artifact_read_target("use_skill", &args),
        Some(ArtifactRead {
            path: "artifacts/tool-results/s/use_skill/c.txt".to_string(),
            offset: 42,
        })
    );
    assert_eq!(
        artifact_read_target("file_read", &json!({"path": "src/main.rs"})),
        None
    );
}

#[tokio::test]
async fn persisted_outcome_reports_the_size_of_the_stored_body() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session");
    let rewritten = "s".repeat(3_000);
    let full = "r".repeat(8_000);

    let (out, outcome) = apply_per_result_persistence(
        rewritten,
        Some(full),
        Some(&store),
        "shell",
        Some("call"),
        1_000,
    )
    .await;

    assert!(outcome.persisted);
    assert_eq!(
        outcome.original_bytes, 8_000,
        "the outcome feeds the artifact index, so it must report the stored body's size"
    );
    assert!(out.contains("original_bytes: 8000"), "{out}");
}

#[test]
fn artifact_read_target_matches_only_file_read_under_the_artifact_directory() {
    let path = "artifacts/tool-results/s/shell/c.txt";
    assert!(artifact_read_target("file_read", &json!({"path": path})).is_some());
    assert_eq!(
        artifact_read_target("file_write", &json!({"path": path, "content": "x"})),
        None,
        "a write to an artifact path is not a read of its content"
    );
    assert_eq!(
        artifact_read_target(
            "use_skill",
            &json!({"skill": "files", "tool": "glob", "args": {"path": path}})
        ),
        None,
        "a wrapped non-read tool is not a read of its content"
    );
}

#[test]
fn artifact_read_target_matches_the_artifact_directory_as_a_path_component() {
    assert!(artifact_read_target(
        "file_read",
        &json!({"path": "./artifacts/tool-results/s/c.txt"})
    )
    .is_some());
    assert_eq!(
        artifact_read_target(
            "file_read",
            &json!({"path": "artifacts/tool-results-backup/report.txt"})
        ),
        None,
        "a sibling directory sharing the prefix is not the artifact directory"
    );
}

#[test]
fn an_artifact_page_stays_within_the_budget_with_a_long_path() {
    let read = ArtifactRead {
        path: format!(
            "artifacts/tool-results/{}/use_skill/call.txt",
            "s".repeat(240)
        ),
        offset: 1_234_567,
    };

    let page = page_artifact_read("y".repeat(5_000), &read, 1_000);
    assert!(
        page.len() <= 1_000,
        "a page must fit the result budget, got {} bytes",
        page.len()
    );
    let body = page
        .find("\n\n[artifact page")
        .expect("continuation marker");
    assert!(
        page.contains(&format!("\"offset\":{}", 1_234_567 + body)),
        "the marker must name the offset right after this page: {page}"
    );
}

#[test]
fn a_page_never_exceeds_the_floored_budget_and_always_advances() {
    // A path too long for its full trailer to leave body room under the floor.
    let read = ArtifactRead {
        path: format!("artifacts/tool-results/{}/c.txt", "p".repeat(600)),
        offset: 7,
    };
    let content = "z".repeat(5_000);

    for budget in [2, 100, MIN_ENVELOPE_ALLOWANCE_BYTES, 700] {
        let page = page_artifact_read(content.clone(), &read, budget);
        let limit = budget.max(MIN_ENVELOPE_ALLOWANCE_BYTES);
        assert!(
            page.len() <= limit,
            "budget {budget}: a page must fit max(budget, floor) = {limit}, got {} bytes",
            page.len()
        );
        let body = page
            .find("\n\n[artifact page")
            .expect("continuation marker");
        assert!(
            body > 0,
            "budget {budget}: every page must advance past its offset"
        );
        assert!(
            page.contains(&format!("\"offset\":{}", 7 + body)),
            "budget {budget}: the marker must name the offset right after this page: {page}"
        );
    }
}

#[test]
fn a_body_redaction_grows_past_the_read_limit_falls_back_to_the_processed_copy() {
    let raw = "call +15551234567 or +15557654321";
    // The limit is the raw size: the raw body fits, its redacted form does not.
    let (chosen, stored) = readable_body(raw, Some("processed copy"), raw.len() as u64)
        .expect("the processed copy fits");
    assert_eq!(
        chosen, "processed copy",
        "a body whose sanitized form exceeds the read limit must not be stored, got {:?}",
        stored.value
    );

    let (kept, _) = readable_body(raw, Some("processed copy"), 10_000).expect("fits");
    assert_eq!(
        kept, raw,
        "a body that stays within the limit is stored as returned"
    );
}

#[test]
fn a_body_is_refused_when_neither_candidate_fits_the_read_limit() {
    let raw = "call +15551234567 or +15557654321";
    let fallback = "fallback +15550001111 +15550002222";
    let limit = raw.len().min(fallback.len()) as u64;
    assert!(
        readable_body(raw, Some(fallback), limit).is_err(),
        "when neither the raw body nor the fallback fits once sanitized, nothing may be stored"
    );
    assert!(
        readable_body(raw, None, limit).is_err(),
        "a body with no fallback that does not fit must not be stored either"
    );
}

#[test]
fn artifact_read_target_follows_only_use_skill_into_a_wrapped_tool() {
    let nested = json!({"tool": "file_read", "args": {"path": "artifacts/tool-results/s/c.txt"}});
    assert!(
        artifact_read_target("use_skill", &nested).is_some(),
        "use_skill forwards file_read's result, so its wrapped read counts"
    );
    for outer in ["glob", "file_write", "shell"] {
        assert_eq!(
            artifact_read_target(outer, &nested),
            None,
            "{outer} carrying tool/args fields is not a wrapper; its result is its own"
        );
    }
}

#[test]
fn a_page_near_the_maximum_offset_neither_overflows_nor_advertises_a_stuck_continuation() {
    let read = ArtifactRead {
        path: "artifacts/tool-results/s/shell/c.txt".to_string(),
        offset: usize::MAX - 10,
    };
    // The offset comes straight from the model's arguments, so the page
    // arithmetic must not overflow on an absurd one.
    let page = std::panic::catch_unwind(|| page_artifact_read("q".repeat(5_000), &read, 1_000));
    assert!(
        page.is_ok(),
        "an offset near usize::MAX must not overflow the page arithmetic"
    );
    let page = page.unwrap();
    assert!(
        page.len() <= 1_000,
        "the result is still bounded, got {} bytes",
        page.len()
    );
    assert!(
        !page.contains("Continue with"),
        "no continuation may be advertised when the next offset cannot advance"
    );
}

#[test]
fn artifact_read_target_rejects_an_explicit_invalid_offset() {
    let path = "artifacts/tool-results/s/shell/c.txt";
    for bad in [json!(-1), json!(1.5), json!("12")] {
        assert_eq!(
            artifact_read_target("file_read", &json!({"path": path, "offset": bad.clone()})),
            None,
            "offset {bad} is not a read file_read serves, so it must not become an artifact read at 0"
        );
    }
    for absent in [json!({"path": path}), json!({"path": path, "offset": null})] {
        assert_eq!(
            artifact_read_target("file_read", &absent).map(|read| read.offset),
            Some(0),
            "an absent or null offset reads from the start"
        );
    }
}
