use super::*;
use crate::openhuman::config::Config;

// ── Mock-backend integration tests ─────────────────────────────
//
// These stand up a real axum HTTP server on a random localhost port,
// point a `ComposioClient` at it, and drive each method end-to-end.
// That exercises the envelope parsing, HTTP plumbing, and URL
// construction in `ComposioClient` — which is otherwise only covered
// by live backend tests.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn build_client_for(base_url: String) -> ComposioClient {
    let inner = Arc::new(crate::openhuman::integrations::IntegrationClient::new(
        base_url,
        "test-token".into(),
    ));
    ComposioClient::new(inner)
}
// Calendar bare-date → RFC 3339 normalization is now covered by
// `execute_prepare::prepare_execute_arguments` (PR #1827); see
// `execute_prepare_tests.rs` for the equivalent test surface that
// supersedes the per-slug `normalize_calendar_query_args` helper
// removed alongside the upstream-main merge.

// ── Factory tests (`create_composio_client`) ────────────────────────
//
// Mirror the four branches the spec demands:
//   1. backend mode with a session JWT — Backend variant
//   2. direct mode + stored api key — Direct variant
//   3. direct mode without api key — explicit error
//   4. unknown mode string — explicit error

fn config_with_session_token(tmp: &tempfile::TempDir) -> crate::openhuman::config::Config {
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    crate::openhuman::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::openhuman::security::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    config
}

// ── Pricing short-circuit ───────────────────────────────────────────

// ── Direct-mode reshapers (`direct_authorize` / `direct_execute` / ─
//   `direct_list_connections` / `direct_list_tools`)
//
// These helpers wrap a `ComposioTool` and reshape v3 responses into
// the backend-proxied envelope types. Most still assert the
// empty/invalid-input paths that don't require HTTP:
//
//   * `direct_authorize` rejects an empty toolkit before any network
//     hit, with an explicit error so the caller can surface it as a
//     400-class user error.
//   * `direct_execute` accepts a None-arguments call and falls
//     through to the underlying tool surface (which then errors on the
//     network call — covered by the integration test in `ops_tests.rs`).
//   * `direct_list_connections` is a thin mapper; the real coverage
//     for its row → ComposioConnection translation lives in the
//     `connected_account_*` tests in `composio_tests.rs`.
//
// `direct_list_tools` IS exercised over HTTP: `ComposioTool::new_with_v3_base`
// points its `/tools` GET at a local axum mock, so we can assert both the
// outbound `tags` filter (repeated query params) and the v3 → backend-envelope
// reshape without touching `backend.composio.dev`.

fn direct_tool_for_test() -> std::sync::Arc<crate::openhuman::tools::ComposioTool> {
    std::sync::Arc::new(crate::openhuman::tools::ComposioTool::new(
        "ck_test_direct",
        Some("default"),
        std::sync::Arc::new(crate::openhuman::security::SecurityPolicy::default()),
    ))
}

/// Like [`direct_tool_for_test`] but with the v3 base pointed at a local
/// mock server so HTTP paths (e.g. `direct_list_tools`) can be asserted.
fn direct_tool_for_mock(base_v3: String) -> std::sync::Arc<crate::openhuman::tools::ComposioTool> {
    direct_tool_for_mock_with_key(base_v3, "ck_test_direct")
}

fn direct_tool_for_mock_with_key(
    base_v3: String,
    api_key: &str,
) -> std::sync::Arc<crate::openhuman::tools::ComposioTool> {
    std::sync::Arc::new(crate::openhuman::tools::ComposioTool::new_with_v3_base(
        api_key,
        Some("default"),
        std::sync::Arc::new(crate::openhuman::security::SecurityPolicy::default()),
        base_v3,
    ))
}

struct DirectAuthFailureGuard {
    key_id: u64,
}

impl DirectAuthFailureGuard {
    fn for_tool(tool: &std::sync::Arc<crate::openhuman::tools::ComposioTool>) -> Self {
        let key_id = tool.auth_key_fingerprint();
        crate::openhuman::integrations::composio::direct_auth::reset_direct_auth_failure(key_id);
        Self { key_id }
    }
}

impl Drop for DirectAuthFailureGuard {
    fn drop(&mut self) {
        crate::openhuman::integrations::composio::direct_auth::reset_direct_auth_failure(
            self.key_id,
        );
    }
}

#[path = "client_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "client_tests_part_02_tests.rs"]
mod part_02_tests;
