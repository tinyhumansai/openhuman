use super::*;
use tinystoragedrivers::{Scope, StorageBackend};

fn secret_scope() -> SecretScope {
    let storage = tinystoragedrivers_testkit::memory_backend();
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    let subject = tinysecurity_bus::SubjectScope {
        user: "user-1".into(),
        workspace: "/workspace".into(),
        agent: "config-secrets".into(),
    };
    SecretScope {
        entry: std::sync::Arc::new(scopes::ScopeEntry::new(
            subject,
            "7".repeat(32),
            rows::Rows::over(std::sync::Arc::clone(scoped.documents())),
        )),
    }
}

#[tokio::test]
async fn secret_scope_exposes_its_real_subject_stream_and_ordered_audit_correlation() {
    let scope = secret_scope();
    assert_eq!(scope.subject().agent, "config-secrets");
    assert_eq!(scope.stream_id(), "7".repeat(32));

    let _guard = scope.lock().await;
    let correlation = scope.correlation(9).await.unwrap();
    assert_eq!(correlation.generation, 9);
    assert_eq!(correlation.expected_tail, 0);
    assert_eq!(correlation.event_id.len(), 32);
}
