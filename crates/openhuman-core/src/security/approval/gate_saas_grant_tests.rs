use super::*;
use std::path::{Path, PathBuf};

fn key(profile: &str) -> PathBuf {
    Path::new("/srv/saas/users").join(profile).join(".secret_key")
}

fn claim<'a>(
    profile: Option<&'a str>,
    tenant_key: Option<&'a Path>,
    requested_key: Option<&'a Path>,
) -> SecretGrantClaim<'a> {
    SecretGrantClaim {
        tenant_profile: profile,
        tenant_key,
        requested: requested_key.map(|path| (path, "scope-under-test")),
    }
}

#[test]
fn a_tenant_unlocking_its_own_secret_scope_is_host_authorised() {
    let alice = key("alice");
    let verdict = saas_host_grant_with(
        true,
        true,
        NATIVE_SECRET_GRANT_TOOL,
        &claim(Some("alice"), Some(&alice), Some(&alice)),
    );
    assert_eq!(
        verdict,
        SaasHostGrant::Authorised {
            profile: "alice".into(),
            scope_id: "scope-under-test".into(),
        }
    );
}

#[test]
fn a_tenant_unlocking_another_tenants_secret_scope_is_refused() {
    let (alice, bob) = (key("alice"), key("bob"));
    let verdict = saas_host_grant_with(
        true,
        true,
        NATIVE_SECRET_GRANT_TOOL,
        &claim(Some("alice"), Some(&alice), Some(&bob)),
    );
    assert!(matches!(verdict, SaasHostGrant::Refused(_)), "{verdict:?}");
}

#[test]
fn a_tenant_unlocking_the_operator_scope_is_refused() {
    let alice = key("alice");
    let operator = Path::new("/srv/saas/operator/.secret_key");
    let verdict = saas_host_grant_with(
        true,
        true,
        NATIVE_SECRET_GRANT_TOOL,
        &claim(Some("alice"), Some(&alice), Some(operator)),
    );
    assert!(matches!(verdict, SaasHostGrant::Refused(_)), "{verdict:?}");
}

#[test]
fn a_secret_grant_without_a_tenant_scope_is_refused() {
    let alice = key("alice");
    for unscoped in [
        claim(None, None, Some(&alice)),
        claim(None, Some(&alice), Some(&alice)),
        // The tenant is named but the host cannot place its secret scope.
        claim(Some("alice"), None, Some(&alice)),
        // The registration the grant would unlock is not live.
        claim(Some("alice"), Some(&alice), None),
    ] {
        let verdict = saas_host_grant_with(true, true, NATIVE_SECRET_GRANT_TOOL, &unscoped);
        assert!(matches!(verdict, SaasHostGrant::Refused(_)), "{verdict:?}");
    }
}

#[test]
fn the_dispatch_without_a_profile_resolves_to_a_refusal_in_saas() {
    // The test process is not SaaS, so an unbound dispatch reads the default
    // tenant, which names no profile.
    let verdict = saas_host_grant(
        &crate::core::runtime::DispatchCtx::unbound(),
        true,
        true,
        NATIVE_SECRET_GRANT_TOOL,
        &serde_json::json!({"registration_id": "unknown-registration"}),
    );
    assert!(matches!(verdict, SaasHostGrant::Refused(_)), "{verdict:?}");
}

#[test]
fn outside_saas_the_host_never_authorises_a_secret_grant() {
    let alice = key("alice");
    let verdict = saas_host_grant_with(
        false,
        true,
        NATIVE_SECRET_GRANT_TOOL,
        &claim(Some("alice"), Some(&alice), Some(&alice)),
    );
    assert_eq!(verdict, SaasHostGrant::NotApplicable);
}

#[test]
fn other_forced_tools_in_saas_are_not_host_authorised() {
    let alice = key("alice");
    for tool in ["browser", "shell", "native_secret_grant_extra"] {
        let verdict = saas_host_grant_with(
            true,
            true,
            tool,
            &claim(Some("alice"), Some(&alice), Some(&alice)),
        );
        assert_eq!(verdict, SaasHostGrant::NotApplicable, "{tool}");
        assert!(
            matches!(
                saas_outcome_with(
                    &crate::core::runtime::DispatchCtx::unbound(),
                    true,
                    &AgentTurnOrigin::Unknown,
                    true,
                    tool,
                ),
                Some(GateOutcome::Deny { .. })
            ),
            "{tool}"
        );
    }
    // An unforced secret grant is no master grant request either.
    let verdict = saas_host_grant_with(
        true,
        false,
        NATIVE_SECRET_GRANT_TOOL,
        &claim(Some("alice"), Some(&alice), Some(&alice)),
    );
    assert_eq!(verdict, SaasHostGrant::NotApplicable);
}

#[tokio::test]
async fn the_host_approves_a_matching_tenants_grant_and_audits_it_as_the_host() {
    let (gate, _dir) = test_gate();
    let (outcome, tracked) = gate
        .saas_host_authorised_grant(
            NATIVE_SECRET_GRANT_TOOL,
            "Unlock configuration secrets",
            serde_json::json!({"registration_id": "alice-registration"}),
            "alice",
            "scope-under-test",
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    assert_eq!(tracked, None, "the module consumes the grant itself");
    let stub = stub_of(&gate);
    let requests = stub.request_ids();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        stub.state(&requests[0]),
        Some(crate::security::approval::engine_stub::State::Approved)
    );
    assert_eq!(
        stub.label(&requests[0]).as_deref(),
        Some(SAAS_HOST_AUTHORISED)
    );
    let seen = stub.seen();
    assert_eq!(seen[0].variant, Variant::Forced);
    assert_eq!(seen[0].tool, NATIVE_SECRET_GRANT_TOOL);
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "no human prompt is left behind"
    );
}

#[tokio::test]
async fn the_host_grant_fails_closed_when_the_engine_cannot_ask() {
    let (gate, _dir) = test_gate();
    stub_of(&gate).refuse_checks(true);
    let (outcome, _) = gate
        .saas_host_authorised_grant(
            NATIVE_SECRET_GRANT_TOOL,
            "Unlock configuration secrets",
            serde_json::json!({"registration_id": "alice-registration"}),
            "alice",
            "scope-under-test",
        )
        .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
}

#[tokio::test]
async fn outside_saas_a_secret_grant_still_waits_for_a_person() {
    let (gate, _dir) = test_gate();
    let gate = Arc::new(gate);
    let worker = Arc::clone(&gate);
    let asked = tokio::spawn(async move {
        turn_origin::with_origin(
            AgentTurnOrigin::Cli,
            worker.intercept_forced(
                NATIVE_SECRET_GRANT_TOOL,
                "Unlock configuration secrets",
                serde_json::json!({"registration_id": "desktop-registration"}),
            ),
        )
        .await
    });
    let pending = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(row) = gate.list_pending().unwrap().into_iter().next() {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the secret grant never parked for a person");
    assert_eq!(pending.tool_name, NATIVE_SECRET_GRANT_TOOL);
    assert!(!asked.is_finished(), "nobody approved it yet");
    let stub = stub_of(&gate);
    assert_ne!(
        stub.label(&pending.request_id).as_deref(),
        Some(SAAS_HOST_AUTHORISED)
    );
    decide_parked(&gate, &pending.request_id, ApprovalDecision::Deny);
    assert!(matches!(asked.await.unwrap(), GateOutcome::Deny { .. }));
}
