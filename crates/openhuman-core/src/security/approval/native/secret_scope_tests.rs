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

#[test]
fn cold_secret_scope_initializes_storage_on_the_storage_bridge() {
    let root = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        workspace_dir: root.path().join("workspace"),
        config_path: root.path().join("config.toml"),
        ..Default::default()
    };
    let subject = tinysecurity_bus::SubjectScope {
        user: "fixture-user".into(),
        workspace: config.workspace_dir.display().to_string(),
        agent: "config-secrets".into(),
    };
    crate::storage::block_on_anyhow(async move {
        let scope = prepare_secret_scope(&config, subject)
            .await
            .map_err(anyhow::Error::msg)?;
        assert_eq!(
            scope
                .correlation(1)
                .await
                .map_err(anyhow::Error::msg)?
                .expected_tail,
            0
        );
        Ok(())
    })
    .expect("cold scope initialization can run on the storage bridge");
}

/// TinySecurity appends every audited permission on its init audit stream. The
/// sink must commit a batch there, and the next correlation must start from the
/// committed tail; an unserved stream denied every path and command check.
#[tokio::test]
async fn the_sink_commits_audited_permissions_on_the_authority_stream() {
    let storage = tinystoragedrivers_testkit::memory_backend();
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    let stream_id = "a".repeat(32);
    let subject = tinysecurity_bus::SubjectScope {
        user: "openhuman".into(),
        workspace: "authority-test".into(),
        agent: "bootstrap".into(),
    };
    let rows = rows::Rows::over(std::sync::Arc::clone(scoped.documents()));
    let scope = authority_scope_over(rows, subject, &stream_id).await;

    let correlation = scope.correlation(1).await.unwrap();
    let batch = tinysecurity_bus::AuditSinkRequest2 {
        scope: tinysecurity_bus::SubjectScope {
            user: "0000000a".into(),
            workspace: "11111111".into(),
            agent: "22222222".into(),
        },
        stream_id: stream_id.clone(),
        expected_tail: correlation.expected_tail,
        events: vec![tinysecurity_bus::AuditEvent2 {
            sequence: correlation.expected_tail + 1,
            event_id: correlation.event_id,
            generation: 1,
            kind: tinysecurity_bus::AuditKind2::Shell,
            attribution: "permission".into(),
            arguments_json: "[]".into(),
            rule_ids: vec![],
        }],
    };
    let receipt = callbacks::append(batch)
        .await
        .expect("the authority stream is served");
    assert_eq!(receipt.new_tail, 1);
    assert_eq!(scope.correlation(1).await.unwrap().expected_tail, 1);
}
