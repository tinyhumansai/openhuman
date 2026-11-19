use super::*;

#[tokio::test]
async fn forced_approval_parks_with_autonomy_disabled_and_releases_exact_request() {
    let (gate, _dir) = test_gate();
    assert!(!gate.config.autonomy.enabled);
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

    let pending = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(row) = gate.list_pending().unwrap().into_iter().next() {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("forced approval never parked");
    assert_eq!(pending.tool_name, "browser");
    assert_eq!(pending.action_summary, "Click Submit on checkout");
    assert_eq!(pending.args_redacted["target"], "Submit");
    assert!(!handle.is_finished());
    decide_parked(&gate, &pending.request_id, ApprovalDecision::ApproveOnce);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));
    assert!(gate.list_pending().unwrap().is_empty());
}

#[tokio::test]
async fn forced_approval_denies_without_routable_web_chat_origin() {
    let (gate, _dir) = test_gate();
    let outcome = gate
        .intercept_forced("browser", "click", serde_json::json!({}))
        .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }));
    assert!(gate.list_pending().unwrap().is_empty());

    let outcome = turn_origin::with_origin(
        AgentTurnOrigin::WebChat {
            thread_id: String::new(),
            client_id: "client".into(),
            request_id: None,
        },
        gate.intercept_forced("browser", "click", serde_json::json!({})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }));
}

#[tokio::test]
async fn forced_approval_ignores_auto_approval_and_rejects_persistent_grants() {
    let (mut gate, _dir) = test_gate();
    gate.config.autonomy.auto_approve_all = true;
    gate.config.autonomy.auto_approve = vec!["browser_forced_test".into()];
    let gate = Arc::new(gate);
    let worker = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            web_origin(),
            worker.intercept_forced("browser_forced_test", "click", serde_json::json!({})),
        )
        .await
    });
    let request_id = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(id) = parked_request_id(&gate) {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("forced approval with auto flags never parked");
    assert!(!handle.is_finished());
    assert!(gate
        .decide(&request_id, ApprovalDecision::ApproveAlwaysForTool)
        .is_err());
    assert!(!handle.is_finished());
    decide_parked(&gate, &request_id, ApprovalDecision::Deny);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Deny { .. }));
}

/// `[browser] unattended_actions` lives in the browser tool, not here: the
/// forced gate itself still refuses every automation and channel origin, so
/// only an operator's explicit browser allow-list lets one through.
#[tokio::test]
async fn forced_approval_still_denies_automation_and_channel_origins() {
    let (gate, _dir) = test_gate();
    for origin in [
        AgentTurnOrigin::TrustedAutomation {
            job_id: "cron-1".into(),
            source: TrustedAutomationSource::Cron,
        },
        AgentTurnOrigin::TrustedAutomation {
            job_id: "flow-1".into(),
            source: TrustedAutomationSource::Workflow {
                require_approval: false,
            },
        },
        AgentTurnOrigin::ExternalChannel {
            channel: "telegram".into(),
            sender: None,
            sender_name: None,
            reply_target: "chat".into(),
            message_id: "m".into(),
            history_key: None,
            channel_session: None,
        },
        AgentTurnOrigin::Unknown,
    ] {
        let outcome = turn_origin::with_origin(
            origin.clone(),
            gate.intercept_forced("browser", "click", serde_json::json!({})),
        )
        .await;
        assert!(matches!(outcome, GateOutcome::Deny { .. }), "{origin:?}");
    }
    assert!(gate.list_pending().unwrap().is_empty());
}

#[test]
fn saas_outcome_is_absent_outside_saas() {
    assert!(saas_outcome_with(
        &crate::core::runtime::DispatchCtx::ambient(),
        false,
        &web_origin(),
        false,
        "shell"
    )
    .is_none());
}

#[test]
fn saas_web_chat_with_a_prompt_route_can_request_human_approval() {
    assert!(saas_prompt_needed(
        &crate::core::runtime::DispatchCtx::ambient(),
        true,
        &web_origin(),
        "shell",
        false
    ));
}

#[test]
fn saas_outcome_never_parks_and_denies_without_an_allowlisted_group() {
    // No agent host is installed here, so no tool group is allowlisted.
    match saas_outcome_with(
        &crate::core::runtime::DispatchCtx::ambient(),
        true,
        &AgentTurnOrigin::Cli,
        false,
        "shell",
    ) {
        Some(GateOutcome::Deny { reason }) => {
            assert!(reason.starts_with(POLICY_DENIED_MARKER), "{reason}");
            assert!(reason.contains("no approval surface for users"), "{reason}");
        }
        other => panic!("expected an immediate deny, got {other:?}"),
    }
}

#[test]
fn saas_web_chat_falls_through_to_per_user_approval_when_allowlist_rejects() {
    assert!(saas_outcome_with(
        &crate::core::runtime::DispatchCtx::ambient(),
        true,
        &web_origin(),
        false,
        "shell"
    )
    .is_none());
}

#[test]
fn saas_forced_approval_needs_a_routable_web_chat_surface() {
    assert!(matches!(
        saas_outcome_with(
            &crate::core::runtime::DispatchCtx::ambient(),
            true,
            &AgentTurnOrigin::Cli,
            true,
            "shell"
        ),
        Some(GateOutcome::Deny { .. })
    ));
    assert!(saas_prompt_needed(
        &crate::core::runtime::DispatchCtx::ambient(),
        true,
        &web_origin(),
        "shell",
        true
    ));
}

#[test]
fn a_saas_human_approval_uses_the_one_time_variant_and_route() {
    assert_eq!(
        one_time_approval_mode(Variant::Standard, false, true),
        (Variant::Forced, true)
    );
    assert_eq!(
        one_time_approval_mode(Variant::Standard, false, false),
        (Variant::Standard, false)
    );
}

