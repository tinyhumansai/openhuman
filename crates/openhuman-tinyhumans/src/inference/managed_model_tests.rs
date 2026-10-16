use super::super::managed_reasoning::{apply_reasoning_hint, with_thread_id};
use super::*;
use openhuman_embed::__host::inference_registry::REASONING_OFF_METADATA_KEY;
use serde_json::Value;
use tinyinference_llm::model::ChatModel;

/// Mirror of the core's `without_reasoning` hint: the metadata flag the managed
/// model turns into `reasoning: { enabled: false }`.
fn without_reasoning(mut request: ModelRequest) -> ModelRequest {
    request.metadata = serde_json::json!({ REASONING_OFF_METADATA_KEY: true });
    request
}
use tinyinference_llm::message::Message;

fn backend() -> OpenHumanBackendModel {
    OpenHumanBackendModel::new(
        Some("https://api.example.test"),
        &ManagedOptions::default(),
        "reasoning-v1",
    )
}

#[test]
fn with_thread_id_injects_explicit_thread() {
    let request = ModelRequest::new(vec![Message::user("hi")]);
    let updated = with_thread_id(request, Some("thread-42"));
    assert_eq!(
        updated.provider_options["thread_id"],
        serde_json::json!("thread-42")
    );
}

#[test]
fn with_thread_id_is_noop_without_explicit_thread() {
    let request = ModelRequest::new(vec![Message::user("hi")]);
    let updated = with_thread_id(request, None);
    assert!(updated.provider_options.get("thread_id").is_none());
}

#[test]
fn managed_model_keeps_its_explicit_thread_without_an_ambient_scope() {
    let model = backend().with_thread_id(Some("  delegate-thread  "));
    assert_eq!(model.thread_id.as_deref(), Some("delegate-thread"));
}

#[test]
fn managed_model_advertises_tool_and_vision_capabilities() {
    let model = backend();
    let profile = model.profile().expect("managed profile");
    assert!(profile.tool_calling);
    assert!(profile.modalities.image_in);
}

#[test]
fn managed_model_supports_only_serializable_native_media_inputs() {
    use tinyinference_llm::model::{InputModality, InputSource};

    let model = backend();
    for mime in ["image/png", "image/jpeg", "image/webp", "image/gif"] {
        for source in [InputSource::Base64, InputSource::Url] {
            assert!(
                model.supports_input(InputModality::Image, mime, source),
                "managed transport should serialize {mime} from {source:?}"
            );
        }
    }

    for mime in ["audio/wav", "audio/x-wav", "audio/mpeg", "audio/mp3"] {
        assert!(
            model.supports_input(InputModality::Audio, mime, InputSource::Base64),
            "managed transport should serialize inline {mime}"
        );
        assert!(
            !model.supports_input(InputModality::Audio, mime, InputSource::Url),
            "managed transport must not advertise URL serialization for {mime}"
        );
    }

    for modality in [InputModality::Image, InputModality::Audio] {
        for source in [InputSource::Base64, InputSource::Url, InputSource::Path] {
            assert!(
                !model.supports_input(modality, "application/pdf", source),
                "managed transport must reject PDF as {modality:?}/{source:?}"
            );
        }
    }
    for modality in [InputModality::Video, InputModality::Document] {
        for source in [InputSource::Base64, InputSource::Url, InputSource::Path] {
            assert!(
                !model.supports_input(modality, "application/octet-stream", source),
                "managed transport must reject {modality:?}/{source:?}"
            );
        }
    }
    assert!(!model.supports_input(InputModality::Image, "image/tiff", InputSource::Base64));
    assert!(!model.supports_input(InputModality::Audio, "audio/ogg", InputSource::Base64));
    assert!(!model.supports_input(InputModality::Image, "image/png", InputSource::Path));
}

#[test]
fn resolve_model_normalizes_blank_and_trims_non_empty_values() {
    assert_eq!(
        resolve_model(""),
        openhuman_embed::__host::config::MODEL_MANAGED_DEFAULT
    );
    assert_eq!(
        resolve_model(" \t\n"),
        openhuman_embed::__host::config::MODEL_MANAGED_DEFAULT
    );
    assert_eq!(resolve_model("  reasoning-v1  "), "reasoning-v1");
    assert_eq!(resolve_model("hint:reasoning"), "hint:reasoning");
}

// ── probe_readiness (B45 — flows provider-connectivity author gate) ────

