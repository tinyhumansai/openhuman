use super::*;

#[test]
fn workflow_scan_permits_bind_workflow_inputs_and_are_single_use() {
    let host = OpenHumanRunContext::new();
    let inputs = serde_json::json!({"target": "repo"});
    host.record_workflow_scan_permit("workflow", &inputs, "call-1", "digest-a");

    assert!(!host.take_workflow_scan_permit("workflow", &inputs, "call-1", "digest-b"));
    assert!(!host.take_workflow_scan_permit(
        "workflow",
        &serde_json::json!({"target": "other"}),
        "call-1",
        "digest-a"
    ));
    assert!(!host.take_workflow_scan_permit("workflow", &inputs, "call-2", "digest-a"));
    assert!(host.take_workflow_scan_permit("workflow", &inputs, "call-1", "digest-a"));
    assert!(!host.take_workflow_scan_permit("workflow", &inputs, "call-1", "digest-a"));
}

#[test]
fn workflow_scan_permit_returns_the_bound_digest_once() {
    let host = OpenHumanRunContext::new();
    let inputs = serde_json::json!({"target": "repo"});
    host.record_workflow_scan_permit("workflow", &inputs, "call-1", "digest-a");

    assert_eq!(
        host.take_workflow_scan_permit_digest("workflow", &inputs, "call-1")
            .as_deref(),
        Some("digest-a")
    );
    assert_eq!(
        host.take_workflow_scan_permit_digest("workflow", &inputs, "call-1"),
        None
    );
}

#[test]
fn workflow_scan_permit_reaches_the_executing_tool_by_call_id() {
    let host = OpenHumanRunContext::new();
    let inputs = serde_json::json!({"target": "repo"});
    host.record_workflow_scan_permit("workflow", &inputs, "call-1", "digest-a");
    let run = host.into_tinyagents(tinyagents_harness::context::RunConfig::new(
        "workflow-permit",
    ));
    let execution = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("call-1".to_owned()),
    );
    let context: &dyn tinytools::ToolRunContext = &execution;

    assert_eq!(
        crate::agent::tinyagents::host::operation_context::take_workflow_scan_permit_for_tool(
            Some(context),
            "workflow",
            &inputs,
        )
        .as_deref(),
        Some("digest-a")
    );
    assert_eq!(
        crate::agent::tinyagents::host::operation_context::take_workflow_scan_permit_for_tool(
            Some(context),
            "workflow",
            &inputs,
        ),
        None
    );
}
