use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn name_is_stable() {
    let s = SessionExpiredSubscriber::new();
    assert_eq!(s.name(), "credentials::session_expired_handler");
}

#[test]
fn domain_filter_is_auth() {
    let s = SessionExpiredSubscriber::new();
    assert_eq!(s.domains(), Some(&["auth"][..]));
}

#[test]
fn stale_session_expiry_does_not_match_a_replacement_token() {
    let expired_request = crate::credentials::backend_credential::credential_fingerprint(
        "request-token-before-rotation",
    );
    assert!(!super::session_expiry_matches_token(
        &expired_request,
        "replacement-token-after-rotation",
    ));
    assert!(super::session_expiry_matches_token(
        &expired_request,
        "request-token-before-rotation",
    ));
}

#[tokio::test]
async fn stale_session_expiry_leaves_replacement_session_and_gate_intact() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::credentials::backend_credential::credential_fingerprint;
    use crate::credentials::session_state::get_session_token;

    let temp = tempfile::tempdir().expect("temporary workspace");
    let config = crate::config::Config {
        workspace_dir: temp.path().join("workspace"),
        action_dir: temp.path().join("workspace"),
        config_path: temp.path().join("config.toml"),
        ..crate::config::Config::default()
    };
    let auth = crate::credentials::AuthService::from_config(&config);
    auth.store_provider_token(
        crate::credentials::APP_SESSION_PROVIDER,
        crate::credentials::DEFAULT_AUTH_PROFILE_NAME,
        "replacement-session-token",
        Default::default(),
        true,
    )
    .await
    .expect("store replacement session");
    let context = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    let ctx = DispatchCtx::new(context);
    llm_gate::set_signed_out(false);

    SessionExpiredSubscriber::new()
        .handle_with(
            &ctx,
            false,
            &DomainEvent::SessionExpired {
                source: "test.old_request".into(),
                reason: "401".into(),
                credential_fingerprint: Some(credential_fingerprint("expired-session-token")),
            },
        )
        .await;

    assert_eq!(
        get_session_token(&config)
            .await
            .expect("read session")
            .as_deref(),
        Some("replacement-session-token"),
        "an old request must not clear the replacement credential"
    );
    assert!(!llm_gate::is_signed_out());
}

#[tokio::test]
async fn handle_ignores_non_auth_events() {
    // Domain filter is advisory — the broadcast bus still delivers all
    // events to every subscriber. The handler must guard internally.
    let s = SessionExpiredSubscriber::new();
    // Reset state we depend on.
    llm_gate::set_signed_out(false);
    s.handle(&DomainEvent::SystemStartup {
        component: "test".into(),
    })
    .await;
    assert!(
        !llm_gate::is_signed_out(),
        "non-auth event must not flip the override"
    );
}

#[tokio::test]
async fn session_expired_in_saas_returns_before_any_teardown() {
    let s = SessionExpiredSubscriber::new();
    llm_gate::set_signed_out(false);
    // In single-user mode this would load the config and may flip the gate;
    // in SaaS it must return at once, leaving the gate untouched.
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        s.handle_with(
            &DispatchCtx::process_default(),
            true,
            &DomainEvent::SessionExpired {
                source: "test".into(),
                reason: "401".into(),
                credential_fingerprint: None,
            },
        ),
    )
    .await
    .expect("SaaS early return must not await the config load");
    assert!(!llm_gate::is_signed_out());
}
