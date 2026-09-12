use super::*;
use crate::openhuman::inference::provider::ProviderRuntimeOptions;
use tinyinference::message::Message;

fn backend() -> OpenHumanBackendModel {
    OpenHumanBackendModel::new(
        Some("https://api.example.test"),
        &ProviderRuntimeOptions::default(),
        "reasoning-v1",
    )
}

#[tokio::test]
async fn with_thread_id_injects_when_ambient_thread_present() {
    thread_context::with_thread_id("thread-42", async {
        let request = ModelRequest::new(vec![Message::user("hi")]);
        let updated = with_thread_id(request);
        assert_eq!(
            updated.provider_options["thread_id"],
            serde_json::json!("thread-42")
        );
    })
    .await;
}

#[test]
fn with_thread_id_is_noop_without_ambient_thread() {
    // No thread scope active → provider_options stays whatever it was (null).
    let request = ModelRequest::new(vec![Message::user("hi")]);
    let updated = with_thread_id(request);
    assert!(updated.provider_options.get("thread_id").is_none());
}

#[test]
fn managed_model_advertises_tool_and_vision_capabilities() {
    let model = backend();
    let profile = model.profile().expect("managed profile");
    assert!(profile.tool_calling);
    assert!(profile.modalities.image_in);
}

#[test]
fn resolve_model_normalizes_blank_and_trims_non_empty_values() {
    assert_eq!(
        resolve_model(""),
        crate::openhuman::config::MODEL_REASONING_V1
    );
    assert_eq!(
        resolve_model(" \t\n"),
        crate::openhuman::config::MODEL_REASONING_V1
    );
    assert_eq!(resolve_model("  reasoning-v1  "), "reasoning-v1");
    assert_eq!(resolve_model("hint:reasoning"), "hint:reasoning");
}

/// The managed `openhuman.{billing,usage}` envelope on `raw` must re-project
/// into the host `UsageInfo` the cost bridge reads — charged USD, cached
/// tokens, and context window — exactly as the legacy legacy model-adapter path did.
#[test]
fn project_managed_usage_recovers_charged_and_cached() {
    use crate::openhuman::agent::tinyagents::model::usage_info_from_response;
    use tinyinference::message::AssistantMessage;
    use tinyinference::usage::Usage;

    let raw = serde_json::json!({
        "openhuman": {
            "usage": { "cached_input_tokens": 128, "context_window": 200000 },
            "billing": { "charged_amount_usd": 0.0042 }
        }
    });
    let response = ModelResponse {
        message: AssistantMessage {
            id: None,
            content: vec![],
            tool_calls: vec![],
            usage: None,
        },
        usage: Some(Usage {
            input_tokens: 1000,
            output_tokens: 50,
            ..Usage::default()
        }),
        finish_reason: None,
        raw: Some(raw),
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    };

    let projected = project_managed_usage(response);
    let usage = usage_info_from_response(&projected).expect("usage recovered");
    assert!(
        (usage.charged_amount_usd - 0.0042).abs() < 1e-9,
        "charged={}",
        usage.charged_amount_usd
    );
    assert_eq!(usage.cached_input_tokens, 128, "cached tokens backfilled");
    assert_eq!(usage.context_window, 200_000);
    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 50);
}

/// A response with no `openhuman` envelope stays untouched — no meta key, no
/// charged USD — so non-managed/billing-free responses aren't fabricated.
#[test]
fn project_managed_usage_is_noop_without_envelope() {
    use crate::openhuman::agent::tinyagents::model::usage_info_from_response;
    use tinyinference::message::AssistantMessage;
    use tinyinference::usage::Usage;

    let response = ModelResponse {
        message: AssistantMessage {
            id: None,
            content: vec![],
            tool_calls: vec![],
            usage: None,
        },
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 3,
            ..Usage::default()
        }),
        finish_reason: None,
        raw: Some(serde_json::json!({ "id": "resp_1" })),
        resolved_model: None,
        continue_turn: None,
        served_from_cache: false,
    };

    let projected = project_managed_usage(response);
    // raw keeps only the wire fields — no meta key injected.
    assert!(projected
        .raw
        .as_ref()
        .unwrap()
        .get("openhuman_usage_meta")
        .is_none());
    let usage = usage_info_from_response(&projected).expect("usage present");
    assert_eq!(usage.charged_amount_usd, 0.0);
    assert_eq!(usage.cached_input_tokens, 3, "crate cached count preserved");
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
    };
    assert!(!is_provider_not_configured_error(&err));
}

fn seed_app_session(dir: &std::path::Path) {
    use crate::openhuman::security::credentials::{
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
    use crate::openhuman::security::credentials::{
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
        &ProviderRuntimeOptions {
            openhuman_dir: Some(dir.to_path_buf()),
            secrets_encrypt: false,
            ..ProviderRuntimeOptions::default()
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

// ── resolve_bearer local-expiry precheck (#5503, part e) ───────────────

#[test]
fn resolve_bearer_fast_fails_session_expired_on_expired_token() {
    // An app-session JWT whose recorded `exp` is in the past must fail the
    // precheck as a `SESSION_EXPIRED` sentinel BEFORE any request is built —
    // so the web-chat classifier routes it to `session_expired` (actionable
    // re-auth) instead of a doomed request that can surface as a misleading
    // "model unavailable" (#5503). No backend is stood up: a correct
    // precheck never reaches the network.
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
        crate::core::observability::is_session_expired_message(&msg),
        "must classify as session-expiry, not model-unavailable: {msg}"
    );
}

#[test]
fn resolve_bearer_returns_token_when_expiry_in_future() {
    // A recorded `exp` comfortably in the future resolves normally — the
    // precheck only rejects the past-expiry case.
    let tmp = tempfile::TempDir::new().unwrap();
    let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    seed_app_session_with_expiry(tmp.path(), &future);
    let backend = backend_pointed_at("127.0.0.1:9", tmp.path());

    let token = backend
        .resolve_bearer()
        .expect("a live (future-exp) managed JWT must resolve");
    assert_eq!(token, "test.session.jwt");
}

#[test]
fn resolve_bearer_returns_token_for_exp_less_offline_session() {
    // Offline / local sessions record no `exp`, so the precheck falls
    // through to presence-only and their behaviour is unchanged (the
    // post-call 401 net still covers a server-side revocation). Guards the
    // #5503 precheck against breaking the offline path.
    let tmp = tempfile::TempDir::new().unwrap();
    seed_app_session(tmp.path());
    let backend = backend_pointed_at("127.0.0.1:9", tmp.path());

    let token = backend
        .resolve_bearer()
        .expect("an exp-less offline session must resolve (presence-only)");
    assert_eq!(token, "test.session.jwt");
}
