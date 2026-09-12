use super::*;
use chrono::{TimeZone, Utc};
use serde_json::json;

fn run_with(id: &str, status: AgentRunStatus, updated_secs: i64) -> AgentRun {
    AgentRun {
        id: id.to_string(),
        kind: tinyagents_session::run_ledger::AgentRunKind::Subagent,
        parent_run_id: None,
        parent_thread_id: Some("thread-1".to_string()),
        agent_id: Some("researcher".to_string()),
        status,
        prompt_ref: None,
        worker_thread_id: None,
        task_board_id: None,
        task_card_id: None,
        checkpoint_path: None,
        checkpoint: None,
        summary: None,
        error: None,
        metadata: json!({}),
        telemetry: None,
        started_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        updated_at: Utc.timestamp_opt(1_700_000_000 + updated_secs, 0).unwrap(),
        completed_at: None,
    }
}

#[test]
fn bucket_for_maps_every_status_to_its_group() {
    assert_eq!(
        bucket_for(AgentRunStatus::AwaitingUser),
        AgentWorkBucket::NeedsInput
    );
    assert_eq!(
        bucket_for(AgentRunStatus::Pending),
        AgentWorkBucket::Working
    );
    assert_eq!(
        bucket_for(AgentRunStatus::Running),
        AgentWorkBucket::Working
    );
    assert_eq!(bucket_for(AgentRunStatus::Paused), AgentWorkBucket::Working);
    assert_eq!(
        bucket_for(AgentRunStatus::Completed),
        AgentWorkBucket::Completed
    );
    assert_eq!(bucket_for(AgentRunStatus::Failed), AgentWorkBucket::Failed);
    assert_eq!(
        bucket_for(AgentRunStatus::Cancelled),
        AgentWorkBucket::Stopped
    );
    assert_eq!(
        bucket_for(AgentRunStatus::Interrupted),
        AgentWorkBucket::Stopped
    );
}

#[test]
fn build_view_always_emits_five_buckets_in_display_order() {
    let view = build_view(vec![]);
    assert_eq!(view.total, 0);
    let order: Vec<AgentWorkBucket> = view.groups.iter().map(|g| g.bucket).collect();
    assert_eq!(order, AgentWorkBucket::ALL.to_vec());
    assert!(view
        .groups
        .iter()
        .all(|g| g.rows.is_empty() && g.count == 0));
}

#[test]
fn build_view_groups_runs_into_correct_buckets() {
    let runs = vec![
        run_with("a", AgentRunStatus::Running, 1),
        run_with("b", AgentRunStatus::AwaitingUser, 2),
        run_with("c", AgentRunStatus::Completed, 3),
        run_with("d", AgentRunStatus::Failed, 4),
        run_with("e", AgentRunStatus::Cancelled, 5),
        run_with("f", AgentRunStatus::Pending, 6),
    ];
    let view = build_view(runs);
    assert_eq!(view.total, 6);

    let group = |bucket: AgentWorkBucket| {
        view.groups
            .iter()
            .find(|g| g.bucket == bucket)
            .expect("bucket present")
    };
    assert_eq!(group(AgentWorkBucket::NeedsInput).count, 1);
    assert_eq!(group(AgentWorkBucket::Working).count, 2); // running + pending
    assert_eq!(group(AgentWorkBucket::Completed).count, 1);
    assert_eq!(group(AgentWorkBucket::Failed).count, 1);
    assert_eq!(group(AgentWorkBucket::Stopped).count, 1);
}

#[test]
fn build_view_preserves_input_order_within_a_bucket() {
    // Caller passes recent-first; projection must not reorder.
    let runs = vec![
        run_with("newest", AgentRunStatus::Running, 30),
        run_with("middle", AgentRunStatus::Running, 20),
        run_with("oldest", AgentRunStatus::Running, 10),
    ];
    let view = build_view(runs);
    let working = view
        .groups
        .iter()
        .find(|g| g.bucket == AgentWorkBucket::Working)
        .unwrap();
    let ids: Vec<&str> = working.rows.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec!["newest", "middle", "oldest"]);
}

#[test]
fn project_row_defaults_telemetry_to_zero_when_absent() {
    let view = build_view(vec![run_with("x", AgentRunStatus::Completed, 1)]);
    let row = view
        .groups
        .iter()
        .flat_map(|g| &g.rows)
        .find(|r| r.run_id == "x")
        .unwrap();
    assert_eq!(row.input_tokens, 0);
    assert_eq!(row.output_tokens, 0);
    assert_eq!(row.cost_usd, 0.0);
    assert_eq!(row.tool_count, 0);
    assert_eq!(row.elapsed_ms, None);
    assert_eq!(row.status, "completed");
    assert_eq!(row.kind, "subagent");
}
