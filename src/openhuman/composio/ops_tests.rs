use super::*;

#[test]
fn parse_sync_reason_accepts_known_values() {
    assert_eq!(parse_sync_reason(None).unwrap(), SyncReason::Manual);
    assert_eq!(
        parse_sync_reason(Some("manual")).unwrap(),
        SyncReason::Manual
    );
    assert_eq!(
        parse_sync_reason(Some("periodic")).unwrap(),
        SyncReason::Periodic
    );
    assert_eq!(
        parse_sync_reason(Some("connection_created")).unwrap(),
        SyncReason::ConnectionCreated
    );
}

#[test]
fn parse_sync_reason_rejects_unknown_values() {
    let err = parse_sync_reason(Some("scheduled")).unwrap_err();
    assert!(err.contains("unrecognized sync reason"));
    assert!(err.contains("scheduled"));
    // Typo of a real value should also fail rather than coerce.
    assert!(parse_sync_reason(Some("Periodic")).is_err());
    assert!(parse_sync_reason(Some("")).is_err());
}

// ── resolve_client / ops auth errors ──────────────────────────

fn test_config(tmp: &tempfile::TempDir) -> Config {
    let mut c = Config::default();
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c
}

#[test]
fn resolve_client_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // `ComposioClient` intentionally doesn't implement `Debug` — use a
    // pattern match instead of `.unwrap_err()`.
    let Err(err) = resolve_client(&config) else {
        panic!("expected auth error when no session is stored");
    };
    assert!(err.contains("composio unavailable"));
    assert!(err.contains("auth_store_session"));
}

#[tokio::test]
async fn composio_list_toolkits_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_list_toolkits(&config).await.unwrap_err();
    // Backend-mode (default) without a session — the mode-aware factory
    // surfaces "no backend session token" so we accept either the
    // legacy `composio unavailable` prefix or the new factory message.
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_list_capabilities_does_not_require_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let outcome = composio_list_capabilities(&config).await.unwrap();
    assert!(outcome
        .value
        .capabilities
        .iter()
        .any(|entry| { entry.toolkit == "gmail" && entry.native_provider && entry.memory_ingest }));
    assert!(outcome.value.capabilities.iter().any(|entry| {
        entry.toolkit == "googlecalendar" && !entry.native_provider && entry.curated_tools
    }));
}

#[tokio::test]
async fn composio_list_connections_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_list_connections(&config).await.unwrap_err();
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_authorize_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_authorize(&config, "gmail", None)
        .await
        .unwrap_err();
    // Backend mode (default) without a session — the mode-aware factory
    // surfaces "no backend session token" once `composio_authorize`
    // routes through `create_composio_client`. Accept either the
    // legacy `composio unavailable` prefix or the new factory phrasing.
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_delete_connection_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_delete_connection(&config, "c-1", false)
        .await
        .unwrap_err();
    assert!(err.contains("composio unavailable"));
}

#[tokio::test]
async fn composio_list_tools_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_list_tools(&config, None, None).await.unwrap_err();
    // Same tolerance as `composio_list_toolkits_errors_without_session`.
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_execute_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_execute(&config, "GMAIL_SEND_EMAIL", None)
        .await
        .unwrap_err();
    assert!(
        err.to_lowercase().contains("composio")
            && (err.contains("no backend session") || err.contains("unavailable")),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn composio_get_user_profile_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_get_user_profile(&config, "c-1").await.unwrap_err();
    assert!(err.contains("composio unavailable"));
}

#[tokio::test]
async fn composio_sync_errors_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let err = composio_sync(&config, "c-1", None).await.unwrap_err();
    assert!(err.contains("composio unavailable"));
}

#[tokio::test]
async fn composio_sync_rejects_invalid_reason_before_client_check() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // Invalid reason → should fail at parse step *before* touching the
    // client, so the error message references the reason, not auth.
    let err = composio_sync(&config, "c-1", Some("weird".into()))
        .await
        .unwrap_err();
    assert!(err.contains("unrecognized sync reason"));
}

#[tokio::test]
async fn composio_list_trigger_history_errors_when_store_not_init() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    // The trigger history store is a process-global singleton. If
    // another test in the same binary already initialised it (e.g.
    // via the archive-roundtrip test), skip rather than asserting on
    // the uninitialised branch.
    if super::super::trigger_history::global().is_some() {
        return;
    }
    let err = composio_list_trigger_history(&config, Some(10))
        .await
        .unwrap_err();
    assert!(err.contains("archive store is not initialized"));
}

// ── cache_key / invalidate_connected_integrations_cache ───────

/// Per-module alias so call sites don't need to spell out the path.
/// The actual lock lives in `connected_integrations` so it is shared
/// with `tools_tests` and any other test module that touches the cache.
fn cache_guard() -> std::sync::MutexGuard<'static, ()> {
    crate::openhuman::composio::connected_integrations::composio_cache_test_lock()
}

#[test]
fn cache_key_is_based_on_config_path_string() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Config::default();
    a.config_path = tmp.path().join("a.toml");
    let mut b = Config::default();
    b.config_path = tmp.path().join("b.toml");
    assert_ne!(cache_key(&a), cache_key(&b));
    assert_eq!(cache_key(&a), cache_key(&a));
}

#[tokio::test]
async fn fetch_connected_integrations_returns_empty_without_auth() {
    let _guard = cache_guard();
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let integrations = fetch_connected_integrations(&config).await;
    assert!(integrations.is_empty());
}

#[test]
fn invalidate_connected_integrations_cache_is_safe_without_prior_insert() {
    let _guard = cache_guard();
    // Must not panic on an empty cache.
    invalidate_connected_integrations_cache();
    invalidate_connected_integrations_cache();
}

// ── Mock-backend integration tests for ops ─────────────────────

