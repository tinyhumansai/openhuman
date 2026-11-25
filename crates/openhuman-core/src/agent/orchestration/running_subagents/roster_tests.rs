use super::format_roster;
use tinyagents_orchestration::subagent::{
    DurableSubagentSessionSummary, DurableSubagentStatus, ParentRosterEntry,
};

#[test]
fn roster_uses_product_guidance_and_shows_live_and_durable_children() {
    let summary = DurableSubagentSessionSummary {
        subagent_session_id: "session-1".into(),
        parent_thread_id: Some("thread-1".into()),
        worker_thread_id: None,
        agent_id: "researcher".into(),
        display_name: None,
        model: None,
        sandbox_mode: "workspace".into(),
        action_root: None,
        task_key: "review-api".into(),
        task_title: "Review the API".into(),
        current_task_id: Some("task-1".into()),
        status: DurableSubagentStatus::Running,
        reusable: true,
        latest_error: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        last_used_at: "2026-01-01T00:00:00Z".into(),
    };
    let entries = [ParentRosterEntry {
        task_id: "task-1".into(),
        agent_id: "researcher".into(),
        live_job: None,
        session: Some(summary),
    }];

    let rendered = format_roster(&entries);

    assert!(rendered.contains("task=task-1"));
    assert!(rendered.contains("status=running"));
    assert!(rendered.contains("reuse its task_key with spawn_subagent"));
    assert!(!rendered.contains("wait_subagent"));
    assert!(!rendered.contains("steer_subagent"));
}
