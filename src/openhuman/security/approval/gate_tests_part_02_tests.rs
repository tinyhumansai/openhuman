use super::*;

#[tokio::test]
async fn pending_for_thread_tracks_request_under_chat_context_and_clears() {
    // This test polls a parked task while the full Rust suite runs thousands
    // of other tests concurrently. Keep its coordination deadline independent
    // of the short TTL used by the timeout behavior tests.
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    // Run intercept inside a scoped chat context + matching WebChat
    // origin (as the web channel does in production).
    let g = gate.clone();
    let ctx = ApprovalChatContext {
        thread_id: "thread-42".into(),
        client_id: "client-1".into(),
    };
    let origin = AgentTurnOrigin::WebChat {
        thread_id: "thread-42".into(),
        client_id: "client-1".into(),
        request_id: Some("req-42".into()),
    };
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            origin,
            APPROVAL_CHAT_CONTEXT.scope(ctx, g.intercept("shell", "run ls", serde_json::json!({}))),
        )
        .await
    });

    // While parked, the thread → request mapping is queryable.
    let mut tries = 0;
    let request_id = loop {
        if let Some(r) = gate.pending_for_thread("thread-42") {
            break r;
        }
        tries += 1;
        assert!(tries < 50, "thread mapping never appeared");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    // Decide via the mapped request_id (as the chat ingress router will).
    decide_parked(&gate, &request_id, ApprovalDecision::ApproveOnce);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));

    // Mapping is cleared once intercept returns.
    assert!(gate.pending_for_thread("thread-42").is_none());
}

/// Regression for #5499: an async-delegated sub-agent carries the `WebChat`
/// origin across the `tokio::spawn` boundary (`spawn_async_subagent` calls
/// `turn_origin::propagate`) but NOT the `APPROVAL_CHAT_CONTEXT` task-local.
/// Before the origin-routing fallback the gate parked with `thread_id:
/// None`, the web-channel surface dropped the `ApprovalRequested` event
/// ("thread/client absent — NOT surfacing"), and the park silently
/// TTL-denied — so a `cron_add` the user asked for in chat never completed.
/// The gate must instead route the park via the thread/client the `WebChat`
/// origin already carries, so the card can surface and be approved.
#[tokio::test]
async fn webchat_origin_routes_park_when_approval_chat_context_absent() {
    // See the companion chat-context routing test above: this is a
    // coordination deadline, not the timeout behavior under test.
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    // WebChat origin scoped, but NO `APPROVAL_CHAT_CONTEXT` — exactly the
    // async sub-agent spawn state (origin propagated, approval context not).
    let g = gate.clone();
    let origin = AgentTurnOrigin::WebChat {
        thread_id: "thread-async".into(),
        client_id: "client-async".into(),
        request_id: Some("req-async".into()),
    };
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            origin,
            g.intercept("cron_add", "schedule daily reminder", serde_json::json!({})),
        )
        .await
    });

    // The park must be routable via the origin's thread even though the
    // approval task-local was never scoped. `thread_to_request` is inserted
    // only when `chat_thread_id` is `Some`, so this mapping appearing proves
    // the origin fallback supplied it.
    let mut tries = 0;
    let request_id = loop {
        if let Some(r) = gate.pending_for_thread("thread-async") {
            break r;
        }
        tries += 1;
        assert!(
            tries < 50,
            "park must be routable via the WebChat origin's thread when \
             APPROVAL_CHAT_CONTEXT is absent (#5499)"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    // A decision on that mapped request resolves the park (the card can
    // surface and be approved), instead of silently TTL-denying.
    decide_parked(&gate, &request_id, ApprovalDecision::ApproveOnce);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));
    assert!(gate.pending_for_thread("thread-async").is_none());
}

