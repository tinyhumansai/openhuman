//! What the gate hands the engine, and what it does with the engine's answers.

use super::*;
use crate::security::approval::engine::{ApprovalEngine, Choice};
use crate::security::approval::engine_stub::State;

/// Park one call on `gate` under `origin` and return the request id once the
/// engine delivered its prompt.
async fn park(
    gate: &Arc<ApprovalGate>,
    origin: AgentTurnOrigin,
    turn: ApprovalTurnContext,
    tool: &'static str,
) -> (tokio::task::JoinHandle<GateOutcome>, String) {
    let worker = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            origin,
            worker.intercept(&turn, tool, "do the thing", serde_json::json!({"k": "v"})),
        )
        .await
    });
    let mut tries = 0;
    let request_id = loop {
        if let Some(id) = parked_request_id(gate) {
            break id;
        }
        tries += 1;
        assert!(tries < 500, "the call never parked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    (handle, request_id)
}

#[tokio::test]
async fn an_engine_that_cannot_answer_denies_and_leaves_nothing_parked() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let stub = stub_of(&gate);
    stub.refuse_checks(true);
    let outcome = turn_origin::with_origin(
        web_origin(),
        gate.intercept(
            &ApprovalTurnContext::chat(chat_ctx()),
            "composio",
            "send",
            serde_json::json!({}),
        ),
    )
    .await;
    match outcome {
        GateOutcome::Deny { reason } => {
            assert!(reason.contains("could not evaluate"), "{reason}");
            assert!(reason.contains("composio"), "{reason}");
        }
        other => panic!("an engine that cannot answer must deny, got {other:?}"),
    }
    assert!(gate.parked.waiters.lock().is_empty());
    assert!(gate.pending_for_thread("t-test").is_none());
    assert!(gate.list_pending().unwrap().is_empty());
}

#[tokio::test]
async fn native_secret_grant_approval_is_left_for_execute_secret2_to_consume() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let worker = Arc::clone(&gate);
    let turn = ApprovalTurnContext::chat(chat_ctx());
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            worker.intercept_audited_for_call(
                &turn,
                "native_secret_grant",
                "Unlock configuration secrets",
                serde_json::json!({"registration_id": "opaque-registration"}),
                None,
            ),
        )
        .await
    });
    let mut tries = 0;
    let request_id = loop {
        if let Some(id) = parked_request_id(&gate) {
            break id;
        }
        tries += 1;
        assert!(tries < 500, "the master grant never parked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    decide_parked(&gate, &request_id, ApprovalDecision::ApproveOnce);

    let (outcome, reported) = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
    assert_eq!(reported, None, "the native module owns grant consumption");
    assert_eq!(stub_of(&gate).state(&request_id), Some(State::Approved));
}

#[tokio::test]
async fn the_users_configuration_reaches_the_engine_as_the_calls_posture() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let gate = Arc::new(gate);
    let tool = "openhuman_test_posture_tool";
    let _policy = crate::security::live_policy::install_scoped(
        Arc::new(crate::security::SecurityPolicy {
            auto_approve: vec![tool.into(), "another_granted_tool".into()],
            ..crate::security::SecurityPolicy::default()
        }),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );
    let (handle, request_id) = park(
        &gate,
        web_origin(),
        ApprovalTurnContext::chat(chat_ctx()),
        "not_on_the_list",
    )
    .await;
    let seen = stub_of(&gate).seen().remove(0);
    assert_eq!(seen.variant, Variant::Standard);
    assert!(seen.posture.require_approval);
    assert!(!seen.posture.auto_approve_all);
    assert!(
        seen.legacy_tool_grants.contains(&tool.to_string()),
        "legacy grants travel only as one-time import data, outside the posture"
    );
    assert_eq!(seen.tool, "not_on_the_list");
    assert_eq!(seen.origin, CallOrigin::Operator);
    assert_eq!(seen.thread.as_deref(), Some("t-test"));
    assert!(seen.flow.is_none());
    assert_eq!(seen.call_id.len(), 8, "{}", seen.call_id);
    assert!(seen.call_id.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(seen.ttl, None, "the module owns the default window");
    assert_eq!(seen.args["k"], "v");
    decide_parked(&gate, &request_id, ApprovalDecision::Deny);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Deny { .. }));
}

