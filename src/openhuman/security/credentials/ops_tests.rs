use super::*;
use crate::openhuman::security::credentials::session_support::local_session_user_id;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::json;
use tempfile::TempDir;
use tokio::net::TcpListener;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, path) };
        Self { key, previous }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    // Storing a session asks the memory driver to re-embed, and resolving a
    // driver that nothing has installed means attempting to load the compiled
    // module — which a unit test cannot do, but takes seconds to fail at.
    // These are credentials tests; binding is not what they are about, and one
    // of them asserts a latency budget that the attempt blows straight through.
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &config.workspace_dir,
        &config.subsystems.memory,
        Default::default(),
        Default::default(),
    );
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

/// A backend that accepts the connection but never answers `/auth/me`, modelling
/// a reachable-but-slow backend whose request hangs far past the store-time
/// validation budget (issue #5166).
async fn spawn_auth_me_hang() -> String {
    let app = Router::new().route(
        "/auth/me",
        get(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            StatusCode::OK
        }),
    );
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

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
