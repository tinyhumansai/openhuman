use super::*;

#[test]
fn schema_requires_task_id() {
    let schema = WaitSubagentTool::new().parameters_schema();
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required list");
    assert!(required.is_empty());
}

#[tokio::test]
async fn missing_task_id_is_rejected() {
    let res = WaitSubagentTool::new().execute(json!({})).await.unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("subagent_session_id"));
}

#[tokio::test]
async fn outside_agent_turn_is_rejected() {
    let res = WaitSubagentTool::new()
        .execute(json!({ "task_id": "sub-1" }))
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(res.output().contains("outside of an agent turn"));
}

#[test]
fn running_wait_message_includes_agent_id_and_tick_instruction() {
    let reference = running_subagents::SubagentResumeRef {
        task_id: "sub-1".into(),
        agent_id: "researcher".into(),
        subagent_session_id: Some("subsess-1".into()),
    };
    let message = format_running_wait_message(Some(&reference), "sub-1", 1);

    assert!(message.contains("Sub-agent `researcher` is still running"));
    assert!(message.contains("[subagent_wait_result]"));
    assert!(message.contains("\"agentId\":\"researcher\""));
    assert!(message.contains("\"timeout_tick\""));
    assert!(message.contains("\"timeout_secs\":1"));
}

/// The tool must own its deadline. Under the inherited per-tool-call
/// timeout the harness killed the wait before it could report "still
/// running", so a long-but-healthy sub-agent surfaced as a tool failure.
#[test]
fn timeout_policy_is_owned_not_inherited() {
    let tool = WaitSubagentTool::new();
    assert!(matches!(
        tool.timeout_policy(&json!({})),
        ToolTimeout::Secs(DEFAULT_TIMEOUT_SECS)
    ));
    assert!(matches!(
        tool.timeout_policy(&json!({"timeout_secs": 1800})),
        ToolTimeout::Secs(1800)
    ));
}

/// The advertised wait and the harness deadline are resolved by one
/// function precisely so they cannot drift apart; if they ever did, the
/// shorter one silently wins and the graceful path is unreachable again.
#[test]
fn policy_and_wait_resolve_the_same_seconds() {
    let tool = WaitSubagentTool::new();
    for args in [
        json!({}),
        json!({"timeout_secs": 1}),
        json!({"timeout_secs": 900}),
        json!({"timeout_secs": 99_999}),
        json!({"timeout_secs": 0}),
    ] {
        let ToolTimeout::Secs(policy) = tool.timeout_policy(&args) else {
            panic!("wait_subagent must return an explicit Secs policy");
        };
        assert_eq!(policy, requested_timeout_secs(&args));
    }
}

/// Out-of-range requests clamp rather than error, so a model asking for a
/// very long wait still gets the longest legal one.
#[test]
fn requested_timeout_clamps_into_range() {
    assert_eq!(requested_timeout_secs(&json!({"timeout_secs": 0})), 1);
    assert_eq!(
        requested_timeout_secs(&json!({"timeout_secs": 99_999})),
        MAX_TIMEOUT_SECS
    );
    assert_eq!(requested_timeout_secs(&json!({})), DEFAULT_TIMEOUT_SECS);
}
