//! Requests an earlier launch left undecided (pending orphans): listed for the
//! default tenant, dismissed only by a denial.

use super::*;
use crate::core::runtime::Tenant;
use crate::security::approval::engine_stub::State;

#[tokio::test]
async fn an_orphan_is_listed_and_a_person_can_deny_it() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    stub.leave_orphan("left-waiting", "shell", Duration::from_secs(600));
    let listed = gate.list_pending().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].request_id, "left-waiting");

    let dismissed = gate
        .decide_as(
            &Tenant::default(),
            None,
            "left-waiting",
            ApprovalDecision::Deny,
        )
        .unwrap()
        .expect("the orphan is dismissed");
    assert_eq!(dismissed.request_id, "left-waiting");
    assert_eq!(dismissed.tool_name, "shell");
    assert_eq!(stub.state("left-waiting"), Some(State::Denied));
    assert_eq!(stub.label("left-waiting").as_deref(), Some("deny"));
    assert!(gate.list_pending().unwrap().is_empty());

    // Dismissed once: a second denial finds nothing to decide.
    assert!(gate
        .decide_as(
            &Tenant::default(),
            None,
            "left-waiting",
            ApprovalDecision::Deny
        )
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn an_orphan_is_never_approved() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    stub.leave_orphan("left-waiting", "shell", Duration::from_secs(600));
    for decision in [
        ApprovalDecision::ApproveOnce,
        ApprovalDecision::ApproveAlwaysForTool,
        ApprovalDecision::ApproveAlwaysForFlow,
    ] {
        let error = gate
            .decide_as(&Tenant::default(), None, "left-waiting", decision)
            .expect_err("approving an earlier launch's request is refused");
        assert!(error.to_string().contains("can only be denied"), "{error}");
    }
    assert_eq!(stub.state("left-waiting"), Some(State::Pending));
    assert_eq!(gate.list_pending().unwrap().len(), 1);
}

#[tokio::test]
async fn only_the_default_tenant_dismisses_an_orphan() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    stub.leave_orphan("left-waiting", "shell", Duration::from_secs(600));
    let profile = Tenant {
        profile: Some("someone-else".into()),
        agent: None,
    };
    assert!(gate
        .decide_as(&profile, None, "left-waiting", ApprovalDecision::Deny)
        .unwrap()
        .is_none());
    // Naming an agent is a decision for that agent's own requests, which an
    // orphan never is, whatever the agent path answers.
    let _ = gate.decide_as(
        &Tenant::default(),
        Some("agent-1"),
        "left-waiting",
        ApprovalDecision::Deny,
    );
    assert_eq!(stub.state("left-waiting"), Some(State::Pending));
}

#[tokio::test]
async fn an_expired_orphan_or_an_unknown_id_is_not_dismissed() {
    let (gate, _dir) = test_gate();
    let stub = stub_of(&gate);
    stub.leave_orphan("expired", "shell", Duration::ZERO);
    for id in ["expired", "never-existed"] {
        assert!(gate
            .decide_as(&Tenant::default(), None, id, ApprovalDecision::Deny)
            .unwrap()
            .is_none());
    }
    assert_eq!(stub.state("expired"), Some(State::Pending));
}
