use super::*;
use serde_json::json;

#[test]
fn task_run_round_trips_and_reads_camel_case_wire() {
    let wire = json!({
        "taskId": "t1",
        "cycleId": "c1",
        "sessionId": "s1",
        "instruction": "summarize the doc",
        "agentId": "orchestrator",
        "timeoutMs": 60000,
    });
    let parsed: TaskRun = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(parsed.task_id, "t1");
    assert_eq!(parsed.cycle_id, "c1");
    assert_eq!(parsed.session_id.as_deref(), Some("s1"));
    assert_eq!(parsed.agent_id.as_deref(), Some("orchestrator"));
    assert_eq!(parsed.timeout_ms, 60000);
    // Re-serialize and confirm it decodes back to the same value.
    let again: TaskRun = serde_json::from_value(serde_json::to_value(&parsed).unwrap()).unwrap();
    assert_eq!(parsed, again);
}

#[test]
fn task_run_defaults_optional_fields() {
    let wire = json!({
        "taskId": "t2",
        "cycleId": "c2",
        "instruction": "go",
    });
    let parsed: TaskRun = serde_json::from_value(wire).unwrap();
    assert!(parsed.session_id.is_none());
    assert!(parsed.agent_id.is_none());
    assert_eq!(parsed.timeout_ms, 0);
}

#[test]
fn task_send_and_abort_round_trip() {
    let send: TaskSend = serde_json::from_value(json!({ "taskId": "t", "input": "yes" })).unwrap();
    assert_eq!(send.input, "yes");
    let abort: TaskAbort = serde_json::from_value(json!({ "taskId": "t" })).unwrap();
    assert_eq!(abort.task_id, "t");
}

#[test]
fn task_result_omits_none_and_round_trips() {
    let res = TaskResult {
        task_id: "t".into(),
        ok: true,
        reply: "done".into(),
        usage: None,
        error: None,
    };
    let v = serde_json::to_value(&res).unwrap();
    assert!(v.get("usage").is_none());
    assert!(v.get("error").is_none());
    assert_eq!(v["taskId"], "t");
    assert_eq!(v["ok"], true);
}

#[test]
fn register_agents_advertises_the_id_key_the_backend_validates() {
    let roster = RegisterAgents {
        agents: vec![AgentDescriptor {
            id: "orchestrator".into(),
            name: "Orchestrator".into(),
            description: "default".into(),
        }],
    };
    let wire = serde_json::to_value(&roster).unwrap();
    // `agentRegistry.hasValidId` keys on `id`; an `agentId` key here would
    // make the whole roster vanish server-side.
    assert_eq!(wire["agents"][0]["id"], "orchestrator");
    assert!(wire["agents"][0].get("agentId").is_none());
    let back: RegisterAgents = serde_json::from_value(wire).unwrap();
    assert_eq!(roster, back);
}

#[test]
fn workflow_descriptor_advertises_declared_inputs_and_omits_them_when_none() {
    // The reader needs to know what to collect before asking for a run;
    // without this it would have to fetch the whole graph to find out.
    let advert = WorkflowDescriptor {
        id: "wf-1".into(),
        name: "Review".into(),
        description: String::new(),
        node_count: 2,
        enabled: Some(true),
        trigger_kind: None,
        agent_id: None,
        workspace_id: None,
        inputs: vec![WorkflowInputDescriptor {
            name: "repo".into(),
            ty: "string".into(),
            description: String::new(),
            required: true,
            default: None,
        }],
    };
    let wire = serde_json::to_value(&advert).unwrap();
    assert_eq!(wire["inputs"][0]["name"], "repo");
    assert_eq!(wire["inputs"][0]["type"], "string");
    assert_eq!(wire["inputs"][0]["required"], true);

    let none = WorkflowDescriptor {
        inputs: Vec::new(),
        ..advert
    };
    let wire = serde_json::to_value(&none).unwrap();
    assert!(
        wire.get("inputs").is_none(),
        "a workflow taking no inputs must not send an empty key"
    );
}

