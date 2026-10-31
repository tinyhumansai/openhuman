//! Steering a parked call: the operator refuses *this* call and hands the
//! agent a directive instead.
//!
//! The property these tests exist for is that a steer takes the **denial**
//! path through the engine. Nothing runs, so nothing may be reserved — a
//! reservation is irreversible (`ApprovalLifecycle2::Reserved`) and would
//! consume an approval for a call that never executes.

use super::*;
use crate::security::approval::engine_stub::State;

/// Park one call and return its request id once the engine delivered the
/// prompt and the waiter is registered.
async fn park_one(
    gate: &Arc<ApprovalGate>,
    tool: &'static str,
) -> (tokio::task::JoinHandle<GateOutcome>, String) {
    let worker = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            worker.intercept(
                &ApprovalTurnContext::chat(chat_ctx()),
                tool,
                "git push --force origin main",
                serde_json::json!({}),
            ),
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
async fn a_steer_hands_the_agent_the_directive_instead_of_running_the_tool() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let (handle, request_id) = park_one(&gate, "shell").await;

    let directive = "push to a branch fix/login and open a PR instead";
    assert!(gate
        .decide_with_directive(
            &request_id,
            ApprovalDecision::Steer,
            Some(directive.to_string()),
        )
        .unwrap()
        .is_some());

    match handle.await.unwrap() {
        GateOutcome::Steer { instructions } => assert_eq!(instructions, directive),
        other => panic!("expected GateOutcome::Steer, got {other:?}"),
    }
}

#[tokio::test]
async fn a_steer_is_a_denial_to_the_engine_and_reserves_nothing() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let stub = stub_of(&gate);
    let (handle, request_id) = park_one(&gate, "shell").await;

    gate.decide_with_directive(
        &request_id,
        ApprovalDecision::Steer,
        Some("do it differently".to_string()),
    )
    .unwrap();
    assert!(handle.await.unwrap().is_steer());

    // The module recorded a refusal — not an approval, and above all not a
    // reservation: the tool never ran.
    assert_eq!(stub.state(&request_id), Some(State::Denied));
}

#[tokio::test]
async fn a_steer_without_a_directive_denies_rather_than_steering_emptily() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let (handle, request_id) = park_one(&gate, "shell").await;

    // The RPC rejects an empty directive; this is the belt behind it.
    gate.decide_with_directive(
        &request_id,
        ApprovalDecision::Steer,
        Some("   ".to_string()),
    )
    .unwrap();

    match handle.await.unwrap() {
        GateOutcome::Deny { reason } => assert!(reason.contains("no instructions"), "{reason}"),
        other => panic!("a directive-less steer must deny, got {other:?}"),
    }
}

#[tokio::test]
async fn a_forced_request_cannot_be_steered() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let worker = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            worker.intercept_forced(
                "browser",
                "Click Submit on checkout",
                serde_json::json!({"action": "click", "target": "Submit"}),
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
        assert!(tries < 500, "the forced call never parked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    // A forced approval was escalated *because* it needs an explicit yes or
    // no; redirecting it would weaken that posture.
    let err = gate
        .decide_with_directive(
            &request_id,
            ApprovalDecision::Steer,
            Some("click the other button".to_string()),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("one-time approval or denial"),
        "{err}"
    );

    // The call is still parked, so answer it properly and let the turn finish.
    decide_parked(&gate, &request_id, ApprovalDecision::Deny);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Deny { .. }));
}

#[tokio::test]
async fn a_steer_is_published_as_its_own_decision() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized above")
        .receiver();
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let (handle, request_id) = park_one(&gate, "browser_click").await;

    let directive = "use the company card, not card X";
    gate.decide_with_directive(
        &request_id,
        ApprovalDecision::Steer,
        Some(directive.to_string()),
    )
    .unwrap();
    assert!(handle.await.unwrap().is_steer());

    match find_approval_decided(&mut rx, &request_id).await {
        crate::core::events::DomainEvent::ApprovalDecided {
            decision,
            resolution,
            ..
        } => {
            assert_eq!(decision, "steer");
            // `resolution` marks a request that ended *without* a person's
            // decision (cancelled/expired). A steer is a decision.
            assert_eq!(resolution, None);
            // The directive is host-only and must never ride the bus.
            assert!(!format!("{decision:?}").contains(directive));
        }
        other => panic!("expected ApprovalDecided, got {other:?}"),
    }
}