#[test]
fn saas_outcome_maps_an_allowlisted_verdict_to_allow() {
    use crate::profiles::tools::{gate_verdict_with, SaasToolGroup};
    let verdict = gate_verdict_with("shell", &[SaasToolGroup::HostShell]);
    assert!(matches!(saas_outcome(verdict), GateOutcome::Allow));
    let verdict = gate_verdict_with("shell", &[SaasToolGroup::HostFiles]);
    assert!(matches!(saas_outcome(verdict), GateOutcome::Deny { .. }));
}

/// Wait for the gate's single parked request, failing the test if none parks.
async fn wait_parked(gate: &ApprovalGate) -> PendingApproval {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(row) = gate.list_pending().unwrap().into_iter().next() {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the forced approval never parked")
}

/// Settings saves an API key over RPC with no chat turn behind it (the origin
/// is `Unknown`). The native secret master grant that save needs parks as an
/// unrouted pending approval the desktop shows, instead of being denied before
/// anyone is asked; an approval lets the save continue.
#[tokio::test]
async fn forced_secret_grant_without_a_chat_route_parks_unrouted_and_allows_on_approve() {
    for origin in [
        None,
        Some(AgentTurnOrigin::Cli),
        Some(AgentTurnOrigin::DirectChat),
    ] {
        let (gate, _dir) = test_gate();
        let gate = Arc::new(gate);
        let worker = Arc::clone(&gate);
        let call_origin = origin.clone();
        let handle = tokio::spawn(async move {
            let call = worker.intercept_forced(
                "native_secret_grant",
                "Unlock configuration secrets",
                serde_json::json!({"registration_id": "opaque-registration"}),
            );
            match call_origin {
                Some(origin) => turn_origin::with_origin(origin, call).await,
                None => call.await,
            }
        });
        let pending = wait_parked(&gate).await;
        assert_eq!(pending.tool_name, "native_secret_grant", "{origin:?}");
        assert_eq!(pending.action_summary, "Unlock configuration secrets");
        assert!(
            !handle.is_finished(),
            "{origin:?}: must wait for the person"
        );
        let seen = stub_of(&gate).seen();
        let call = seen.last().expect("the engine was asked");
        assert_eq!(call.variant, Variant::Forced, "{origin:?}: one-time grant");
        assert_eq!(call.thread, None, "{origin:?}: no chat thread to route to");
        assert_eq!(
            call.origin,
            CallOrigin::Operator,
            "{origin:?}: a person decides"
        );
        decide_parked(&gate, &pending.request_id, ApprovalDecision::ApproveOnce);
        assert!(
            matches!(handle.await.unwrap(), GateOutcome::Allow),
            "{origin:?}"
        );
    }
}

#[tokio::test]
async fn forced_secret_grant_without_a_chat_route_fails_closed_on_deny() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let worker = Arc::clone(&gate);
    let handle = tokio::spawn(async move {
        worker
            .intercept_forced(
                "native_secret_grant",
                "Unlock configuration secrets",
                serde_json::json!({"registration_id": "opaque-registration"}),
            )
            .await
    });
    let pending = wait_parked(&gate).await;
    decide_parked(&gate, &pending.request_id, ApprovalDecision::Deny);
    match handle.await.unwrap() {
        GateOutcome::Deny { reason } => assert!(reason.contains("denied"), "{reason}"),
        other => panic!("a denied secret grant must fail closed, got {other:?}"),
    }
    assert!(gate.list_pending().unwrap().is_empty());
}

#[tokio::test]
async fn forced_secret_grant_from_automation_or_a_channel_still_denies_without_asking() {
    let (gate, _dir) = test_gate();
    for origin in [
        AgentTurnOrigin::TrustedAutomation {
            job_id: "cron-1".into(),
            source: TrustedAutomationSource::Cron,
        },
        AgentTurnOrigin::TrustedAutomation {
            job_id: "bg-1".into(),
            source: TrustedAutomationSource::Background,
        },
        AgentTurnOrigin::ExternalChannel {
            channel: "telegram".into(),
            sender: None,
            sender_name: None,
            reply_target: "chat".into(),
            message_id: "m".into(),
            history_key: None,
            channel_session: None,
        },
    ] {
        let outcome = turn_origin::with_origin(
            origin.clone(),
            gate.intercept_forced(
                "native_secret_grant",
                "Unlock configuration secrets",
                serde_json::json!({"registration_id": "opaque-registration"}),
            ),
        )
        .await;
        assert!(matches!(outcome, GateOutcome::Deny { .. }), "{origin:?}");
    }
    assert!(gate.list_pending().unwrap().is_empty());
}

/// A correlated forced grant parks exactly like the plain one and carries the
/// caller's correlation id, which is how a host answering it for the operator
/// recognises its own request among other pending grants.
#[tokio::test]
async fn correlated_forced_secret_grant_parks_with_its_correlation_id() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let worker = Arc::clone(&gate);
    let handle = tokio::spawn(async move {
        worker
            .intercept_forced_correlated(
                "native_secret_grant",
                "Unlock configuration secrets",
                serde_json::json!({"registration_id": "opaque-registration"}),
                "corr-1",
            )
            .await
    });
    let pending = wait_parked(&gate).await;
    assert_eq!(pending.tool_name, "native_secret_grant");
    assert_eq!(pending.tool_call_id.as_deref(), Some("corr-1"));
    assert_eq!(
        stub_of(&gate).seen().last().unwrap().variant,
        Variant::Forced
    );
    decide_parked(&gate, &pending.request_id, ApprovalDecision::ApproveOnce);
    assert!(matches!(handle.await.unwrap(), GateOutcome::Allow));
}