#[test]
fn workflow_descriptor_omits_blank_name_and_description() {
    let advert = WorkflowDescriptor {
        id: "wf-1".into(),
        name: String::new(),
        description: String::new(),
        node_count: 5,
        enabled: Some(true),
        trigger_kind: Some("cron".into()),
        agent_id: None,
        workspace_id: None,
        inputs: Vec::new(),
    };
    let wire = serde_json::to_value(&advert).unwrap();
    assert_eq!(wire["id"], "wf-1");
    assert_eq!(wire["nodeCount"], 5);
    assert_eq!(wire["enabled"], true);
    assert_eq!(wire["triggerKind"], "cron");
    // Absent, never `""` — the port declares these optional precisely
    // because the wire omits them.
    assert!(wire.get("name").is_none());
    assert!(wire.get("description").is_none());
    assert!(wire.get("agentId").is_none());
    let back: WorkflowDescriptor = serde_json::from_value(wire).unwrap();
    assert_eq!(advert, back);
}

#[test]
fn register_workflows_omits_absent_batch_agent_id() {
    let batch = RegisterWorkflows {
        workflows: vec![],
        agent_id: None,
    };
    let wire = serde_json::to_value(&batch).unwrap();
    assert!(wire["workflows"].as_array().unwrap().is_empty());
    assert!(wire.get("agentId").is_none());
}

#[test]
fn workflow_request_reads_every_op_from_the_wire() {
    for (wire_op, expected) in [
        ("get", WorkflowOp::Get),
        ("node_kinds", WorkflowOp::NodeKinds),
        ("runs", WorkflowOp::Runs),
        ("copilot", WorkflowOp::Copilot),
    ] {
        let parsed: WorkflowRequest =
            serde_json::from_value(json!({ "requestId": "r1", "op": wire_op })).unwrap();
        assert_eq!(parsed.op, expected);
        assert_eq!(parsed.request_id, "r1");
        assert!(parsed.workflow_id.is_none());
    }
    // An op this build does not know is a decode error, not a silent drop.
    assert!(serde_json::from_value::<WorkflowRequest>(
        json!({ "requestId": "r1", "op": "apply_ops" })
    )
    .is_err());
}

#[test]
fn workflow_request_reads_the_op_specific_fields() {
    let parsed: WorkflowRequest = serde_json::from_value(json!({
        "requestId": "r2",
        "op": "copilot",
        "instruction": "add a slack step",
        "workflowId": "wf-1",
        "agentId": "orchestrator",
    }))
    .unwrap();
    assert_eq!(parsed.op, WorkflowOp::Copilot);
    assert_eq!(parsed.instruction.as_deref(), Some("add a slack step"));
    assert_eq!(parsed.workflow_id.as_deref(), Some("wf-1"));
    assert_eq!(parsed.agent_id.as_deref(), Some("orchestrator"));
}

#[test]
fn workflow_result_omits_the_unused_arm() {
    let ok = WorkflowResult {
        request_id: "r".into(),
        ok: true,
        data: Some(json!({ "graph": [] })),
        error: None,
    };
    let wire = serde_json::to_value(&ok).unwrap();
    assert_eq!(wire["requestId"], "r");
    assert_eq!(wire["ok"], true);
    assert!(wire.get("error").is_none());

    let failed = WorkflowResult {
        request_id: "r".into(),
        ok: false,
        data: None,
        error: Some("unknown workflow".into()),
    };
    let wire = serde_json::to_value(&failed).unwrap();
    assert_eq!(wire["ok"], false);
    assert_eq!(wire["error"], "unknown workflow");
    assert!(wire.get("data").is_none());
}

#[test]
fn copilot_outcome_matches_the_library_shape() {
    let outcome = CopilotOutcome {
        reply: "added the step".into(),
        changes: vec!["node:slack added".into()],
        created: None,
    };
    let wire = serde_json::to_value(&outcome).unwrap();
    assert_eq!(wire["reply"], "added the step");
    assert_eq!(wire["changes"][0], "node:slack added");
    assert!(wire.get("created").is_none());
}

#[test]
fn capabilities_request_defaults_a_missing_agent_id() {
    let parsed: CapabilitiesRequest = serde_json::from_value(json!({ "probeId": "p1" })).unwrap();
    assert_eq!(parsed.probe_id, "p1");
    assert!(parsed.agent_id.is_empty());
    let wire = serde_json::to_value(CapabilitiesResult {
        probe_id: "p1".into(),
        capabilities: json!({ "ready": true }),
    })
    .unwrap();
    assert_eq!(wire["probeId"], "p1");
    assert_eq!(wire["capabilities"]["ready"], true);
}