#[tokio::test]
async fn an_agent_whose_approvals_are_off_reaches_the_engine_as_no_requirement() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    // The agent's own context says approvals are off; see gate_agent_tests for how
    // such a context is built. The posture it yields is what the engine is told.
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let posture = gate.posture(&ctx, Variant::Standard);
    assert!(
        posture.require_approval,
        "the default agent requires approval"
    );
    let forced = gate.posture(&ctx, Variant::Forced);
    assert!(forced.require_approval);
    assert!(!forced.auto_approve_all);
}

#[tokio::test]
async fn a_forced_call_is_its_own_posture_and_ignores_every_shortcut() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let gate = Arc::new(gate);
    let tool = "openhuman_test_forced_posture";
    let _policy = crate::security::live_policy::install_scoped(
        Arc::new(crate::security::SecurityPolicy {
            auto_approve: vec![tool.into()],
            auto_approve_all: true,
            ..crate::security::SecurityPolicy::default()
        }),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );
    let worker = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            worker.intercept_forced(tool, "one time only", serde_json::json!({})),
        )
        .await
    });
    let mut tries = 0;
    let request_id = loop {
        if let Some(id) = parked_request_id(&gate) {
            break id;
        }
        tries += 1;
        assert!(tries < 500, "the forced call never parked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let seen = stub_of(&gate).seen().remove(0);
    assert_eq!(seen.variant, Variant::Forced);
    assert!(seen.posture.require_approval);
    assert!(!seen.posture.auto_approve_all, "auto-approve is ignored");
    assert!(
        seen.legacy_tool_grants.is_empty(),
        "forced decisions do not import legacy grants"
    );
    decide_parked(&gate, &request_id, ApprovalDecision::ApproveOnce);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));
}

#[tokio::test]
async fn a_workflow_that_requires_approval_is_never_approved_by_the_global_tool_list() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let gate = Arc::new(gate);
    let tool = "openhuman_test_flow_strict";
    let _policy = crate::security::live_policy::install_scoped(
        Arc::new(crate::security::SecurityPolicy {
            auto_approve: vec![tool.into()],
            ..crate::security::SecurityPolicy::default()
        }),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );
    gate.insert_flow_trust("flow-strict", "other_trusted_tool")
        .unwrap();

    let (handle, request_id) = park(
        &gate,
        flow_origin("flow-strict", true),
        ApprovalTurnContext::default().with_flow_run(FlowRunContext {
            flow_id: "flow-strict".into(),
            run_id: "run-1".into(),
        }),
        tool,
    )
    .await;
    let seen = stub_of(&gate).seen().remove(0);
    assert_eq!(seen.variant, Variant::FlowStrict);
    assert!(
        seen.legacy_tool_grants.is_empty(),
        "strict-flow calls do not import global legacy grants"
    );
    assert_eq!(seen.flow.as_deref(), Some("flow-strict"));
    // The flow's own trust is the engine's durable state, not the posture's.
    assert_eq!(
        gate.list_flow_trust("flow-strict").unwrap(),
        ["other_trusted_tool"]
    );
    decide_parked(&gate, &request_id, ApprovalDecision::Deny);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Deny { .. }));
}

#[tokio::test]
async fn copilot_and_sub_agent_parks_name_their_origin_class_and_window() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let (handle, request_id) = park(
        &gate,
        web_origin(),
        ApprovalTurnContext::chat(chat_ctx()).with_copilot_stream(),
        "composio",
    )
    .await;
    let seen = stub_of(&gate).seen().remove(0);
    assert_eq!(seen.origin, CallOrigin::Copilot);
    assert_eq!(seen.ttl, None);
    decide_parked(&gate, &request_id, ApprovalDecision::Deny);
    handle.await.unwrap();

    // A WebChat turn with no chat context is the detached sub-agent path.
    let (handle, request_id) = park(
        &gate,
        web_origin(),
        ApprovalTurnContext::default(),
        "composio",
    )
    .await;
    let seen = stub_of(&gate).seen().pop().unwrap();
    assert_eq!(seen.origin, CallOrigin::SubAgent);
    assert_eq!(seen.ttl, None);
    decide_parked(&gate, &request_id, ApprovalDecision::Deny);
    handle.await.unwrap();
}