#[test]
fn is_provider_not_configured_error_matches_exact_backend_shape() {
    let err = ProviderError {
        provider: "OpenHuman".to_string(),
        model: None,
        status: Some(400),
        code: Some("BAD_REQUEST".to_string()),
        message: "API key not configured for provider".to_string(),
        retryable: false,
        retry_after_ms: None,
        raw: None,
        ..ProviderError::default()
    };
    assert!(is_provider_not_configured_error(&err));
}

#[test]
fn is_provider_not_configured_error_rejects_other_400s() {
    // A 400 that isn't the "no provider configured" class (e.g. a bad
    // request shape) must NOT be classified as provider-not-configured —
    // only the exact backend-confirmed signal should ever reject.
    let err = ProviderError {
        provider: "OpenHuman".to_string(),
        model: None,
        status: Some(400),
        code: Some("BAD_REQUEST".to_string()),
        message: "invalid request: messages must not be empty".to_string(),
        retryable: false,
        retry_after_ms: None,
        raw: None,
        ..ProviderError::default()
    };
    assert!(!is_provider_not_configured_error(&err));
}

#[test]
fn is_provider_not_configured_error_tolerates_not_configured_for_provider_wording_drift() {
    // The `code_is_bad_request` branch still matches the narrower
    // "not configured for provider" substring even when it isn't
    // introduced by the exact "api key" prefix — tolerance for backend
    // message wording drift, not a broadening to any "not configured".
    let err = ProviderError {
        provider: "OpenHuman".to_string(),
        model: None,
        status: Some(400),
        code: Some("BAD_REQUEST".to_string()),
        message: "credentials not configured for provider 'anthropic'".to_string(),
        retryable: false,
        retry_after_ms: None,
        raw: None,
        ..ProviderError::default()
    };
    assert!(is_provider_not_configured_error(&err));
}

#[test]
fn is_provider_not_configured_error_rejects_generic_not_configured_400() {
    // Tightened contract (finding D): a 400 `BAD_REQUEST` whose message
    // contains only the generic word "not configured" — but not the
    // specific "not configured for provider" phrasing — must fail OPEN,
    // not be misclassified as the provider-key signal. Otherwise an
    // unrelated backend validation error ("model X not configured", "this
    // feature is not configured for your account", …) would falsely
    // reject a run/proposal as a provider problem.
    let err = ProviderError {
        provider: "OpenHuman".to_string(),
        model: None,
        status: Some(400),
        code: Some("BAD_REQUEST".to_string()),
        message: "webhook target not configured".to_string(),
        retryable: false,
        retry_after_ms: None,
        raw: None,
        ..ProviderError::default()
    };
    assert!(!is_provider_not_configured_error(&err));
}

#[test]
fn is_provider_not_configured_error_rejects_non_400_status() {
    let err = ProviderError {
        provider: "OpenHuman".to_string(),
        model: None,
        status: Some(401),
        code: None,
        message: "API key not configured for provider".to_string(),
        retryable: false,
        retry_after_ms: None,
        raw: None,
        ..ProviderError::default()
    };
    assert!(!is_provider_not_configured_error(&err));
}

fn seed_app_session(dir: &std::path::Path) {
    use openhuman_embed::__host::security::credentials::{
        AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
    };
    AuthService::new(dir, false)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "test.session.jwt",
            std::collections::HashMap::new(),
            true,
        )
        .expect("seed app-session token");
}

/// Seed an app-session profile whose recorded `exp` metadata is `expires_at`
/// (RFC3339) so the `resolve_bearer` local-expiry precheck (#5503, part e)
/// can be exercised without a live backend.
fn seed_app_session_with_expiry(dir: &std::path::Path, expires_at: &str) {
    use openhuman_embed::__host::security::credentials::{
        session_support::SESSION_EXPIRES_AT_META, AuthService, APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
    };
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(SESSION_EXPIRES_AT_META.to_string(), expires_at.to_string());
    AuthService::new(dir, false)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "test.session.jwt",
            metadata,
            true,
        )
        .expect("seed app-session token with expiry");
}

fn backend_pointed_at(addr: &str, dir: &std::path::Path) -> OpenHumanBackendModel {
    OpenHumanBackendModel::new(
        Some(&format!("http://{addr}")),
        &ManagedOptions {
            openhuman_dir: Some(dir.to_path_buf()),
            secrets_encrypt: false,
            ..ManagedOptions::default()
        },
        "reasoning-v1",
    )
}

