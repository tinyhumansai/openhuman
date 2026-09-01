//! Round 21 focused raw coverage for memory_sync + memory_tree gaps.
//!
//! Hermetic: temp workspaces, loopback Composio backend where still
//! applicable, and no real network. Run with `--test-threads=1` because
//! config/HOME/workspace env vars and the global memory client are
//! process-global.
//!
//! # What changed here
//!
//! tinymemory v1.13.4 deleted the ENTIRE in-process Composio provider
//! registry (`ComposioProvider` trait, `ProviderContext`, the concrete
//! per-toolkit provider structs including `GmailProvider` and
//! `LinearProvider`, `register_provider`/`init_default_providers`) — see
//! `crate::openhuman::integrations::composio::providers`'s module docs for
//! the full account. None of it has a replacement in this crate: it now
//! lives in the separately-versioned `tinyconnectors` module, reachable only
//! via a live loaded module over the bus (a real network download plus a
//! `dlopen`), which this file's own "no real network" design rules out.
//!
//! `gmail_post_process_slims_wrapped_messages_and_honours_raw_flag` and the
//! provider-internals half of
//! `linear_provider_profile_tasks_sync_and_periodic_bookkeeping_use_loopback`
//! (constructing a `GmailProvider`/`LinearProvider` directly and driving it
//! against a loopback Composio execute API) tested exactly that deleted,
//! relocated capability — Gmail's nested-payload post-processing, Linear's
//! profile fetch, task normalization, and cursor-paginated sync. There is
//! nothing left in this crate to assert that behaviour against; it is
//! reported as a coverage gap rather than silently dropped. What replaces
//! them below is the current, real, network-free entry point that stands in
//! its place: `integrations::composio::ops::{composio_get_user_profile,
//! composio_sync}`, which — with `modules.enabled = false` — refuse cleanly
//! and deterministically instead of reaching a provider.
//! `integrations::composio::periodic::record_sync_success` (moved from
//! `memory::sync::composio::periodic`, which no longer exists) is still real
//! and is still exercised directly.
//!
//! `slack_sync_status_rpc_reads_mock_connections_and_persisted_state` is
//! rewritten rather than deleted, onto a genuinely different current
//! behaviour: `providers::slack::rpc::sync_status_rpc`'s own doc comment
//! (`src/openhuman/memory/sync/composio/providers/slack/rpc.rs`) explains
//! that it is now a **deliberately degraded read** — the connector module
//! keeps its cursor and daily-request budget internally and exposes neither
//! outside of an actual `Sync` call, so every per-connection detail field
//! (`per_channel_cursors`, `synced_ids_count`, `requests_used_today`,
//! `daily_request_limit`) is hardcoded to its zero value rather than read
//! back from anywhere. Persisting a `SyncState` before calling it (the old
//! test's approach) no longer has any effect on the response, because
//! nothing reads it — so this test now asserts the zero-value degraded shape
//! and the log line that explains it, matching the RPC's own documented
//! contract instead of a behaviour it no longer has.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use axum::routing::get;
use axum::{Json, Router};
use chrono::{TimeZone, Utc};
use serde_json::json;
use tempfile::TempDir;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::integrations::composio::ops::{
    composio_get_user_profile, composio_sync,
};
use openhuman_core::openhuman::integrations::composio::periodic::record_sync_success;
use openhuman_core::openhuman::memory::sync::composio::providers::slack::rpc::{
    sync_status_rpc, SyncStatusRequest,
};
use openhuman_core::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use tinycortex::memory::score::embed::{pack_embedding, EMBEDDING_DIM};
use tinymemory_core::global as memory_global;
use tinymemory_core::store::chunks::store::with_connection;
use tinymemory_core::store::content::atomic::stage_summary;
use tinymemory_core::store::content::{SummaryComposeInput, SummaryTreeKind};
use tinymemory_core::store::trees::types::{SummaryNode, Tree, TreeKind};
use tinymemory_core::tree::retrieval::source::query_source;
// Engine-direct for the same reason as the other retrieval e2e suites (#5560).
use tinymemory_core::tree::tree::store as tree_store;
use tinymemory_core::tree::tree::TreeStatus;

static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;
static MEMORY_SEAMS_INIT: OnceLock<()> = OnceLock::new();

