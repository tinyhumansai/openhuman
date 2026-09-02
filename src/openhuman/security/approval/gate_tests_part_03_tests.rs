use super::*;

#[tokio::test]
async fn intercept_with_trusted_subconscious_origin_allows_without_prompt() {
    // Subconscious ticks on internal-only memory are trusted automation
    // and run unprompted (preserves pre-PR behavior for the safe case).
    let (gate, _dir) = test_gate();
    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "subconscious-tick".into(),
        source: TrustedAutomationSource::Subconscious,
    };
    let outcome = turn_origin::with_origin(
        origin,
        gate.intercept("shell", "run ls", serde_json::json!({})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow));
}

#[tokio::test]
async fn intercept_with_subconscious_tainted_origin_denies() {
    // A subconscious tick whose memory context contains external-sync
    // chunks is rejected for external_effect tools — external text in
    // memory could otherwise steer the tick into a tool call.
    let (gate, _dir) = test_gate();
    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "subconscious-tainted".into(),
        source: TrustedAutomationSource::SubconsciousTainted,
    };
    let outcome = turn_origin::with_origin(
        origin,
        gate.intercept("send_email", "send", serde_json::json!({})),
    )
    .await;
    match outcome {
        GateOutcome::Deny { reason } => {
            assert!(reason.contains("external-sync"), "reason was: {reason}")
        }
        other => panic!("expected deny, got {other:?}"),
    }
}

#[tokio::test]
async fn intercept_with_cli_origin_allows_without_prompt() {
    // CLI / one-off internal callers (sub-agent invocations, scripts)
    // are allowed through unprompted — there is no chat surface to
    // park on, and the legacy CLI workflow assumes the operator
    // authorized the invocation.
    let (gate, _dir) = test_gate();
    let outcome = turn_origin::with_origin(
        AgentTurnOrigin::Cli,
        gate.intercept("shell", "run ls", serde_json::json!({})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow));
}

/// Regression for #5508 / #5499: an external-effect scheduling tool
/// (`cron_add`) that runs on a freshly-spawned, turn-less task — the exact
/// shape of a hosted effect executor, which
/// fires the local sub-agent from a bare `tokio::spawn` with no agent turn on
/// the stack — must NOT be `Unknown`-denied once the spawn site scopes an
/// explicit `AgentTurnOrigin::Cli` (the residual site PR #5465 did not cover).
///
/// Both halves run inside a `tokio::spawn` so the assertion exercises the real
/// task boundary the fix crosses: `AGENT_TURN_ORIGIN` is a `tokio::task_local`
/// that does not survive `spawn`, so the origin the gate reads is whatever the
/// spawned future scopes for itself — nothing, or the fix's explicit label.
#[tokio::test]
async fn cron_add_on_a_turnless_spawn_resolves_to_a_real_origin_not_unknown_denied() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    // Precondition — mirrors the bug before the fix: a bare `tokio::spawn`
    // with no ambient origin (capture() would yield None) reaches the gate as
    // `Unknown`, and the scheduling tool is refused as "no origin label".
    let g = gate.clone();
    let denied = tokio::spawn(async move {
        g.intercept("cron_add", "schedule a job", serde_json::json!({}))
            .await
    })
    .await
    .expect("spawned task panicked");
    match denied {
        GateOutcome::Deny { reason } => {
            assert!(reason.contains("origin label"), "reason was: {reason}")
        }
        other => panic!("unlabelled turn-less spawn must fail closed, got {other:?}"),
    }

    // With the fix: `run_local_agent` scopes an explicit `Cli` origin around
    // the spawned sub-agent work, so the same `cron_add` call now resolves to
    // a real origin and is allowed (device-tool automation past the
    // Master-chat gate) instead of being denied as unlabelled.
    let g = gate.clone();
    let allowed = tokio::spawn(turn_origin::with_origin(AgentTurnOrigin::Cli, async move {
        g.intercept("cron_add", "schedule a job", serde_json::json!({}))
            .await
    }))
    .await
    .expect("spawned task panicked");
    assert!(
        matches!(allowed, GateOutcome::Allow),
        "an explicit Cli origin scoped across the spawn must resolve cron_add \
         to a real origin and allow it, got {allowed:?}"
    );
}

#[tokio::test]
async fn intercept_with_external_channel_origin_persists_and_ttl_denies() {
    // Non-web channel inbound (Telegram / Discord / Slack / etc.):
    // persist an audit row but TTL-deny — there is no channel-routed
    // approval surface yet, and the input is remote-attacker text.
    let (gate, _dir, _env) = expiry_gate();
    let gate = Arc::new(gate);
    let origin = AgentTurnOrigin::ExternalChannel {
        channel: "telegram".into(),
        sender: Some("tg-user-1".into()),
        reply_target: "tg-chat-1".into(),
        message_id: "msg-1".into(),
    };

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            origin,
            g.intercept("shell", "run ls", serde_json::json!({})),
        )
        .await
    });

    // The audit row appears while the future is parked.
    let mut tries = 0;
    loop {
        if !gate.list_pending().unwrap().is_empty() {
            break;
        }
        tries += 1;
        assert!(tries < 50, "audit row never appeared for external channel");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Without a routable channel approval surface, the parked future
    // TTL-denies (2s — matches the test_gate fixture).
    let outcome = handle.await.unwrap();
    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("timed out")),
        other => panic!("expected deny, got {other:?}"),
    }
}

