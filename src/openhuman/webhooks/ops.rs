use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::openhuman::webhooks::{
    WebhookDebugLogListResult, WebhookDebugLogsClearedResult, WebhookDebugRegistrationsResult,
    WebhookRequest, WebhookResponseData,
};
use crate::rpc::RpcOutcome;
use base64::Engine;
use reqwest::Method;
use serde_json::Value;
use std::collections::HashMap;

fn require_token(config: &Config) -> Result<String, String> {
    get_session_token(config)?
        .and_then(|v| {
            let t = v.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        })
        .ok_or_else(|| "no backend session token; run auth_store_session first".to_string())
}

async fn get_authed_value(
    config: &Config,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let token = require_token(config)?;
    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    client
        .authed_json(&token, method, path, body)
        .await
        .map_err(|e| e.to_string())
}

pub async fn list_registrations() -> Result<RpcOutcome<WebhookDebugRegistrationsResult>, String> {
    let registrations = Vec::new();
    let count = 0usize;

    Ok(RpcOutcome::single_log(
        WebhookDebugRegistrationsResult { registrations },
        format!("webhooks.list_registrations returned {count} registration(s)"),
    ))
}

pub async fn list_logs(
    limit: Option<usize>,
) -> Result<RpcOutcome<WebhookDebugLogListResult>, String> {
    let _ = limit;
    let logs = Vec::new();
    let count = 0usize;

    Ok(RpcOutcome::single_log(
        WebhookDebugLogListResult { logs },
        format!("webhooks.list_logs returned {count} log entrie(s)"),
    ))
}

pub async fn clear_logs() -> Result<RpcOutcome<WebhookDebugLogsClearedResult>, String> {
    let cleared = 0usize;

    Ok(RpcOutcome::single_log(
        WebhookDebugLogsClearedResult { cleared },
        format!("webhooks.clear_logs removed {cleared} log entrie(s)"),
    ))
}

pub async fn register_echo(
    tunnel_uuid: &str,
    tunnel_name: Option<String>,
    backend_tunnel_id: Option<String>,
) -> Result<RpcOutcome<WebhookDebugRegistrationsResult>, String> {
    let _ = (tunnel_name, backend_tunnel_id);
    let registrations = Vec::new();

    Ok(RpcOutcome::single_log(
        WebhookDebugRegistrationsResult { registrations },
        format!("webhooks.register_echo registered tunnel {tunnel_uuid}"),
    ))
}

pub async fn unregister_echo(
    tunnel_uuid: &str,
) -> Result<RpcOutcome<WebhookDebugRegistrationsResult>, String> {
    let registrations = Vec::new();

    Ok(RpcOutcome::single_log(
        WebhookDebugRegistrationsResult { registrations },
        format!("webhooks.unregister_echo removed tunnel {tunnel_uuid}"),
    ))
}

pub fn build_echo_response(request: &WebhookRequest) -> WebhookResponseData {
    let response_body = serde_json::json!({
        "ok": true,
        "echo": {
            "correlationId": request.correlation_id,
            "tunnelId": request.tunnel_id,
            "tunnelUuid": request.tunnel_uuid,
            "tunnelName": request.tunnel_name,
            "method": request.method,
            "path": request.path,
            "query": request.query,
            "headers": request.headers,
            "bodyBase64": request.body,
        }
    });

    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    headers.insert("x-openhuman-webhook-target".to_string(), "echo".to_string());

    WebhookResponseData {
        correlation_id: request.correlation_id.clone(),
        status_code: 200,
        headers,
        body: base64::engine::general_purpose::STANDARD.encode(response_body.to_string()),
    }
}

pub async fn list_tunnels(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/webhooks/core", None).await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnels fetched"))
}

pub async fn create_tunnel(
    config: &Config,
    name: &str,
    description: Option<String>,
) -> Result<RpcOutcome<Value>, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is required".to_string());
    }
    let mut body_map = serde_json::Map::new();
    body_map.insert(
        "name".to_string(),
        serde_json::Value::String(name.to_string()),
    );
    if let Some(desc) = description {
        let desc = desc.trim().to_string();
        if !desc.is_empty() {
            body_map.insert("description".to_string(), serde_json::Value::String(desc));
        }
    }
    let body = serde_json::Value::Object(body_map);
    let data = get_authed_value(config, Method::POST, "/webhooks/core", Some(body)).await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel created"))
}

pub async fn get_tunnel(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }
    let encoded_id = urlencoding::encode(id);
    let data = get_authed_value(
        config,
        Method::GET,
        &format!("/webhooks/core/{encoded_id}"),
        None,
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel fetched"))
}

pub async fn update_tunnel(
    config: &Config,
    id: &str,
    payload: Value,
) -> Result<RpcOutcome<Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }
    let encoded_id = urlencoding::encode(id);
    let data = get_authed_value(
        config,
        Method::PATCH,
        &format!("/webhooks/core/{encoded_id}"),
        Some(payload),
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel updated"))
}

pub async fn delete_tunnel(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("id is required".to_string());
    }
    let encoded_id = urlencoding::encode(id);
    let data = get_authed_value(
        config,
        Method::DELETE,
        &format!("/webhooks/core/{encoded_id}"),
        None,
    )
    .await?;
    Ok(RpcOutcome::single_log(data, "webhook tunnel deleted"))
}

pub async fn get_bandwidth(config: &Config) -> Result<RpcOutcome<Value>, String> {
    let data = get_authed_value(config, Method::GET, "/webhooks/core/bandwidth", None).await?;
    Ok(RpcOutcome::single_log(data, "webhook bandwidth fetched"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::credentials::{
        AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
    };
    use axum::{
        extract::Path,
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
        assert!(out.logs.iter().any(|l| l.contains("registered tunnel uuid-1")));
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
            resp.headers.get("x-openhuman-webhook-target").map(String::as_str),
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
            assert!(get_tunnel(&config, id).await.unwrap_err().contains("id is required"));
            assert!(delete_tunnel(&config, id).await.unwrap_err().contains("id is required"));
            assert!(update_tunnel(&config, id, json!({}))
                .await
                .unwrap_err()
                .contains("id is required"));
        }
    }

    // ── Authed HTTP round-trips via a mock backend ───────────────

    #[tokio::test]
    async fn list_tunnels_hits_webhooks_core_endpoint_and_returns_payload() {
        let app = Router::new().route(
            "/webhooks/core",
            get(|| async { Json(json!({"tunnels": [{"id": "t-1"}]})) }),
        );
        let base = spawn_mock_backend(app).await;
        let tmp = TempDir::new().unwrap();
        let config = config_with_backend(&tmp, base);
        let out = list_tunnels(&config).await.unwrap();
        assert_eq!(out.value["tunnels"][0]["id"], json!("t-1"));
        assert!(out.logs.iter().any(|l| l.contains("webhook tunnels fetched")));
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
        let app = Router::new().route(
            "/webhooks/core/{id}",
            get(|Path(id): Path<String>| async move { Json(json!({ "id": id })) }),
        );
        let base = spawn_mock_backend(app).await;
        let tmp = TempDir::new().unwrap();
        let config = config_with_backend(&tmp, base);
        let out = get_tunnel(&config, "  abc-123  ").await.unwrap();
        // Server should see the trimmed id.
        assert_eq!(out.value["id"], json!("abc-123"));
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
}