fn ensure_memory_seams() {
    MEMORY_SEAMS_INIT.get_or_init(|| {
        std::thread::Builder::new()
            .name("memory-sync-tree-round21-raw-coverage-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                openhuman_core::openhuman::memory::host_impls::install_memory_host_seams(Arc::new(
                    Config::default(),
                ));
            })
            .expect("spawn round21 memory tree seam installer")
            .join()
            .expect("round21 memory tree seam installer panicked");
    });
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct EnvGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: impl AsRef<Path>) -> Self {
        let old = std::env::var_os(key);
        unsafe { std::env::set_var(key, value.as_ref()) };
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var_os(key);
        unsafe { std::env::remove_var(key) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.old {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn config_in(tmp: &TempDir) -> Config {
    ensure_memory_seams();
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.memory_tree.embedding_endpoint = None;
    config.memory_tree.embedding_model = None;
    config.memory_tree.embedding_strict = false;
    config
}

async fn persist_config(config: &Config) {
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
    config.save().await.expect("save config");
}

fn store_session(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "round21-session-token",
            HashMap::new(),
            true,
        )
        .expect("store app session token");
}

async fn loopback_router(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("loopback addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve loopback");
    });
    (format!("http://{addr}"), handle)
}

/// What `gmail_post_process_slims_wrapped_messages_and_honours_raw_flag` and
/// the profile/tasks half of
/// `linear_provider_profile_tasks_sync_and_periodic_bookkeeping_use_loopback`
/// used to cover — see the module doc comment for why that coverage cannot
/// be expressed here any more. `composio_get_user_profile` is the real,
/// current entry point standing in its place for "fetch a provider's
/// profile", and it refuses cleanly and deterministically — no network, no
/// provider — when no connectors module is loaded.
#[tokio::test]
async fn composio_get_user_profile_refuses_cleanly_for_gmail_and_linear_without_a_loaded_module() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");

    let mut config = config_in(&tmp);
    config.modules.enabled = false;
    persist_config(&config).await;
    store_session(&config);

    for connection_id in ["conn-gmail-round21", "conn-linear-round21"] {
        let error = composio_get_user_profile(&config, connection_id)
            .await
            .expect_err("profile fetch must refuse without a loaded connectors module");
        assert!(
            error.contains("modules are disabled in configuration"),
            "unexpected error for {connection_id}: {error}"
        );
    }

    // `composio_sync` — the entry point standing in for the deleted
    // `LinearProvider::sync` / periodic bookkeeping loop — refuses the same
    // way: the toolkit resolution it needs is itself module-mediated.
    let sync_error = composio_sync(&config, "conn-linear-round21", Some("manual".to_string()))
        .await
        .expect_err("sync must refuse without a loaded connectors module");
    assert!(
        sync_error.contains("modules are disabled in configuration"),
        "unexpected sync error: {sync_error}"
    );

    // `record_sync_success` (moved from the deleted `memory::sync::composio::
    // periodic` to `integrations::composio::periodic`) is untouched by the
    // deletion — it is a pure process-local bookkeeping call the periodic
    // scheduler uses to avoid immediately re-firing a sync it just ran. It
    // exposes no public reader, so this — like the original test — only
    // proves it is callable and does not panic.
    record_sync_success("linear", "conn-linear-round21");
    record_sync_success("linear", "conn-linear-round21");
}

#[tokio::test]
async fn slack_sync_status_rpc_reports_the_degraded_zero_value_shape() {
    let _guard = env_lock();
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = EnvGuard::set_path("OPENHUMAN_WORKSPACE", tmp.path());
    let _home = EnvGuard::set_path("HOME", tmp.path());
    let _backend = EnvGuard::unset("BACKEND_URL");
    let mut config = config_in(&tmp);
    let router = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {
                    "connections": [
                        { "id": "conn-slack-round21", "toolkit": "slack", "status": "ACTIVE" },
                        { "id": "conn-slack-pending", "toolkit": "slack", "status": "PENDING" },
                        { "id": "conn-gmail-round21", "toolkit": "gmail", "status": "ACTIVE" }
                    ]
                }
            }))
        }),
    );
    let (base, server) = loopback_router(router).await;
    config.api_url = Some(base);
    persist_config(&config).await;
    store_session(&config);
    memory_global::init(config.workspace_dir.clone()).expect("memory global");

    // `list_slack_connections` (what `sync_status_rpc` calls first) still
    // goes through the old backend HTTP client factory, untouched by the
    // tinyconnectors migration — so the loopback connections router above
    // still drives it. What changed is everything after: there is no more
    // per-connection detail to read back (see module doc comment), so this
    // no longer seeds a `SyncState` before calling the RPC — nothing would
    // read it.
    let outcome = sync_status_rpc(&config, SyncStatusRequest::default())
        .await
        .expect("status rpc");
    assert_eq!(
        outcome.value.connections.len(),
        1,
        "only the active slack connection qualifies"
    );
    let row = &outcome.value.connections[0];
    assert_eq!(row.connection_id, "conn-slack-round21");
    assert_eq!(row.per_channel_cursors, "{}");
    assert_eq!(row.synced_ids_count, 0);
    assert_eq!(row.requests_used_today, 0);
    assert_eq!(row.daily_request_limit, 0);
    assert!(
        outcome
            .logs
            .iter()
            .any(|line| line.contains("connections=1") && line.contains("no longer available")),
        "status log should explain the degraded read: {:?}",
        outcome.logs
    );

    server.abort();
}