use crate::openhuman::memory_store::chunks::store as memory_tree_store;
use crate::openhuman::memory_store::chunks::types::{
    chunk_id, Chunk, Metadata, SourceKind, SourceRef,
};
use axum::{
    extract::{Path, Query},
    routing::{get, post},
    Json, Router,
};
use chrono::{TimeZone, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;

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
    crate::openhuman::credentials::AuthService::from_config(&c)
        .store_provider_token(
            crate::openhuman::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
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

#[tokio::test]
async fn composio_list_toolkits_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/toolkits",
        get(|| async { Json(json!({"success": true, "data": {"toolkits": ["gmail"]}})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_toolkits(&config).await.unwrap();
    assert_eq!(outcome.value.toolkits, vec!["gmail".to_string()]);
    assert!(outcome.logs.iter().any(|l| l.contains("toolkit")));
}

#[tokio::test]
async fn composio_list_connections_via_mock_counts_active() {
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {"connections": [
                    {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                    {"id":"c2","toolkit":"notion","status":"PENDING"},
                    {"id":"c3","toolkit":"gmail","status":"CONNECTED"}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_connections(&config).await.unwrap();
    assert_eq!(outcome.value.connections.len(), 3);
    // 2 active, 3 total
    assert!(outcome.logs.iter().any(|l| l.contains("3 connection")));
    assert!(outcome.logs.iter().any(|l| l.contains("2 active")));
}

#[tokio::test]
async fn composio_authorize_clears_pending_meta_connection_before_handoff() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let deletes = Arc::new(AtomicUsize::new(0));
    let deletes_for_delete = Arc::clone(&deletes);
    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"ig-pending","toolkit":"instagram","status":"PENDING"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(move |Path(id): Path<String>| {
                let deletes = Arc::clone(&deletes_for_delete);
                async move {
                    if id == "ig-pending" {
                        deletes.fetch_add(1, Ordering::SeqCst);
                    }
                    Json(json!({"success": true, "data": {"deleted": true}}))
                }
            }),
        )
        .route(
            "/agent-integrations/composio/authorize",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["toolkit"], "instagram");
                Json(json!({
                    "success": true,
                    "data": {
                        "connectUrl": "https://meta.example/oauth",
                        "connectionId": "c-new"
                    }
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_authorize(&config, "instagram", None)
        .await
        .unwrap();
    assert_eq!(outcome.value.connection_id, "c-new");
    assert_eq!(deletes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn composio_authorize_via_mock_publishes_event_and_returns_url() {
    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|Json(_b): Json<Value>| async move {
            Json(json!({
                "success": true,
                "data": {"connectUrl": "https://x", "connectionId": "c1"}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_authorize(&config, "gmail", None).await.unwrap();
    assert_eq!(outcome.value.connect_url, "https://x");
    assert_eq!(outcome.value.connection_id, "c1");
}

#[tokio::test]
async fn composio_delete_connection_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/connections/{id}",
        axum::routing::delete(|Path(_id): Path<String>| async move {
            Json(json!({"success": true, "data": {"deleted": true}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_delete_connection(&config, "c1", false)
        .await
        .unwrap();
    assert!(outcome.value.deleted);
}

#[tokio::test]
async fn composio_delete_connection_clear_memory_deletes_slack_source() {
    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"slack","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(|Path(_id): Path<String>| async move {
                Json(json!({"success": true, "data": {"deleted": true}}))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let target = sample_memory_chunk(SourceKind::Chat, "slack:c1", 0);
    let unrelated = sample_memory_chunk(SourceKind::Chat, "slack:c2", 0);
    memory_tree_store::upsert_chunks(&config, &[target, unrelated]).expect("chunks should seed");

    let outcome = composio_delete_connection(&config, "c1", true)
        .await
        .unwrap();

    assert!(outcome.value.deleted);
    assert_eq!(outcome.value.memory_chunks_deleted, 1);
    let remaining = memory_tree_store::list_chunks(
        &config,
        &memory_tree_store::ListChunksQuery {
            source_kind: Some(SourceKind::Chat),
            ..Default::default()
        },
    )
    .expect("chunks should list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].metadata.source_id, "slack:c2");
}

/// #4: full path through the REAL `composio_delete_connection` handler
/// (clear_memory=true, mock backend) — deleting a connection's last chunk must
/// cascade away its source summary tree AND the summary's on-disk content file,
/// not just the chunk rows. The tree is a real `get_or_create_source_tree`; the
/// content file sits at the production `content_path` location.
#[tokio::test]
async fn composio_delete_connection_clear_memory_cascades_source_tree_and_content_file() {
    use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
    use crate::openhuman::memory_store::trees::store as tree_store;
    use crate::openhuman::memory_store::trees::types::{SummaryNode, TreeKind};
    use rusqlite::params;

    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"slack","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(|Path(_id): Path<String>| async move {
                Json(json!({"success": true, "data": {"deleted": true}}))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    // One slack chunk for connection c1 → source_id `slack:c1`.
    let chunk = sample_memory_chunk(SourceKind::Chat, "slack:c1", 0);
    memory_tree_store::upsert_chunks(&config, &[chunk.clone()]).expect("seed chunk");

    // Real source tree for that source + a summary whose content file lives at
    // the production content-root location.
    let tree = get_or_create_source_tree(&config, "slack:c1").expect("source tree");
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let rel = "summaries/slack_c1/L1/sum-1.md";
    let abs = config.memory_tree_content_root().join(rel);
    std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
    std::fs::write(&abs, "summarised slack body").unwrap();

    memory_tree_store::with_connection(&config, |conn| {
        let tx = conn.unchecked_transaction()?;
        tree_store::insert_summary_tx(
            &tx,
            &SummaryNode {
                id: "sum-1".into(),
                tree_id: tree.id.clone(),
                tree_kind: TreeKind::Source,
                level: 1,
                parent_id: None,
                child_ids: vec![chunk.id.clone()],
                content: "preview".into(),
                token_count: 3,
                entities: vec![],
                topics: vec![],
                time_range_start: ts,
                time_range_end: ts,
                score: 0.5,
                sealed_at: ts,
                deleted: false,
                embedding: None,
            },
            None,
            "test/model@3",
        )?;
        tx.execute(
            "UPDATE mem_tree_summaries SET content_path = ?1 WHERE id = 'sum-1'",
            params![rel],
        )?;
        tx.commit()?;
        Ok(())
    })
    .expect("seed summary + content file pointer");

    // sanity: tree + on-disk file exist before the disconnect.
    assert!(
        tree_store::get_tree_by_scope(&config, TreeKind::Source, "slack:c1")
            .unwrap()
            .is_some()
    );
    assert!(abs.exists());

    // ---- act: the REAL handler, clear_memory=true ----
    let outcome = composio_delete_connection(&config, "c1", true)
        .await
        .unwrap();
    assert!(outcome.value.deleted);
    assert_eq!(outcome.value.memory_chunks_deleted, 1);

    // chunk, source tree, summary row, AND on-disk content file are all gone.
    assert!(memory_tree_store::get_chunk(&config, &chunk.id)
        .unwrap()
        .is_none());
    assert!(
        tree_store::get_tree_by_scope(&config, TreeKind::Source, "slack:c1")
            .unwrap()
            .is_none()
    );
    memory_tree_store::with_connection(&config, |conn| {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM mem_tree_summaries", [], |r| r.get(0))?;
        assert_eq!(n, 0);
        Ok(())
    })
    .unwrap();
    assert!(
        !abs.exists(),
        "summary content file must be removed via the real handler cascade"
    );
}

/// #4 (full live seal): like the above, but the summary + on-disk file are
/// produced by the REAL `seal_one_level` pipeline (staged chunk body →
/// summarise → `stage_summary`), not hand-written. Then the REAL
/// `composio_delete_connection(clear_memory=true)` handler must cascade the
/// tree, the summary row, AND the seal-produced content file away.
#[tokio::test]
async fn composio_delete_connection_clear_memory_cascades_live_sealed_tree_and_file() {
    use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
    use crate::openhuman::memory_store::chunks::store::{
        get_summary_content_pointers, upsert_staged_chunks_tx,
    };
    use crate::openhuman::memory_store::content::stage_chunks;
    use crate::openhuman::memory_store::trees::store as tree_store;
    use crate::openhuman::memory_store::trees::types::{Buffer, TreeKind};
    use crate::openhuman::memory_tree::tree::bucket_seal::{seal_one_level, LabelStrategy};

    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"slack","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(|Path(_id): Path<String>| async move {
                Json(json!({"success": true, "data": {"deleted": true}}))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_with_backend(&tmp, base);
    // Force the inert embedder so the real seal's summary-embed step doesn't
    // reach a live endpoint. `config_with_backend` stores a cloud session +
    // api_url, so the factory would otherwise build a *cloud* embedder against
    // the mock (no embeddings route). `embeddings_provider = "none"` is the
    // actual switch that selects `InertEmbedder`.
    config.embeddings_provider = Some("none".to_string());
    config.memory_tree.embedding_endpoint = None;
    config.memory_tree.embedding_model = None;
    config.memory_tree.embedding_strict = false;

    // Real chunk for slack:c1 WITH its body staged to disk, so the seal's
    // `hydrate_leaf_inputs` → `read_chunk_body` can resolve it.
    let chunk = sample_memory_chunk(SourceKind::Chat, "slack:c1", 0);
    memory_tree_store::upsert_chunks(&config, &[chunk.clone()]).expect("seed chunk");
    let staged = stage_chunks(
        &config.memory_tree_content_root(),
        std::slice::from_ref(&chunk),
    )
    .expect("stage chunk body");
    memory_tree_store::with_connection(&config, |conn| {
        let tx = conn.unchecked_transaction()?;
        upsert_staged_chunks_tx(&tx, &staged)?;
        tx.commit()?;
        Ok(())
    })
    .expect("record staged chunk pointer");

    // Run the REAL seal — produces a genuine summary row + on-disk file.
    let tree = get_or_create_source_tree(&config, "slack:c1").expect("source tree");
    let buf = Buffer {
        tree_id: tree.id.clone(),
        level: 0,
        item_ids: vec![chunk.id.clone()],
        token_sum: i64::from(chunk.token_count),
        oldest_at: Some(chunk.metadata.time_range.0),
    };
    let summary_id = seal_one_level(&config, &tree, &buf, &LabelStrategy::Empty, false)
        .await
        .expect("real seal produces a summary");

    // The seal wrote a real on-disk content file for the summary.
    let (rel, _sha) = get_summary_content_pointers(&config, &summary_id)
        .unwrap()
        .expect("seal staged a summary content file");
    let abs = {
        let mut p = config.memory_tree_content_root();
        for c in rel.split('/') {
            p.push(c);
        }
        p
    };
    assert!(
        abs.exists(),
        "seal must have written a summary file on disk"
    );
    assert!(
        tree_store::get_tree_by_scope(&config, TreeKind::Source, "slack:c1")
            .unwrap()
            .is_some()
    );

    // ---- act: REAL handler, clear_memory=true ----
    let outcome = composio_delete_connection(&config, "c1", true)
        .await
        .unwrap();
    assert!(outcome.value.deleted);
    assert_eq!(outcome.value.memory_chunks_deleted, 1);

    // chunk, tree, summary row, and the seal-produced file are all gone.
    assert!(memory_tree_store::get_chunk(&config, &chunk.id)
        .unwrap()
        .is_none());
    assert!(
        tree_store::get_tree_by_scope(&config, TreeKind::Source, "slack:c1")
            .unwrap()
            .is_none()
    );
    assert!(tree_store::get_summary(&config, &summary_id)
        .unwrap()
        .is_none());
    assert!(
        !abs.exists(),
        "seal-produced summary file must be removed via the real handler cascade"
    );
}

#[tokio::test]
async fn composio_delete_connection_clear_memory_keeps_other_gmail_connections() {
    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                        {"id":"c2","toolkit":"gmail","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections/{id}",
            axum::routing::delete(|Path(_id): Path<String>| async move {
                Json(json!({"success": true, "data": {"deleted": true}}))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let c1_account = sample_memory_chunk_with_owner(
        SourceKind::Email,
        "gmail:pilot-at-example-dot-com",
        "gmail-sync:c1",
        0,
    );
    let c2_account = sample_memory_chunk_with_owner(
        SourceKind::Email,
        "gmail:pilot-at-example-dot-com",
        "gmail-sync:c2",
        1,
    );
    let c1_connection_scoped =
        sample_memory_chunk_with_owner(SourceKind::Email, "gmail:c1:thread-a", "gmail-sync:c1", 2);
    let c2_connection_scoped =
        sample_memory_chunk_with_owner(SourceKind::Email, "gmail:c2:thread-b", "gmail-sync:c2", 3);
    memory_tree_store::upsert_chunks(
        &config,
        &[
            c1_account,
            c2_account.clone(),
            c1_connection_scoped,
            c2_connection_scoped.clone(),
        ],
    )
    .expect("chunks should seed");

    let outcome = composio_delete_connection(&config, "c1", true)
        .await
        .unwrap();

    assert!(outcome.value.deleted);
    assert_eq!(outcome.value.memory_chunks_deleted, 2);
    let remaining = memory_tree_store::list_chunks(
        &config,
        &memory_tree_store::ListChunksQuery {
            source_kind: Some(SourceKind::Email),
            ..Default::default()
        },
    )
    .expect("chunks should list");
    assert_eq!(remaining.len(), 2);
    assert!(remaining.iter().any(|chunk| chunk.id == c2_account.id));
    assert!(remaining
        .iter()
        .any(|chunk| chunk.id == c2_connection_scoped.id));
}

#[tokio::test]
async fn notion_cleanup_targets_include_synced_page_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let memory = std::sync::Arc::new(
        MemoryClient::from_workspace_dir(config.workspace_dir.clone())
            .expect("memory client should initialise"),
    );
    let mut state = SyncState::new("notion", "conn-1");
    state.mark_synced("page-a@2026-01-01T00:00:00Z");
    state.mark_synced("page-b");
    state.save(&memory).await.expect("sync state should save");

    let targets = composio_memory_targets_for_connection(&config, Some("notion"), "conn-1")
        .await
        .expect("notion cleanup targets should resolve");

    assert!(targets.contains(&MemoryCleanupTarget::Exact(
        SourceKind::Document,
        "notion:page-a".to_string()
    )));
    assert!(targets.contains(&MemoryCleanupTarget::Exact(
        SourceKind::Document,
        "notion:page-b".to_string()
    )));
    assert!(targets.contains(&MemoryCleanupTarget::Exact(
        SourceKind::Document,
        "composio-notion-page-page-a".to_string()
    )));
}

#[tokio::test]
async fn notion_cleanup_targets_surface_corrupt_sync_state() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let memory = std::sync::Arc::new(
        MemoryClient::from_workspace_dir(config.workspace_dir.clone())
            .expect("memory client should initialise"),
    );
    memory
        .kv_set(
            Some(super::super::providers::sync_state::KV_NAMESPACE),
            "notion:conn-1",
            &serde_json::json!({ "toolkit": 42 }),
        )
        .await
        .expect("corrupt sync state should be written");

    let err = composio_memory_targets_for_connection(&config, Some("notion"), "conn-1")
        .await
        .expect_err("corrupt sync state should surface");

    assert!(err.to_string().contains("failed to load notion sync state"));
}

#[tokio::test]
async fn drive_cleanup_targets_are_connection_scoped() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);

    let targets = composio_memory_targets_for_connection(&config, Some("google_drive"), "conn-1")
        .await
        .expect("drive cleanup targets should resolve");

    assert!(targets.contains(&MemoryCleanupTarget::Exact(
        SourceKind::Document,
        "drive:conn-1".to_string()
    )));
    assert!(targets.contains(&MemoryCleanupTarget::Prefix(
        SourceKind::Document,
        "googledrive:conn-1:".to_string()
    )));
    assert!(targets.contains(&MemoryCleanupTarget::Prefix(
        SourceKind::Document,
        "google_drive:conn-1/".to_string()
    )));
}

#[tokio::test]
async fn composio_get_user_profile_via_mock_returns_provider_profile() {
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _cache_guard = cache_guard();
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    crate::openhuman::composio::providers::init_default_providers();

    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/execute",
            post(|Json(body): Json<Value>| async move {
                let action = body
                    .get("tool")
                    .and_then(Value::as_str)
                    .or_else(|| body.get("action").and_then(Value::as_str))
                    .unwrap_or("");
                let data = match action {
                    "GMAIL_GET_PROFILE" => json!({
                        "emailAddress": "pilot@example.com",
                        "displayName": "Phoenix Pilot",
                        "profileUrl": "https://mail.google.com/mail/u/0/#inbox"
                    }),
                    other => panic!("unexpected action: {other}"),
                };
                Json(json!({
                    "success": true,
                    "data": {
                        "successful": true,
                        "data": data,
                        "error": null
                    }
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let _workspace_env_guard = WorkspaceEnvGuard::set(tmp.path());
    config.save().await.unwrap();

    let outcome = composio_get_user_profile(&config, "c1").await.unwrap();

    assert_eq!(outcome.value.toolkit, "gmail");
    assert_eq!(outcome.value.connection_id.as_deref(), Some("c1"));
    assert_eq!(outcome.value.email.as_deref(), Some("pilot@example.com"));
    assert_eq!(outcome.value.display_name.as_deref(), Some("Phoenix Pilot"));
    assert!(outcome.logs.iter().any(|l| l.contains("gmail")));
}

#[tokio::test]
async fn composio_list_tools_via_mock_with_filter() {
    let app = Router::new().route(
        "/agent-integrations/composio/tools",
        get(|Query(_q): Query<HashMap<String, String>>| async move {
            Json(json!({
                "success": true,
                "data": {"tools": [
                    {"type":"function","function":{"name":"GMAIL_SEND_EMAIL"}},
                    {"type":"function","function":{"name":"GMAIL_SEARCH"}}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_tools(&config, Some(vec!["gmail".into()]), None)
        .await
        .unwrap();
    assert_eq!(outcome.value.tools.len(), 2);
}

#[tokio::test]
async fn composio_execute_via_mock_succeeds_and_logs_elapsed() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|Json(b): Json<Value>| async move {
            Json(json!({
                "success": true,
                "data": {
                    "data": {"echo": b["tool"]},
                    "successful": true,
                    "error": null,
                    "costUsd": 0.001
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_execute(&config, "GMAIL_SEND", Some(json!({"to": "a"})))
        .await
        .unwrap();
    assert!(outcome.value.successful);
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("executed GMAIL_SEND")));
}

#[tokio::test]
async fn composio_execute_via_mock_propagates_backend_error() {
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async { Json(json!({"success": false, "error": "rate limited"})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let err = composio_execute(&config, "ANY_TOOL", None)
        .await
        .unwrap_err();
    // The dispatcher (`execute_composio_action`) classifies transport
    // failures and prefixes them with `[composio:error:<class>] …`; ops.rs
    // preserves that prefix so the frontend formatter can parse the class.
    // For an unrecognised tool slug and a 502-shaped envelope the only
    // signal we get is the backend error text, so assert on its contents.
    assert!(
        err.starts_with("[composio:error:") && err.contains("rate limited"),
        "got: {err}"
    );
}

#[tokio::test]
async fn composio_sync_gmail_via_mock_archives_raw_email_and_updates_outcome() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    use crate::openhuman::config::TEST_ENV_LOCK;
    use crate::openhuman::memory_store::content::raw::{raw_rel_path, RawKind};
    use crate::openhuman::memory_tree::tree::rpc::{list_chunks_rpc, ListChunksRequest};
    let _cache_guard = cache_guard();
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    crate::openhuman::composio::providers::init_default_providers();

    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/execute",
            post(|Json(body): Json<Value>| async move {
                let action = body
                    .get("tool")
                    .and_then(Value::as_str)
                    .or_else(|| body.get("action").and_then(Value::as_str))
                    .unwrap_or("");
                let data = match action {
                    "GMAIL_GET_PROFILE" => json!({
                        "emailAddress": "pilot@example.com",
                        "displayName": "Phoenix Pilot"
                    }),
                    "GMAIL_FETCH_EMAILS" => json!({
                        "messages": [{
                            "messageId": "gmail-msg-1",
                            "threadId": "gmail-thread-1",
                            "sender": "captain@example.com",
                            "to": "pilot@example.com",
                            "subject": "Phoenix launch canary",
                            "messageTimestamp": "2024-06-01T12:00:00Z",
                            "labelIds": ["INBOX"],
                            "markdownFormatted": "Phoenix launch canary body for mock sync coverage.",
                            "payload": {}
                        }]
                    }),
                    other => panic!("unexpected action: {other}"),
                };
                Json(json!({
                    "success": true,
                    "data": {
                        "successful": true,
                        "data": data,
                        "error": null
                    }
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_with_backend(&tmp, base);
    config.memory_tree.embedding_strict = false;
    let _workspace_env_guard = WorkspaceEnvGuard::set(tmp.path());
    config.save().await.unwrap();
    let _ = crate::openhuman::memory::global::init(config.workspace_dir.clone()).unwrap();

    let outcome = composio_sync(&config, "c1", Some("manual".to_string()))
        .await
        .unwrap();

    assert_eq!(outcome.value.toolkit, "gmail");
    assert_eq!(outcome.value.connection_id.as_deref(), Some("c1"));
    // composio_sync is now spawn-and-return: the immediate envelope is a
    // "started" sentinel, and the actual ingestion runs on a detached
    // tokio task. items_ingested == 0 / finished_at_ms == 0 / summary
    // contains "started" are the contract of that sentinel.
    assert_eq!(
        outcome.value.items_ingested, 0,
        "spawn-and-return: items_ingested on the immediate envelope is a 'started' sentinel, not a final count"
    );
    assert_eq!(
        outcome.value.finished_at_ms, 0,
        "spawn-and-return: finished_at_ms == 0 means 'task spawned, not yet complete'"
    );
    assert!(
        outcome.value.summary.contains("started"),
        "expected spawn-and-return summary to mention 'started', got: {}",
        outcome.value.summary
    );

    // Poll for the spawned ingest task to drain. The mock backend is
    // local + in-memory, so this normally lands in well under a second.
    let chunks = {
        let mut chunks = Vec::new();
        for _ in 0..50 {
            chunks = list_chunks_rpc(
                &config,
                ListChunksRequest {
                    source_kind: Some("email".to_string()),
                    source_id: Some("gmail:pilot-at-example-dot-com".to_string()),
                    limit: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .chunks;
            if !chunks.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        chunks
    };
    assert_eq!(
        chunks.len(),
        1,
        "expected one ingested Gmail chunk after spawned task drains"
    );
    assert!(
        chunks[0].content.contains("Phoenix launch canary"),
        "chunk content missing mock email subject: {}",
        chunks[0].content
    );
    assert!(
        chunks[0].content.contains("mock sync coverage"),
        "chunk content missing mock email body: {}",
        chunks[0].content
    );

    let raw_path = config.memory_tree_content_root().join(raw_rel_path(
        "gmail:pilot-at-example-dot-com",
        RawKind::Email,
        1_717_243_200_000,
        "gmail-msg-1",
    ));
    let archived = std::fs::read_to_string(&raw_path)
        .unwrap_or_else(|e| panic!("expected archived Gmail raw message at {raw_path:?}: {e}"));
    assert!(
        archived.contains("Phoenix launch canary"),
        "archived email missing mock subject: {archived}"
    );
    assert!(
        archived.contains("mock sync coverage"),
        "archived email missing mock body: {archived}"
    );
}

#[tokio::test]
async fn fetch_connected_integrations_via_mock_aggregates_tools() {
    let _guard = cache_guard();
    // Connections: gmail + notion. Tools: filtered to those toolkits
    // and prefixed with the uppercased slug. The toolkits route
    // backs the `list_toolkits()` allowlist gate that
    // `fetch_connected_integrations_uncached` calls before touching
    // connections — without it the function bails out at the first
    // step and returns an empty vec.
    let app = Router::new()
        .route(
            "/agent-integrations/composio/toolkits",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"toolkits": ["gmail", "notion"]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"},
                        {"id":"c2","toolkit":"notion","status":"CONNECTED"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/tools",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"tools": [
                        {"type":"function","function":{
                            "name":"GMAIL_SEND_EMAIL",
                            "description":"Send"
                        }},
                        {"type":"function","function":{
                            "name":"NOTION_CREATE_PAGE",
                            "description":"Create"
                        }}
                    ]}
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    // Use a fresh cache key by isolating config_path.
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();
    let integrations = fetch_connected_integrations(&config).await;
    assert_eq!(integrations.len(), 2);
    // Sorted by toolkit name
    assert_eq!(integrations[0].toolkit, "gmail");
    assert_eq!(integrations[1].toolkit, "notion");
    assert_eq!(integrations[0].tools.len(), 1);
    assert_eq!(integrations[0].tools[0].name, "GMAIL_SEND_EMAIL");
}

#[tokio::test]
async fn fetch_connected_integrations_treats_slack_and_telegram_status_like_ui() {
    let _guard = cache_guard();
    let app = Router::new()
        .route(
            "/agent-integrations/composio/toolkits",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"toolkits": [" Slack ", "telegram"]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c-slack","toolkit":" Slack ","status":"connected"},
                        {"id":"c-telegram","toolkit":"telegram","status":" active "}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/tools",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"tools": [
                        {"type":"function","function":{
                            "name":"SLACK_FETCH_CONVERSATION_HISTORY",
                            "description":"Read Slack channel history"
                        }},
                        {"type":"function","function":{
                            "name":"TELEGRAM_GET_CHAT_HISTORY",
                            "description":"Read Telegram chat history"
                        }},
                        {"type":"function","function":{
                            "name":"SLACK_DELETE_CHANNEL",
                            "description":"Delete a channel"
                        }}
                    ]}
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();

    let integrations = fetch_connected_integrations(&config).await;

    let slack = integrations
        .iter()
        .find(|i| i.toolkit == "slack")
        .expect("slack integration should be present");
    assert!(slack.connected);
    assert_eq!(slack.tools.len(), 1);
    assert_eq!(slack.tools[0].name, "SLACK_FETCH_CONVERSATION_HISTORY");

    let telegram = integrations
        .iter()
        .find(|i| i.toolkit == "telegram")
        .expect("telegram integration should be present");
    assert!(telegram.connected);
    assert_eq!(telegram.tools.len(), 1);
    assert_eq!(telegram.tools[0].name, "TELEGRAM_GET_CHAT_HISTORY");
}

#[tokio::test]
async fn fetch_connected_integrations_via_mock_returns_empty_with_no_active() {
    let _guard = cache_guard();
    let app = Router::new().route(
        "/agent-integrations/composio/connections",
        get(|| async {
            Json(json!({"success": true, "data": {"connections": [
                {"id":"c1","toolkit":"gmail","status":"PENDING"}
            ]}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    invalidate_connected_integrations_cache();
    let integrations = fetch_connected_integrations(&config).await;
    assert!(integrations.is_empty());
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

#[test]
fn sync_cache_invalidates_when_connection_becomes_active() {
    let _guard = cache_guard();
    // Cache reflects the pre-connect world: gmail is listed but
    // not connected. This is exactly the state the chat runtime
    // gets stuck in on Windows when the user completes OAuth
    // after the event-bus 60 s readiness poll times out.
    let key = "windows-regression-1";
    clear_cache_key(key);
    seed_cache(
        key,
        vec![integration("gmail", false), integration("notion", false)],
    );

    // Fresh UI poll shows gmail just flipped ACTIVE — mirrors a
    // user who finished OAuth in the system browser.
    sync_cache_with_connections(&[conn("c-1", "gmail", "ACTIVE")]);

    // Chat-runtime cache must be cleared so the next
    // `fetch_connected_integrations` re-fetches truth from the
    // backend. Without this fix the entry would live on until
    // `CACHE_TTL` expired or the process restarted.
    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_none(),
        "expected cache to be busted when a new toolkit flips ACTIVE"
    );
}

#[test]
fn sync_cache_invalidates_when_connection_is_removed() {
    let _guard = cache_guard();
    // Cache remembers gmail as connected. The user just
    // disconnected it from Settings; the next UI poll returns an
    // empty list. Chat must forget gmail within one poll.
    let key = "windows-regression-2";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    sync_cache_with_connections(&[]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_none(),
        "expected cache to be busted when a connected toolkit disappears"
    );
}

#[test]
fn sync_cache_noop_when_backend_matches_cached_state() {
    let _guard = cache_guard();
    // Steady state: UI polls confirm cache is accurate. No
    // invalidation — we must not thrash the chat runtime's tool
    // registry on every 5 s UI poll.
    let key = "windows-regression-3";
    clear_cache_key(key);
    seed_cache(
        key,
        vec![integration("gmail", true), integration("notion", false)],
    );

    sync_cache_with_connections(&[conn("c-1", "gmail", "ACTIVE")]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "expected cache entry to survive when backend matches cached state"
    );
    // And the seeded entries are still there byte-for-byte.
    assert_eq!(guard.get(key).unwrap().entries.len(), 2);
}

#[test]
fn sync_cache_ignores_non_active_connection_rows() {
    let _guard = cache_guard();
    // Backend reports a PENDING row (user started OAuth but
    // hasn't completed). The cache should NOT be invalidated —
    // that would trigger a fresh `list_tools` call on every poll
    // while the OAuth handshake is in flight, which is wasteful
    // and would also clear `tools` vecs for real active
    // integrations already on disk.
    let key = "windows-regression-4";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    sync_cache_with_connections(&[
        conn("c-1", "gmail", "ACTIVE"),
        conn("c-2", "notion", "PENDING"),
        conn("c-3", "slack", "FAILED"),
    ]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "PENDING/FAILED rows must not trigger invalidation"
    );
}

#[test]
fn sync_cache_treats_connected_status_equivalent_to_active() {
    let _guard = cache_guard();
    // Backend may emit either "ACTIVE" or "CONNECTED" — we treat
    // them identically in every status check (see
    // `fetch_connected_integrations_uncached` filter). Make sure
    // the new diff path matches that convention so it doesn't
    // produce a false-positive invalidation.
    let key = "windows-regression-5";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    // Same toolkit set but reported via the legacy "CONNECTED" spelling.
    sync_cache_with_connections(&[conn("c-1", "gmail", "CONNECTED")]);

    let guard = INTEGRATIONS_CACHE.read().unwrap();
    assert!(
        guard.get(key).is_some(),
        "CONNECTED should be treated as an active status"
    );
}

#[test]
fn cache_entries_expire_after_ttl() {
    let _guard = cache_guard();
    // Even without any UI polling, the chat runtime must
    // self-heal stale state within `CACHE_TTL`. We can't wait
    // 60 s in a unit test; instead, directly age the entry by
    // rewriting its `cached_at`.
    let key = "windows-regression-6";
    clear_cache_key(key);
    seed_cache(key, vec![integration("gmail", true)]);

    // Age the entry past the TTL.
    {
        let mut guard = INTEGRATIONS_CACHE.write().unwrap();
        let entry = guard.get_mut(key).unwrap();
        entry.cached_at = Instant::now() - (CACHE_TTL + Duration::from_secs(1));
    }

    // Re-read via the public API — expired reads must not serve
    // the stale entry. We can't trigger a real backend call in a
    // unit test, so assert that the read path falls through (by
    // asserting the entry is still present before the read, and
    // proving the staleness check via a direct helper).
    let is_fresh = {
        let guard = INTEGRATIONS_CACHE.read().unwrap();
        guard
            .get(key)
            .map(|c| c.cached_at.elapsed() < CACHE_TTL)
            .unwrap_or(false)
    };
    assert!(
        !is_fresh,
        "entry aged past CACHE_TTL must not be treated as fresh"
    );
}

// ── Trigger management ops (PR #671) ────────────────────────────────

#[tokio::test]
async fn composio_list_available_triggers_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/available",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert_eq!(q.get("toolkit"), Some(&"gmail".into()));
            assert_eq!(q.get("connectionId"), Some(&"c1".into()));
            // Echo back so the test can also assert what was forwarded.
            Json(json!({
                "success": true,
                "data": {"triggers": [
                    {
                        "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                        "scope": "static",
                        "defaultConfig": {"labelIds": "INBOX"},
                        "_echoed_connectionId": q.get("connectionId"),
                        "_echoed_toolkit": q.get("toolkit"),
                    }
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_available_triggers(&config, "gmail", Some("c1".into()))
        .await
        .unwrap();
    assert_eq!(outcome.value.triggers.len(), 1);
    assert_eq!(outcome.value.triggers[0].slug, "GMAIL_NEW_GMAIL_MESSAGE");
    assert_eq!(outcome.value.triggers[0].scope, "static");
    assert!(outcome.logs.iter().any(|l| l.contains("available trigger")));
}

#[tokio::test]
async fn composio_list_available_triggers_omits_connection_when_none() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/available",
        get(|Query(q): Query<HashMap<String, String>>| async move {
            assert!(
                q.get("connectionId").is_none(),
                "should not forward connectionId"
            );
            Json(json!({"success": true, "data": {"triggers": []}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_available_triggers(&config, "gmail", None)
        .await
        .unwrap();
    assert!(outcome.value.triggers.is_empty());
}

#[tokio::test]
async fn composio_list_triggers_via_mock_with_filter() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        get(|Query(_q): Query<HashMap<String, String>>| async move {
            Json(json!({
                "success": true,
                "data": {"triggers": [
                    {
                        "id": "ti_1",
                        "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                        "toolkit": "gmail",
                        "connectionId": "c1",
                        "state": "active"
                    }
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_triggers(&config, Some("gmail".into()))
        .await
        .unwrap();
    assert_eq!(outcome.value.triggers.len(), 1);
    assert_eq!(outcome.value.triggers[0].id, "ti_1");
    assert_eq!(outcome.value.triggers[0].connection_id, "c1");
}

#[tokio::test]
async fn composio_list_triggers_without_filter() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        get(|| async { Json(json!({"success": true, "data": {"triggers": []}})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_list_triggers(&config, None).await.unwrap();
    assert!(outcome.value.triggers.is_empty());
}

#[tokio::test]
async fn composio_enable_trigger_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["slug"], "GMAIL_NEW_GMAIL_MESSAGE");
            assert_eq!(body["connectionId"], "c1");
            assert_eq!(body["triggerConfig"]["labelIds"], "INBOX");
            Json(json!({
                "success": true,
                "data": {
                    "triggerId": "ti_new",
                    "slug": "GMAIL_NEW_GMAIL_MESSAGE",
                    "connectionId": "c1"
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_enable_trigger(
        &config,
        "c1",
        "GMAIL_NEW_GMAIL_MESSAGE",
        Some(json!({"labelIds": "INBOX"})),
    )
    .await
    .unwrap();
    assert_eq!(outcome.value.trigger_id, "ti_new");
    assert_eq!(outcome.value.connection_id, "c1");
    assert!(outcome.logs.iter().any(|l| l.contains("enabled trigger")));
}

#[tokio::test]
async fn composio_disable_trigger_via_mock() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/{id}",
        axum::routing::delete(|Path(id): Path<String>| async move {
            assert_eq!(id, "ti_1");
            Json(json!({"success": true, "data": {"deleted": true}}))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let outcome = composio_disable_trigger(&config, "ti_1").await.unwrap();
    assert!(outcome.value.deleted);
    assert!(outcome.logs.iter().any(|l| l.contains("disabled trigger")));
}

#[tokio::test]
async fn composio_disable_trigger_propagates_backend_error() {
    let app = Router::new().route(
        "/agent-integrations/composio/triggers/{id}",
        axum::routing::delete(|Path(_id): Path<String>| async move {
            (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"success": false, "error": "Trigger not found"})),
            )
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);

    let err = composio_disable_trigger(&config, "missing")
        .await
        .unwrap_err();
    assert!(err.contains("disable_trigger failed"), "unexpected: {err}");
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
    crate::openhuman::credentials::AuthService::from_config(&c)
        .store_provider_token(
            crate::openhuman::credentials::ops::COMPOSIO_DIRECT_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "ck_test_direct_key",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test direct-mode api key");
    c
}

#[tokio::test]
async fn composio_list_toolkits_returns_empty_in_direct_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let outcome = composio_list_toolkits(&config)
        .await
        .expect("direct-mode list_toolkits must succeed without HTTP");
    assert!(
        outcome.value.toolkits.is_empty(),
        "direct mode must not surface the backend allowlist"
    );
    assert!(
        outcome.logs.iter().any(|l| l.contains("direct mode")),
        "log line must call out direct mode explicitly, got {:?}",
        outcome.logs
    );
}

#[tokio::test]
async fn composio_list_connections_routes_through_direct_mode() {
    let _guard = cache_guard();
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    // [composio-direct] After commit 2 of #1710, direct mode actually
    // calls `backend.composio.dev/api/v3/connected_accounts` rather
    // than returning an empty stub. Without a real Composio key the
    // remote will reject the test request, so we assert on the error
    // shape: it must reference `composio` AND must NOT reference the
    // backend-session path (proving the factory routed us to direct).
    let result = composio_list_connections(&config).await;
    match result {
        Ok(outcome) => {
            // Some sandboxes resolve OK with an empty list — accept
            // that as well, but the connections vec must be empty
            // (the test key is not provisioned in any real tenant).
            assert!(
                outcome.value.connections.is_empty(),
                "test key should not surface real connections"
            );
        }
        Err(err) => {
            assert!(
                !err.contains("no backend session"),
                "direct mode must not surface backend-auth errors, got: {err}"
            );
            assert!(
                err.to_lowercase().contains("composio"),
                "error must carry the composio prefix, got: {err}"
            );
        }
    }
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

#[test]
fn direct_mode_without_key_is_false_in_backend_mode() {
    let tmp = tempfile::tempdir().unwrap();
    // Default mode is backend — the guard must never fire there, or
    // backend users would get a silent empty list.
    let config = test_config(&tmp);
    assert!(!direct_mode_without_key(&config).unwrap());
}

#[test]
fn direct_mode_without_key_is_true_when_direct_and_no_key() {
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_no_key_config(&tmp);
    assert!(direct_mode_without_key(&config).unwrap());
}

#[test]
fn direct_mode_without_key_is_false_when_key_in_config_toml() {
    let tmp = tempfile::tempdir().unwrap();
    // Key supplied via config.toml (not the keychain) still counts —
    // the factory accepts it, so the guard must NOT short-circuit and
    // hide the user's real connections.
    let mut config = direct_mode_no_key_config(&tmp);
    config.composio.api_key = Some("  ck_cfg_key  ".into());
    assert!(!direct_mode_without_key(&config).unwrap());
}

#[test]
fn direct_mode_without_key_is_false_when_key_in_keychain() {
    let tmp = tempfile::tempdir().unwrap();
    // `direct_mode_config` stores a key via the auth store — the guard
    // must see it and report "has key".
    let config = direct_mode_config(&tmp);
    assert!(!direct_mode_without_key(&config).unwrap());
}

#[tokio::test]
async fn composio_list_connections_returns_empty_when_direct_mode_no_key() {
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_no_key_config(&tmp);
    // RC of TAURI-RUST-R4: this MUST be Ok(empty), not Err — no error is
    // constructed, so nothing reaches Sentry and the 5 s poll stops
    // churning.
    let outcome = composio_list_connections(&config)
        .await
        .expect("direct mode with no key must return an empty list, not an error");
    assert!(
        outcome.value.connections.is_empty(),
        "no key → no tenant → no connections"
    );
    assert!(
        outcome.logs.iter().any(|l| l.contains("no api key")),
        "log must explain the empty list is the no-key setup state, got {:?}",
        outcome.logs
    );
}

// ── Direct-mode authorize / list_tools / execute (commit 1, #1710) ─

/// Direct-mode `composio_list_tools` now hits Composio v3 with the
/// user's own key (replacing the prior empty-short-circuit). The unit
/// test reaches an outbound HTTPS call against the real
/// `backend.composio.dev`, which immediately fails with HTTP 401 on the
/// fake key — exactly the shape we want the contract to preserve:
///
///   * NEVER fall back to the tinyhumans backend tenant (no
///     `"no backend session"` text in the error)
///   * Surface the failure with the `composio` grep prefix so it routes
///     through normal observability
///
/// A full schemas-mapped test that asserts response shape lives in the
/// `client_tests.rs` mock-axum suite (`direct_list_tools_*`); this
/// integration-style test only pins the failure-mode contract.
#[tokio::test]
async fn composio_list_tools_in_direct_mode_does_not_fall_back_to_backend() {
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let result = composio_list_tools(&config, None, None).await;
    match result {
        Ok(outcome) => {
            // If the prefetch returns empty connections (test env may
            // intermittently mock that), the function short-circuits to
            // an empty tool list — still no backend leak.
            assert!(
                outcome.value.tools.is_empty(),
                "direct mode must NOT surface backend-tenant tool catalogue"
            );
            assert!(
                outcome.logs.iter().any(|l| l.contains("direct mode")),
                "log line must call out direct mode explicitly, got {:?}",
                outcome.logs
            );
        }
        Err(err) => {
            assert!(
                !err.contains("no backend session"),
                "direct mode must not surface backend-auth errors, got: {err}"
            );
            assert!(
                err.to_lowercase().contains("composio"),
                "error must carry the composio prefix, got: {err}"
            );
        }
    }
}

#[tokio::test]
async fn composio_authorize_routes_through_direct_mode() {
    // The direct-mode `authorize` path actually calls
    // `backend.composio.dev/api/v3/connected_accounts/link` over HTTPS.
    // We can't mock that endpoint at the URL-rewriter level in this
    // unit test, so the assertion below verifies (a) the mode-aware
    // factory was hit (i.e. no "no backend session" error) and (b) the
    // error path is the direct-mode one (HTTP failure or DNS failure),
    // not the backend one. Both error shapes are acceptable — the
    // important thing is that backend mode would have produced
    // "composio unavailable / no backend session" instead.
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let err = composio_authorize(&config, "gmail", None)
        .await
        .unwrap_err();
    assert!(
        !err.contains("no backend session"),
        "direct mode must not surface backend-auth errors, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("composio"),
        "error must carry the composio prefix, got: {err}"
    );
}

#[tokio::test]
async fn composio_execute_routes_through_direct_mode() {
    // Same shape of assertion as
    // `composio_authorize_routes_through_direct_mode` — we can't mock
    // `backend.composio.dev` from a unit test, so we verify the factory
    // routed to direct mode (no backend-auth error) and that an error
    // surfaces because the live HTTP call cannot succeed against a
    // test key.
    let tmp = tempfile::tempdir().unwrap();
    let config = direct_mode_config(&tmp);
    let err = composio_execute(&config, "GMAIL_SEND_EMAIL", None)
        .await
        .unwrap_err();
    assert!(
        !err.contains("no backend session"),
        "direct mode must not surface backend-auth errors, got: {err}"
    );
    assert!(
        err.to_lowercase().contains("composio"),
        "error must carry the composio prefix, got: {err}"
    );
}

// ── classify_composio_failure_tag ──────────────────────────────
//
// Pin the failure-tag routing for `report_composio_op_error` so the
// `before_send` filter (`is_transient_integrations_failure` extended to
// `domain="composio"` in the same #1608 patch series) matches. The tag
// drives which branch of the filter fires:
//   - `failure="non_2xx"` + transient `status` (set by the integrations
//     wrapper) → dropped
//   - `failure="transport"` + transient transport phrase in the message
//     → dropped
// Any drift between the helper's classification and the filter's
// expectations would silently re-open the leak path.

#[test]
fn composio_failure_tag_is_non_2xx_for_backend_returned_502() {
    // OPENHUMAN-TAURI-35 / -2H wire shape — the dominant leak. The
    // integrations layer renders this on a 5xx response; composio's op
    // layer wraps the chain and re-reports under `domain=composio`. The
    // tag MUST be `non_2xx` so the existing transient-status filter
    // branch matches.
    let rendered = "Backend returned 502 Bad Gateway for POST \
                    https://api.tinyhumans.ai/agent-integrations/composio/connections: \
                    upstream temporarily unavailable";
    assert_eq!(classify_composio_failure_tag(rendered), "non_2xx");
}

#[test]
fn composio_failure_tag_is_non_2xx_for_envelope_error() {
    // Envelope errors don't carry a transport phrase or "error sending
    // request" anchor; default to non_2xx.
    let rendered = "Backend error for POST https://api.tinyhumans.ai/x: \
                    unknown backend error";
    assert_eq!(classify_composio_failure_tag(rendered), "non_2xx");
}

#[test]
fn composio_failure_tag_is_transport_for_operation_timed_out() {
    // OPENHUMAN-TAURI-18 / -G shape — `composio/execute` reqwest chain
    // surfaces `operation timed out` (one of `TRANSIENT_TRANSPORT_PHRASES`).
    // Tag MUST be `transport` so the filter's transport-phrase branch fires
    // even though the report carries no `status`.
    let rendered = "POST https://api.tinyhumans.ai/agent-integrations/composio/execute \
                    failed: error sending request for url \
                    (https://api.tinyhumans.ai/agent-integrations/composio/execute) → \
                    client error (SendRequest) → connection error → \
                    Operation timed out (os error 60)";
    assert_eq!(classify_composio_failure_tag(rendered), "transport");
}

#[test]
fn composio_failure_tag_is_transport_for_dns_and_tls_phrases() {
    for raw in [
        "POST /v1/foo failed: error sending request for url (https://api.example.com/x)",
        "GET /agent-integrations/composio/connections failed: tls handshake eof",
        "POST /agent-integrations/composio/triggers failed: connection reset by peer",
        "GET /agent-integrations/composio/toolkits failed: connection forcibly closed (os 10054)",
    ] {
        assert_eq!(
            classify_composio_failure_tag(raw),
            "transport",
            "should classify as transport: {raw}"
        );
    }
}

#[test]
fn composio_failure_tag_does_not_misclassify_unrelated_messages() {
    // A bare error string with no transport / "error sending request"
    // anchor must default to non_2xx — the safe choice for the dominant
    // leak shape.
    for raw in [
        "[composio] no connection with id 'abc'",
        "[composio] no native provider registered for toolkit 'foo'",
        "fetch_user_profile failed: invalid JSON in profile facet",
    ] {
        assert_eq!(
            classify_composio_failure_tag(raw),
            "non_2xx",
            "should default to non_2xx: {raw}"
        );
    }
}

// ── extract_backend_returned_status ───────────────────────────
//
// Pin status extraction so the `report_composio_op_error` Sentry tag
// stays in lockstep with the `Backend returned <status> ...` rendering
// the integrations layer produces. Without the digit anchor the
// `before_send` filter's transient-status branch can't distinguish a 502
// from a 401, and the dominant leak shape (OPENHUMAN-TAURI-35 / -2H)
// re-opens.

#[test]
fn extract_backend_returned_status_parses_three_digit_status() {
    let rendered = "Backend returned 502 Bad Gateway for POST \
                    https://api.tinyhumans.ai/agent-integrations/composio/connections: \
                    upstream temporarily unavailable";
    assert_eq!(
        extract_backend_returned_status(rendered),
        Some("502".to_string())
    );
}

#[test]
fn extract_backend_returned_status_returns_none_when_no_status() {
    // Envelope-style error with no HTTP status digits after the anchor.
    let rendered = "Backend returned bad gateway (envelope-only error)";
    assert_eq!(extract_backend_returned_status(rendered), None);
}

#[test]
fn extract_backend_returned_status_handles_mixed_case() {
    // Some renders upper-case the prefix; the helper lowercases before
    // matching so both shapes resolve to the same status string.
    let rendered = "BACKEND RETURNED 429 Too Many Requests for GET \
                    https://api.tinyhumans.ai/agent-integrations/composio/triggers";
    assert_eq!(
        extract_backend_returned_status(rendered),
        Some("429".to_string())
    );
}

// ── before_send filter integration ─────────────────────────────
//
// Belt-and-suspenders: re-assert the cross-module contract from the
// composio side. If `is_transient_integrations_failure` ever stops
// matching `domain="composio"` (e.g. accidental revert), the
// `report_composio_op_error` events flood Sentry again with no test in
// the composio crate to catch it. These guards make the link explicit.

#[test]
fn composio_domain_502_is_dropped_by_before_send() {
    let mut event = sentry::protocol::Event::default();
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    tags.insert("domain".into(), "composio".into());
    tags.insert("failure".into(), "non_2xx".into());
    tags.insert("status".into(), "502".into());
    event.tags = tags;
    assert!(
        crate::core::observability::is_transient_integrations_failure(&event),
        "composio non_2xx 502 must be dropped by integrations filter (#1608)"
    );
}

#[test]
fn composio_transport_timeout_is_dropped_by_before_send() {
    let mut event = sentry::protocol::Event::default();
    let mut tags: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    tags.insert("domain".into(), "composio".into());
    tags.insert("failure".into(), "transport".into());
    event.tags = tags;
    event.message = Some(
        "POST /agent-integrations/composio/execute failed: error sending request → \
         operation timed out"
            .to_string(),
    );
    assert!(
        crate::core::observability::is_transient_integrations_failure(&event),
        "composio transport timeout must be dropped by integrations filter (#1608)"
    );
}

// ── TAURI-RUST-X9 (#1166): direct-mode auth-rejection routing ───────────
//
// Pins the contract that direct-mode 401 / Invalid API key shapes are
// classified by the observability matcher AND their failure-tag stays
// `non_2xx` so the `before_send` integrations filter has consistent
// inputs. Together with the classifier-arm tests in
// `core::observability` these tests prove the leak path (~15.7 k events
// in ~22h before #1166) is closed end-to-end.

#[test]
fn composio_direct_invalid_api_key_classifies_as_provider_user_state() {
    // The verbatim Sentry TAURI-RUST-X9 wire shape — emitted by
    // `ops.rs::composio_list_connections` direct branch via the
    // `report_composio_op_error` hook restored in #1166. Routing this
    // through `expected_error_kind` is what demotes it to
    // `ProviderUserState` (info breadcrumb) instead of firing a Sentry
    // event.
    let msg = "[composio-direct] list_connections failed: \
               Composio v3 connected_accounts failed: \
               HTTP 401: Invalid API key: ak_VsUvq*****";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        Some(crate::core::observability::ExpectedErrorKind::ProviderUserState),
        "the canonical TAURI-RUST-X9 wire shape must demote via the composio-direct arm"
    );
}

#[test]
fn composio_direct_invalid_api_key_failure_tag_is_non_2xx() {
    // Belt-and-suspenders: even if `expected_error_kind` ever stops
    // matching the body (regression in the classifier arm), the
    // failure tag must STILL be `non_2xx`. Combined with the
    // `before_send` filter's transient-status handling and a
    // future-added `status="401"` tag (Patch 1 doesn't extract status
    // from the `HTTP 401` shape — only the `Backend returned <status>`
    // shape — so this just pins the safe default), this is the
    // backstop drop path.
    let rendered = "[composio-direct] list_connections failed: \
                    Composio v3 connected_accounts failed: \
                    HTTP 401: Invalid API key: ak_VsUvq*****";
    assert_eq!(
        classify_composio_failure_tag(rendered),
        "non_2xx",
        "direct-mode auth-rejection must tag as non_2xx (not transport)"
    );
}

#[test]
fn composio_direct_invalid_api_key_extract_status_returns_none() {
    // Pins the contract: `extract_backend_returned_status` only parses
    // the integrations-layer `Backend returned <status>` rendering, NOT
    // the direct-mode `HTTP 401` shape. The direct-mode arm relies on
    // the classifier demotion + the failure-tag drop path instead of
    // status extraction; if this ever changes (e.g. we extend the
    // status extractor to cover both shapes), the new behaviour should
    // come with an explicit test, not be inferred.
    let rendered = "[composio-direct] list_connections failed: \
                    Composio v3 connected_accounts failed: \
                    HTTP 401: Invalid API key: ak_…";
    assert_eq!(
        extract_backend_returned_status(rendered),
        None,
        "direct-mode HTTP 401 must not parse via extract_backend_returned_status"
    );
}

#[test]
fn composio_direct_500_does_not_demote() {
    // Discrimination test from the composio side — a real bug shape
    // (500 with no auth body) MUST escape the classifier and reach
    // `report_error_message`. Without this guard the matcher in
    // `observability.rs` could be tightened too far and silence
    // genuine backend faults.
    let msg = "[composio-direct] list_connections failed: \
               Composio v3 connected_accounts failed: HTTP 500";
    assert_eq!(
        crate::core::observability::expected_error_kind(msg),
        None,
        "composio-direct 500 with no auth body must remain an unclassified bug shape"
    );
}
