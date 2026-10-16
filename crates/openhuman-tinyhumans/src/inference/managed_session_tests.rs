use super::*;

// #6932: the offline local profile is a valid sign-in with no TinyHumans
// account behind it. `classify_session_token` reports its `exp`-less token
// `Live`, so managed inference used to send it, collect a backend
// `401 "Invalid token"` and surface that as an expired session.
#[test]
fn the_offline_local_session_cannot_authenticate_managed_inference() {
    use openhuman_embed::__host::security::credentials::session_support::{
        SessionTokenCheck, LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE,
    };

    let error = managed_bearer(SessionTokenCheck::Live("header.payload.local".to_string()))
        .expect_err("a local session has no managed bearer");

    assert_eq!(
        error.to_string(),
        LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE
    );
}

#[test]
fn the_refusal_does_not_read_as_an_expired_session() {
    use openhuman_embed::__host::security::credentials::session_support::SessionTokenCheck;

    let error = managed_bearer(SessionTokenCheck::Live("header.payload.local".to_string()))
        .expect_err("a local session has no managed bearer");

    // The whole point of the fix: this must not reach the sign-out path that
    // the backend's 401 envelope used to trigger.
    assert!(
        !openhuman_embed::__host::core::observability::is_session_expired_message(
            &error.to_string()
        )
    );
}

#[test]
fn a_signed_in_session_still_authenticates_managed_inference() {
    use openhuman_embed::__host::security::credentials::session_support::SessionTokenCheck;

    let bearer = managed_bearer(SessionTokenCheck::Live(
        "header.payload.signature".to_string(),
    ))
    .expect("a hosted session is the bearer");

    assert_eq!(bearer, "header.payload.signature");
}

use tinyinference_llm::model::{ModelStreamItem, ProviderError};

/// #6724 (review): a 401 reported *inside* a stream must start re-auth just
/// like a failed `stream()` call does.
#[tokio::test]
async fn an_in_band_401_publishes_session_expired() {
    use openhuman_embed::__host::core::events::DomainEvent;

    openhuman_embed::__host::core::bus::init()
        .await
        .expect("bus init");
    let mut rx = openhuman_embed::__host::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();

    super::super::managed_calls::observe_in_band_failure(&ModelStreamItem::ProviderFailed(
        ProviderError {
            provider: super::super::managed_model::PROVIDER_LABEL.to_string(),
            status: Some(401),
            message: "TEST_MARKER_IN_BAND token expired".to_string(),
            ..ProviderError::default()
        },
    ));

    let mut source_seen = None;
    loop {
        match rx.try_recv() {
            Ok(DomainEvent::SessionExpired { source, reason })
                if reason.contains("TEST_MARKER_IN_BAND") =>
            {
                source_seen = Some(source);
                break;
            }
            Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
    assert_eq!(
        source_seen.as_deref(),
        Some("openhuman_backend_model.stream(401)"),
        "an in-band 401 must publish SessionExpired"
    );
}
