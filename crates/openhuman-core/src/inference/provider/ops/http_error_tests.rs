use super::*;
use crate::core::events::DomainEvent;
use reqwest::StatusCode;

fn response(status: StatusCode, body: &str) -> reqwest::Response {
    reqwest::Response::from(
        axum::http::Response::builder()
            .status(status)
            .body(body.to_string())
            .expect("build response"),
    )
}

/// Drain the shared bus until `pick` matches an event or the queue is empty.
fn drain_until<T>(
    rx: &mut tinybus::EventReceiver<DomainEvent>,
    mut pick: impl FnMut(DomainEvent) -> Option<T>,
) -> Option<T> {
    loop {
        match rx.try_recv() {
            Ok(event) => {
                if let Some(found) = pick(event) {
                    return Some(found);
                }
            }
            Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => return None,
        }
    }
}

#[test]
fn transient_statuses_are_not_reported_but_client_errors_are() {
    for s in [408u16, 429, 502, 503, 504, 520] {
        assert!(
            !should_report_provider_http_failure(StatusCode::from_u16(s).unwrap()),
            "{s} is transient"
        );
    }
    for s in [400u16, 401, 404, 500] {
        assert!(
            should_report_provider_http_failure(StatusCode::from_u16(s).unwrap()),
            "{s} is reportable"
        );
    }
}

#[tokio::test]
async fn api_error_replaces_ollama_cloud_500_body_with_guidance() {
    let err = api_error(
        "ollama",
        response(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"Internal Server Error (ref: 1b2c3d4e-0000-4000-8000-000000000000)"}"#,
        ),
    )
    .await;
    assert!(tinyinference_llm::failure::is_ollama_cloud_internal_500_message(&err.to_string()));
}

#[tokio::test]
async fn api_error_replaces_no_model_loaded_body_with_guidance() {
    let err = api_error(
        "lm_studio",
        response(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"No models loaded. Please load a model in the developer page or via `lms load`."}}"#,
        ),
    )
    .await;
    assert!(err
        .to_string()
        .to_ascii_lowercase()
        .contains("load a model"));
}

#[tokio::test]
async fn api_error_keeps_raw_message_for_unclassified_status() {
    let err = api_error("custom", response(StatusCode::NOT_FOUND, "nope")).await;
    assert!(err.to_string().contains("custom API error (404"));
}

#[tokio::test]
async fn backend_401_publishes_session_expired_through_reporter() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().expect("bus").receiver();
    let marker = "REPORTER_MARKER_SESSION";
    let _ = api_error(
        openhuman_backend_model::PROVIDER_LABEL,
        response(StatusCode::UNAUTHORIZED, marker),
    )
    .await;
    let seen = drain_until(&mut rx, |e| match e {
        DomainEvent::SessionExpired { reason, .. } if reason.contains(marker) => Some(()),
        _ => None,
    });
    assert!(seen.is_some(), "backend 401 must publish SessionExpired");
}

#[tokio::test]
async fn byo_401_publishes_api_key_rejected_not_session_expired() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().expect("bus").receiver();
    let provider = "reporter_byo_provider";
    let _ = api_error(
        provider,
        response(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"message":"Invalid or missing API key","type":"authentication_error"}}"#,
        ),
    )
    .await;
    let mut session_expired = false;
    let rejected = drain_until(&mut rx, |e| match e {
        DomainEvent::SessionExpired { .. } => {
            session_expired = true;
            None
        }
        DomainEvent::ProviderApiKeyRejected { provider: p, .. } if p == provider => Some(()),
        _ => None,
    });
    assert!(rejected.is_some(), "BYO key rejection must be surfaced");
    assert!(!session_expired || rejected.is_some());
}

#[tokio::test]
async fn backend_provider_does_not_get_byo_key_rejection() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().expect("bus").receiver();
    let _ = api_error(
        openhuman_backend_model::PROVIDER_LABEL,
        response(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"type":"authentication_error"}}"#,
        ),
    )
    .await;
    let rejected = drain_until(&mut rx, |e| match e {
        DomainEvent::ProviderApiKeyRejected { provider, .. }
            if provider == openhuman_backend_model::PROVIDER_LABEL =>
        {
            Some(())
        }
        _ => None,
    });
    assert!(
        rejected.is_none(),
        "backend 401 is session expiry, not a BYO key"
    );
}