#[tokio::test]
async fn waiter_future_dropped_mid_park_evicts_waiter_clears_routing_and_denies_row() {
    // #4774: once a turn future can be torn down *externally* (the #4746
    // harness wall-clock backstop / #4751 outer web backstop firing while a
    // tool call is parked), dropping the intercept future must not leak the
    // waiter, the thread→request routing mapping, or the still-open pending
    // row. The `WaiterGuard` Drop impl runs the cleanup the timeout match
    // arms would otherwise own.
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    // Build the parked future with the WebChat origin + chat context scoped,
    // exactly like the production web channel caller — but drive it locally
    // so we can drop it mid-park instead of resolving it.
    let g = gate.clone();
    // `Box::pin` (not `tokio::pin!`) so `drop(fut)` below drops the *future
    // itself* — and thus the `WaiterGuard` saved in its async state — rather
    // than just a `Pin<&mut _>` reference.
    let mut fut = Box::pin(turn_origin::with_origin(
        web_origin(),
        APPROVAL_CHAT_CONTEXT.scope(
            chat_ctx(),
            g.intercept("shell", "run rm", serde_json::json!({})),
        ),
    ));

    // Poll it just long enough to register the waiter, persist the pending
    // row, and park on the TTL timeout. Nothing resolves it, so the outer
    // timeout must elapse with the future still pending.
    let parked = tokio::time::timeout(Duration::from_millis(200), &mut fut).await;
    assert!(
        parked.is_err(),
        "future should still be parked, not resolved"
    );

    // Capture the request_id from the routing mapping while parked, and
    // confirm the waiter + pending row exist before teardown.
    let request_id = gate
        .pending_for_thread("t-test")
        .expect("thread→request mapping must exist while parked");
    assert!(
        gate.waiters.lock().contains_key(&request_id),
        "waiter must be registered while parked"
    );
    assert!(
        matches!(store::get_decision(&gate.config, &request_id), Ok(None)),
        "pending row must be open (undecided) while parked"
    );

    // External teardown: the wall-clock backstop tears the turn future down
    // mid-park. This skips the timeout match arms entirely.
    drop(fut);

    // The RAII guard must have run the cleanup on drop.
    assert!(
        !gate.waiters.lock().contains_key(&request_id),
        "waiter must be evicted when the parked future is dropped"
    );
    assert!(
        gate.pending_for_thread("t-test").is_none(),
        "thread→request routing must be cleared on external teardown"
    );
    assert!(
        matches!(
            store::get_decision(&gate.config, &request_id),
            Ok(Some(ApprovalDecision::Deny))
        ),
        "pending row must be denied when the parked future is dropped"
    );
}

// ── caller park bound (issue #4756) ──────────────────────────────
//
// A caller (composio_connect) can cap the park via
// `intercept_audited_bounded`. When the bound elapses before the gate's own
// TTL the gate must abandon the park cancellation-safely: return `None`,
// clear the thread→request routing so a later reply is not mis-routed (the
// codex concern), yet LEAVE the `pending_approvals` row open so a later
// card-click still resolves it in the DB.
#[tokio::test]
async fn intercept_audited_bounded_abandons_park_and_leaves_row_pending() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let g = gate.clone();
    let ctx = ApprovalChatContext {
        thread_id: "thread-bound".into(),
        client_id: "client-1".into(),
    };
    let origin = AgentTurnOrigin::WebChat {
        thread_id: "thread-bound".into(),
        client_id: "client-1".into(),
        request_id: Some("req-bound".into()),
    };
    // 100ms caller bound — far below the gate TTL — so the bound is what
    // elapses, not the gate's own timeout.
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            origin,
            APPROVAL_CHAT_CONTEXT.scope(
                ctx,
                g.intercept_audited_bounded(
                    "shell",
                    "run ls",
                    serde_json::json!({}),
                    Some(Duration::from_millis(100)),
                ),
            ),
        )
        .await
    });

    // While parked, the thread → request mapping is queryable.
    let mut tries = 0;
    let request_id = loop {
        if let Some(r) = gate.pending_for_thread("thread-bound") {
            break r;
        }
        tries += 1;
        assert!(tries < 50, "thread mapping never appeared");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };

    // The bound elapses → `None`, so the caller renders its own fast path
    // instead of the park resolving to a Deny.
    let resolved = handle.await.unwrap();
    assert!(
        resolved.is_none(),
        "caller park bound must surface as None, not a resolved outcome"
    );

    // Routing is cleared so a later reply is not mis-routed to the abandoned
    // request (the codex #4756 concern).
    assert!(
        gate.pending_for_thread("thread-bound").is_none(),
        "thread → request mapping must be cleared on caller-bound abandon"
    );

    // The row is LEFT open — a later human card-click still resolves it.
    let decided = gate
        .decide(&request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    assert!(
        decided.is_some(),
        "pending row must survive the abandon so a later card-click resolves it"
    );
}

