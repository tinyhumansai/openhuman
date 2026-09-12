use super::*;

#[test]
fn schema_has_no_required_fields() {
    let schema = ListSubagentsTool::new().parameters_schema();
    assert!(schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required")
        .is_empty());
}

#[test]
fn summary_projection_does_not_include_history() {
    let raw = serde_json::to_string(&DurableSubagentSessionSummary {
        subagent_session_id: "subsess-1".into(),
        parent_thread_id: Some("thread-1".into()),
        worker_thread_id: Some("worker-1".into()),
        agent_id: "researcher".into(),
        display_name: Some("Researcher".into()),
        toolkit: None,
        model: Some("agentic-v1".into()),
        sandbox_mode: "workspace".into(),
        action_root: None,
        task_key: "task".into(),
        task_title: "Task".into(),
        current_task_id: Some("sub-1".into()),
        status: subagent_sessions::DurableSubagentStatus::Idle,
        reusable: true,
        latest_error: None,
        created_at: "now".into(),
        updated_at: "now".into(),
        last_used_at: "now".into(),
    })
    .unwrap();
    assert!(!raw.contains("latestHistory"));
}
