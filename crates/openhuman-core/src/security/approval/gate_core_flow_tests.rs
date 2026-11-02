use super::*;
use crate::security::approval::engine::{ApprovalEngine, Choice};
use crate::security::approval::engine_stub::State;

#[test]
fn guard_cleanup_only_clears_routing_it_still_owns() {
    // Regression for #4774: on external turn teardown a replacement turn may
    // have already parked a new approval on the same thread and
    // overwritten the routing entry. The dropped guard for the *old* request
    // must not clobber the *new* request's mapping.
    let (gate, _dir) = test_gate();

    gate.parked
        .thread_to_request
        .lock()
        .insert("thread-1".into(), "req-new".into());

    // Stale guard for the superseded request is a no-op.
    gate.parked
        .clear_thread_route_if_owned("thread-1", "req-old");
    assert_eq!(
        gate.pending_for_thread("thread-1").as_deref(),
        Some("req-new")
    );

    // The owning request's guard clears its own routing.
    gate.parked
        .clear_thread_route_if_owned("thread-1", "req-new");
    assert!(gate.pending_for_thread("thread-1").is_none());
}

#[tokio::test]
async fn approve_once_returns_allow() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                "composio",
                "send slack",
                serde_json::json!({}),
            ),
        )
        .await
    });

    // Wait for pending row to land.
    let mut tries = 0;
    let pending = loop {
        let list = gate.list_pending().unwrap();
        if let Some(p) = list.into_iter().next() {
            break p;
        }
        tries += 1;
        assert!(tries < 50, "pending row never appeared");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
        .unwrap();

    let outcome = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
}

#[tokio::test]
async fn deny_returns_deny_with_reason() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                "pushover",
                "send push",
                serde_json::json!({}),
            ),
        )
        .await
    });

    let pending = loop {
        if let Some(p) = gate.list_pending().unwrap().into_iter().next() {
            break p;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    gate.decide(&pending.request_id, ApprovalDecision::Deny)
        .unwrap();

    let outcome = handle.await.unwrap();
    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("pushover")),
        other => panic!("expected deny, got {other:?}"),
    }
}