/// Tests for `effective_ttl` env-override parsing.
///
/// These run serially (they mutate the process env) via the shared
/// `TEST_ENV_LOCK`; the lock is the same one used by `auto_approve_tool_skips_prompt`
/// and the live_policy tests so they cannot clobber each other in parallel.
///
/// Guarded on `debug_assertions`: the override is compiled out of release
/// builds, so this assertion only holds under `cargo test` (debug). The
/// fallback tests below hold in either build.
#[cfg(debug_assertions)]
#[test]
fn effective_ttl_uses_env_override_when_valid() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, _dir) = test_gate_with_ttl(BOOT_TTL_UNDER_TEST);
    unsafe { std::env::set_var("OPENHUMAN_APPROVAL_TTL_SECS", "42") };
    assert_eq!(
        gate.effective_ttl(),
        Duration::from_secs(42),
        "valid OPENHUMAN_APPROVAL_TTL_SECS must override boot-time TTL"
    );
    unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
}

#[test]
fn effective_ttl_falls_back_to_boot_ttl_for_garbage_value() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, _dir) = test_gate_with_ttl(BOOT_TTL_UNDER_TEST);
    unsafe { std::env::set_var("OPENHUMAN_APPROVAL_TTL_SECS", "not-a-number") };
    assert_eq!(
        gate.effective_ttl(),
        BOOT_TTL_UNDER_TEST,
        "garbage OPENHUMAN_APPROVAL_TTL_SECS must fall back to boot-time TTL"
    );
    unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
}

#[test]
fn effective_ttl_falls_back_to_boot_ttl_when_unset() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, _dir) = test_gate_with_ttl(BOOT_TTL_UNDER_TEST);
    unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
    assert_eq!(
        gate.effective_ttl(),
        BOOT_TTL_UNDER_TEST,
        "unset OPENHUMAN_APPROVAL_TTL_SECS must fall back to boot-time TTL"
    );
}

/// Tests for `resolve_park_ttl` — the pure clamp-selection helper behind
/// the copilot-streaming TTL shortening (fix/flows-copilot-approval-ttl).
/// Exercised directly (rather than by actually parking + waiting out a
/// multi-minute TTL) so the assertions stay fast and deterministic.
mod resolve_park_ttl_tests {
    use super::super::*;

    #[test]
    fn default_park_keeps_the_full_ttl() {
        let default_ttl = DEFAULT_APPROVAL_TTL;
        assert_eq!(
            ApprovalGate::resolve_park_ttl(default_ttl, false),
            default_ttl,
            "a plain park (no copilot stream) must not be clamped"
        );
    }

    #[test]
    fn copilot_stream_shortens_a_default_ten_minute_park() {
        let default_ttl = DEFAULT_APPROVAL_TTL;
        assert_eq!(
            ApprovalGate::resolve_park_ttl(default_ttl, true),
            COPILOT_APPROVAL_TTL,
            "a flows_build copilot-streaming park must clamp to COPILOT_APPROVAL_TTL"
        );
        assert!(
            COPILOT_APPROVAL_TTL < DEFAULT_APPROVAL_TTL,
            "the copilot clamp must actually be shorter than the default TTL"
        );
    }

    #[test]
    fn a_clamp_never_extends_a_shorter_boot_time_ttl() {
        // Mirrors production's env-override guard: a clamp may only
        // narrow, never widen, the gate's own effective TTL (e.g. a
        // debug-only `OPENHUMAN_APPROVAL_TTL_SECS=60` override that is
        // already shorter than either clamp).
        let short_ttl = Duration::from_secs(60);
        assert_eq!(
            ApprovalGate::resolve_park_ttl(short_ttl, true),
            short_ttl,
            "copilot clamp must not extend a boot-time TTL that is already shorter"
        );
    }
}

