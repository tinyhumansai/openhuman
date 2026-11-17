use super::*;

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
    use crate::triage::{remote_trigger_origin, TriggerEnvelope};

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
    let turn = ApprovalTurnContext::default();
    let mut fut = Box::pin(turn_origin::with_origin(
        remote_trigger_origin(&envelope),
        gate.intercept(
            &turn,
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
    use crate::triage::local_trigger_origin;

    let (gate, _dir) = test_gate();
    let outcome = turn_origin::with_origin(
        local_trigger_origin(),
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
        .intercept(
            &ApprovalTurnContext::default(),
            "shell",
            "run ls",
            serde_json::json!({}),
        )
        .await;
    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("origin label")),
        other => panic!("expected deny, got {other:?}"),
    }
    assert!(gate.pending_for_thread("thread-42").is_none());
}

#[tokio::test]
async fn intercept_with_trusted_cron_origin_denies_external_effect() {
    // Creating a cron job does not authorize its future tool effects.
    let (gate, _dir) = test_gate();
    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "cron-42".into(),
        source: TrustedAutomationSource::Cron,
    };
    let outcome = turn_origin::with_origin(
        origin,
        gate.intercept(
            &ApprovalTurnContext::default(),
            "shell",
            "run ls",
            serde_json::json!({}),
        ),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }));
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "denied cron effects must not persist a pending row"
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
        gate.intercept(
            &ApprovalTurnContext::default(),
            "composio",
            "post to slack",
            serde_json::json!({}),
        ),
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
    // `ExternalChannel`, since there is no flow
    // review surface to route the prompt to yet (B3).
    let (gate, _dir, env) = expiry_gate().await;
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
            g.intercept(
                &ApprovalTurnContext::default(),
                "composio",
                "post to slack",
                serde_json::json!({}),
            ),
        )
        .await
    });

    let mut tries = 0;
    while parked_request_id(&gate).is_none() {
        tries += 1;
        assert!(
            tries < 50,
            "approval waiter never appeared for require_approval workflow origin"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    drop(env);
    let outcome = handle.await.unwrap();
    match outcome {
        GateOutcome::Deny { reason } => assert!(reason.contains("timed out")),
        other => panic!("expected deny, got {other:?}"),
    }
}

/// A parked approval must be recoverable from its thread alone.
///
/// The card is delivered to the UI as ONE fire-and-forget socket emit
/// (`web_chat::event_bus` → `openhuman_rpc::server::socketio::emit_web_channel_event`). If that
/// emit misses — the addressed client's room is empty because it reloaded, the
/// rejoining socket was not yet in the thread room, or the bridge dropped the
/// frame on broadcast lag — nothing re-sends it, and the turn stays parked with
/// no card and no way for the user to act. Recovering the full row from the
/// thread is what lets a (re)joining socket rebuild the card, so the durable
/// park stops depending on a single delivery.
#[tokio::test]
async fn a_parked_approval_is_recoverable_from_its_thread_for_replay() {
    let (gate, _dir) = test_gate_with_ttl(Duration::from_secs(10));
    let gate = Arc::new(gate);

    let g = gate.clone();
    let ctx = ApprovalChatContext {
        thread_id: "thread-replay".into(),
        client_id: "client-that-went-away".into(),
        request_id: None,
    };
    let origin = AgentTurnOrigin::WebChat {
        thread_id: "thread-replay".into(),
        client_id: "client-that-went-away".into(),
        request_id: Some("req-replay".into()),
    };
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            origin,
            g.intercept(
                &ApprovalTurnContext::chat(ctx),
                "create_workflow",
                "create a workflow",
                serde_json::json!({ "name": "nightly" }),
            ),
        )
        .await
    });

    let mut tries = 0;
    loop {
        if gate.pending_for_thread("thread-replay").is_some() {
            break;
        }
        tries += 1;
        assert!(tries < 50, "thread mapping never appeared");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let row = gate.parked_request_for_thread("thread-replay").expect(
        "a parked approval must be recoverable from its thread, or a client that missed the \
         single live emit can never rebuild the card and the turn stays parked forever",
    );
    assert_eq!(
        row.tool_name, "create_workflow",
        "the recovered row must carry the payload the card renders"
    );
    assert_eq!(
        gate.pending_for_thread("thread-replay").as_deref(),
        Some(row.request_id.as_str()),
        "the recovered row must be the one actually parked on this thread"
    );

    // Another thread must not inherit it — replay is thread-scoped.
    assert!(
        gate.parked_request_for_thread("thread-unrelated").is_none(),
        "a thread with nothing parked must have nothing to replay"
    );

    gate.decide(&row.request_id, ApprovalDecision::Deny)
        .unwrap();
    let _ = handle.await.unwrap();

    assert!(
        gate.parked_request_for_thread("thread-replay").is_none(),
        "a decided approval must not be replayed to the next socket that joins"
    );
}

#[tokio::test]
async fn intercept_audited_for_call_threads_tool_call_id_onto_the_pending_row_and_request() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept_audited_for_call(
                &ApprovalTurnContext::chat(chat_ctx()),
                "composio",
                "send slack",
                serde_json::json!({}),
                Some("call-abc"),
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
        assert!(tries < 50, "pending row never appeared");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(pending.tool_call_id.as_deref(), Some("call-abc"));

    decide_parked(&gate, &pending.request_id, ApprovalDecision::ApproveOnce);
    let (outcome, _id) = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
}

#[tokio::test]
async fn timeout_publishes_approval_decided_with_expired_resolution() {
    crate::core::bus::init().await.expect("bus init");
    let mut event_rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized above")
        .receiver();

    let (gate, _dir, env) = expiry_gate().await;
    let gate = Arc::new(gate);
    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            g.intercept_audited_for_call(
                &ApprovalTurnContext::chat(chat_ctx()),
                "composio",
                "timed out",
                serde_json::json!({}),
                Some("call-expire"),
            ),
        )
        .await
    });
    let mut tries = 0;
    let request_id = loop {
        if let Some(p) = gate.list_pending().unwrap().into_iter().next() {
            break p.request_id;
        }
        tries += 1;
        assert!(tries < 50, "audit row never appeared for timeout test");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    drop(env);

    let event = tokio::time::timeout(
        Duration::from_secs(5),
        find_approval_decided(&mut event_rx, &request_id),
    )
    .await
    .expect("timed out waiting for ApprovalDecided");
    match event {
        crate::core::events::DomainEvent::ApprovalDecided {
            decision,
            resolution,
            tool_call_id,
            ..
        } => {
            assert_eq!(decision, "deny");
            assert_eq!(resolution.as_deref(), Some("expired"));
            assert_eq!(tool_call_id.as_deref(), Some("call-expire"));
        }
        other => panic!("expected ApprovalDecided, got {other:?}"),
    }

    let (outcome, _id) = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Deny { .. }));
}
