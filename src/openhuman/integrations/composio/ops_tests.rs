use super::*;
use crate::openhuman::agent::context::prompt::IntegrationConnection;

use crate::openhuman::integrations::composio::module_client::module_guard;

// ── resolve_client / ops auth errors ──────────────────────────

fn test_config(tmp: &tempfile::TempDir) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c
}

// ── cache_key / invalidate_connected_integrations_cache ───────

/// Per-module alias so call sites don't need to spell out the path.
/// The actual lock lives in `connected_integrations` so it is shared
/// with `tools_tests` and any other test module that touches the cache.
fn cache_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::openhuman::integrations::composio::connected_integrations::composio_cache_test_lock()
}

// ── Mock-backend integration tests for ops ─────────────────────

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use chrono::{TimeZone, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;
use tinymemory_api::chunks::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};

struct WorkspaceEnvGuard {
    previous: Option<std::ffi::OsString>,
}

impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
        Self { previous }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(prev) => unsafe {
                std::env::set_var("OPENHUMAN_WORKSPACE", prev);
            },
            None => unsafe {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            },
        }
    }
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(prev) => unsafe {
                std::env::set_var(self.key, prev);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

struct DirectAuthFailureGuard {
    key_id: u64,
}

impl DirectAuthFailureGuard {
    fn new(api_key: &str) -> Self {
        let key_id =
            crate::openhuman::integrations::composio::direct_auth::fingerprint_api_key(api_key);
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

async fn start_mock_backend(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Wait until the axum accept loop is actually serving — not just
    // until the kernel-level TCP socket is bound. Without this, fast
    // tests can fire a request before `axum::serve` starts polling and
    // occasionally see connection resets / hangs on loaded CI.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut backoff = std::time::Duration::from_millis(2);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("mock backend at {addr} did not become ready in time");
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(std::time::Duration::from_millis(50));
    }

    format!("http://127.0.0.1:{}", addr.port())
}

fn config_with_backend(tmp: &tempfile::TempDir, base: String) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c.api_url = Some(base);
    crate::openhuman::security::credentials::AuthService::from_config(&c)
        .store_provider_token(
            crate::openhuman::security::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    c
}

fn sample_memory_chunk(source_kind: SourceKind, source_id: &str, seq: u32) -> Chunk {
    sample_memory_chunk_with_owner(source_kind, source_id, "alice@example.com", seq)
}

fn sample_memory_chunk_with_owner(
    source_kind: SourceKind,
    source_id: &str,
    owner: &str,
    seq: u32,
) -> Chunk {
    let ts = Utc
        .timestamp_millis_opt(1_700_000_000_000 + i64::from(seq))
        .unwrap();
    let content = format!("composio memory {source_id} {owner} {seq}");
    Chunk {
        id: chunk_id(source_kind, source_id, seq, &content),
        content,
        metadata: Metadata {
            source_kind,
            source_id: source_id.to_string(),
            owner: owner.to_string(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec!["composio".to_string()],
            source_ref: Some(SourceRef::new(format!("composio://{source_id}/{seq}"))),
            path_scope: None,
        },
        token_count: 12,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    }
}

// ── Windows-observed sync regression coverage (issue #749) ────
//
// These tests exercise the cross-platform defenses layered on top
// of the `ComposioConnectionCreated` → `wait_for_connection_active`
// event-bus invalidation path — which can miss on Windows when the
// OAuth handoff outruns the 60 s readiness poll. They use the ops
// helpers directly (no mock backend needed) so they're deterministic
// and don't depend on the tokio runtime's scheduling.
//
// Every test uses a unique cache key (a unique &str literal) and
// clears only *its* key before seeding, so they can safely run in
// parallel with each other and with any other test in the binary
// that mutates `INTEGRATIONS_CACHE` (e.g. the mock-backend tests
// above call `invalidate_connected_integrations_cache()`, which
// would otherwise wipe our seeded state mid-run).

/// Remove just the test's own cache entry. Preferred over
/// [`invalidate_connected_integrations_cache`] inside these tests
/// because it can't be clobbered by — nor clobber — parallel tests
/// that also touch the global cache.
fn clear_cache_key(key: &str) {
    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        guard.remove(key);
    }
}

/// Seed the process-wide cache with `integrations` keyed by `key`
/// and an `Instant::now()` timestamp. Used by tests that want to
/// drive cache behaviour without going through a backend fetch.
fn seed_cache(key: &str, integrations: Vec<ConnectedIntegration>) {
    let mut guard = INTEGRATIONS_CACHE.write().unwrap();
    guard.insert(
        key.to_string(),
        CachedIntegrations {
            entries: integrations,
            cached_at: Instant::now(),
        },
    );
}

/// Build a minimal `ConnectedIntegration` for cache-seeding tests.
/// Only `toolkit` + `connected` matter for diff-based invalidation.
fn integration(toolkit: &str, connected: bool) -> ConnectedIntegration {
    ConnectedIntegration {
        toolkit: toolkit.to_string(),
        description: String::new(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected,
        connections: if connected {
            vec![IntegrationConnection {
                connection_id: format!("c-1"),
                label: None,
                is_default: true,
            }]
        } else {
            Vec::new()
        },
        non_active_status: None,
    }
}

/// Build a minimal backend connection row for
/// `sync_cache_with_connections` tests.
fn conn(id: &str, toolkit: &str, status: &str) -> super::super::types::ComposioConnection {
    // The real type has a handful of optional metadata fields we
    // don't care about here — construct via serde so the test
    // stays decoupled from struct-field churn.
    serde_json::from_value(json!({
        "id": id,
        "toolkit": toolkit,
        "status": status,
    }))
    .expect("deserialize test ComposioConnection")
}

// ── Direct-mode list_* short-circuits ─────────────────────────────
//
// [composio-direct] When `config.composio.mode == "direct"`, the
// `composio_list_toolkits` / `composio_list_connections` ops must NOT
// silently fall through to the backend tenant's data — that's the
// bug the user reported in #1710 (toggled to Direct, still saw
// tinyhumans-tenant connections). We return empty responses with
// explicit log lines so the UI / agent surface stays honest about
// where the data is (or isn't) coming from.

/// Set up a config with `composio.mode = "direct"` and a stored
/// direct-mode API key (so `create_composio_client` succeeds).
fn direct_mode_config(tmp: &tempfile::TempDir) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.into();
    crate::openhuman::security::credentials::AuthService::from_config(&c)
        .store_provider_token(
            crate::openhuman::security::credentials::ops::COMPOSIO_DIRECT_PROVIDER,
            crate::openhuman::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "ck_test_direct_key",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test direct-mode api key");
    c
}

// ── Direct mode with no API key yet (Sentry TAURI-RUST-R4) ────────
//
// Direct mode selected but no key configured is a valid user *setup*
// state, not an operation failure. `composio_list_connections` must
// return an empty list (no key → no tenant → no connections) instead of
// erroring, so the desktop UI's 5 s poll stops funnelling the factory's
// "no api key is configured" error to Sentry on every tick.

/// Direct mode selected, but NO key in the keychain and none in
/// `config.toml`. The mode-aware factory would bail here with
/// "composio direct mode selected but no api key is configured".
fn direct_mode_no_key_config(tmp: &tempfile::TempDir) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.into();
    c
}

// ── enrich_connections_with_identity ──────────────────────────────────
//
// `enrich_connections_with_identity` reads through the bound memory driver
// now (`identity_store::load_connected_identities`) rather than a
// process-global engine client, so its tests bind a driver per test with
// `memory::test_support::install_memory_driver_for_test` instead of the
// `tinymemory_core::global::init` helper this file used to carry.

fn make_connections_response(
    conns: &[(&str, &str, &str)],
) -> super::super::types::ComposioConnectionsResponse {
    let connections = conns
        .iter()
        .map(|(id, toolkit, status)| conn(id, toolkit, status))
        .collect();
    super::super::types::ComposioConnectionsResponse { connections }
}

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "ops_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "ops_tests_part_04_tests.rs"]
mod part_04_tests;