#[tokio::test]
async fn aborting_older_chat_waiter_preserves_newer_thread_route() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let old_gate = gate.clone();
    let old_handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            old_gate.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                "composio",
                "old action",
                serde_json::json!({}),
            ),
        )
        .await
    });

    let mut tries = 0;
    let old_request_id = loop {
        if let Some(request_id) = gate.pending_for_thread("t-test") {
            break request_id;
        }
        tries += 1;
        assert!(tries < 1_000, "old chat approval route never appeared");
        tokio::task::yield_now().await;
    };

    let new_gate = gate.clone();
    let new_handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            new_gate.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                "composio",
                "new action",
                serde_json::json!({}),
            ),
        )
        .await
    });

    let mut tries = 0;
    let new_request_id = loop {
        if let Some(request_id) = gate.pending_for_thread("t-test") {
            if request_id != old_request_id {
                break request_id;
            }
        }
        tries += 1;
        assert!(tries < 1_000, "new chat approval route never appeared");
        tokio::task::yield_now().await;
    };

    old_handle.abort();
    assert!(old_handle.await.unwrap_err().is_cancelled());

    assert_eq!(
        gate.pending_for_thread("t-test").as_deref(),
        Some(new_request_id.as_str())
    );
    assert!(!gate.parked.waiters.lock().contains_key(&old_request_id));
    assert!(gate.parked.waiters.lock().contains_key(&new_request_id));
    // The torn-down turn's request is cancelled in the engine (off the dropped
    // future, so give it a moment).
    let stub = stub_of(&gate);
    let mut tries = 0;
    while stub.state(&old_request_id) != Some(State::Cancelled) {
        tries += 1;
        assert!(tries < 500, "the abandoned request was never cancelled");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_ne!(stub.state(&new_request_id), Some(State::Cancelled));

    gate.decide(&new_request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    assert!(matches!(new_handle.await.unwrap(), GateOutcome::Allow));
    assert!(gate.pending_for_thread("t-test").is_none());
}

#[tokio::test]
async fn durable_tool_trust_skips_prompt() {
    let (gate, _dir) = test_gate();
    let tool = "openhuman_test_always_allow_tool";
    gate.grant_tool_trust(
        &crate::core::runtime::Tenant::default(),
        Variant::Standard,
        tool,
    )
    .unwrap();

    // A durable module grant is approved without parking — even with a live chat
    // context present — as an audited automatic approval, not a pending request.
    // The granted tool scope is the process subject for this test.
    let outcome = gate
        .intercept(
            &ApprovalTurnContext::chat(chat_ctx()),
            tool,
            "noop",
            serde_json::json!({}),
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "an auto-approved call must not create a pending approval row"
    );
    let stub = stub_of(&gate);
    let ids = stub.request_ids();
    assert_eq!(ids.len(), 1, "the grant is a recorded approval");
    assert_eq!(stub.attribution(&ids[0]), Some("auto:tool-grant"));
    assert!(
        !stub.seen()[0]
            .legacy_tool_grants
            .contains(&tool.to_string()),
        "the durable grant is resolved by the module, not imported from legacy config"
    );
}

/// "Always allow" on a parked `create_workflow` round-trips: the decided row is
/// persisted to `autonomy.auto_approve` (what `approval::rpc::approval_decide`
/// does for `ApproveAlwaysForTool`; the RPC itself reads the process-wide gate,
/// which tests must not install), and the next `create_workflow` is allowed
/// without a second prompt.
#[tokio::test]
async fn approve_always_for_create_workflow_persists_and_skips_the_next_prompt() {
    let (gate, dir) = test_gate();
    let gate = Arc::new(gate);
    let _env = EnvVarGuard::workspace_async(dir.path()).await;
    crate::security::live_policy::install(
        Arc::new(crate::security::SecurityPolicy::default()),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let intercept = |gate: Arc<ApprovalGate>| {
        tokio::spawn(async move {
            turn_origin::with_origin(
                web_origin(),
                gate.intercept(
                    &ApprovalTurnContext::chat(chat_ctx()),
                    "create_workflow",
                    "create a workflow",
                    serde_json::json!({}),
                ),
            )
            .await
        })
    };

    let first = intercept(gate.clone());
    let pending = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(p) = gate
                .list_pending()
                .unwrap()
                .into_iter()
                .find(|p| p.tool_name == "create_workflow")
            {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the first create_workflow must park for approval");
    let row = gate
        .decide(&pending.request_id, ApprovalDecision::ApproveAlwaysForTool)
        .unwrap()
        .expect("the parked create_workflow row is decided");
    assert!(matches!(first.await.unwrap(), GateOutcome::Allow));
    crate::config::ops::add_auto_approve_tool(
        &crate::core::runtime::DispatchCtx::process_default(),
        &row.tool_name,
    )
    .await
    .expect("persist the Always-allow choice");

    let saved = crate::config::ops::load_config_with_timeout()
        .await
        .unwrap();
    assert!(
        saved
            .autonomy
            .auto_approve
            .iter()
            .any(|tool| tool == "create_workflow"),
        "Always allow must persist create_workflow to autonomy.auto_approve"
    );

    let second = tokio::time::timeout(Duration::from_secs(5), intercept(gate.clone()))
        .await
        .expect("the second create_workflow must not park for approval")
        .unwrap();
    assert!(matches!(second, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "the second create_workflow must not re-prompt"
    );

    // Leave no allowlisted create_workflow behind for sibling gate tests.
    crate::security::live_policy::install(
        Arc::new(crate::security::SecurityPolicy::default()),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );
}

/// With `auto_approve_all: true`, a WebChat-origin call resolves to
/// `Allow` immediately — no pending row is created and the chat context
/// is never consulted, proving the short-circuit fires above the park.
#[tokio::test]
async fn auto_approve_all_resolves_allow() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let policy = crate::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::security::SecurityPolicy::default()
    };
    // Scoped: restores whatever live_policy held before this test on drop
    // (including on panic), so a leaked `auto_approve_all: true` can never
    // reach a sibling gate test that doesn't hold `TEST_ENV_LOCK`.
    let _policy_guard = crate::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let outcome = turn_origin::with_origin(
        web_origin(),
        gate.intercept(
            &ApprovalTurnContext::default(),
            "openhuman_test_aaa_webchat",
            "noop",
            serde_json::json!({}),
        ),
    )
    .await;

    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "auto_approve_all must short-circuit before any pending row is persisted"
    );
}

/// An `auto_approve_all` approval is recorded: skipping the prompt is exactly
/// the decision an auditor needs to see, so the engine holds an approved,
/// attributed request for it rather than nothing.
#[tokio::test]
async fn auto_approve_all_is_a_recorded_automatic_approval() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let policy = crate::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let (outcome, request_id) = turn_origin::with_origin(
        web_origin(),
        gate.intercept_audited(
            &ApprovalTurnContext::default(),
            "openhuman_test_aaa_audit",
            "noop",
            serde_json::json!({}),
        ),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow));

    let stub = stub_of(&gate);
    let request_id = request_id.expect("an engine-approved call hands back its request id");
    assert_eq!(stub.attribution(&request_id), Some("auto:host-policy"));
    assert_eq!(stub.state(&request_id), Some(State::Reserved));
    let seen = &stub.seen()[0];
    assert!(seen.posture.auto_approve_all);
    assert_eq!(seen.tool, "openhuman_test_aaa_audit");
    gate.record_execution(&request_id, ExecutionOutcome::Success, None);
    assert_eq!(
        stub.state(&request_id),
        Some(State::Executed(ExecutionOutcome::Success))
    );
}

