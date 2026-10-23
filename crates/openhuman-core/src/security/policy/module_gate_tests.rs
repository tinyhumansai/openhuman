use super::*;
use crate::modules::security::gap::stub::{Asked, StubPort};
use crate::modules::security::gap::{install_port, BudgetOp};
use std::sync::Arc;
use tinysecurity_bus::{BudgetDecision2, PathCheckKind};

fn policy() -> SecurityPolicy {
    SecurityPolicy {
        workspace_dir: "/w".into(),
        action_dir: "/a".into(),
        ..SecurityPolicy::default()
    }
}

#[tokio::test]
async fn a_batch_is_one_module_call_with_the_policys_scope() {
    let stub = Arc::new(StubPort::allowing().paths(|scope, _, path| {
        scope.action_dir == std::path::Path::new("/a") && !path.ends_with("secret")
    }));
    let _port = install_port(stub.clone());
    let paths: Vec<PathBuf> = vec!["/a/x".into(), "/a/secret".into(), "/a/y".into()];
    let answers = policy()
        .check_paths(PathKind::StringAllowed, &paths)
        .await
        .unwrap();
    assert_eq!(answers, vec![true, false, true]);
    assert_eq!(
        stub.asked(),
        vec![Asked::Paths(PathCheckKind::StringAllowed, paths)],
        "one question for the whole batch"
    );
}

#[tokio::test]
async fn each_path_kind_maps_to_the_module_kind() {
    for (kind, bus) in [
        (PathKind::StringAllowed, PathCheckKind::StringAllowed),
        (PathKind::ResolvedRead, PathCheckKind::ResolvedRead),
        (PathKind::ResolvedWrite, PathCheckKind::ResolvedWrite),
        (PathKind::Internal, PathCheckKind::Internal),
    ] {
        let stub = Arc::new(StubPort::allowing());
        let _port = install_port(stub.clone());
        assert!(
            policy()
                .path_allowed(kind, std::path::Path::new("/a/x"))
                .await
        );
        assert_eq!(
            stub.asked(),
            vec![Asked::Paths(bus, vec!["/a/x".into()])],
            "{kind:?}"
        );
    }
}

#[tokio::test]
async fn an_unavailable_module_refuses_every_path_and_marks_everything_internal() {
    let _port = install_port(Arc::new(StubPort::unavailable()));
    let policy = policy();
    assert!(
        !policy
            .path_allowed(PathKind::StringAllowed, std::path::Path::new("/a/x"))
            .await
    );
    assert!(policy.is_internal(std::path::Path::new("/a/x")).await);
    assert!(policy.is_internal_blocking(std::path::Path::new("/a/x")));
    assert!(!policy.path_allowed_blocking(PathKind::StringAllowed, std::path::Path::new("/a/x")));
    let paths: Vec<PathBuf> = vec!["/a/x".into(), "/a/y".into()];
    assert_eq!(
        policy.check_paths_blocking(PathKind::ResolvedRead, &paths),
        vec![false, false]
    );
}

#[test]
fn the_blocking_batch_asks_the_module_once() {
    let stub = Arc::new(StubPort::allowing());
    let _port = install_port(stub.clone());
    let paths: Vec<PathBuf> = vec!["/a/x".into(), "/a/y".into(), "/a/z".into()];
    assert_eq!(
        policy().check_paths_blocking(PathKind::StringAllowed, &paths),
        vec![true; 3]
    );
    assert_eq!(stub.asked().len(), 1);
}

// -- budget -----------------------------------------------------------------

#[tokio::test]
async fn spending_an_action_asks_the_module_and_refuses_with_the_agent_facing_text() {
    let stub = Arc::new(
        StubPort::allowing().budget(|_, _| BudgetDecision2::RateLimited { max_per_hour: 7 }),
    );
    let _port = install_port(stub.clone());
    let error = policy().spend_action("demo").await.unwrap_err();
    assert!(error.contains("Rate limit exceeded"), "{error}");
    assert!(error.contains("7 actions/hour"), "{error}");
    assert_eq!(stub.asked(), vec![Asked::Budget(BudgetOp::Spend)]);

    let _port = install_port(Arc::new(
        StubPort::allowing().budget(|_, _| BudgetDecision2::ReadOnlyMode),
    ));
    let error = policy().spend_action("demo").await.unwrap_err();
    assert!(
        error.contains(tinytools::status::POLICY_BLOCKED_MARKER),
        "{error}"
    );
    assert!(error.contains("read-only mode"), "{error}");
    assert!(error.contains("'demo'"), "{error}");

    let _port = install_port(Arc::new(StubPort::allowing()));
    policy().spend_action("demo").await.unwrap();
}

#[tokio::test]
async fn a_module_that_cannot_answer_refuses_the_action() {
    let _port = install_port(Arc::new(StubPort::unavailable()));
    let error = policy().spend_action("demo").await.unwrap_err();
    assert!(
        error.contains(tinytools::status::POLICY_BLOCKED_MARKER),
        "{error}"
    );
}

#[test]
fn the_tier_reads_the_budget_without_spending_it() {
    let stub = Arc::new(StubPort::allowing().budget(|_, _| BudgetDecision2::ReadOnlyMode));
    {
        let _port = install_port(stub.clone());
        assert!(!policy().tier_allows_acting_blocking());
        assert_eq!(stub.asked(), vec![Asked::Budget(BudgetOp::Peek)]);
    }
    // A spent hourly budget is not a tier refusal.
    let _port = install_port(Arc::new(
        StubPort::allowing().budget(|_, _| BudgetDecision2::RateLimited { max_per_hour: 1 }),
    ));
    assert!(policy().tier_allows_acting_blocking());
    let _port = install_port(Arc::new(StubPort::unavailable()));
    assert!(!policy().tier_allows_acting_blocking());
}

#[test]
fn the_budget_question_names_the_shared_harness_scope() {
    // Shell runs and acting tools share one ledger only if they register the same
    // settings: the harness flow at the execute stage.
    let policy = SecurityPolicy {
        max_actions_per_hour: 11,
        ..policy()
    };
    let scope = policy.budget_policy().unwrap();
    assert_eq!(scope.max_actions_per_hour, 11);
    assert_eq!(
        scope.execution_mode,
        tinysecurity_bus::CommandExecutionMode::HarnessGated
    );
}
