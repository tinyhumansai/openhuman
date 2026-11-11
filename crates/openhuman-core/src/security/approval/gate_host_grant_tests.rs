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