/// Control test: with `auto_approve_all: false` (the default), a
/// WebChat-origin call parks normally — it does NOT resolve to `Allow`
/// until a decision is sent on the oneshot.
#[tokio::test]
async fn auto_approve_all_off_still_parks() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let policy = crate::security::SecurityPolicy {
        auto_approve_all: false,
        ..crate::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                "openhuman_test_aaa_off",
                "noop",
                serde_json::json!({}),
            ),
        )
        .await
    });

    // The call must actually park: poll for the pending row instead of
    // racing an immediate result.
    let mut tries = 0;
    let pending = loop {
        let rows = gate.list_pending().unwrap();
        if let Some(p) = rows.into_iter().next() {
            break p;
        }
        tries += 1;
        assert!(
            tries < 50,
            "pending row never appeared — call resolved without parking"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    let outcome = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
}

/// `auto_approve_all: true` must NOT override an `Unknown` origin — the
/// gate still fails closed for unlabelled call sites.
#[tokio::test]
async fn auto_approve_all_does_not_override_unknown() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let policy = crate::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    // No `with_origin` scope at all — mirrors an unlabelled call site,
    // which `turn_origin::current()` maps to `AgentTurnOrigin::Unknown`.
    let outcome = gate
        .intercept(
            &ApprovalTurnContext::default(),
            "openhuman_test_aaa_unknown",
            "noop",
            serde_json::json!({}),
        )
        .await;

    match outcome {
        // The deny message is specific and actionable (issues #5508 / #5499,
        // 2nd acceptance criterion): it names the missing origin label, calls
        // out the scheduling/external-effect tools it affects, and frames it
        // as an internal wiring gap rather than user error.
        GateOutcome::Deny { reason } => {
            assert!(reason.contains("origin label"), "reason was: {reason}");
            assert!(reason.contains("cron_add"), "reason was: {reason}");
            assert!(reason.contains("external-effect"), "reason was: {reason}");
        }
        other => panic!("expected deny, got {other:?}"),
    }
}

/// `auto_approve_all: true` overrides a `Workflow { require_approval: true }`
/// origin — normally the user's per-flow "gate every action" choice forces
/// a park, but the blanket bypass sits above that check too.
#[tokio::test]
async fn auto_approve_all_overrides_require_approval_workflow() {
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let policy = crate::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "flow-1".into(),
        source: TrustedAutomationSource::Workflow {
            require_approval: true,
        },
    };
    let outcome = turn_origin::with_origin(
        origin,
        gate.intercept(
            &ApprovalTurnContext::default(),
            "openhuman_test_aaa_workflow",
            "noop",
            serde_json::json!({}),
        ),
    )
    .await;

    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "auto_approve_all must short-circuit before any pending row is persisted"
    );
}

/// The `auto_approve_all` × remote-origin-triage interaction, pinned by
/// name because it is a **decision**, not an emergent behaviour.
///
/// Since openhuman#5634 a Composio/webhook payload reaching
/// `triage.escalate` carries `Workflow { require_approval: true }`, so
/// normally it parks for a person — that is
/// `a_remote_triage_escalation_parks_with_an_audit_row_rather_than_an_unknown_denial`
/// above. With `auto_approve_all` on it is allowed without parking: the user's
/// blanket setting reaches the engine as the posture of the call, as for every
/// other origin but an unlabelled one. (The engine now also records that
/// approval, where the host's old gate left no trail at all.)
///
/// The gate owner accepted that rather than carving out an exception:
/// https://github.com/tinyhumansai/openhuman/issues/5634#issuecomment-5396604125
///
/// So this test exists to be *broken on purpose*. If a future change makes
/// this origin ignore the posture, this fails, and whoever is making that
/// change has to reopen the decision instead of discovering the behaviour by
/// accident. Deleting it to make a change pass is the one wrong response.
#[tokio::test]
async fn auto_approve_all_allows_a_remote_triage_dispatch_without_parking() {
    use crate::triage::{remote_trigger_origin, TriggerEnvelope};

    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let (gate, dir) = test_gate();
    let policy = crate::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let envelope = TriggerEnvelope::from_composio(
        "gmail",
        "new_message",
        "ti_meta",
        "ti_bCCTKZlajKi4",
        serde_json::json!({ "subject": "hello" }),
    );

    let outcome = turn_origin::with_origin(
        remote_trigger_origin(&envelope),
        gate.intercept(
            &ApprovalTurnContext::default(),
            "triage.escalate",
            "escalate to orchestrator",
            serde_json::json!({}),
        ),
    )
    .await;

    assert!(
        matches!(outcome, GateOutcome::Allow),
        "auto_approve_all opts into the bypass globally, including remote triage; got {outcome:?}"
    );
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "the posture approves the call before any park, so nothing waits for a person"
    );
    assert_eq!(
        stub_of(&gate).seen()[0].variant,
        Variant::FlowStrict,
        "a workflow that requires approval is its own posture variant"
    );
}

