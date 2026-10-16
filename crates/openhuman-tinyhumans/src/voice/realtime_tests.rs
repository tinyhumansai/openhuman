use super::*;
use serde_json::json;

#[test]
fn parses_the_wrapped_success_envelope() {
    let raw = json!({ "success": true, "data": { "signedUrl": "wss://x", "agentId": "a1", "userToken": "tok" } });
    let out = parse_signed_url_response(&raw).unwrap();
    assert_eq!(out.signed_url, "wss://x");
    assert_eq!(out.agent_id, "a1");
    assert_eq!(out.user_token, "tok");
}

#[test]
fn tolerates_a_bare_object() {
    let raw = json!({ "signedUrl": "wss://y", "agentId": "a2" });
    let out = parse_signed_url_response(&raw).unwrap();
    assert_eq!(out.signed_url, "wss://y");
    assert_eq!(out.agent_id, "a2");
}

#[test]
fn user_token_defaults_empty_against_an_older_backend() {
    let raw = json!({ "data": { "signedUrl": "wss://y", "agentId": "a2" } });
    let out = parse_signed_url_response(&raw).unwrap();
    assert_eq!(out.user_token, "");
}

#[test]
fn secure_url_guard_allows_https_and_loopback_http_only() {
    assert!(ensure_secure_backend_url("https://api.tinyhumans.ai").is_ok());
    assert!(ensure_secure_backend_url("http://localhost:5005").is_ok());
    assert!(ensure_secure_backend_url("http://127.0.0.1:5005/").is_ok());
    let err = ensure_secure_backend_url("http://api.tinyhumans.ai").unwrap_err();
    assert!(err.contains("non-HTTPS"), "{err}");
}

#[test]
fn secure_url_guard_allows_ipv6_loopback() {
    // Bracketed IPv6 loopback must be accepted — the previous first-`:`
    // split turned `[::1]:5005` into `"["` and wrongly rejected it.
    assert!(ensure_secure_backend_url("http://[::1]:5005").is_ok());
    assert!(ensure_secure_backend_url("http://[::1]:5005/").is_ok());
    assert!(ensure_secure_backend_url("http://[::1]").is_ok());
    // A non-loopback bracketed IPv6 host is still rejected over http://.
    let err = ensure_secure_backend_url("http://[2001:db8::1]:5005").unwrap_err();
    assert!(err.contains("non-HTTPS"), "{err}");
}

#[test]
fn errors_when_signed_url_is_absent() {
    let raw = json!({ "data": { "agentId": "a3" } });
    let err = parse_signed_url_response(&raw).unwrap_err();
    assert!(err.contains("no signed_url"), "{err}");
}

#[test]
fn result_serializes_snake_case_for_the_wire() {
    let json = serde_json::to_value(VoiceAgentSignedUrl {
        signed_url: "wss://z".into(),
        agent_id: "a4".into(),
        user_token: "tok".into(),
    })
    .unwrap();
    assert_eq!(json.get("signed_url").unwrap(), "wss://z");
    assert_eq!(json.get("agent_id").unwrap(), "a4");
    assert_eq!(json.get("user_token").unwrap(), "tok");
}

/// A runtime holding only a TinyHumans API key (no session) can mint a
/// realtime voice-agent URL; the key rides `x-api-key`, never `Authorization`.
#[tokio::test]
async fn mint_signed_url_authenticates_with_the_api_key() {
    use axum::{http::HeaderMap, routing::get, Json, Router};
    use std::sync::{Arc, Mutex};

    type SeenHeaders = Option<(Option<String>, Option<String>)>;
    let seen: Arc<Mutex<SeenHeaders>> = Arc::default();
    let app = Router::new().route(
        "/voice-agent/get-signed-url",
        get({
            let seen = Arc::clone(&seen);
            move |headers: HeaderMap| async move {
                let header = |name: &str| {
                    headers
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned)
                };
                *seen.lock().unwrap() = Some((header("x-api-key"), header("authorization")));
                Json(json!({
                    "success": true,
                    "data": { "signedUrl": "wss://voice", "agentId": "a1", "userToken": "u" }
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: dir.path().join("workspace"),
        config_path: dir.path().join("config.toml"),
        api_url: Some(format!("http://{addr}")),
        ..Config::default()
    };
    openhuman_embed::__host::security::credentials::api_key::store_api_key(
        &config,
        "tiny_test_voice",
    )
    .unwrap();

    let out = mint_voice_agent_signed_url(&config)
        .await
        .expect("an API key alone mints a voice-agent URL");
    assert_eq!(out.value.signed_url, "wss://voice");
    let (api_key, authorization) = seen.lock().unwrap().clone().expect("request reached mock");
    assert_eq!(api_key.as_deref(), Some("tiny_test_voice"));
    assert_eq!(authorization, None);
}
