//! Wire-shape parity for the SDK transport: what the core's classification
//! code relies on, pinned against a mock backend.

use super::*;
use serde_json::json;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `install` writes the core's process-global transport slot; the tests that
/// touch it must not interleave. Async-aware so the guard can live across
/// the awaits in those tests.
fn global_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn request<'a>(
    base: &'a str,
    method: reqwest::Method,
    path: &'a str,
    credential: Option<&'a BackendCredential>,
    unwrap: bool,
) -> BackendRequest<'a> {
    BackendRequest {
        profile: TransportProfile::Api,
        base_url: base,
        method,
        path,
        query: &[],
        body: None,
        credential,
        unwrap_envelope: unwrap,
    }
}

#[tokio::test]
async fn session_credential_rides_bearer_with_sdk_and_product_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/teams/me/usage"))
        .and(header("authorization", "Bearer jwt.a.b"))
        .and(header("x-sdk-client", "tinyhumans-rust"))
        .and(header("x-sdk-name", "openhuman"))
        .and(header_exists("x-core-version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {"remainingUsd": 1.5}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let transport = SdkBackendTransport::new().unwrap();
    let credential = BackendCredential::Session("jwt.a.b\n".into());
    let value = transport
        .send_json(request(
            &server.uri(),
            reqwest::Method::GET,
            "/teams/me/usage",
            Some(&credential),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(value, json!({"remainingUsd": 1.5}));
}

#[tokio::test]
async fn api_key_credential_rides_x_api_key_and_never_authorization() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/payments/summary"))
        .and(header("x-api-key", "th_key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let transport = SdkBackendTransport::new().unwrap();
    let credential = BackendCredential::ApiKey("th_key".into());
    let value = transport
        .send_json(request(
            &server.uri(),
            reqwest::Method::GET,
            "/payments/summary",
            Some(&credential),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(value, json!({"ok": true}));
    let received = server.received_requests().await.unwrap();
    assert!(received[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn non_2xx_maps_to_status_with_parsed_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "Invalid token"})))
        .mount(&server)
        .await;

    let transport = SdkBackendTransport::new().unwrap();
    let err = transport
        .send_json(request(
            &server.uri(),
            reqwest::Method::GET,
            "/x",
            None,
            true,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        BackendTransportError::Status { status: 401, ref body } if body == &json!({"error": "Invalid token"})
    ));
}

#[tokio::test]
async fn failed_envelope_on_2xx_maps_to_envelope_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/y"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": false,
            "error": "Toolkit X is not enabled",
            "errorCode": "TOOLKIT_DISABLED"
        })))
        .mount(&server)
        .await;

    let transport = SdkBackendTransport::new().unwrap();
    let err = transport
        .send_json(request(
            &server.uri(),
            reqwest::Method::POST,
            "/y",
            None,
            true,
        ))
        .await
        .unwrap_err();
    match err {
        BackendTransportError::Envelope {
            error, error_code, ..
        } => {
            assert_eq!(error, "Toolkit X is not enabled");
            assert_eq!(error_code.as_deref(), Some("TOOLKIT_DISABLED"));
        }
        other => panic!("expected Envelope, got {other:?}"),
    }
}

#[tokio::test]
async fn raw_envelope_is_returned_when_not_unwrapping() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/agent-integrations/pricing"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"success": true, "data": {"a": 1}})),
        )
        .mount(&server)
        .await;

    let transport = SdkBackendTransport::new().unwrap();
    let value = transport
        .send_json(request(
            &server.uri(),
            reqwest::Method::GET,
            "/agent-integrations/pricing",
            None,
            false,
        ))
        .await
        .unwrap();
    assert_eq!(value, json!({"success": true, "data": {"a": 1}}));
}