#[tokio::test]
async fn timeout_returns_deny() {
    let (gate, _dir, env) = expiry_gate().await;
    let gate = Arc::new(gate);
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                "composio",
                "timed out",
                serde_json::json!({}),
            ),
        )
        .await
    });
    let mut tries = 0;
    while gate.list_pending().unwrap().is_empty() {
        tries += 1;
        assert!(tries < 50, "audit row never appeared for timeout test");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(env);
    let outcome = handle.await.unwrap();
    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("timed out")),
        other => panic!("expected deny, got {other:?}"),
    }
}

/// T-M3 (flows `cancel_flow_run`): the gate has no special-casing per tool
/// name — any call intercepted under a chat origin/context with no
/// matching auto-allowlist entry parks and, absent a human decision,
/// times out to `Deny` rather than executing. This pins that
/// `cancel_flow_run` — now that `builder_tools::CancelFlowRunTool`
/// reports `external_effect() == true` (T-M3) so
/// `ApprovalSecurityMiddleware` routes it through exactly this call —
/// genuinely parks for a real approval decision instead of running
/// unapproved, mirroring `timeout_returns_deny` above.
#[tokio::test]
async fn cancel_flow_run_parks_for_approval_when_a_gate_is_present() {
    let (gate, _dir, env) = expiry_gate().await;
    let gate = Arc::new(gate);
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                "cancel_flow_run",
                "cancel run r-1 of flow f-1",
                serde_json::json!({ "flow_id": "f-1", "run_id": "r-1" }),
            ),
        )
        .await
    });
    let mut tries = 0;
    while gate.list_pending().unwrap().is_empty() {
        tries += 1;
        assert!(tries < 50, "audit row never appeared for cancel_flow_run");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(env);
    let outcome = handle.await.unwrap();
    // No decision ever arrives — the call must NOT auto-execute. It
    // parks until the gate's TTL elapses, then denies (never `Allow`).
    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("timed out")),
        other => {
            panic!("expected the parked cancel_flow_run call to time out to Deny, got {other:?}")
        }
    }
}

#[tokio::test]
async fn decide_unknown_id_is_noop() {
    let (gate, _dir) = test_gate();
    let decided = gate
        .decide("does-not-exist", ApprovalDecision::ApproveOnce)
        .unwrap();
    assert!(decided.is_none());
}

/// TAURI-RUST-5EH: a `decide` miss must be classified — already-decided and
/// expired rows are benign (`AlreadyResolved`), while an id that was never
/// persisted is a genuine lost registration (`NeverRegistered`) that stays a
/// Sentry signal.
#[tokio::test]
async fn classify_decide_miss_distinguishes_resolved_from_unknown() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);

    // Never stored → genuine loss, keep visible.
    assert_eq!(
        gate.classify_decide_miss("never-existed"),
        DecideMiss::NeverRegistered
    );

    // Stored and decided, but never parked here: a decision finds nothing to
    // resolve, and the miss is the benign already-decided race.
    stub.leave_pending("req-decided", "composio", Duration::from_secs(600));
    stub.decide("req-decided", Choice::Once, "approve_once")
        .await
        .unwrap();
    assert!(gate
        .decide("req-decided", ApprovalDecision::Deny)
        .unwrap()
        .is_none());
    assert_eq!(
        gate.classify_decide_miss("req-decided"),
        DecideMiss::AlreadyResolved
    );

    // A request that expired is resolved too.
    stub.leave_pending("req-expired", "composio", Duration::from_millis(1));
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(stub.expire("req-expired").await.unwrap());
    assert!(gate
        .decide("req-expired", ApprovalDecision::ApproveOnce)
        .unwrap()
        .is_none());
    assert_eq!(
        gate.classify_decide_miss("req-expired"),
        DecideMiss::AlreadyResolved
    );

    // Stored and still pending, but not parked by this process: the registration
    // was lost, which stays a signal.
    stub.leave_pending("req-orphan", "composio", Duration::from_secs(600));
    assert_eq!(
        gate.classify_decide_miss("req-orphan"),
        DecideMiss::NeverRegistered
    );
}
