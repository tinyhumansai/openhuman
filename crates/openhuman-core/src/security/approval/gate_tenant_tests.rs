//! Approval routes key on the tenant: two SaaS profiles' default agents (no
//! agent id) parked on one thread id never share a route, and the desktop's
//! keys stay byte-identical.

use super::*;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet, Tenant};

fn tenant(profile: Option<&str>, agent: Option<&str>) -> Tenant {
    Tenant {
        profile: profile.map(str::to_owned),
        agent: agent.map(str::to_owned),
    }
}

/// A profile's context as the profile host derives it: a profile, no agent.
fn profile_ctx(profile: &str) -> Arc<CoreContext> {
    let parent = CoreContext::for_test(DomainSet::full(), Some(std::env::temp_dir()));
    parent.derive_with(
        ContextOverlay::new(
            Config::default(),
            DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .profile(profile),
    )
}

#[test]
fn desktop_route_keys_are_unchanged() {
    assert_eq!(thread_route_key(&tenant(None, None), "t-1"), "t-1");
    assert_eq!(
        thread_route_key(&tenant(None, Some("alpha")), "t-1"),
        "alpha\u{1f}t-1"
    );
}

#[test]
fn profile_route_keys_are_distinct_per_profile_and_from_the_desktop() {
    let a = thread_route_key(&tenant(Some("alice"), None), "t-1");
    let b = thread_route_key(&tenant(Some("bob"), None), "t-1");
    assert_ne!(a, b);
    assert_ne!(a, "t-1");
    assert_ne!(
        a,
        thread_route_key(&tenant(Some("alice"), Some("alice")), "t-1")
    );
    assert!(a.starts_with('\u{1e}'), "a profile key is marked: {a:?}");
}

fn park_as(
    gate: &Arc<ApprovalGate>,
    ctx: Arc<CoreContext>,
) -> tokio::task::JoinHandle<(GateOutcome, Option<String>)> {
    let gate = Arc::clone(gate);
    tokio::spawn(CoreContext::scope(
        ctx,
        turn_origin::with_origin(web_origin(), async move {
            let turn = ApprovalTurnContext::chat(chat_ctx());
            gate.intercept_audited(&turn, "shell", "run it", serde_json::json!({}))
                .await
        }),
    ))
}

/// The request `tenant` parked on `t-test`. Returns as soon as it is parked;
/// the ceiling only turns a turn that never parks into a failure instead of a
/// hang, and is far above any scheduling delay a loaded CI runner adds.
async fn parked_for(gate: &ApprovalGate, tenant: &Tenant) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        if let Some(request_id) = gate.pending_for_tenant_thread(tenant, "t-test") {
            return request_id;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{tenant:?} never parked");
}

#[tokio::test]
async fn two_profiles_default_agents_park_on_one_thread_id_apart() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    // Names no other suite uses: the process-wide lease-fence registry is keyed
    // by profile, and a fence another test registered for "alice" would refuse
    // this test's writes to that profile's scope.
    let (alice, bob) = (profile_ctx("gate-alice"), profile_ctx("gate-bob"));

    let alice_turn = park_as(&gate, Arc::clone(&alice));
    let bob_turn = park_as(&gate, Arc::clone(&bob));
    let alice_request = parked_for(&gate, &tenant(Some("gate-alice"), None)).await;
    let bob_request = parked_for(&gate, &tenant(Some("gate-bob"), None)).await;
    assert_ne!(alice_request, bob_request);

    // Each profile's own lookup finds only its own request.
    let seen_by = |ctx: Arc<CoreContext>| {
        let gate = Arc::clone(&gate);
        CoreContext::scope(ctx, async move { gate.pending_for_thread("t-test") })
    };
    assert_eq!(
        seen_by(Arc::clone(&alice)).await,
        Some(alice_request.clone())
    );
    assert_eq!(seen_by(Arc::clone(&bob)).await, Some(bob_request.clone()));
    // The process's own (unprofiled) route for the thread stays empty.
    assert!(gate.pending_for_agent_thread(None, "t-test").is_none());

    gate.decide(&alice_request, ApprovalDecision::Deny).unwrap();
    gate.decide(&bob_request, ApprovalDecision::Deny).unwrap();
    let (alice_outcome, _) = alice_turn.await.unwrap();
    let (bob_outcome, _) = bob_turn.await.unwrap();
    assert!(
        matches!(alice_outcome, GateOutcome::Deny { .. }),
        "{alice_outcome:?}"
    );
    assert!(
        matches!(bob_outcome, GateOutcome::Deny { .. }),
        "{bob_outcome:?}"
    );
    assert!(
        seen_by(alice).await.is_none(),
        "a decision clears the route"
    );
    assert!(seen_by(bob).await.is_none());
}