#[derive(Clone)]
struct StaticChatResponse {
    status: axum::http::StatusCode,
    body: Value,
}

async fn static_chat_handler(
    axum::extract::State(s): axum::extract::State<StaticChatResponse>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    (s.status, axum::Json(s.body)).into_response()
}

async fn spawn_static_chat_server(status: axum::http::StatusCode, body: Value) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = axum::Router::new()
        .route(
            "/openai/v1/chat/completions",
            axum::routing::post(static_chat_handler),
        )
        .with_state(StaticChatResponse { status, body });
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    addr.to_string()
}

#[path = "managed_model_stream_tests.rs"]
mod stream_tests;

async fn slow_chat_handler() -> axum::response::Response {
    use axum::response::IntoResponse;
    // Longer than the probe's 5s timeout — the probe must return before
    // this ever resolves.
    tokio::time::sleep(Duration::from_secs(8)).await;
    (
        axum::http::StatusCode::OK,
        axum::Json(serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": "pong" } }]
        })),
    )
        .into_response()
}

async fn spawn_slow_chat_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let app = axum::Router::new().route(
        "/openai/v1/chat/completions",
        axum::routing::post(slow_chat_handler),
    );
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    addr.to_string()
}

#[tokio::test]
async fn probe_readiness_surfaces_api_key_not_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_app_session(tmp.path());
    let addr = spawn_static_chat_server(
        axum::http::StatusCode::BAD_REQUEST,
        serde_json::json!({
            "success": false,
            "error": "API key not configured for provider",
            "errorCode": "BAD_REQUEST"
        }),
    )
    .await;
    let backend = backend_pointed_at(&addr, tmp.path());

    let err = backend
        .probe_readiness()
        .await
        .expect_err("a confirmed provider-not-configured 400 must reject");
    assert!(
        err.to_ascii_lowercase()
            .contains("api key not configured for provider"),
        "error must surface the backend's own message: {err}"
    );
}

#[tokio::test]
async fn probe_readiness_fails_open_on_timeout_or_5xx() {
    // 5xx sub-case: a transient backend failure must never block authoring.
    let tmp = tempfile::TempDir::new().unwrap();
    seed_app_session(tmp.path());
    let addr = spawn_static_chat_server(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        serde_json::json!({ "error": "temporarily unavailable" }),
    )
    .await;
    let backend = backend_pointed_at(&addr, tmp.path());
    backend
        .probe_readiness()
        .await
        .expect("a transient 5xx must fail open (Ok)");

    // Timeout sub-case: a hung backend must fail open once the 5s probe
    // timeout fires, without waiting for the slow handler to respond.
    let tmp2 = tempfile::TempDir::new().unwrap();
    seed_app_session(tmp2.path());
    let addr2 = spawn_slow_chat_server().await;
    let backend2 = backend_pointed_at(&addr2, tmp2.path());
    let started = std::time::Instant::now();
    backend2
        .probe_readiness()
        .await
        .expect("a hung backend must fail open (Ok) once the 5s timeout fires");
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "probe must return around the 5s timeout, not wait for the slow handler"
    );
}

#[path = "managed_model_reasoning_tests.rs"]
mod reasoning_tests;

// ── shared pooled client (time to first token) ──────────────────────────

/// A TCP relay in front of `upstream` that counts the connections it accepts.
async fn spawn_counting_relay(
    upstream: String,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind relay");
    let addr = listener.local_addr().expect("relay addr").to_string();
    let counter = accepted.clone();
    tokio::spawn(async move {
        while let Ok((mut inbound, _)) = listener.accept().await {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let upstream = upstream.clone();
            tokio::spawn(async move {
                if let Ok(mut outbound) = tokio::net::TcpStream::connect(&upstream).await {
                    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                }
            });
        }
    });
    (addr, accepted)
}

/// Every managed call used to build its own `reqwest::Client`, so each call
/// opened a new connection (a fresh TCP + TLS handshake against the real
/// backend) before its first token. Consecutive calls must now reuse one.
#[tokio::test]
async fn consecutive_managed_calls_reuse_one_connection() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_app_session(tmp.path());
    let upstream = spawn_static_chat_server(
        axum::http::StatusCode::OK,
        serde_json::json!({
            "id": "chatcmpl-pool",
            "object": "chat.completion",
            "created": 1,
            "model": "reasoning-v1",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "ok" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        }),
    )
    .await;
    let (relay, accepted) = spawn_counting_relay(upstream).await;
    let backend = backend_pointed_at(&relay, tmp.path());

    for call in 0..3 {
        backend
            .probe_readiness()
            .await
            .unwrap_or_else(|error| panic!("managed call {call} failed: {error}"));
    }

    assert_eq!(
        accepted.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "three consecutive managed calls must share one pooled connection"
    );
}

