use super::*;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::{ChatModel, ModelRequest};

#[test]
fn managed_model_preserves_the_openhuman_response_cache_identity() {
    let model = build_chat_model(
        Some("https://api.example.test"),
        &ManagedOptions::default(),
        "reasoning-v1",
        true,
        None,
        false,
    )
    .expect("managed owner model");

    assert_eq!(
        model.cache_identity().as_deref(),
        Some("openhuman:https://api.example.test/openai/v1:reasoning-v1")
    );
}

#[test]
fn managed_factory_passes_product_default_to_native_model_when_configured_model_is_blank() {
    let configured = build_chat_model(
        Some("https://api.example.test"),
        &ManagedOptions::default(),
        "  reasoning-v1  ",
        true,
        None,
        false,
    )
    .expect("configured model");
    assert_eq!(
        configured.cache_identity().as_deref(),
        Some("openhuman:https://api.example.test/openai/v1:reasoning-v1")
    );

    let blank = build_chat_model(
        Some("https://api.example.test"),
        &ManagedOptions::default(),
        "  ",
        true,
        None,
        false,
    )
    .expect("blank model falls back to the product default");
    assert_eq!(
        blank.cache_identity().as_deref(),
        Some("openhuman:https://api.example.test/openai/v1:openrouter/deepseek/deepseek-v4-flash")
    );
}

#[test]
fn managed_native_model_exposes_tool_vision_and_physical_attempt_capabilities() {
    use tinyinference_llm::model::{InputModality, InputSource};

    let model = build_chat_model(
        Some("https://api.example.test"),
        &ManagedOptions::default(),
        "reasoning-v1",
        true,
        None,
        false,
    )
    .expect("managed native model");

    let profile = model.profile().expect("managed model profile");
    assert!(profile.tool_calling);
    assert!(profile.modalities.image_in);
    assert!(model.tracks_physical_attempts());
    assert!(model.supports_input(InputModality::Image, "image/png", InputSource::Base64));
}

#[tokio::test]
async fn managed_bearer_keeps_the_stored_key_transport_guard() {
    let tmp = tempfile::TempDir::new().expect("temporary credentials");
    openhuman_embed::__host::credentials::api_key::store_api_key_in(
        tmp.path(),
        false,
        "test-managed-key",
    )
    .await
    .expect("store test API key");
    let options = ManagedOptions {
        openhuman_dir: Some(tmp.path().to_path_buf()),
        secrets_encrypt: false,
        ..ManagedOptions::default()
    };

    assert_eq!(
        options
            .resolve_bearer("https://api.tinyhumans.ai/openai/v1")
            .await
            .expect("managed TLS endpoint accepts the API key"),
        "test-managed-key"
    );
    assert_eq!(
        options
            .resolve_bearer("http://127.0.0.1:9999/openai/v1")
            .await
            .expect("loopback is a valid local test endpoint"),
        "test-managed-key"
    );
    let plaintext = options
        .resolve_bearer("http://api.example.test/openai/v1")
        .await
        .expect_err("a plaintext remote endpoint must not receive the stored key");
    assert!(plaintext.to_string().contains("http://api.example.test"));
    let error = options
        .resolve_bearer("https://api.example.test/openai/v1")
        .await
        .expect_err("a foreign HTTPS origin must not receive the stored key");
    assert!(error.to_string().contains("unmanaged or insecure"));
}

async fn seed_app_session(dir: &std::path::Path, expires_at: Option<&str>) {
    use openhuman_embed::__host::credentials::{
        session_state::SESSION_EXPIRES_AT_META, AuthService, APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
    };
    let mut metadata = std::collections::HashMap::new();
    if let Some(expires_at) = expires_at {
        metadata.insert(SESSION_EXPIRES_AT_META.to_string(), expires_at.to_string());
    }
    AuthService::new(dir, false)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "test.session.jwt",
            metadata,
            true,
        )
        .await
        .expect("seed app-session token");
}

fn managed_options_at(dir: &std::path::Path) -> ManagedOptions {
    ManagedOptions {
        openhuman_dir: Some(dir.to_path_buf()),
        secrets_encrypt: false,
        ..ManagedOptions::default()
    }
}

