use super::*;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use crate::security::SecurityPolicy;

fn agent_ctx(
    id: &str,
    policy: Option<SecurityPolicy>,
    approvals_disabled: bool,
) -> Arc<CoreContext> {
    let parent = CoreContext::for_test(DomainSet::full(), Some(std::env::temp_dir()));
    let mut overlay = ContextOverlay::new(
        Config::default(),
        DomainSet::kernel(),
        crate::tools::toolpacks::ToolGroups::none(),
    )
    .session_agent(id);
    overlay.agent_policy = policy.map(Arc::new);
    overlay.approvals_disabled = approvals_disabled;
    parent.derive_with(overlay)
}

/// Runs one gated call under `ctx` and `origin`; `None` means it parked.
async fn gated_call(
    gate: &ApprovalGate,
    ctx: Arc<CoreContext>,
    origin: AgentTurnOrigin,
    tool: &str,
) -> Option<GateOutcome> {
    CoreContext::scope(
        ctx,
        turn_origin::with_origin(
            origin,
            gate.intercept_audited_bounded(
                &ApprovalTurnContext::chat(chat_ctx()),
                tool,
                "run it",
                serde_json::json!({}),
                Some(Duration::from_millis(100)),
            ),
        ),
    )
    .await
    .map(|(outcome, _)| outcome)
}

#[tokio::test]
async fn an_agent_with_the_gate_off_runs_unparked_while_a_sibling_parks() {
    let (gate, _dir) = test_gate();

    let open = gated_call(
        &gate,
        agent_ctx("gate-off", None, true),
        web_origin(),
        "shell",
    )
    .await;
    assert!(matches!(open, Some(GateOutcome::Allow)), "got {open:?}");

    let parked = gated_call(
        &gate,
        agent_ctx("gate-on", None, false),
        web_origin(),
        "shell",
    )
    .await;
    assert!(parked.is_none(), "the sibling must park, got {parked:?}");
}

#[tokio::test]
async fn the_gate_off_switch_still_refuses_an_unlabelled_origin() {
    let (gate, _dir) = test_gate();
    let outcome = gated_call(
        &gate,
        agent_ctx("gate-off-unknown", None, true),
        AgentTurnOrigin::Unknown,
        "shell",
    )
    .await;
    assert!(
        matches!(outcome, Some(GateOutcome::Deny { .. })),
        "got {outcome:?}"
    );
}

#[tokio::test]
async fn each_agent_answers_from_its_own_auto_approve_list() {
    let (gate, _dir) = test_gate();
    let listed = SecurityPolicy {
        auto_approve: vec!["shell".into()],
        auto_approve_all: false,
        ..SecurityPolicy::default()
    };
    let unlisted = SecurityPolicy {
        auto_approve: Vec::new(),
        auto_approve_all: false,
        ..SecurityPolicy::default()
    };

    let allowed = gated_call(
        &gate,
        agent_ctx("listed", Some(listed), false),
        web_origin(),
        "shell",
    )
    .await;
    assert!(
        matches!(allowed, Some(GateOutcome::Allow)),
        "got {allowed:?}"
    );

    let parked = gated_call(
        &gate,
        agent_ctx("unlisted", Some(unlisted), false),
        web_origin(),
        "shell",
    )
    .await;
    assert!(parked.is_none(), "an agent without the grant must park");
}