/// Integration regression test for the streaming-to-gate contract
/// (CodeRabbit review on PR #5112): `resolve_park_ttl` is covered directly
/// above, but that alone doesn't prove `intercept_audited_inner` actually
/// persists the clamped TTL when the copilot-streaming context is scoped.
/// Builds a gate with the full `DEFAULT_APPROVAL_TTL` boot TTL (unlike
/// `test_gate()` before this suite stopped hard-coding a 2s window, which
/// would make this assertion vacuous), scopes
/// `APPROVAL_COPILOT_STREAM_CONTEXT` alongside the chat context + WebChat
/// origin the way `flows::ops::flows_build` does in production, and
/// inspects the persisted `expires_at` on the pending row.
#[tokio::test]
async fn copilot_streaming_park_persists_the_clamped_expiry() {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let session = format!("session-{}", uuid::Uuid::new_v4());
    let gate = ApprovalGate::new(config, session, DEFAULT_APPROVAL_TTL);
    let gate = Arc::new(gate);

    let before = chrono::Utc::now();
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                APPROVAL_COPILOT_STREAM_CONTEXT.scope(
                    (),
                    g.intercept("composio", "send slack", serde_json::json!({})),
                ),
            ),
        )
        .await
    });

    let pending = loop {
        if let Some(p) = gate.list_pending().unwrap().into_iter().next() {
            break p;
        }
        tokio::task::yield_now().await;
    };

    let expires_at = pending
        .expires_at
        .expect("a parked approval always sets expires_at");
    let ttl_persisted = expires_at - before;
    assert!(
        ttl_persisted
            <= chrono::Duration::from_std(COPILOT_APPROVAL_TTL).unwrap()
                + chrono::Duration::seconds(5),
        "copilot-streaming park must persist an expires_at clamped to COPILOT_APPROVAL_TTL \
         (180s), not the gate's full {:?} boot TTL — got a {ttl_persisted} window",
        DEFAULT_APPROVAL_TTL
    );
    assert!(
        ttl_persisted < chrono::Duration::from_std(DEFAULT_APPROVAL_TTL).unwrap(),
        "sanity: the persisted expiry must be shorter than the unclamped default TTL"
    );

    gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    let outcome = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
}

#[test]
fn parse_approval_reply_maps_yes_no_and_rejects_other() {
    for y in ["yes", "Y", " OK ", "approve", "Allow", "okay"] {
        assert_eq!(
            super::super::parse_approval_reply(y),
            Some(ApprovalDecision::ApproveOnce),
            "{y}"
        );
    }
    for n in ["no", "N", "deny", "Denied"] {
        assert_eq!(
            super::super::parse_approval_reply(n),
            Some(ApprovalDecision::Deny),
            "{n}"
        );
    }
    // Anything else is NOT an answer → caller cancels + redirects.
    for other in [
        "maybe",
        "actually do Y instead",
        "",
        "yep nope",
        "sure thing",
    ] {
        assert_eq!(super::super::parse_approval_reply(other), None, "{other}");
    }
}

/// openhuman#5634: the six triage dispatch sites scoped no origin, so every
/// proactive escalation reached this gate as `Unknown` and was refused —
/// `intercept_with_unknown_origin_denies` below is that behaviour.
///
/// A remote trigger now carries
/// `TrustedAutomation { Workflow { require_approval: true } }`, which parks
/// and persists the `pending_approvals` row instead. This asserts the park
/// and the row, not a successful escalation: with no surface able to decide
/// a background park these still TTL-deny (openhuman#5746). The gain is the
/// audit trail, not restored function.
#[tokio::test]
async fn a_remote_triage_escalation_parks_with_an_audit_row_rather_than_an_unknown_denial() {
    use crate::openhuman::agent::triage::{remote_trigger_origin, TriggerEnvelope};

    let (gate, _dir) = test_gate();
    let envelope = TriggerEnvelope::from_composio(
        "gmail",
        "new_message",
        "ti_meta",
        "ti_bCCTKZlajKi4",
        serde_json::json!({ "subject": "hello" }),
    );

    // `Box::pin` + a short timeout drives the future into the park without
    // waiting out the TTL; nothing decides it, so it must still be pending.
    let mut fut = Box::pin(turn_origin::with_origin(
        remote_trigger_origin(&envelope),
        gate.intercept(
            "triage.escalate",
            "escalate to orchestrator",
            serde_json::json!({}),
        ),
    ));
    let parked = tokio::time::timeout(Duration::from_millis(300), &mut fut).await;
    assert!(
        parked.is_err(),
        "a remote escalation must park for a decision, not resolve immediately \
         (an immediate Deny here is the `Unknown` regression this pins)"
    );

    let pending = gate.list_pending().unwrap();
    assert_eq!(
        pending.len(),
        1,
        "the park must persist exactly one pending_approvals row, got {pending:?}"
    );
    assert_eq!(pending[0].tool_name, "triage.escalate");
}

