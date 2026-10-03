use super::*;
use crate::config::test_env::EnvVarGuard;
use crate::config::{default_root_openhuman_dir, user_openhuman_dir, write_active_user_id, Config};
use crate::security::credentials::session_support::local_session_user_id;
use crate::security::credentials::{
    identity, session_support, AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::json;
use tempfile::TempDir;
use tokio::net::TcpListener;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    config
}

fn jwt_with_payload(payload: serde_json::Value) -> String {
    let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
    format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig")
}

async fn spawn_auth_me_status(status: StatusCode) -> String {
    let app = Router::new().route("/auth/me", get(move || async move { status }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// Persist a live (unexpired) app-session profile for `user_id` and return the
/// user-scoped `Config` that reads it back, mirroring the on-disk state of an
/// already-signed-in install.
///
/// Written directly through `AuthService` rather than via `store_session` so the
/// caller's mock backend only has to answer the route under test — `store_session`
/// would additionally need `/auth/me` to succeed first, and re-scopes the profile
/// to the resolved user directory as a side effect.
fn store_live_session(user_id: &str) -> Config {
    let root_dir = default_root_openhuman_dir().unwrap();
    write_active_user_id(&root_dir, user_id).unwrap();
    let user_dir = user_openhuman_dir(&root_dir, user_id);
    std::fs::create_dir_all(user_dir.join("workspace")).unwrap();
    let config = Config {
        config_path: user_dir.join("config.toml"),
        workspace_dir: user_dir.join("workspace"),
        action_dir: user_dir.join("workspace"),
        ..Config::default()
    };
    let token = jwt_with_payload(json!({
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()
    }));
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("user_id".to_string(), user_id.to_string());
    metadata.insert(
        "user_json".to_string(),
        json!({ "id": user_id }).to_string(),
    );
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            &token,
            metadata,
            true,
        )
        .unwrap();
    config
}

#[path = "ops_credential_tests.rs"]
mod credential_tests;
#[path = "ops_provider_oauth_tests.rs"]
mod provider_oauth_tests;
