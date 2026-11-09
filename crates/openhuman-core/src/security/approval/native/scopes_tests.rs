use super::*;
use tinystoragedrivers::{Scope, StorageBackend};

fn subject(agent: &str) -> SubjectScope {
    SubjectScope {
        user: "local".into(),
        workspace: "/work".into(),
        agent: agent.into(),
    }
}

fn entry(agent: &str) -> Arc<ScopeEntry> {
    let storage = tinystoragedrivers_testkit::memory_backend();
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    Arc::new(ScopeEntry::new(
        subject(agent),
        "5".repeat(32),
        Rows::over(Arc::clone(scoped.documents())),
    ))
}

#[test]
fn scope_registration_never_replays_legacy_tool_grants() {
    let granted = grants();
    assert!(granted.tools.is_empty());
    // Per-tool trust is imported through immutable durable receipts, never the
    // mutable scope posture. Flow trust is also independently durable.
    assert!(granted.flows.is_empty());
    assert!(granted.is_valid());
}

#[test]
fn preconfigured_scope_registration_replaces_the_initialized_scope() {
    assert_eq!(
        registration_method(true, false),
        names::methods::REGISTER_APPROVAL_SCOPE2
    );
    assert_eq!(
        registration_method(true, true),
        names::methods::REPLACE_APPROVAL_SCOPE2
    );
    assert_eq!(
        registration_method(false, false),
        names::methods::REPLACE_APPROVAL_SCOPE2
    );
}

#[test]
fn a_subject_key_distinguishes_user_workspace_and_agent() {
    let a = subject_key(&subject("agent"));
    assert_ne!(a, subject_key(&subject("agent#forced")));
    assert_ne!(
        a,
        subject_key(&SubjectScope {
            workspace: "/other".into(),
            ..subject("agent")
        })
    );
}

#[test]
fn the_scope_id_is_the_subjects_and_looks_like_the_module_expects() {
    let one = entry("agent");
    let again = entry("agent");
    let forced = entry("agent#forced");
    assert_eq!(one.scope_id.len(), 32);
    assert!(one.scope_id.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(one.scope_id, again.scope_id);
    assert_ne!(one.scope_id, forced.scope_id);
}

#[tokio::test]
async fn a_bound_handle_resolves_the_subject_for_callbacks() {
    let bound = entry("handle-test-agent");
    let handle = SubjectScope {
        user: "u".repeat(64),
        workspace: "w".repeat(64),
        agent: "handle-test-agent".repeat(4),
    };
    assert!(by_handle(&handle).await.is_none(), "unknown until bound");
    bound.bind(handle.clone()).await;
    assert_eq!(bound.handle(), Some(&handle));
    let found = by_handle(&handle).await.expect("bound");
    assert!(Arc::ptr_eq(&found, &bound));
    // Binding again keeps the first handle.
    bound.bind(subject("other")).await;
    assert_eq!(bound.handle(), Some(&handle));
}

#[test]
fn a_sweep_is_due_once_per_interval() {
    let scope = entry("agent");
    assert!(scope.sweep_due(std::time::Duration::from_secs(30)));
    assert!(!scope.sweep_due(std::time::Duration::from_secs(30)));
    assert!(scope.sweep_due(std::time::Duration::ZERO));
}