#[tokio::test]
async fn sdk_route_policy_refuses_unexposed_routes_before_sending() {
    let server = MockServer::start().await;
    let transport = SdkBackendTransport::new().unwrap();
    let err = transport
        .send_json(request(
            &server.uri(),
            reqwest::Method::POST,
            "/admin/coupons",
            None,
            true,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(err, BackendTransportError::RouteNotExposed(m, p) if m == "POST" && p.contains("admin"))
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn install_shares_one_transport_with_the_hosted_client() {
    let _registry = crate::registry_test_lock_async().await;

    let _guard = global_lock().lock().await;
    let installed = crate::install(crate::InstallOptions::default()).unwrap();
    assert!(crate::is_installed());
    assert_eq!(installed.name(), "tinyhumans-sdk");
    let shared = crate::client::shared_transport().unwrap();
    assert!(std::sync::Arc::ptr_eq(&installed, &shared));
}

#[tokio::test]
async fn backend_client_round_trips_through_the_installed_transport() {
    let _registry = crate::registry_test_lock_async().await;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/announcements/latest"))
        .and(header("authorization", "Bearer jwt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {"id": "a1"}
        })))
        .mount(&server)
        .await;

    let _guard = global_lock().lock().await;
    let _t = crate::install(crate::InstallOptions::default()).unwrap();
    let client = crate::client::BackendClient::new(&server.uri()).unwrap();
    let value = client
        .authed_json("jwt", reqwest::Method::GET, "/announcements/latest", None)
        .await
        .unwrap();
    assert_eq!(value, json!({"id": "a1"}));
}

/// `install` registers the hosted RPC proxies with the core's registry: the
/// billing controller is dispatchable by its unchanged wire name and reaches
/// the backend through the SDK transport.
#[tokio::test]
async fn install_registers_hosted_controllers_that_dispatch_through_the_transport() {
    let _registry = crate::registry_test_lock_async().await;

    use openhuman_embed::__host::core::all::{
        namespace_description, rpc_method_from_parts, schema_for_rpc_method,
    };

    let _guard = global_lock().lock().await;
    let _t = crate::install(crate::InstallOptions::default()).unwrap();

    for (ns, f) in [
        ("billing", "get_balance"),
        ("team", "get_usage"),
        ("referral", "get_stats"),
        ("announcements", "get_latest"),
        // Account-bound methods that share a namespace with core controllers.
        ("webhooks", "list_tunnels"),
        ("webhooks", "get_bandwidth"),
        ("auth", "create_channel_link_token"),
        ("auth", "oauth_connect"),
        ("channels", "telegram_login_start"),
        ("channels", "discord_link_check"),
        // ...alongside the core's own controllers in those namespaces.
        ("webhooks", "list_registrations"),
        ("auth", "get_state"),
        ("channels", "list"),
    ] {
        assert!(
            rpc_method_from_parts(ns, f).is_some(),
            "{ns}.{f} must be registered after install()"
        );
    }
    assert!(schema_for_rpc_method("openhuman.billing_get_balance").is_some());
    assert!(namespace_description("billing").is_some());
    assert!(namespace_description("team").is_some());
    assert!(namespace_description("webhooks").is_some());

    // A second install is a no-op for the registry, not a collision.
    crate::install(crate::InstallOptions::default()).unwrap();
}

/// The core asks the installed transport where the backend is: control-plane
/// calls skip an `api_url` that is really an inference endpoint, managed
/// inference honours it.
#[test]
fn base_url_applies_the_inference_endpoint_guard_per_purpose() {
    let transport = SdkBackendTransport::new().unwrap();
    let ollama = Some("http://127.0.0.1:11434/v1");
    let control = transport.base_url(ollama, BaseUrlPurpose::ControlPlane);
    assert_ne!(control, "http://127.0.0.1:11434/v1");
    assert!(!control.contains("11434"), "{control}");
    assert_eq!(
        transport.base_url(ollama, BaseUrlPurpose::Inference),
        "http://127.0.0.1:11434/v1"
    );
    assert_eq!(
        transport.base_url(
            Some("https://api.tinyhumans.ai/openai/v1/chat/completions"),
            BaseUrlPurpose::ControlPlane
        ),
        "https://api.tinyhumans.ai"
    );
}

#[test]
fn product_identity_and_attribution_come_from_this_crate() {
    let _guard = crate::backend::product::product_identity_test_lock();
    crate::backend::product::reset_product_identity_for_test();
    let transport = SdkBackendTransport::new().unwrap();
    assert_eq!(
        transport.product_identity(),
        crate::backend::DEFAULT_PRODUCT_IDENTITY
    );
    let headers = transport.attribution_headers();
    assert_eq!(
        headers
            .get(crate::backend::PRODUCT_IDENTITY_HEADER)
            .and_then(|v| v.to_str().ok()),
        Some(crate::backend::DEFAULT_PRODUCT_IDENTITY)
    );
    assert!(headers.get("x-core-version").is_some());
}

#[tokio::test]
async fn core_channel_routes_recover_from_the_transports_typed_404s() {
    let _registry = crate::registry_test_lock_async().await;

    use crate::client::HostedApiError as BackendApiError;
    // POST + DELETE only, mirroring the deployed router: PATCH falls through
    // to Express's HTML 404 (#5230); DELETE answers a handler-level 404.
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/channels/telegram/messages/1103"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string("Cannot PATCH /channels/telegram/messages/1103"),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/channels/telegram/messages/1103"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"success": false, "error": "gone"})),
        )
        .mount(&server)
        .await;

    let _guard = global_lock().lock().await;
    let _t = crate::install(crate::InstallOptions::default()).unwrap();
    let client = crate::client::BackendClient::new(&server.uri()).unwrap();

    let err = client
        .send_channel_edit("telegram", "1103", "jwt", json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<BackendApiError>(),
            Some(BackendApiError::ChannelEditUnsupported { provider, message_id })
                if provider == "telegram" && message_id == "1103"
        ),
        "edit: {err:#}"
    );

    let err = client
        .send_channel_delete("telegram", "1103", "jwt")
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<BackendApiError>(),
            Some(BackendApiError::MessageNotFound { provider, message_id })
                if provider == "telegram" && message_id == "1103"
        ),
        "delete: {err:#}"
    );
}
