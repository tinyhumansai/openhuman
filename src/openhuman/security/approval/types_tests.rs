use super::*;

#[test]
fn approval_decision_round_trips() {
    for d in [
        ApprovalDecision::ApproveOnce,
        ApprovalDecision::ApproveAlwaysForTool,
        ApprovalDecision::ApproveAlwaysForFlow,
        ApprovalDecision::Deny,
    ] {
        assert_eq!(ApprovalDecision::from_str(d.as_str()), Some(d));
    }
}

#[test]
fn from_str_unknown_decision_is_none() {
    assert!(ApprovalDecision::from_str("maybe").is_none());
}

#[test]
fn is_approve_true_for_approval_variants_only() {
    assert!(ApprovalDecision::ApproveOnce.is_approve());
    assert!(ApprovalDecision::ApproveAlwaysForTool.is_approve());
    assert!(ApprovalDecision::ApproveAlwaysForFlow.is_approve());
    assert!(!ApprovalDecision::Deny.is_approve());
}

#[test]
fn approve_always_for_flow_serializes_as_snake_case() {
    let s = serde_json::to_string(&ApprovalDecision::ApproveAlwaysForFlow).unwrap();
    assert_eq!(s, "\"approve_always_for_flow\"");
}

#[test]
fn source_context_flow_round_trips_as_internally_tagged_json() {
    let ctx = ApprovalSourceContext::Flow {
        flow_id: "flow-1".to_string(),
        run_id: "run-1".to_string(),
        node_id: None,
    };
    let json = serde_json::to_value(&ctx).unwrap();
    assert_eq!(json["kind"], "flow");
    assert_eq!(json["flow_id"], "flow-1");
    assert_eq!(json["run_id"], "run-1");
    assert!(
        json.get("node_id").is_none(),
        "None node_id must be omitted, not null"
    );

    let back: ApprovalSourceContext = serde_json::from_value(json).unwrap();
    assert_eq!(back, ctx);
}

#[test]
fn pending_approval_source_context_defaults_to_none_and_is_omitted_when_absent() {
    let p = PendingApproval::new(
        "req-1",
        "composio",
        "send email",
        serde_json::json!({}),
        None,
    );
    assert!(p.source_context.is_none());
    let json = serde_json::to_value(&p).unwrap();
    assert!(
        json.get("source_context").is_none(),
        "absent source_context must not be serialized as null: {json}"
    );
}

#[test]
fn with_source_context_attaches_flow_context() {
    let p = PendingApproval::new(
        "req-1",
        "composio",
        "send email",
        serde_json::json!({}),
        None,
    )
    .with_source_context(ApprovalSourceContext::Flow {
        flow_id: "flow-1".to_string(),
        run_id: "run-1".to_string(),
        node_id: Some("node-1".to_string()),
    });
    match p.source_context {
        Some(ApprovalSourceContext::Flow {
            flow_id,
            run_id,
            node_id,
        }) => {
            assert_eq!(flow_id, "flow-1");
            assert_eq!(run_id, "run-1");
            assert_eq!(node_id.as_deref(), Some("node-1"));
        }
        other => panic!("expected Flow source_context, got {other:?}"),
    }
}

#[test]
fn approval_decision_serializes_as_snake_case() {
    let s = serde_json::to_string(&ApprovalDecision::ApproveAlwaysForTool).unwrap();
    assert_eq!(s, "\"approve_always_for_tool\"");
}

#[test]
fn execution_outcome_round_trips() {
    for o in [
        ExecutionOutcome::Success,
        ExecutionOutcome::Failure,
        ExecutionOutcome::Aborted,
    ] {
        assert_eq!(ExecutionOutcome::from_str(o.as_str()), Some(o));
    }
    assert!(ExecutionOutcome::from_str("partial").is_none());
}

#[test]
fn execution_outcome_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_string(&ExecutionOutcome::Success).unwrap(),
        "\"success\""
    );
    assert_eq!(
        serde_json::to_string(&ExecutionOutcome::Aborted).unwrap(),
        "\"aborted\""
    );
}

/// Regression guard. Earlier revisions of [`PendingApproval`]
/// exposed a `session_id: String` field — when an operator had
/// set the RPC bearer to a stable value, that field carried the
/// raw credential, and Debug-formatting / serializing a pending
/// row was enough to leak it. Both surfaces are exercised here.
#[test]
fn pending_approval_debug_and_serialize_do_not_carry_session_id() {
    let p = PendingApproval {
        request_id: "req-1".to_string(),
        tool_name: "composio".to_string(),
        action_summary: "send slack message".to_string(),
        args_redacted: serde_json::json!({ "tool_slug": "SLACK_SEND" }),
        created_at: Utc::now(),
        expires_at: None,
        source_context: None,
    };
    let dbg = format!("{p:?}");
    assert!(
        !dbg.contains("session_id"),
        "Debug output must not surface session_id: {dbg}"
    );
    let json = serde_json::to_value(&p).unwrap();
    assert!(
        json.get("session_id").is_none(),
        "Serialized JSON must not surface session_id: {json}"
    );

    let audit = ApprovalAuditEntry {
        request_id: "req-1".to_string(),
        tool_name: "composio".to_string(),
        action_summary: "send slack message".to_string(),
        args_redacted: serde_json::json!({ "tool_slug": "SLACK_SEND" }),
        created_at: Utc::now(),
        expires_at: None,
        decided_at: Utc::now(),
        decision: ApprovalDecision::ApproveOnce,
    };
    let audit_dbg = format!("{audit:?}");
    assert!(
        !audit_dbg.contains("session_id"),
        "ApprovalAuditEntry Debug must not surface session_id: {audit_dbg}"
    );
    let audit_json = serde_json::to_value(&audit).unwrap();
    assert!(
        audit_json.get("session_id").is_none(),
        "ApprovalAuditEntry JSON must not surface session_id: {audit_json}"
    );
}
