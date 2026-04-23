use super::*;
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use axum::{
    extract::Path,
    http::HeaderMap,
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde_json::json;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn store_session_token(config: &Config, token: &str) {
    let service = AuthService::from_config(config);
    service
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            token,
            std::collections::HashMap::new(),
            true,
        )
        .expect("store session token");
}

async fn spawn_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Poll for readiness so the accept loop is live before the
    // first authed HTTP call — same pattern used by composio/ops.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut backoff = std::time::Duration::from_millis(2);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("mock backend at {addr} did not become ready");
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(std::time::Duration::from_millis(50));
    }
    format!("http://127.0.0.1:{}", addr.port())
}

fn config_with_backend(tmp: &TempDir, base: String) -> Config {
    let mut c = test_config(tmp);
    c.api_url = Some(base);
    store_session_token(&c, "test-session-token");
    c
}

// ── require_token ─────────────────────────────────────────────

#[test]
fn require_token_errors_when_no_session_stored() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = require_token(&config).unwrap_err();
    assert!(
        err.contains("no backend session token"),
        "expected 'no backend session token', got: {err}"
    );
}

#[test]
fn require_token_returns_stored_token_trimmed() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    store_session_token(&config, "  tok-123  ");
    let got = require_token(&config).expect("token");
    assert_eq!(got, "tok-123");
}

#[test]
fn require_token_rejects_whitespace_only_stored_token() {
    // A token that exists in the store but is just whitespace must
    // be treated as absent — otherwise downstream HTTP calls would
    // send an empty `Authorization: Bearer` header.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    store_session_token(&config, "   ");
    let err = require_token(&config).unwrap_err();
    assert!(err.contains("no backend session token"));
}

// ── Stub ops (list/clear/register/unregister) ────────────────

#[tokio::test]
async fn list_registrations_returns_empty_payload_with_count_log() {
    let out = list_registrations().await.unwrap();
    assert!(out.value.registrations.is_empty());
    assert!(out.logs.iter().any(|l| l.contains("returned 0")));
}

#[tokio::test]
async fn list_logs_returns_empty_and_ignores_limit() {
    let out = list_logs(Some(50)).await.unwrap();
    assert!(out.value.logs.is_empty());
    assert!(out.logs.iter().any(|l| l.contains("returned 0")));

    // `None` limit path.
    let out2 = list_logs(None).await.unwrap();
    assert!(out2.value.logs.is_empty());
}

#[tokio::test]
async fn clear_logs_reports_zero_cleared() {
    let out = clear_logs().await.unwrap();
    assert_eq!(out.value.cleared, 0);
    assert!(out.logs.iter().any(|l| l.contains("removed 0")));
}

#[tokio::test]
async fn register_echo_surfaces_tunnel_uuid_in_log() {
    let out = register_echo("uuid-1", Some("name".into()), Some("btid-1".into()))
        .await
        .unwrap();
    assert!(out.value.registrations.is_empty());
    assert!(out
        .logs
        .iter()
        .any(|l| l.contains("registered tunnel uuid-1")));
}

#[tokio::test]
async fn unregister_echo_surfaces_tunnel_uuid_in_log() {
    let out = unregister_echo("uuid-1").await.unwrap();
    assert!(out.value.registrations.is_empty());
    assert!(out.logs.iter().any(|l| l.contains("removed tunnel uuid-1")));
}

// ── build_echo_response ───────────────────────────────────────

#[test]
fn build_echo_response_encodes_request_fields_and_sets_headers() {
    let mut query = std::collections::HashMap::new();
    query.insert("q".to_string(), "1".to_string());
    let mut headers = std::collections::HashMap::new();
    headers.insert("X-Foo".to_string(), json!("bar"));
    let req = WebhookRequest {
        correlation_id: "c-1".into(),
        tunnel_id: "tid-1".into(),
        tunnel_uuid: "uuid-1".into(),
        tunnel_name: "hook".into(),
        method: "POST".into(),
        path: "/p".into(),
        headers,
        query,
        body: "cGF5bG9hZA==".into(), // base64 of "payload"
    };
    let resp = build_echo_response(&req);

    assert_eq!(resp.correlation_id, "c-1");
    assert_eq!(resp.status_code, 200);
    assert_eq!(
        resp.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        resp.headers
            .get("x-openhuman-webhook-target")
            .map(String::as_str),
        Some("echo")
    );
    // Decode the body and check the echoed fields survived the round-trip.
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(resp.body.as_bytes())
        .expect("base64 body");
    let v: serde_json::Value = serde_json::from_slice(&decoded).expect("json body");
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["echo"]["correlationId"], json!("c-1"));
    assert_eq!(v["echo"]["method"], json!("POST"));
    assert_eq!(v["echo"]["path"], json!("/p"));
    assert_eq!(v["echo"]["bodyBase64"], json!("cGF5bG9hZA=="));
}

// ── Validation on trimmed inputs ──────────────────────────────

#[tokio::test]
async fn create_tunnel_rejects_empty_or_whitespace_name() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    for name in ["", "   "] {
        let err = create_tunnel(&config, name, None).await.unwrap_err();
        assert!(
            err.contains("name is required"),
            "expected 'name is required' for `{name:?}`, got: {err}"
        );
    }
}