#[tokio::test]
async fn memory_tree_source_query_filters_reranks_and_hydrates_manual_summaries() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in(&tmp);
    std::fs::create_dir_all(config.memory_tree_content_root()).expect("content root");
    seed_source_summary(
        &config,
        "slack:#round21",
        "summary-round21-chat",
        "Full chat summary body from disk.",
        1_780_313_600_000,
        Some(one_hot(0)),
    );
    seed_source_summary(
        &config,
        "gmail:round21@example.test",
        "summary-round21-email",
        "Full email summary body from disk.",
        1_780_227_200_000,
        None,
    );

    let all = query_source(&config, None, None, None, None, 0)
        .await
        .expect("all source query");
    assert_eq!(all.total, 2);
    assert_eq!(all.hits.len(), 2);

    let chat = query_source(
        &config,
        None,
        Some(tinymemory_core::store::chunks::types::SourceKind::Chat),
        None,
        Some("semantic query keeps embedded rows first"),
        10,
    )
    .await
    .expect("chat query");
    assert_eq!(chat.hits.len(), 1);
    assert_eq!(chat.hits[0].tree_scope, "slack:#round21");
    assert_eq!(chat.hits[0].content, "Full chat summary body from disk.");

    let missing = query_source(&config, Some("slack:#missing"), None, None, None, 10)
        .await
        .expect("missing source");
    assert!(missing.hits.is_empty());
}

fn one_hot(index: usize) -> Vec<f32> {
    let mut values = vec![0.0; EMBEDDING_DIM];
    values[index] = 1.0;
    values
}

fn seed_source_summary(
    config: &Config,
    scope: &str,
    summary_id: &str,
    body: &str,
    timestamp_ms: i64,
    embedding: Option<Vec<f32>>,
) {
    let ts = Utc.timestamp_millis_opt(timestamp_ms).unwrap();
    let tree = Tree {
        id: format!("tree:{summary_id}"),
        kind: TreeKind::Source,
        scope: scope.to_string(),
        ask: None,
        root_id: Some(summary_id.to_string()),
        max_level: 1,
        status: TreeStatus::Active,
        created_at: ts,
        last_sealed_at: Some(ts),
    };
    tree_store::insert_tree(config, &tree).expect("insert source tree");

    let node = SummaryNode {
        id: summary_id.to_string(),
        tree_id: tree.id.clone(),
        tree_kind: TreeKind::Source,
        level: 1,
        parent_id: None,
        child_ids: vec!["leaf-a".to_string(), "leaf-b".to_string()],
        content: "preview only".to_string(),
        token_count: 64,
        entities: vec!["round21".to_string()],
        topics: vec!["coverage".to_string()],
        time_range_start: ts,
        time_range_end: ts,
        score: 0.75,
        sealed_at: ts,
        deleted: false,
        embedding: embedding.clone(),
        doc_id: None,
        version_ms: None,
    };
    let staged = stage_summary(
        &config.memory_tree_content_root(),
        &SummaryComposeInput {
            summary_id: &node.id,
            tree_kind: SummaryTreeKind::Source,
            tree_id: &node.tree_id,
            tree_scope: &tree.scope,
            level: node.level,
            child_ids: &node.child_ids,
            child_basenames: None,
            child_count: node.child_ids.len(),
            time_range_start: node.time_range_start,
            time_range_end: node.time_range_end,
            sealed_at: node.sealed_at,
            body,
        },
        scope,
    )
    .expect("stage summary body");
    let embedding_blob = embedding.as_ref().map(|values| pack_embedding(values));

    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO mem_tree_summaries (
                id, tree_id, tree_kind, level, parent_id,
                child_ids_json, content, token_count,
                entities_json, topics_json,
                time_range_start_ms, time_range_end_ms,
                score, sealed_at_ms, deleted, embedding,
                content_path, content_sha256
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            rusqlite::params![
                node.id,
                node.tree_id,
                node.tree_kind.as_str(),
                node.level,
                node.parent_id,
                serde_json::to_string(&node.child_ids).unwrap(),
                node.content,
                node.token_count,
                serde_json::to_string(&node.entities).unwrap(),
                serde_json::to_string(&node.topics).unwrap(),
                node.time_range_start.timestamp_millis(),
                node.time_range_end.timestamp_millis(),
                node.score,
                node.sealed_at.timestamp_millis(),
                node.deleted as i64,
                embedding_blob,
                staged.content_path,
                staged.content_sha256,
            ],
        )?;
        Ok(())
    })
    .expect("insert summary row");
}
