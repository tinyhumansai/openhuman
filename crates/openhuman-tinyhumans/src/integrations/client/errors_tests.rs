use super::*;

#[tokio::test]
async fn session_401_event_carries_the_client_request_credential_fingerprint() {
    use openhuman_embed::__host::core::events::DomainEvent;

    openhuman_embed::__host::core::bus::init()
        .await
        .expect("initialize event bus");
    let mut events = openhuman_embed::__host::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();
    let fingerprint =
        openhuman_embed::__host::credentials::backend_credential::credential_fingerprint(
            "request-session-token",
        );

    handle_session_jwt_unauthorized(
        "GET",
        "/agent-integrations",
        "https://api.example.test/agent-integrations",
        "Invalid token",
        fingerprint.clone(),
    );

    loop {
        match events.try_recv() {
            Ok(DomainEvent::SessionExpired {
                credential_fingerprint: Some(observed),
                ..
            }) => {
                assert_eq!(observed, fingerprint);
                break;
            }
            Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => panic!("session 401 must publish its request fingerprint"),
        }
    }
}
