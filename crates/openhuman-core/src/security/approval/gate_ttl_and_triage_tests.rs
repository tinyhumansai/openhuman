use super::*;
use crate::security::approval::engine_stub::State;

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
        request_id: None,
    };
    let origin = AgentTurnOrigin::WebChat {
        thread_id: "thread-42".into(),
        client_id: "client-1".into(),
        request_id: Some("req-42".into()),
    };
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            origin,
            g.intercept(
                &ApprovalTurnContext::chat(ctx),
                "shell",
                "run ls",
                serde_json::json!({}),
            ),
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
/// `turn_origin::propagate`) but NOT the chat approval context.
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

    // WebChat origin scoped, but NO chat approval context — exactly the
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
            g.intercept(
                &ApprovalTurnContext::default(),
                "cron_add",
                "schedule daily reminder",
                serde_json::json!({}),
            ),
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
             the chat approval context is absent (#5499)"
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
    let turn = ApprovalTurnContext::chat(chat_ctx());
    let mut fut = Box::pin(turn_origin::with_origin(
        web_origin(),
        g.intercept(&turn, "shell", "run rm", serde_json::json!({})),
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
        gate.parked.waiters.lock().contains_key(&request_id),
        "waiter must be registered while parked"
    );
    assert_eq!(
        stub_of(&gate).state(&request_id),
        Some(State::Pending),
        "the request must be open (undecided) while parked"
    );

    // External teardown: the wall-clock backstop tears the turn future down
    // mid-park. This skips the timeout match arms entirely.
    drop(fut);

    // The RAII guard must have run the cleanup on drop.
    assert!(
        !gate.parked.waiters.lock().contains_key(&request_id),
        "waiter must be evicted when the parked future is dropped"
    );
    assert!(
        gate.pending_for_thread("t-test").is_none(),
        "thread→request routing must be cleared on external teardown"
    );
    // The engine cancels the request off the dropped future.
    let stub = stub_of(&gate);
    let mut tries = 0;
    while stub.state(&request_id) != Some(State::Cancelled) {
        tries += 1;
        assert!(
            tries < 500,
            "the request must be cancelled when the parked future is dropped"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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
        request_id: None,
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
            g.intercept_audited_bounded(
                &ApprovalTurnContext::chat(ctx),
                "shell",
                "run ls",
                serde_json::json!({}),
                Some(Duration::from_millis(100)),
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
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let (gate, _dir) = test_gate_with_ttl(BOOT_TTL_UNDER_TEST);
    unsafe { std::env::set_var("OPENHUMAN_APPROVAL_TTL_SECS", "42") };
    assert_eq!(
        gate.effective_ttl(),
        Some(Duration::from_secs(42)),
        "valid OPENHUMAN_APPROVAL_TTL_SECS must override boot-time TTL"
    );
    unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
}

#[test]
fn effective_ttl_falls_back_to_boot_ttl_for_garbage_value() {
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let (gate, _dir) = test_gate_with_ttl(BOOT_TTL_UNDER_TEST);
    unsafe { std::env::set_var("OPENHUMAN_APPROVAL_TTL_SECS", "not-a-number") };
    assert_eq!(
        gate.effective_ttl(),
        Some(BOOT_TTL_UNDER_TEST),
        "garbage OPENHUMAN_APPROVAL_TTL_SECS must fall back to boot-time TTL"
    );
    unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
}

#[test]
fn effective_ttl_falls_back_to_boot_ttl_when_unset() {
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let (gate, _dir) = test_gate_with_ttl(BOOT_TTL_UNDER_TEST);
    unsafe { std::env::remove_var("OPENHUMAN_APPROVAL_TTL_SECS") };
    assert_eq!(
        gate.effective_ttl(),
        Some(BOOT_TTL_UNDER_TEST),
        "unset OPENHUMAN_APPROVAL_TTL_SECS must fall back to boot-time TTL"
    );
}

/// Integration regression test for the streaming-to-gate contract
/// (CodeRabbit review on PR #5112): proves `intercept_audited_inner` hands the
/// engine the copilot origin, so the stored expiry is the module's shorter
/// copilot window. Builds a gate that asks for no window of its own, sets
/// the copilot flag alongside the chat context + WebChat
/// origin the way `flows::ops::flows_build` does in production, and
/// inspects the persisted `expires_at` on the pending row.
#[tokio::test]
async fn copilot_streaming_park_persists_the_clamped_expiry() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let before = chrono::Utc::now();
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept(
                &ApprovalTurnContext::chat(chat_ctx()).with_copilot_stream(),
                "composio",
                "send slack",
                serde_json::json!({}),
            ),
        )
        .await
    });

    let mut tries = 0;
    let pending = loop {
        if let Some(p) = gate.list_pending().unwrap().into_iter().next() {
            break p;
        }
        tries += 1;
        assert!(tries < 500, "the copilot park never appeared");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    let expires_at = pending
        .expires_at
        .expect("a parked approval always sets expires_at");
    let ttl_persisted = expires_at - before;
    assert!(
        ttl_persisted
            <= chrono::Duration::from_std(MODULE_SHORT_WINDOW).unwrap()
                + chrono::Duration::seconds(5),
        "copilot-streaming park must persist an expires_at the module's copilot \
         window (180s), not its full {:?} window — got a {ttl_persisted} window",
        MODULE_DEFAULT_WINDOW
    );
    assert!(
        ttl_persisted < chrono::Duration::from_std(MODULE_DEFAULT_WINDOW).unwrap(),
        "sanity: the persisted expiry must be shorter than the unclamped default TTL"
    );

    gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    let outcome = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
}