#[test]
fn resolve_bearer_fast_fails_session_expired_on_expired_token() {
    let tmp = tempfile::TempDir::new().unwrap();
    let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    seed_app_session_with_expiry(tmp.path(), &past);
    let backend = backend_pointed_at("127.0.0.1:9", tmp.path());

    let err = backend
        .resolve_bearer()
        .expect_err("an expired managed JWT must fast-fail the precheck");
    let msg = err.to_string();
    assert!(
        msg.contains("SESSION_EXPIRED"),
        "must carry the SESSION_EXPIRED sentinel the classifier keys on: {msg}"
    );
    assert!(
        openhuman_embed::__host::core::observability::is_session_expired_message(&msg),
        "must classify as session-expiry, not model-unavailable: {msg}"
    );
}

#[test]
fn resolve_bearer_returns_token_when_expiry_in_future() {
    let tmp = tempfile::TempDir::new().unwrap();
    let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    seed_app_session_with_expiry(tmp.path(), &future);
    let backend = backend_pointed_at("127.0.0.1:9", tmp.path());

    let token = backend
        .resolve_bearer()
        .expect("a live (future-exp) managed JWT must resolve");
    assert_eq!(token, "test.session.jwt");
}

fn backend_with_api_key(api_url: &str, dir: &std::path::Path) -> OpenHumanBackendModel {
    openhuman_embed::__host::security::credentials::api_key::store_api_key_in(
        dir,
        false,
        "th_test_key",
    )
    .expect("seed api key");
    OpenHumanBackendModel::new(
        Some(api_url),
        &ManagedOptions {
            openhuman_dir: Some(dir.to_path_buf()),
            secrets_encrypt: false,
            ..ManagedOptions::default()
        },
        "reasoning-v1",
    )
}

#[test]
fn resolve_bearer_refuses_a_stored_api_key_over_plaintext_non_loopback() {
    // A non-HTTPS, non-loopback managed endpoint must never carry the
    // TinyHumans API key as a bearer — that puts a durable credential (not a
    // short-lived session JWT) on the wire in the clear.
    let tmp = tempfile::TempDir::new().unwrap();
    let backend = backend_with_api_key("http://api.example.test", tmp.path());

    let err = backend
        .resolve_bearer()
        .expect_err("a plaintext non-loopback endpoint must refuse the api-key bearer");
    let msg = err.to_string();
    assert!(
        msg.contains("refusing to send")
            && msg.contains("unmanaged or insecure")
            && msg.contains("api.example.test"),
        "error must name the refusal and the offending endpoint: {msg}"
    );
}

#[test]
fn resolve_bearer_sends_a_stored_api_key_to_managed_https() {
    let tmp = tempfile::TempDir::new().unwrap();
    let backend = backend_with_api_key("https://api.tinyhumans.ai", tmp.path());

    let token = backend
        .resolve_bearer()
        .expect("managed HTTPS must be allowed to carry the api-key bearer");
    assert_eq!(token, "th_test_key");
}

#[test]
fn resolve_bearer_rejects_foreign_https_for_api_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let backend = backend_with_api_key("https://api.example.test", tmp.path());
    let error = backend.resolve_bearer().unwrap_err();
    assert!(error.to_string().contains("unmanaged or insecure"));
}

#[test]
fn resolve_bearer_sends_a_stored_api_key_over_plain_loopback() {
    let tmp = tempfile::TempDir::new().unwrap();
    let backend = backend_with_api_key("http://127.0.0.1:9999", tmp.path());

    let token = backend
        .resolve_bearer()
        .expect("loopback http must still be allowed for local testing");
    assert_eq!(token, "th_test_key");
}

#[test]
fn resolve_bearer_returns_token_for_exp_less_offline_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_app_session(tmp.path());
    let backend = backend_pointed_at("127.0.0.1:9", tmp.path());

    let token = backend
        .resolve_bearer()
        .expect("an exp-less offline session must resolve (presence-only)");
    assert_eq!(token, "test.session.jwt");
}
