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

fn tenant(name: &str) -> SecretScope {
    let storage = tinystoragedrivers_testkit::memory_backend();
    let scoped = storage.for_scope(&Scope::new("local").unwrap()).unwrap();
    SecretScope {
        entry: std::sync::Arc::new(scopes::ScopeEntry::new(
            tinysecurity_bus::SubjectScope {
                user: name.into(),
                workspace: format!("/workspaces/{name}"),
                agent: "config-secrets".into(),
            },
            format!("{name:0>32}"),
            rows::Rows::over(std::sync::Arc::clone(scoped.documents())),
        )),
    }
}

fn permission_batch(
    stream_id: &str,
    correlations: &[&tinysecurity_bus::AuditCorrelation2],
) -> tinysecurity_bus::AuditSinkRequest2 {
    tinysecurity_bus::AuditSinkRequest2 {
        scope: tinysecurity_bus::SubjectScope {
            user: "0000000a".into(),
            workspace: "11111111".into(),
            agent: "22222222".into(),
        },
        stream_id: stream_id.to_owned(),
        expected_tail: correlations[0].expected_tail,
        events: correlations
            .iter()
            .enumerate()
            .map(|(offset, correlation)| tinysecurity_bus::AuditEvent2 {
                sequence: correlation.expected_tail + 1 + offset as u64,
                event_id: correlation.event_id.clone(),
                generation: 1,
                kind: tinysecurity_bus::AuditKind2::Shell,
                attribution: "permission".into(),
                arguments_json: "[]".into(),
                rule_ids: vec![],
            })
            .collect(),
    }
}

/// TinySecurity appends every tenant's audited permissions on its one init
/// audit stream. Each batch is committed to the store of the tenant whose
/// correlation it carries and never to another tenant's; a batch the host did
/// not correlate, or one mixing tenants, is refused.
#[tokio::test]
async fn two_tenants_permission_audits_never_land_in_each_others_store() {
    let stream_id = "b".repeat(32);
    let subject = tinysecurity_bus::SubjectScope {
        user: "openhuman".into(),
        workspace: "bootstrap".into(),
        agent: "bootstrap".into(),
    };
    let alice = tenant("alice");
    let bob = tenant("bob");
    let alice_audit = permission_audit(&alice, subject.clone(), &stream_id);
    let bob_audit = permission_audit(&bob, subject, &stream_id);
    let tail = |scope: &SecretScope| {
        let rows = scope.entry.rows.clone();
        let stream_id = stream_id.clone();
        async move { rows.tail(&stream_id).await.unwrap() }
    };

    let a = alice_audit.correlation(1).await.unwrap();
    let b = bob_audit.correlation(1).await.unwrap();
    assert!(
        callbacks::append(permission_batch(&stream_id, &[&a, &b]))
            .await
            .is_err(),
        "a batch spanning two tenants is refused"
    );

    callbacks::append(permission_batch(&stream_id, &[&a]))
        .await
        .expect("alice's check is committed");
    assert_eq!(tail(&alice).await, 1);
    assert_eq!(
        tail(&bob).await,
        0,
        "nothing of alice's reaches bob's store"
    );

    callbacks::append(permission_batch(&stream_id, &[&b]))
        .await
        .expect("bob's check is committed");
    assert_eq!(tail(&bob).await, 1);
    assert_eq!(
        tail(&alice).await,
        1,
        "nothing of bob's reaches alice's store"
    );

    alice_audit.settle(&a.event_id);
    let mut stranger = a.clone();
    stranger.event_id = "f".repeat(32);
    stranger.expected_tail = 1;
    assert!(
        callbacks::append(permission_batch(&stream_id, &[&stranger]))
            .await
            .is_err(),
        "an event the host never correlated is refused"
    );
    assert_eq!(tail(&alice).await, 1);
    bob_audit.settle(&b.event_id);
}
