use super::*;

#[test]
fn guard_cleanup_only_clears_routing_it_still_owns() {
    // Regression for #4774: on external turn teardown a replacement turn may
    // have already parked a new approval on the same thread and
    // overwritten the routing entry. The dropped guard for the *old* request
    // must not clobber the *new* request's mapping.
    let (gate, _dir) = test_gate();

    gate.thread_to_request
        .lock()
        .insert("thread-1".into(), "req-new".into());

    // Stale guard for the superseded request is a no-op.
    gate.clear_thread_route_if_owned("thread-1", "req-old");
    assert_eq!(
        gate.pending_for_thread("thread-1").as_deref(),
        Some("req-new")
    );

    // The owning request's guard clears its own routing.
    gate.clear_thread_route_if_owned("thread-1", "req-new");
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
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                g.intercept("composio", "send slack", serde_json::json!({})),
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
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                g.intercept("pushover", "send push", serde_json::json!({})),
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
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                old_gate.intercept("composio", "old action", serde_json::json!({})),
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
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                new_gate.intercept("composio", "new action", serde_json::json!({})),
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
    assert!(!gate.waiters.lock().contains_key(&old_request_id));
    assert!(gate.waiters.lock().contains_key(&new_request_id));
    assert_eq!(
        store::get_decision(&gate.config, &old_request_id).unwrap(),
        Some(ApprovalDecision::Deny)
    );

    gate.decide(&new_request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    assert!(matches!(new_handle.await.unwrap(), GateOutcome::Allow));
    assert!(gate.pending_for_thread("t-test").is_none());
}

#[tokio::test]
async fn auto_approve_tool_skips_prompt() {
    // The gate reads the "Always allow" allowlist from the process-global
    // live policy. Serialize with the other tests that install/reload it
    // (the `live_policy` module test + the autonomy `ops` tests, which all
    // take this same lock) so a parallel install can't clobber ours mid-test.
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = test_gate();

    // A tool name unique to this test so leaving it in the global allowlist
    // afterwards can't make a sibling gate test (which use "composio" /
    // "pushover") skip its expected prompt.
    let tool = "openhuman_test_always_allow_tool";
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve: vec![tool.into()],
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    crate::openhuman::security::live_policy::install(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    // An allow-listed tool short-circuits the gate to `Allow` immediately —
    // before any parking — even with a live chat context present, and
    // without persisting a pending row. The shortcut runs regardless of
    // origin (it's the user's persisted "Always allow" allowlist), so we
    // do not need to scope an origin for this case.
    let outcome = APPROVAL_CHAT_CONTEXT
        .scope(
            chat_ctx(),
            gate.intercept(tool, "noop", serde_json::json!({})),
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "an auto-approved call must not create a pending approval row"
    );
}

/// With `auto_approve_all: true`, a WebChat-origin call resolves to
/// `Allow` immediately — no pending row is created and the chat context
/// is never consulted, proving the short-circuit fires above the park.
#[tokio::test]
async fn auto_approve_all_resolves_allow() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = test_gate();
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    // Scoped: restores whatever live_policy held before this test on drop
    // (including on panic), so a leaked `auto_approve_all: true` can never
    // reach a sibling gate test that doesn't hold `TEST_ENV_LOCK`.
    let _policy_guard = crate::openhuman::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let outcome = turn_origin::with_origin(
        web_origin(),
        gate.intercept("openhuman_test_aaa_webchat", "noop", serde_json::json!({})),
    )
    .await;

    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "auto_approve_all must short-circuit before any pending row is persisted"
    );
}

/// Control test: with `auto_approve_all: false` (the default), a
/// WebChat-origin call parks normally — it does NOT resolve to `Allow`
/// until a decision is sent on the oneshot.
#[tokio::test]
async fn auto_approve_all_off_still_parks() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = test_gate();
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve_all: false,
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::openhuman::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                g.intercept("openhuman_test_aaa_off", "noop", serde_json::json!({})),
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

/// `auto_approve_all: true` must NOT override a `SubconsciousTainted`
/// origin — the gate still hard-denies it (indirect prompt injection
/// defense).
#[tokio::test]
async fn auto_approve_all_does_not_override_subconscioustainted() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = test_gate();
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::openhuman::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "job-tainted".into(),
        source: TrustedAutomationSource::SubconsciousTainted,
    };
    let outcome = turn_origin::with_origin(
        origin,
        gate.intercept("openhuman_test_aaa_tainted", "noop", serde_json::json!({})),
    )
    .await;

    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("external-sync")),
        other => panic!("expected deny, got {other:?}"),
    }
}

/// `auto_approve_all: true` must NOT override an `Unknown` origin — the
/// gate still fails closed for unlabelled call sites.
#[tokio::test]
async fn auto_approve_all_does_not_override_unknown() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = test_gate();
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::openhuman::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    // No `with_origin` scope at all — mirrors an unlabelled call site,
    // which `turn_origin::current()` maps to `AgentTurnOrigin::Unknown`.
    let outcome = gate
        .intercept("openhuman_test_aaa_unknown", "noop", serde_json::json!({}))
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