/// The counterpart: a locally initiated triage dispatch keeps the authority
/// its caller already had, so it is allowed without a prompt and writes no
/// row. Pinned alongside the remote case because the security decision on
/// openhuman#5634 is that these two are *different*, and a later
/// simplification to one blanket label would have to break one of them.
#[tokio::test]
async fn a_local_triage_escalation_is_allowed_without_a_prompt() {
    use crate::openhuman::agent::triage::local_trigger_origin;

    let (gate, _dir) = test_gate();
    let outcome = turn_origin::with_origin(
        local_trigger_origin(),
        gate.intercept(
            "triage.escalate",
            "escalate to orchestrator",
            serde_json::json!({}),
        ),
    )
    .await;

    assert!(
        matches!(outcome, GateOutcome::Allow),
        "a locally initiated escalation must not be gated, got {outcome:?}"
    );
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "a trust-root origin persists no pending row"
    );
}

#[tokio::test]
async fn intercept_with_unknown_origin_denies() {
    // Unlabelled call site (no origin scope) maps to `Unknown` and is
    // rejected. This replaces the previous "no chat context → Allow"
    // legacy behaviour: the gate now refuses to execute external_effect
    // tools from unlabelled call sites.
    let (gate, _dir) = test_gate();
    let outcome = gate
        .intercept("shell", "run ls", serde_json::json!({}))
        .await;
    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("origin label")),
        other => panic!("expected deny, got {other:?}"),
    }
    assert!(gate.pending_for_thread("thread-42").is_none());
}

#[tokio::test]
async fn intercept_with_trusted_cron_origin_allows_without_prompt() {
    // Cron jobs the user explicitly authorized run trusted automation;
    // the gate allows without prompt and does not persist a row.
    let (gate, _dir) = test_gate();
    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "cron-42".into(),
        source: TrustedAutomationSource::Cron,
    };
    let outcome = turn_origin::with_origin(
        origin,
        gate.intercept("shell", "run ls", serde_json::json!({})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "trusted cron must not persist a pending row"
    );
}

#[tokio::test]
async fn intercept_with_workflow_origin_trust_root_allows_without_prompt() {
    // A saved+enabled flow's pre-declared tool/HTTP action (trust root,
    // `require_approval: false`) is allowed without a prompt.
    let (gate, _dir) = test_gate();
    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "flow-1".into(),
        source: TrustedAutomationSource::Workflow {
            require_approval: false,
        },
    };
    let outcome = turn_origin::with_origin(
        origin,
        gate.intercept("composio", "post to slack", serde_json::json!({})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "a trusted workflow action must not persist a pending row"
    );
}

#[tokio::test]
async fn intercept_with_workflow_require_approval_persists_and_ttl_denies() {
    // A per-flow `require_approval: true` toggle forces every external
    // action through the HITL gate even though the origin carries a
    // trust root — same conservative park-and-audit shape as
    // `GoalContinuation` / `ExternalChannel`, since there is no flow
    // review surface to route the prompt to yet (B3).
    let (gate, _dir, _env) = expiry_gate();
    let gate = Arc::new(gate);
    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "flow-2".into(),
        source: TrustedAutomationSource::Workflow {
            require_approval: true,
        },
    };

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            origin,
            g.intercept("composio", "post to slack", serde_json::json!({})),
        )
        .await
    });

    let mut tries = 0;
    loop {
        if !gate.list_pending().unwrap().is_empty() {
            break;
        }
        tries += 1;
        assert!(
            tries < 50,
            "audit row never appeared for require_approval workflow origin"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let outcome = handle.await.unwrap();
    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("timed out")),
        other => panic!("expected deny, got {other:?}"),
    }
}