#[tokio::test]
async fn an_unlabelled_call_is_approved_only_by_the_users_grant_of_that_exact_tool() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, _dir) = test_gate();
    let tool = "openhuman_test_unlabelled_grant";
    gate.grant_tool_trust(
        &crate::core::runtime::Tenant::default(),
        Variant::Standard,
        tool,
    )
    .unwrap();
    let _policy = crate::security::live_policy::install_scoped(
        Arc::new(crate::security::SecurityPolicy {
            auto_approve_all: true,
            ..crate::security::SecurityPolicy::default()
        }),
        std::env::temp_dir(),
        std::env::temp_dir(),
    );
    let (outcome, request_id) = gate
        .intercept_audited(
            &ApprovalTurnContext::default(),
            tool,
            "no origin",
            serde_json::json!({}),
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    let stub = stub_of(&gate);
    assert_eq!(
        stub.attribution(&request_id.unwrap()),
        Some("auto:tool-grant"),
        "the grant approved it, not auto-approve-everything"
    );
    // The gate carries no legacy grant in its posture; TinySecurity's durable
    // whole-tool state is the only reason an unlabelled call is approved.
    let seen = stub.seen().remove(0);
    assert_eq!(seen.origin, CallOrigin::Unlabelled);
    assert!(!seen.legacy_tool_grants.contains(&tool.to_string()));

    // Auto-approve-everything does not approve an unlabelled call to any other tool.
    let (outcome, id) = gate
        .intercept_audited(
            &ApprovalTurnContext::default(),
            "some_other_tool",
            "no origin",
            serde_json::json!({}),
        )
        .await;
    assert!(
        matches!(&outcome, GateOutcome::Deny { reason } if reason.contains("missing its origin label")),
        "{outcome:?}"
    );
    assert!(id.is_none());
    let asked = stub.seen();
    assert_eq!(asked.len(), 2, "the engine decides, not the gate");
    assert_eq!(asked[1].origin, CallOrigin::Unlabelled);
    assert!(!asked[1]
        .legacy_tool_grants
        .iter()
        .any(|t| t == "some_other_tool"));
}

#[tokio::test]
async fn an_audited_call_is_reserved_and_an_unaudited_one_is_not() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let tool = "openhuman_test_reserve_tool";
    let _policy = crate::security::live_policy::install_scoped(
        Arc::new(crate::security::SecurityPolicy {
            auto_approve_all: true,
            ..crate::security::SecurityPolicy::default()
        }),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );
    let stub = stub_of(&gate);

    let (outcome, id) = turn_origin::with_origin(
        web_origin(),
        gate.intercept_audited(
            &ApprovalTurnContext::default(),
            tool,
            "audited",
            serde_json::json!({}),
        ),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow));
    let id = id.expect("an audited call hands back its request id");
    assert_eq!(stub.state(&id), Some(State::Reserved));
    gate.record_execution(&id, ExecutionOutcome::Failure, Some("boom"));
    assert_eq!(
        stub.state(&id),
        Some(State::Executed(ExecutionOutcome::Failure))
    );

    // The unaudited entry points never learn the id, so nothing is reserved that
    // could never be closed.
    let outcome = turn_origin::with_origin(
        web_origin(),
        gate.intercept(
            &ApprovalTurnContext::default(),
            tool,
            "unaudited",
            serde_json::json!({}),
        ),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow));
    let granted: Vec<String> = stub
        .request_ids()
        .into_iter()
        .filter(|request| request != &id)
        .collect();
    assert_eq!(granted.len(), 1);
    assert_eq!(stub.state(&granted[0]), Some(State::Approved));
}

#[tokio::test]
async fn a_call_that_cannot_be_reserved_is_denied() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let _policy = crate::security::live_policy::install_scoped(
        Arc::new(crate::security::SecurityPolicy {
            auto_approve_all: true,
            ..crate::security::SecurityPolicy::default()
        }),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );
    stub_of(&gate).fail_reserve(true);
    let (outcome, id) = turn_origin::with_origin(
        web_origin(),
        gate.intercept_audited(
            &ApprovalTurnContext::default(),
            "openhuman_test_unreservable",
            "approved but not reservable",
            serde_json::json!({}),
        ),
    )
    .await;
    match outcome {
        GateOutcome::Deny { reason } => {
            assert!(reason.contains("could not be reserved"), "{reason}")
        }
        other => panic!("an approval that cannot be reserved must deny, got {other:?}"),
    }
    assert!(id.is_none());
}

