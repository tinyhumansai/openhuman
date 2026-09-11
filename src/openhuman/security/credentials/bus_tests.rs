use super::*;

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
    scheduler_gate::set_signed_out(false);
    s.handle(&DomainEvent::SystemStartup {
        component: "test".into(),
    })
    .await;
    assert!(
        !scheduler_gate::is_signed_out(),
        "non-auth event must not flip the override"
    );
}