/// `auto_approve_all: true` overrides the `GoalContinuation` bypass —
/// normally that origin skips the per-tool allowlist and always parks,
/// but the blanket bypass sits above that check and allows immediately.
#[tokio::test]
async fn auto_approve_all_overrides_bypass_shortcut() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = test_gate();
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::openhuman::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "goal-1".into(),
        source: TrustedAutomationSource::GoalContinuation,
    };
    let outcome = turn_origin::with_origin(
        origin,
        gate.intercept("openhuman_test_aaa_goal", "noop", serde_json::json!({})),
    )
    .await;

    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "auto_approve_all must short-circuit before any pending row is persisted"
    );
}

/// `auto_approve_all: true` overrides a `Workflow { require_approval: true }`
/// origin — normally the user's per-flow "gate every action" choice forces
/// a park, but the blanket bypass sits above that check too.
#[tokio::test]
async fn auto_approve_all_overrides_require_approval_workflow() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = test_gate();
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::openhuman::security::live_policy::install_scoped(
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
        gate.intercept("openhuman_test_aaa_workflow", "noop", serde_json::json!({})),
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
/// normally it parks and writes a `pending_approvals` row — that is
/// `a_remote_triage_escalation_parks_with_an_audit_row_rather_than_an_unknown_denial`
/// above. With `auto_approve_all` on it is allowed immediately and writes
/// no row, which means for those users #5634 moved this path from
/// `Unknown` → hard Deny to Allow-with-no-audit-trail.
///
/// The gate owner accepted that rather than carving out an exception:
/// https://github.com/tinyhumansai/openhuman/issues/5634#issuecomment-5396604125
///
/// So this test exists to be *broken on purpose*. If a future change adds
/// this origin to the bypass exclusion list, this fails, and whoever is
/// making that change has to reopen the decision instead of discovering the
/// behaviour by accident. Deleting it to make a change pass is the one
/// wrong response.
#[tokio::test]
async fn auto_approve_all_allows_a_remote_triage_dispatch_without_an_audit_row() {
    use crate::openhuman::agent::triage::{remote_trigger_origin, TriggerEnvelope};

    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = test_gate();
    let policy = crate::openhuman::security::SecurityPolicy {
        auto_approve_all: true,
        ..crate::openhuman::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::openhuman::security::live_policy::install_scoped(
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
            "triage.escalate",
            "escalate to orchestrator",
            serde_json::json!({}),
        ),
    )
    .await;

    assert!(
        matches!(outcome, GateOutcome::Allow),
        "auto_approve_all opts into the bypass globally, including remote triage;              got {outcome:?}"
    );
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "the bypass short-circuits before the park, so no pending_approvals row is              written — this is the documented cost of the flag, not a defect"
    );
}

#[tokio::test]
async fn timeout_returns_deny() {
    let (gate, _dir, _env) = expiry_gate();
    let gate = Arc::new(gate);
    let outcome = turn_origin::with_origin(
        web_origin(),
        APPROVAL_CHAT_CONTEXT.scope(
            chat_ctx(),
            gate.intercept("composio", "timed out", serde_json::json!({})),
        ),
    )
    .await;
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
    let (gate, _dir, _env) = expiry_gate();
    let gate = Arc::new(gate);
    let outcome = turn_origin::with_origin(
        web_origin(),
        APPROVAL_CHAT_CONTEXT.scope(
            chat_ctx(),
            gate.intercept(
                "cancel_flow_run",
                "cancel run r-1 of flow f-1",
                serde_json::json!({ "flow_id": "f-1", "run_id": "r-1" }),
            ),
        ),
    )
    .await;
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

    // Never persisted → genuine loss, keep visible.
    assert_eq!(
        gate.classify_decide_miss("never-existed"),
        DecideMiss::NeverRegistered
    );

    // Persist + decide a row, then a second decide misses → already-decided.
    let pending = PendingApproval::new(
        "req-decided",
        "composio",
        "send email",
        serde_json::json!({}),
        Some(chrono::Utc::now() + chrono::Duration::minutes(10)),
    );
    store::insert_pending(&gate.config, &pending, &gate.session_id).unwrap();
    assert!(gate
        .decide("req-decided", ApprovalDecision::ApproveOnce)
        .unwrap()
        .is_some());
    // The conditional UPDATE now matches 0 rows (decided_at set).
    assert!(gate
        .decide("req-decided", ApprovalDecision::Deny)
        .unwrap()
        .is_none());
    assert_eq!(
        gate.classify_decide_miss("req-decided"),
        DecideMiss::AlreadyResolved
    );

    // A row past its expiry is lazily denied by `decide`'s expire pass, so
    // its decide miss is also benign (the persisted decision exists).
    let expired = PendingApproval::new(
        "req-expired",
        "composio",
        "send email",
        serde_json::json!({}),
        Some(chrono::Utc::now() - chrono::Duration::minutes(1)),
    );
    store::insert_pending(&gate.config, &expired, &gate.session_id).unwrap();
    assert!(gate
        .decide("req-expired", ApprovalDecision::ApproveOnce)
        .unwrap()
        .is_none());
    assert_eq!(
        gate.classify_decide_miss("req-expired"),
        DecideMiss::AlreadyResolved
    );
}