#[tokio::test]
async fn an_agent_removed_before_the_approved_call_runs_cancels_it() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    stub.leave_pending(
        "approved-then-removed",
        "composio",
        Duration::from_secs(600),
    );
    stub.decide("approved-then-removed", Choice::Once, "approve_once")
        .await
        .unwrap();
    let scope = crate::security::approval::ApprovalScope::default();
    scope.close("agent_removed");
    let (outcome, id) = gate
        .reserved(
            "approved-then-removed".into(),
            "composio",
            Some(&scope),
            true,
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }));
    assert!(id.is_none());
    assert_eq!(stub.state("approved-then-removed"), Some(State::Cancelled));
}

#[tokio::test]
async fn a_request_that_expires_while_its_decision_races_it_honors_the_decision() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    stub.leave_pending("raced", "composio", Duration::from_secs(600));
    stub.decide("raced", Choice::Once, "approve_once")
        .await
        .unwrap();
    // The window ended just as the decision committed: expiry finds it decided.
    assert!(!gate.expire_with_retry("raced").await.unwrap());
    let (outcome, id) = {
        let (_tx, rx) = oneshot::channel::<ApprovalDecision>();
        let mut elapsed = false;
        gate.resolve_park(
            "raced",
            "composio",
            Duration::from_millis(5),
            rx,
            false,
            &mut elapsed,
            Duration::from_secs(600),
            None,
            true,
        )
        .await
    };
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    assert_eq!(id.as_deref(), Some("raced"));

    // An untracked caller cannot report how it ended, so the race is denied.
    stub.leave_pending("raced-untracked", "composio", Duration::from_secs(600));
    stub.decide("raced-untracked", Choice::Once, "approve_once")
        .await
        .unwrap();
    let (_tx, rx) = oneshot::channel::<ApprovalDecision>();
    let mut elapsed = false;
    let (outcome, _) = gate
        .resolve_park(
            "raced-untracked",
            "composio",
            Duration::from_millis(5),
            rx,
            false,
            &mut elapsed,
            Duration::from_secs(600),
            None,
            false,
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
}

#[tokio::test]
async fn a_request_the_engine_no_longer_holds_is_denied_when_its_window_ends() {
    let (gate, _dir) = test_gate();
    // The engine does not know this request, so there is nothing to honor; the
    // gate fails closed.
    let (_tx, rx) = oneshot::channel::<ApprovalDecision>();
    let mut elapsed = false;
    let (outcome, id) = gate
        .resolve_park(
            "unknown-to-the-engine",
            "composio",
            Duration::from_millis(5),
            rx,
            false,
            &mut elapsed,
            Duration::from_secs(600),
            None,
            true,
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
    assert!(id.is_none());
}

#[test]
fn arguments_are_always_a_bounded_object() {
    assert_eq!(
        object_arguments(serde_json::json!({"a": 1})),
        serde_json::json!({"a": 1})
    );
    assert_eq!(
        object_arguments(serde_json::Value::Null),
        serde_json::json!({})
    );
    assert_eq!(
        object_arguments(serde_json::json!(["x"])),
        serde_json::json!({ "value": ["x"] })
    );
    let huge = serde_json::json!({ "blob": "x".repeat(MAX_ARGUMENT_BYTES + 1) });
    let stood_in = object_arguments(huge);
    assert_eq!(stood_in["truncated"], true);
    assert!(stood_in["bytes"].as_u64().unwrap() > MAX_ARGUMENT_BYTES as u64);
}

#[test]
fn a_call_correlation_is_eight_hex_characters_and_not_repeated() {
    let ids: std::collections::HashSet<String> = (0..64).map(|_| fresh_call_id()).collect();
    assert!(ids.len() > 60, "random correlations should not collide");
    assert!(ids
        .iter()
        .all(|id| id.len() == 8 && id.bytes().all(|b| b.is_ascii_hexdigit())));
}

#[tokio::test]
async fn pending_requests_are_listed_from_this_process_only_and_by_tenant() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let stub = stub_of(&gate);
    // A request an earlier launch left pending was never parked here.
    stub.leave_pending("orphan", "composio", Duration::from_secs(600));
    let (handle, request_id) = park(
        &gate,
        web_origin(),
        ApprovalTurnContext::chat(chat_ctx()),
        "composio",
    )
    .await;
    let listed = gate.list_pending().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].request_id, request_id);
    assert_eq!(listed[0].tool_name, "composio");
    assert_eq!(listed[0].args_redacted["k"], "v");
    assert!(listed[0].expires_at.is_some());
    assert_eq!(
        gate.parked_request_for_thread("t-test")
            .map(|row| row.request_id),
        Some(request_id.clone())
    );
    // Deciding the orphan finds nothing to resolve: it was never parked here.
    assert!(gate
        .decide("orphan", ApprovalDecision::ApproveOnce)
        .unwrap()
        .is_none());
    decide_parked(&gate, &request_id, ApprovalDecision::Deny);
    handle.await.unwrap();
    assert!(gate.list_pending().unwrap().is_empty());
    // The decision is history, newest first, with what the user chose.
    let history = gate.list_recent_decisions(10).unwrap();
    assert_eq!(history[0].request_id, request_id);
    assert_eq!(history[0].decision, ApprovalDecision::Deny);
}

