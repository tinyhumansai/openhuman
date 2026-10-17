use super::*;

#[test]
fn subscriber_identity_and_domain() {
    let subscriber = CredentialRefreshSubscriber;
    assert_eq!(subscriber.name(), "search::credential_refresh");
    assert_eq!(subscriber.domains(), Some(&["auth"][..]));
}

#[test]
fn credential_changed_is_an_auth_event() {
    let event = DomainEvent::CredentialChanged {
        kind: "session".into(),
        tenant: None,
    };
    assert_eq!(event.domain(), "auth");
}

#[tokio::test]
async fn other_auth_events_are_ignored() {
    // A SessionExpired event must not trigger a refresh on its own — the
    // teardown's clear_credential publishes CredentialChanged instead. The
    // handler returns before touching config or the module.
    CredentialRefreshSubscriber
        .handle(&DomainEvent::SessionExpired {
            source: "test".into(),
            reason: "expired".into(),
        })
        .await;
}

#[tokio::test]
async fn refresh_is_a_no_op_while_the_module_is_not_loaded() {
    CredentialRefreshSubscriber
        .handle(&DomainEvent::CredentialChanged {
            kind: "cleared".into(),
            tenant: None,
        })
        .await;
}
