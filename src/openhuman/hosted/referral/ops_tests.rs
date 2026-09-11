use super::*;
use crate::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use axum::{
    routing::{get, post},
    Json, Router,
};
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn store_session_token(config: &Config, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            token,
            std::collections::HashMap::new(),
            true,
        )
        .expect("store token");
}

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
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

// ── require_token (private helper) ────────────────────────────

#[test]
fn require_token_errors_without_stored_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = require_token(&config).unwrap_err();
    assert!(err.contains("no backend session token"));
}

#[test]
fn require_token_trims_stored_value() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    store_session_token(&config, "  tok  ");
    assert_eq!(require_token(&config).unwrap(), "tok");
}

#[test]
fn require_token_rejects_whitespace_only_stored_token() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    store_session_token(&config, "   ");
    assert!(require_token(&config)
        .unwrap_err()
        .contains("no backend session token"));
}

// ── get_stats ────────────────────────────────────────────────

#[tokio::test]
async fn get_stats_errors_without_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = get_stats(&config).await.unwrap_err();
    assert!(err.contains("no backend session token"));
}

#[tokio::test]
async fn get_stats_returns_backend_payload_with_log() {
    let app = Router::new().route(
        "/referral/stats",
        get(|| async { Json(json!({"referrals": 3, "earned_cents": 1500})) }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);
    let out = get_stats(&config).await.unwrap();
    assert_eq!(out.value["referrals"], json!(3));
    assert!(out
        .logs
        .iter()
        .any(|l| l.contains("referral stats fetched")));
}

// ── claim_referral ───────────────────────────────────────────

#[tokio::test]
async fn claim_referral_errors_without_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = claim_referral(&config, "ABC", None).await.unwrap_err();
    assert!(err.contains("no backend session token"));
}

#[tokio::test]
async fn claim_referral_posts_trimmed_code_and_drops_whitespace_fingerprint() {
    let app = Router::new().route(
        "/referral/claim",
        post(|Json(body): Json<Value>| async move { Json(json!({ "echoed": body })) }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);

    // Code is trimmed; whitespace-only fingerprint must be dropped.
    let out = claim_referral(&config, "  ABC-123  ", Some("   "))
        .await
        .unwrap();
    assert_eq!(out.value["echoed"]["code"], json!("ABC-123"));
    assert!(
        out.value["echoed"].get("deviceFingerprint").is_none(),
        "whitespace-only fingerprint must be dropped"
    );
    assert!(out
        .logs
        .iter()
        .any(|l| l.contains("referral claim accepted")));
}

#[tokio::test]
async fn claim_referral_forwards_non_empty_device_fingerprint_trimmed() {
    let app = Router::new().route(
        "/referral/claim",
        post(|Json(body): Json<Value>| async move { Json(json!({ "echoed": body })) }),
    );
    let base = spawn_mock(app).await;
    let tmp = TempDir::new().unwrap();
    let config = config_with_backend(&tmp, base);
    let out = claim_referral(&config, "CODE", Some("  fp-1  "))
        .await
        .unwrap();
    assert_eq!(out.value["echoed"]["deviceFingerprint"], json!("fp-1"));
}