#[tokio::test]
async fn managed_bearer_fast_fails_an_expired_app_session() {
    let tmp = tempfile::TempDir::new().expect("temporary credentials");
    let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    seed_app_session(tmp.path(), Some(&past)).await;

    let error = managed_options_at(tmp.path())
        .resolve_bearer("http://127.0.0.1:9/openai/v1")
        .await
        .expect_err("expired session should fail locally before transport");
    let message = error.to_string();
    assert!(message.contains("SESSION_EXPIRED"), "{message}");
    assert!(
        openhuman_embed::__host::core::observability::is_session_expired_message(&message),
        "expired managed session must retain its host classification: {message}"
    );
}

#[tokio::test]
async fn managed_bearer_resolves_a_future_expiry_app_session() {
    let tmp = tempfile::TempDir::new().expect("temporary credentials");
    let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    seed_app_session(tmp.path(), Some(&future)).await;

    assert_eq!(
        managed_options_at(tmp.path())
            .resolve_bearer("http://127.0.0.1:9/openai/v1")
            .await
            .expect("a future expiry is valid"),
        "test.session.jwt"
    );
}

#[tokio::test]
async fn managed_session_bearer_refuses_a_plaintext_remote_endpoint() {
    let tmp = tempfile::TempDir::new().expect("temporary credentials");
    seed_app_session(tmp.path(), Some("2999-01-01T00:00:00Z")).await;
    let options = managed_options_at(tmp.path());

    let error = options
        .resolve_bearer("http://10.0.0.1:1/openai/v1")
        .await
        .expect_err("plaintext remote endpoint must not receive the session token");
    assert!(error.to_string().contains("insecure endpoint"), "{error}");
    assert_eq!(
        options
            .resolve_bearer("https://api.example.test/openai/v1")
            .await
            .expect("HTTPS stays allowed"),
        "test.session.jwt"
    );
}

#[tokio::test]
async fn managed_bearer_resolves_exp_less_hosted_app_sessions() {
    let tmp = tempfile::TempDir::new().expect("temporary credentials");
    seed_app_session(tmp.path(), None).await;

    assert_eq!(
        managed_options_at(tmp.path())
            .resolve_bearer("http://127.0.0.1:9/openai/v1")
            .await
            .expect("an exp-less hosted session resolves by presence"),
        "test.session.jwt"
    );
}

#[tokio::test]
async fn managed_api_key_precedes_an_expired_app_session() {
    let tmp = tempfile::TempDir::new().expect("temporary credentials");
    let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    seed_app_session(tmp.path(), Some(&past)).await;
    openhuman_embed::__host::credentials::api_key::store_api_key_in(
        tmp.path(),
        false,
        "managed-api-key",
    )
    .await
    .expect("store API key");

    assert_eq!(
        managed_options_at(tmp.path())
            .resolve_bearer("https://api.tinyhumans.ai/openai/v1")
            .await
            .expect("the configured API key takes precedence"),
        "managed-api-key"
    );
}

#[tokio::test]
async fn managed_bearer_refusal_redacts_foreign_endpoint_details() {
    let tmp = tempfile::TempDir::new().expect("temporary credentials");
    openhuman_embed::__host::credentials::api_key::store_api_key_in(
        tmp.path(),
        false,
        "SYNTHETIC_BEARER_MARKER",
    )
    .await
    .expect("store test API key");
    let options = ManagedOptions {
        openhuman_dir: Some(tmp.path().to_path_buf()),
        secrets_encrypt: false,
        ..ManagedOptions::default()
    };
    for (endpoint, markers) in [
        (
            "https://synthetic-user:SYNTHETIC_PASSWORD@foreign.example/private/SYNTHETIC_PATH?token=SYNTHETIC_QUERY#SYNTHETIC_FRAGMENT",
            [
                "synthetic-user",
                "SYNTHETIC_PASSWORD",
                "SYNTHETIC_PATH",
                "SYNTHETIC_QUERY",
                "SYNTHETIC_FRAGMENT",
            ],
        ),
        (
            "https://foreign.example/private/SYNTHETIC_PATH?token=SYNTHETIC_QUERY#SYNTHETIC_FRAGMENT",
            [
                "unused-userinfo",
                "unused-password",
                "SYNTHETIC_PATH",
                "SYNTHETIC_QUERY",
                "SYNTHETIC_FRAGMENT",
            ],
        ),
    ] {
        let error = options
            .resolve_bearer(endpoint)
            .await
            .expect_err("foreign endpoint must remain rejected");
        let diagnostic = error.to_string();
        for marker in markers.into_iter().chain(["SYNTHETIC_BEARER_MARKER"]) {
            assert!(
                !diagnostic.contains(marker),
                "refusal diagnostic exposed {marker}: {diagnostic}"
            );
        }
        assert!(diagnostic.contains("refusing to send"), "{diagnostic}");
    }
}