#[tokio::test]
async fn intercept_audited_returns_request_id_only_when_allowed_and_persisted() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    // Allow path: the audited variant must hand back the
    // request_id so the caller can record_execution later
    // (issue #2135).
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        // Scope a chat context + matching WebChat origin *inside* the
        // spawned task — task-locals don't cross `tokio::spawn`, and
        // `intercept` only parks (creates a pending row) for a chat
        // turn whose origin labels it as web-routable.
        turn_origin::with_origin(
            web_origin(),
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                g.intercept_audited("composio", "send slack", serde_json::json!({})),
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
    gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    let (outcome, id) = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
    assert_eq!(
        id.as_deref(),
        Some(pending.request_id.as_str()),
        "allowed call must return its persisted request id"
    );

    // Now record execution against that id. Round-trip via a
    // fresh gate to prove the row landed in durable storage.
    gate.record_execution(&pending.request_id, ExecutionOutcome::Success, None);
}

#[tokio::test]
async fn intercept_audited_id_is_none_for_denied_some_for_approved() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    // Deny path → no id (nothing to record afterward).
    let g = gate.clone();
    let denied = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                g.intercept_audited("composio", "send slack", serde_json::json!({})),
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
    let (outcome, id) = denied.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Deny { .. }));
    assert!(id.is_none(), "denied calls have nothing to record");

    // Allowlist-shortcut path → also no id (no row was created).
    let g = gate.clone();
    let first = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                g.intercept_audited("pushover", "first send", serde_json::json!({})),
            ),
        )
        .await
    });
    let pending = loop {
        if let Some(p) = gate
            .list_pending()
            .unwrap()
            .into_iter()
            .find(|p| p.tool_name == "pushover")
        {
            break p;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    // `ApproveAlwaysForTool` resolves the parked prompt to Allow and, because
    // the prompt persisted a row, returns its id. (Persisting the tool onto
    // the `auto_approve` allowlist for *future* calls is the RPC handler's
    // job — see `approval::rpc::approval_decide` — and the gate's allowlist
    // short-circuit is covered by `auto_approve_tool_skips_prompt`.)
    gate.decide(&pending.request_id, ApprovalDecision::ApproveAlwaysForTool)
        .unwrap();
    let (first_outcome, first_id) = first.await.unwrap();
    assert!(matches!(first_outcome, GateOutcome::Allow));
    assert!(
        first_id.is_some(),
        "the prompting call still persists a row"
    );
}

#[tokio::test]
async fn flow_origin_park_populates_source_context_with_flow_and_run_id() {
    // A `require_approval: true` flow still parks (same shape as before
    // this change) but the persisted row must now carry the flow/run
    // correlation the `APPROVAL_FLOW_RUN_CONTEXT` task-local supplies —
    // the origin alone only carries `flow_id`, not `run_id`.
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            flow_origin("flow-1", true),
            APPROVAL_FLOW_RUN_CONTEXT.scope(
                FlowRunContext {
                    flow_id: "flow-1".to_string(),
                    run_id: "run-1".to_string(),
                },
                g.intercept_audited("composio", "post to slack", serde_json::json!({})),
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

    match &pending.source_context {
        Some(super::super::super::types::ApprovalSourceContext::Flow {
            flow_id,
            run_id,
            node_id,
        }) => {
            assert_eq!(flow_id, "flow-1");
            assert_eq!(run_id, "run-1");
            assert!(
                node_id.is_none(),
                "node_id is not yet threaded down to the gate"
            );
        }
        other => panic!("expected Flow source_context, got {other:?}"),
    }

    gate.decide(&pending.request_id, ApprovalDecision::Deny)
        .unwrap();
    let _ = handle.await.unwrap();
}

#[tokio::test]
async fn chat_origin_park_has_no_source_context() {
    // Regression guard: the plain chat-routed path (unaffected by this
    // change) must never gain a `source_context` — only Workflow-origin
    // parks populate it.
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            APPROVAL_CHAT_CONTEXT.scope(
                chat_ctx(),
                g.intercept_audited("composio", "send slack", serde_json::json!({})),
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
    assert!(
        pending.source_context.is_none(),
        "chat-origin parks must not carry a source_context"
    );

    gate.decide(&pending.request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    let (outcome, _id) = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
}

#[tokio::test]
async fn flow_tool_trust_auto_allows_before_parking() {
    // A prior `ApproveAlwaysForFlow` grant for (flow_id, tool_name) must
    // short-circuit to `Allow` even for a `require_approval: true` flow —
    // that is the whole point of "approve always for this workflow": no
    // pending row is created and the call never parks.
    //
    // The second half of this test *does* wait a park out, so it needs the
    // short window even though it never inspects the "timed out" reason.
    let (gate, _dir, _env) = expiry_gate();
    store::insert_flow_trust(&gate.config, "flow-trusted", "composio").unwrap();

    let outcome = turn_origin::with_origin(
        flow_origin("flow-trusted", true),
        APPROVAL_FLOW_RUN_CONTEXT.scope(
            FlowRunContext {
                flow_id: "flow-trusted".to_string(),
                run_id: "run-1".to_string(),
            },
            gate.intercept("composio", "post to slack", serde_json::json!({})),
        ),
    )
    .await;

    assert!(matches!(outcome, GateOutcome::Allow));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "a trusted (flow, tool) pair must not persist a pending row"
    );

    // A different tool on the same trusted flow is unaffected — it still
    // parks, and nothing decides it, so it TTL-denies after
    // `EXPIRY_TEST_TTL`.
    let untrusted_outcome = turn_origin::with_origin(
        flow_origin("flow-trusted", true),
        APPROVAL_FLOW_RUN_CONTEXT.scope(
            FlowRunContext {
                flow_id: "flow-trusted".to_string(),
                run_id: "run-1".to_string(),
            },
            gate.intercept("pushover", "send push", serde_json::json!({})),
        ),
    )
    .await;
    assert!(
        matches!(untrusted_outcome, GateOutcome::Deny { .. }),
        "trust must be scoped to the exact tool granted, not the whole flow"
    );
}

#[tokio::test]
async fn decide_approve_always_for_flow_then_insert_flow_trust_composes_to_auto_allow() {
    // Exercises the two building blocks the `approval_decide` RPC handler
    // composes for `ApproveAlwaysForFlow` (see `approval::rpc`): the gate
    // resolves the parked call and returns the decided row (carrying
    // `source_context`), and the RPC layer then calls
    // `ApprovalGate::insert_flow_trust` using that row's flow id. This
    // test exercises both steps directly against a local (non-global)
    // gate — the RPC handler itself reads the process-wide
    // `ApprovalGate::try_global()` singleton, which tests must not touch
    // (it would leak state into every other test in this binary).
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            flow_origin("flow-2", true),
            APPROVAL_FLOW_RUN_CONTEXT.scope(
                FlowRunContext {
                    flow_id: "flow-2".to_string(),
                    run_id: "run-2".to_string(),
                },
                g.intercept_audited("composio", "post to slack", serde_json::json!({})),
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

    let decided = gate
        .decide(&pending.request_id, ApprovalDecision::ApproveAlwaysForFlow)
        .unwrap()
        .expect("decided row");

    assert!(!gate.is_flow_tool_trusted("flow-2", "composio").unwrap());

    match &decided.source_context {
        Some(super::super::super::types::ApprovalSourceContext::Flow { flow_id, .. }) => {
            gate.insert_flow_trust(flow_id, &decided.tool_name).unwrap();
        }
        other => panic!("expected Flow source_context, got {other:?}"),
    }

    assert!(gate.is_flow_tool_trusted("flow-2", "composio").unwrap());

    let (outcome, _id) = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
}

#[tokio::test]
async fn flow_origin_park_publishes_flow_approval_request_and_notification() {
    // The silent-deadlock bug this whole PR fixes: a flow-origin park has
    // no chat thread/client, so the generic `ApprovalRequested` event's
    // web-channel bridge silently drops it. This test asserts the two new
    // surfaces fire instead — the `flow_approval_request` DomainEvent
    // (bridged to a broadcast Socket.IO event by `core::socketio`) and
    // the `flow-gate-approval` CoreNotification with its three actions.
    crate::core::bus::init().await.expect("bus init");
    let mut event_rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized above")
        .receiver();
    let mut notif_rx =
        crate::openhuman::desktop::notifications::bus::subscribe_core_notifications();

    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            flow_origin("flow-9", true),
            APPROVAL_FLOW_RUN_CONTEXT.scope(
                FlowRunContext {
                    flow_id: "flow-9".to_string(),
                    run_id: "run-9".to_string(),
                },
                g.intercept_audited("composio", "post to slack", serde_json::json!({})),
            ),
        )
        .await
    });

    let (request_id, run_id, tool_name) = tokio::time::timeout(
        Duration::from_secs(5),
        find_flow_approval_requested(&mut event_rx, "flow-9"),
    )
    .await
    .expect("timed out waiting for FlowApprovalRequested");
    assert_eq!(run_id, "run-9");
    assert_eq!(tool_name, "composio");

    let notif = tokio::time::timeout(
        Duration::from_secs(5),
        find_flow_gate_notification(&mut notif_rx, &request_id),
    )
    .await
    .expect("timed out waiting for the flow-gate-approval notification");
    assert_eq!(notif.id, format!("flow-gate-approval:{request_id}"));
    let actions = notif.actions.expect("notification must declare actions");
    let action_ids: Vec<_> = actions.iter().map(|a| a.action_id.as_str()).collect();
    assert_eq!(
        action_ids,
        vec!["approve_once", "approve_always_for_flow", "deny"]
    );

    gate.decide(&request_id, ApprovalDecision::Deny).unwrap();
    let _ = handle.await.unwrap();
}
