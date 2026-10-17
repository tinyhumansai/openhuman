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
        "shell"
    )
    .is_none());
}

#[test]
fn saas_outcome_never_parks_and_denies_without_an_allowlisted_group() {
    // No agent host is installed here, so no tool group is allowlisted.
    match saas_outcome_with(&crate::core::runtime::DispatchCtx::ambient(), true, "shell") {
        Some(GateOutcome::Deny { reason }) => {
            assert!(reason.starts_with(POLICY_DENIED_MARKER), "{reason}");
            assert!(reason.contains("no approval surface"), "{reason}");
        }
        other => panic!("expected an immediate deny, got {other:?}"),
    }
}

#[test]
fn saas_outcome_maps_an_allowlisted_verdict_to_allow() {
    use crate::profiles::tools::{gate_verdict_with, SaasToolGroup};
    let verdict = gate_verdict_with("shell", &[SaasToolGroup::HostShell]);
    assert!(matches!(saas_outcome(verdict), GateOutcome::Allow));
    let verdict = gate_verdict_with("shell", &[SaasToolGroup::HostFiles]);
    assert!(matches!(saas_outcome(verdict), GateOutcome::Deny { .. }));
}