#[tokio::test]
async fn a_saas_call_the_deployment_allows_is_recorded_as_an_automatic_approval() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    let (outcome, id) = gate
        .saas_approved(
            "shell",
            "run ls",
            serde_json::json!({"cmd": "ls"}),
            true,
            CallOrigin::Operator,
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    let id = id.expect("a tracked SaaS call hands back its request id");
    assert_eq!(stub.state(&id), Some(State::Reserved));
    assert_eq!(stub.attribution(&id), Some("auto:host-policy"));
    let seen = stub.seen().remove(0);
    assert!(
        !seen.posture.require_approval,
        "the deployment is the pre-authorization"
    );
    assert!(seen.thread.is_none() && seen.flow.is_none());
    assert_eq!(seen.origin, CallOrigin::Operator);

    // An untracked caller gets the same approval without a reservation.
    let (outcome, id) = gate
        .saas_approved(
            "shell",
            "run ls",
            serde_json::json!({}),
            false,
            CallOrigin::Operator,
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(id.is_none());
}

#[tokio::test]
async fn the_saas_automatic_path_refuses_when_the_engine_needs_a_person() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    // An engine that insists on a person gets a surface that refuses to ask one.
    stub.always_pending(true);
    let (outcome, id) = gate
        .saas_approved(
            "shell",
            "run ls",
            serde_json::json!({}),
            true,
            CallOrigin::Operator,
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
    assert!(id.is_none());
    assert!(gate.parked.waiters.lock().is_empty());

    stub.always_pending(false);
    stub.refuse_checks(true);
    let (outcome, _) = gate
        .saas_approved(
            "shell",
            "run ls",
            serde_json::json!({}),
            true,
            CallOrigin::Operator,
        )
        .await;
    assert!(
        matches!(&outcome, GateOutcome::Deny { reason } if reason.contains("could not record")),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_scheduled_job_stays_refused_in_saas_whatever_the_tool_groups_allow() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    let (outcome, id) = gate
        .saas_approved(
            "shell",
            "run ls",
            serde_json::json!({}),
            true,
            CallOrigin::Cron,
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
    assert!(id.is_none());
    assert_eq!(stub.seen().remove(0).origin, CallOrigin::Cron);
}

#[tokio::test]
async fn flow_trust_granted_through_the_gate_is_the_engines_and_revocable() {
    let (gate, _dir) = test_gate();
    gate.insert_flow_trust("flow-1", "shell").unwrap();
    gate.insert_flow_trust("flow-1", "http_request").unwrap();
    gate.insert_flow_trust("flow-2", "shell").unwrap();
    assert!(gate.is_flow_tool_trusted("flow-1", "shell").unwrap());
    assert!(!gate.is_flow_tool_trusted("flow-2", "http_request").unwrap());
    assert_eq!(
        gate.list_flow_trust("flow-1").unwrap(),
        ["http_request", "shell"]
    );
    assert_eq!(
        gate.delete_flow_trust("flow-1", Some(&["shell".to_string(), "nope".to_string()]))
            .unwrap(),
        1
    );
    assert_eq!(gate.delete_flow_trust("flow-1", None).unwrap(), 1);
    assert!(gate.list_flow_trust("flow-1").unwrap().is_empty());
    assert_eq!(gate.list_flow_trust("flow-2").unwrap(), ["shell"]);
}
