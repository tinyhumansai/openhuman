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
            },
        ),
    )
    .await
    .expect("SaaS early return must not await the config load");
    assert!(!llm_gate::is_signed_out());
}
