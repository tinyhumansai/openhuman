use super::*;

fn config_in(dir: &std::path::Path) -> Config {
    Config {
        config_path: dir.join("config.toml"),
        workspace_dir: dir.join("workspace"),
        ..Config::default()
    }
}

fn grant_args(registration_id: &str) -> serde_json::Value {
    serde_json::json!({ "registration_id": registration_id })
}

/// Without the host's scope nothing changes: the grant is the gate's to decide.
#[tokio::test]
async fn outside_the_host_scope_a_secret_grant_is_not_host_authorised() {
    let args = grant_args("never-registered");
    assert_eq!(
        embedder_host_grant(true, NATIVE_SECRET_GRANT_TOOL, &args),
        SaasHostGrant::NotApplicable
    );
    assert!(
        host_authorised_grant_without_gate(NATIVE_SECRET_GRANT_TOOL, "unlock", args)
            .await
            .is_none(),
        "no gate and no host authorisation stays unanswered (refused upstream)"
    );
}

/// Inside the scope only a forced secret master grant for the scoped config
/// directory's own live secret scope is authorised; anything else is refused
/// or left to the gate, never approved.
#[tokio::test]
async fn the_host_scope_authorises_only_its_own_live_secret_grant() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    host_authorised_secret_grant(&config, async {
        let args = grant_args("never-registered");
        assert_eq!(
            embedder_host_grant(false, NATIVE_SECRET_GRANT_TOOL, &args),
            SaasHostGrant::NotApplicable,
            "an unforced call is not a master grant"
        );
        assert_eq!(
            embedder_host_grant(true, "shell", &args),
            SaasHostGrant::NotApplicable,
            "only the secret master grant"
        );
        assert_eq!(
            embedder_host_grant(true, NATIVE_SECRET_GRANT_TOOL, &args),
            SaasHostGrant::Refused("the secret scope to unlock is not live"),
            "a registration this process does not hold is never approved"
        );
        match host_authorised_grant_without_gate(NATIVE_SECRET_GRANT_TOOL, "unlock", args).await {
            Some(GateOutcome::Deny { reason }) => {
                assert!(reason.starts_with(POLICY_DENIED_MARKER), "{reason}");
            }
            other => panic!("an unknown registration must be denied, got {other:?}"),
        }
    })
    .await;
}

/// The claim check the host scope reuses: another directory's key is refused.
#[test]
fn the_host_scope_refuses_another_directorys_secret_scope() {
    let own = std::path::Path::new("/srv/embed/a/.secret_key");
    let other = std::path::Path::new("/srv/embed/b/.secret_key");
    let claim = SecretGrantClaim {
        tenant_profile: Some("embedder"),
        tenant_key: Some(own),
        requested: Some((other, "scope-b")),
    };
    assert_eq!(
        saas_host_grant_with(true, true, NATIVE_SECRET_GRANT_TOOL, &claim),
        SaasHostGrant::Refused("the secret scope is not this tenant's own")
    );
}

/// Whether the host authorisation is in force here: inside it, a grant for a
/// registration this process does not hold is refused rather than left to the
/// gate.
fn host_authorises_here() -> bool {
    embedder_host_grant(
        true,
        NATIVE_SECRET_GRANT_TOOL,
        &grant_args("never-registered"),
    ) != SaasHostGrant::NotApplicable
}

fn booted_context(config: &Config) -> std::sync::Arc<crate::core::runtime::CoreContext> {
    crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config.clone(),
    )
}

/// The authorisation belongs to the host's own task: a task spawned from
/// inside its scope, even one that carries the dispatch context, holds none.
#[tokio::test]
async fn a_task_spawned_inside_the_host_scope_is_not_host_authorised() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    let context = booted_context(&config);
    crate::core::runtime::CoreContext::scope(context, async {
        host_authorised_secret_grant(&config, async {
            assert!(host_authorises_here(), "the host's own task is authorised");
            let spawned = tokio::spawn(crate::core::runtime::CoreContext::propagate(async {
                host_authorises_here()
            }))
            .await
            .unwrap();
            assert!(!spawned, "a spawned task must not inherit the authorisation");
        })
        .await;
    })
    .await;
}

/// A context captured inside the host scope authorises nothing once the
/// scope has ended.
#[tokio::test]
async fn a_context_captured_inside_the_host_scope_authorises_nothing_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    let context = booted_context(&config);
    crate::core::runtime::CoreContext::scope(context, async {
        let captured = host_authorised_secret_grant(&config, async {
            crate::core::runtime::CoreContext::scoped()
        })
        .await;
        assert!(!host_authorises_here());
        if let Some(captured) = captured {
            let after = crate::core::runtime::CoreContext::scope(captured, async {
                host_authorises_here()
            })
            .await;
            assert!(!after, "a captured context must not outlive the host scope");
        }
    })
    .await;
}

/// Work inside the host scope that runs under another context (a derived or
/// tenant context scoped for one step) keeps the authorisation, as the host's
/// own task still does that work.
#[tokio::test]
async fn the_host_scope_survives_a_nested_context_scope() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    let nested = booted_context(&config);
    host_authorised_secret_grant(&config, async {
        let inside = crate::core::runtime::CoreContext::scope(nested, async {
            host_authorises_here()
        })
        .await;
        assert!(inside, "a nested context scope keeps the host's authorisation");
    })
    .await;
    assert!(!host_authorises_here(), "nothing outlives the scope");
}