#[test]
fn managed_endpoint_diagnostic_is_origin_only_or_a_constant_fallback() {
    assert_eq!(
        endpoint_origin_for_log(
            "https://synthetic-user:SYNTHETIC_PASSWORD@foreign.example/private/SYNTHETIC_PATH?token=SYNTHETIC_QUERY#SYNTHETIC_FRAGMENT"
        ),
        "https://foreign.example"
    );
    assert_eq!(
        endpoint_origin_for_log("data:text/plain,SYNTHETIC_PATH"),
        "<invalid endpoint>"
    );
    assert_eq!(endpoint_origin_for_log("not a url"), "<invalid endpoint>");
}

/// Serves `status` for every chat request, with `marker` in the error body.
async fn serve_chat_status(
    status: axum::http::StatusCode,
    marker: &'static str,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    use axum::response::IntoResponse;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let addr = listener.local_addr().expect("fixture address");
    let app = axum::Router::new().route(
        "/openai/v1/chat/completions",
        axum::routing::post(move || async move {
            (
                status,
                axum::Json(serde_json::json!({ "error": { "message": marker } })),
            )
                .into_response()
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, server)
}

/// Only a 401 on the session credential expires the session: a 403 is a
/// refusal and an API-key 401 is a rejected key.
#[tokio::test]
async fn managed_auth_failure_expires_the_session_only_on_a_session_401() {
    use axum::http::StatusCode;
    use openhuman_embed::__host::core::events::DomainEvent;

    openhuman_embed::__host::core::bus::init()
        .await
        .expect("initialize event bus");
    let cases: [(&str, bool, StatusCode, &'static str, Option<&str>); 4] = [
        (
            "session 401",
            false,
            StatusCode::UNAUTHORIZED,
            "HOST_AUTH_SESSION_401",
            Some("openhuman_backend_model.invoke(401)"),
        ),
        (
            "session 403",
            false,
            StatusCode::FORBIDDEN,
            "HOST_AUTH_SESSION_403",
            None,
        ),
        (
            "api-key 401",
            true,
            StatusCode::UNAUTHORIZED,
            "HOST_AUTH_API_KEY_401",
            None,
        ),
        (
            "api-key 403",
            true,
            StatusCode::FORBIDDEN,
            "HOST_AUTH_API_KEY_403",
            None,
        ),
    ];
    for (name, api_key, status, marker, expected) in cases {
        let mut events = openhuman_embed::__host::core::bus::BUS
            .get()
            .expect("event bus initialized")
            .receiver();
        let tmp = tempfile::TempDir::new().expect("temporary credentials");
        seed_app_session(tmp.path(), Some("2999-01-01T00:00:00Z")).await;
        if api_key {
            openhuman_embed::__host::credentials::api_key::store_api_key_in(
                tmp.path(),
                false,
                "test-managed-key",
            )
            .await
            .expect("store test API key");
        }
        let (addr, server) = serve_chat_status(status, marker).await;
        let model = build_chat_model(
            Some(&format!("http://{addr}")),
            &managed_options_at(tmp.path()),
            "reasoning-v1",
            true,
            None,
            true,
        )
        .expect("build owner model");

        assert!(model
            .invoke(&(), ModelRequest::new(vec![Message::user("ping")]))
            .await
            .is_err());
        server.abort();

        let mut source = None;
        let mut fingerprint = None;
        loop {
            match events.try_recv() {
                Ok(DomainEvent::SessionExpired {
                    source: value,
                    reason,
                    credential_fingerprint: value_fingerprint,
                }) if reason.contains(marker) => {
                    source = Some(value);
                    fingerprint = value_fingerprint;
                    break;
                }
                Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        assert_eq!(source.as_deref(), expected, "{name}");
        if expected.is_some() {
            assert_eq!(
                fingerprint.as_deref(),
                Some(
                    openhuman_embed::__host::credentials::backend_credential::credential_fingerprint(
                        "test.session.jwt",
                    )
                    .as_str(),
                ),
                "managed expiry must carry the bearer used by its failed request"
            );
        }
    }
}

/// A late API-key 401 must not be read as the session's: the failure carries
/// the credential its own request was sent with, not whichever credential the
/// model resolved most recently.
#[tokio::test]
async fn a_late_api_key_401_after_a_session_request_keeps_the_session() {
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use openhuman_embed::__host::core::events::DomainEvent;
    use openhuman_embed::__host::credentials::AuthService;
    use std::sync::Arc;
    use tokio::sync::Notify;

    openhuman_embed::__host::core::bus::init()
        .await
        .expect("initialize event bus");
    let mut events = openhuman_embed::__host::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();
    let tmp = tempfile::TempDir::new().expect("temporary credentials");
    seed_app_session(tmp.path(), Some("2999-01-01T00:00:00Z")).await;
    let api_key_profile = openhuman_embed::__host::credentials::api_key::store_api_key_in(
        tmp.path(),
        false,
        "test-managed-key",
    )
    .await
    .expect("store test API key");

    // The API-key request is held until the session request has resolved its
    // own bearer and failed.
    let key_request_arrived = Arc::new(Notify::new());
    let release_key_request = Arc::new(Notify::new());
    let (arrived, release) = (key_request_arrived.clone(), release_key_request.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture server");
    let addr = listener.local_addr().expect("fixture address");
    let app = axum::Router::new().route(
        "/openai/v1/chat/completions",
        axum::routing::post(move |headers: HeaderMap| {
            let (arrived, release) = (arrived.clone(), release.clone());
            async move {
                let api_key = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer test-managed-key");
                let (status, marker) = if api_key {
                    arrived.notify_one();
                    release.notified().await;
                    (StatusCode::UNAUTHORIZED, "HOST_AUTH_LATE_API_KEY_401")
                } else {
                    (StatusCode::FORBIDDEN, "HOST_AUTH_SESSION_403")
                };
                (
                    status,
                    axum::Json(serde_json::json!({ "error": { "message": marker } })),
                )
                    .into_response()
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let model = Arc::new(
        build_chat_model(
            Some(&format!("http://{addr}")),
            &managed_options_at(tmp.path()),
            "reasoning-v1",
            true,
            None,
            true,
        )
        .expect("build owner model"),
    );

    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let key_model = model.clone();
        let key_request = tokio::spawn(async move {
            key_model
                .invoke(&(), ModelRequest::new(vec![Message::user("ping")]))
                .await
                .is_err()
        });
        key_request_arrived.notified().await;

        // The stored credential switches to the session mid-flight.
        assert!(AuthService::new(tmp.path(), false)
            .remove_profile(&api_key_profile.provider, &api_key_profile.profile_name)
            .await
            .expect("remove test API key"));
        assert!(model
            .invoke(&(), ModelRequest::new(vec![Message::user("ping")]))
            .await
            .is_err());

        release_key_request.notify_one();
        assert!(key_request.await.expect("API-key request task"));
    })
    .await
    .expect("the held API-key request completes");
    server.abort();

    loop {
        match events.try_recv() {
            Ok(DomainEvent::SessionExpired { source, reason, .. }) => {
                assert!(
                    !reason.contains("HOST_AUTH_LATE_API_KEY_401")
                        && !reason.contains("HOST_AUTH_SESSION_403"),
                    "a refusal signed the user out: {source}: {reason}"
                );
            }
            Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
            Err(_) => break,
        }
    }
}