#[tokio::test]
async fn an_agent_without_auto_approve_all_parks_under_a_process_policy_that_has_it() {
    let (gate, dir) = test_gate();
    let process = Arc::new(SecurityPolicy {
        auto_approve_all: true,
        ..SecurityPolicy::default()
    });
    let _guard = crate::security::live_policy::install_scoped(
        process,
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    let booted = gated_call(
        &gate,
        CoreContext::for_test(DomainSet::full(), Some(dir.path().to_path_buf())),
        web_origin(),
        "shell",
    )
    .await;
    assert!(matches!(booted, Some(GateOutcome::Allow)), "got {booted:?}");

    let strict = SecurityPolicy {
        auto_approve: Vec::new(),
        auto_approve_all: false,
        ..SecurityPolicy::default()
    };
    let parked = gated_call(
        &gate,
        agent_ctx("strict", Some(strict), false),
        web_origin(),
        "shell",
    )
    .await;
    assert!(
        parked.is_none(),
        "the agent's own policy must win over the process blanket approval"
    );
}

fn park(
    gate: &Arc<ApprovalGate>,
    agent: &str,
) -> tokio::task::JoinHandle<(GateOutcome, Option<String>)> {
    let gate = Arc::clone(gate);
    let ctx = agent_ctx(agent, None, false);
    tokio::spawn(CoreContext::scope(
        ctx,
        turn_origin::with_origin(web_origin(), async move {
            let turn = ApprovalTurnContext::chat(chat_ctx());
            gate.intercept_audited(&turn, "shell", "run it", serde_json::json!({}))
                .await
        }),
    ))
}

async fn parked_for(gate: &ApprovalGate, agent: &str) -> String {
    for _ in 0..200 {
        if let Some(request_id) = gate.pending_for_agent_thread(Some(agent), "t-test") {
            return request_id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{agent} never parked");
}

async fn requested_agent(
    rx: &mut tinybus::events::EventReceiver<crate::core::events::DomainEvent>,
    expected_request_id: &str,
) -> Option<String> {
    loop {
        match rx.recv().await {
            Some(crate::core::events::DomainEvent::ApprovalRequested {
                request_id,
                agent_id,
                ..
            }) if request_id == expected_request_id => return agent_id,
            Some(_) => continue,
            None => panic!("the bus closed before the expected event arrived"),
        }
    }
}

#[tokio::test]
async fn agents_parked_on_one_thread_id_are_routed_and_decided_apart() {
    crate::core::bus::init().await.expect("bus init");
    let mut events = crate::core::bus::BUS
        .get()
        .expect("event bus initialized above")
        .receiver();
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);

    let alpha = park(&gate, "route-alpha");
    let beta = park(&gate, "route-beta");
    let alpha_request = parked_for(&gate, "route-alpha").await;
    let beta_request = parked_for(&gate, "route-beta").await;
    assert_ne!(alpha_request, beta_request);
    assert!(
        gate.pending_for_agent_thread(None, "t-test").is_none(),
        "the process's own route for the thread stays empty"
    );
    assert!(
        gate.pending_for_thread("t-test").is_none(),
        "an unscoped reply must not reach an agent's request"
    );

    let seen = tokio::time::timeout(
        Duration::from_secs(5),
        requested_agent(&mut events, &alpha_request),
    )
    .await
    .expect("ApprovalRequested for alpha");
    assert_eq!(seen.as_deref(), Some("route-alpha"));

    let alpha_rows = gate
        .list_pending_for_agent(
            &crate::core::runtime::DispatchCtx::ambient(),
            Some("route-alpha"),
        )
        .unwrap();
    assert_eq!(alpha_rows.len(), 1);
    assert_eq!(alpha_rows[0].request_id, alpha_request);
    assert_eq!(alpha_rows[0].agent_id.as_deref(), Some("route-alpha"));
    assert!(gate
        .list_pending_for_agent(&crate::core::runtime::DispatchCtx::ambient(), None)
        .unwrap()
        .is_empty());

    let refused = gate
        .decide_for_agent("route-alpha", &beta_request, ApprovalDecision::ApproveOnce)
        .expect_err("alpha must not decide beta's request");
    assert_eq!(
        refused.downcast_ref::<ApprovalError>(),
        Some(&ApprovalError::WrongAgent {
            request_id: beta_request.clone()
        })
    );
    assert_eq!(
        gate.pending_for_agent_thread(Some("route-beta"), "t-test"),
        Some(beta_request.clone()),
        "beta stays parked after the refused decision"
    );

    let decided = gate
        .decide_for_agent("route-alpha", &alpha_request, ApprovalDecision::ApproveOnce)
        .unwrap()
        .expect("alpha's row decided");
    assert_eq!(decided.agent_id.as_deref(), Some("route-alpha"));
    let (outcome, _) = alpha.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Allow), "got {outcome:?}");
    assert!(
        !beta.is_finished(),
        "alpha's decision must not release beta"
    );

    let denied = gate
        .deny_all_for_agent(
            &crate::core::runtime::DispatchCtx::ambient(),
            "route-beta",
            "agent_removed",
        )
        .unwrap();
    assert_eq!(denied, 1);
    let (outcome, _) = beta.await.unwrap();
    assert!(
        matches!(outcome, GateOutcome::Deny { .. }),
        "got {outcome:?}"
    );
    let decided = tokio::time::timeout(
        Duration::from_secs(5),
        find_approval_decided(&mut events, &beta_request),
    )
    .await
    .expect("ApprovalDecided for beta");
    match decided {
        crate::core::events::DomainEvent::ApprovalDecided {
            agent_id,
            resolution,
            decision,
            ..
        } => {
            assert_eq!(agent_id.as_deref(), Some("route-beta"));
            assert_eq!(resolution.as_deref(), Some("agent_removed"));
            assert_eq!(decision, "deny");
        }
        other => panic!("unexpected event {other:?}"),
    }
    assert!(gate
        .list_pending_for_agent(
            &crate::core::runtime::DispatchCtx::ambient(),
            Some("route-beta")
        )
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn an_agent_cannot_decide_a_request_the_process_parked() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let g = Arc::clone(&gate);
    let process = tokio::spawn(turn_origin::with_origin(web_origin(), async move {
        let turn = ApprovalTurnContext::chat(chat_ctx());
        g.intercept_audited(&turn, "shell", "run it", serde_json::json!({}))
            .await
    }));
    let request_id = loop {
        if let Some(request_id) = gate.pending_for_agent_thread(None, "t-test") {
            break request_id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };

    let refused = gate.decide_for_agent("intruder", &request_id, ApprovalDecision::Deny);
    assert!(
        refused.is_err(),
        "an agent must not decide a process request"
    );

    gate.decide(&request_id, ApprovalDecision::Deny).unwrap();
    let (outcome, _) = process.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Deny { .. }));
}