#[tokio::test]
async fn id_bearing_tunnel_ops_reject_empty_or_whitespace_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    for id in ["", "   "] {
        assert!(get_tunnel(&config, id)
            .await
            .unwrap_err()
            .contains("id is required"));
        assert!(delete_tunnel(&config, id)
            .await
            .unwrap_err()
            .contains("id is required"));
        assert!(update_tunnel(&config, id, json!({}))
            .await
            .unwrap_err()
            .contains("id is required"));
    }
}

// ── Authed HTTP round-trips via a mock backend ───────────────

#[tokio::test]
async fn list_tunnels_hits_webhooks_core_endpoint_and_returns_payload() {
    // Inspect the inbound Authorization header so we catch regressions
    // where the JWT stops being forwarded (or is sent with the wrong
    // scheme). `config_with_backend` stores `test-session-token`, so
    // the header must be `Bearer test-session-token`.
    let app = Router::new().route(
        "/webhooks/core",
        get(|headers: HeaderMap| async move {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            assert_eq!(
                auth, "Bearer test-session-token",
                "authorization header must forward the stored session token"
            );
            Json(json!({"tunnels": [{"id": "t-1"}]}))
        }),
    );
    let base = spawn_mock_backend(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);
    let out = list_tunnels(&config).await.unwrap();
    assert_eq!(out.value["tunnels"][0]["id"], json!("t-1"));
    assert!(out
        .logs
        .iter()
        .any(|l| l.contains("webhook tunnels fetched")));
}

#[tokio::test]
async fn create_tunnel_posts_name_and_optional_description() {
    let app = Router::new().route(
        "/webhooks/core",
        post(|Json(body): Json<serde_json::Value>| async move {
            // Echo back the received body so the test can verify
            // trimming and optional-description handling.
            Json(json!({ "echoed": body }))
        }),
    );
    let base = spawn_mock_backend(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);

    // Description with surrounding whitespace must be trimmed into
    // the outgoing payload; empty description must be dropped.
    let out = create_tunnel(&config, "  my-hook  ", Some("  desc  ".into()))
        .await
        .unwrap();
    assert_eq!(out.value["echoed"]["name"], json!("my-hook"));
    assert_eq!(out.value["echoed"]["description"], json!("desc"));

    let out2 = create_tunnel(&config, "nodesc", Some("   ".into()))
        .await
        .unwrap();
    assert_eq!(out2.value["echoed"]["name"], json!("nodesc"));
    assert!(
        out2.value["echoed"].get("description").is_none(),
        "whitespace-only description must not be forwarded"
    );
}

#[tokio::test]
async fn get_tunnel_encodes_id_in_path() {
    // Use an id full of reserved URL characters so we actually verify
    // percent-encoding on the outbound path. axum's `Path` extractor
    // decodes before handing us the string, so the server must see
    // the trimmed, *decoded* form of the id.
    let app = Router::new().route(
        "/webhooks/core/{id}",
        get(|Path(id): Path<String>| async move { Json(json!({ "id": id })) }),
    );
    let base = spawn_mock_backend(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);
    let raw_id = "  abc:/?#[ ]@!$&'()*+,;=%  ";
    let trimmed = raw_id.trim();
    let out = get_tunnel(&config, raw_id).await.unwrap();
    assert_eq!(
        out.value["id"],
        json!(trimmed),
        "server should receive the trimmed, decoded id — proves the client \
         percent-encoded reserved chars instead of sending them raw"
    );
}

#[tokio::test]
async fn update_tunnel_patches_id_with_body() {
    let app = Router::new().route(
        "/webhooks/core/{id}",
        patch(
            |Path(id): Path<String>, Json(body): Json<serde_json::Value>| async move {
                Json(json!({ "id": id, "patched": body }))
            },
        ),
    );
    let base = spawn_mock_backend(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);
    let out = update_tunnel(&config, "t-1", json!({"name":"renamed","isActive":true}))
        .await
        .unwrap();
    assert_eq!(out.value["id"], json!("t-1"));
    assert_eq!(out.value["patched"]["name"], json!("renamed"));
    assert_eq!(out.value["patched"]["isActive"], json!(true));
}

#[tokio::test]
async fn delete_tunnel_deletes_by_id() {
    let app = Router::new().route(
        "/webhooks/core/{id}",
        delete(|Path(id): Path<String>| async move { Json(json!({"deleted": id})) }),
    );
    let base = spawn_mock_backend(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);
    let out = delete_tunnel(&config, "t-42").await.unwrap();
    assert_eq!(out.value["deleted"], json!("t-42"));
}

#[tokio::test]
async fn get_bandwidth_fetches_the_bandwidth_endpoint() {
    let app = Router::new().route(
        "/webhooks/core/bandwidth",
        get(|| async { Json(json!({"remaining": 1024})) }),
    );
    let base = spawn_mock_backend(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);
    let out = get_bandwidth(&config).await.unwrap();
    assert_eq!(out.value["remaining"], json!(1024));
}

#[tokio::test]
async fn authed_http_calls_surface_require_token_error_without_session() {
    // No token stored → all authed endpoints should error with the
    // shared "no backend session token" message before any network
    // call is made.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(list_tunnels(&config)
        .await
        .unwrap_err()
        .contains("no backend session token"));
    assert!(get_bandwidth(&config)
        .await
        .unwrap_err()
        .contains("no backend session token"));
}
