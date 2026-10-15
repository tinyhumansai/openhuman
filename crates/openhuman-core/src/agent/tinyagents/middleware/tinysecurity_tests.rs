use super::*;

fn is_deny(d: &PolicyDecision) -> bool {
    matches!(d, PolicyDecision::Deny { .. })
}

#[test]
fn allow_maps_to_allow() {
    assert!(matches!(
        verdict_to_policy_decision(&Verdict::Allow),
        PolicyDecision::Allow
    ));
}

#[test]
fn deny_maps_to_deny_without_payload() {
    let d = verdict_to_policy_decision(&Verdict::Deny {
        reason: DenialReason::Floor,
    });
    assert!(is_deny(&d));
}

#[test]
fn require_approval_carries_request_id() {
    let d = verdict_to_policy_decision(&Verdict::RequireApproval {
        request_id: "req-1".into(),
    });
    match d {
        PolicyDecision::RequireApproval { reason } => assert!(reason.contains("req-1")),
        _ => panic!("expected RequireApproval"),
    }
}

#[test]
fn deny_dominates_other_decisions() {
    let combined = combine_decisions(vec![
        PolicyDecision::Allow,
        PolicyDecision::RequireApproval { reason: "r".into() },
        PolicyDecision::Deny { reason: "d".into() },
        PolicyDecision::Allow,
    ]);
    assert!(is_deny(&combined));
}

#[test]
fn require_approval_beats_allow() {
    let combined = combine_decisions(vec![
        PolicyDecision::Allow,
        PolicyDecision::RequireApproval { reason: "r".into() },
    ]);
    assert!(matches!(combined, PolicyDecision::RequireApproval { .. }));
}

#[test]
fn empty_combination_allows() {
    assert!(matches!(combine_decisions(vec![]), PolicyDecision::Allow));
}

fn test_ctx() -> RunContext<OpenHumanRunContext> {
    RunContext::new(
        tinyagents_harness::context::RunConfig::new("tinysecurity-test"),
        OpenHumanRunContext::new(),
    )
}

fn test_call() -> TaToolCall {
    TaToolCall {
        id: "c1".into(),
        name: "shell".into(),
        arguments: serde_json::json!({}),
        invalid: None,
    }
}

#[tokio::test]
async fn module_fault_denies_without_retry() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let policy = TinySecurityPolicy::new(Arc::new(move |_| {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { Err("module crashed".to_string()) })
    }));
    let decision = policy.check(&test_ctx(), &test_call()).await;
    assert!(is_deny(&decision), "a module fault must never allow");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "no retry"
    );
}

#[tokio::test]
async fn policy_maps_require_approval_verdict() {
    let policy = TinySecurityPolicy::new(Arc::new(|_| {
        Box::pin(async {
            Ok(Verdict::RequireApproval {
                request_id: "req-9".into(),
            })
        })
    }));
    match policy.check(&test_ctx(), &test_call()).await {
        PolicyDecision::RequireApproval { reason } => assert!(reason.contains("req-9")),
        other => panic!("expected RequireApproval, got {other:?}"),
    }
}
