//! Boot over a backend holding the host domains' v1 collections: every record
//! arrives in its v2 collection and shape, the v1 collections are emptied, and
//! the import markers an older build wrote still count as finished imports.

use std::sync::Arc;

use serde_json::{json, Value};
use tinystoragedrivers::schema::Migrations;
use tinystoragedrivers::{
    DocumentStore, DocumentStoreExt, Kv, MemoryStorage, Precondition, Query, Scope, ScopedStorage,
    StorageBackend,
};

use super::boot::prepare_and_migrate;
use super::import_state::{migrate_legacy_markers, step_id};

async fn seed(docs: &Arc<dyn DocumentStore>, collection: &str, id: &str, body: Value) {
    docs.ensure_collection(&tinystoragedrivers::CollectionSpec::new(collection))
        .await
        .unwrap();
    docs.put(collection, id, body, Precondition::Absent)
        .await
        .unwrap();
}

async fn get(docs: &Arc<dyn DocumentStore>, collection: &str, id: &str) -> Value {
    docs.get(collection, id)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{collection}/{id} is missing"))
        .doc
}

async fn rows(docs: &Arc<dyn DocumentStore>, collection: &str) -> usize {
    docs.query_all(collection, &Query::all())
        .await
        .map_or(0, |rows| rows.len())
}

fn local(backend: &Arc<dyn StorageBackend>) -> ScopedStorage {
    backend.for_scope(&Scope::local()).unwrap()
}

#[tokio::test]
async fn the_host_domains_v1_records_arrive_in_the_v2_collections() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let scoped = local(&backend);
    let docs = Arc::clone(scoped.documents());

    seed(
        &docs,
        "approvals",
        "req-1",
        json!({
            "tool_name": "shell", "action_summary": "ls", "args_redacted": "{}",
            "session_id": "s", "created_at": "1970-01-01T00:00:01Z",
            "expires_at": "1970-01-01T00:00:05Z", "expires_ts": 5_000_000_000_i64,
            "pending": true,
        }),
    )
    .await;
    seed(
        &docs,
        "approval_flow_trust",
        "4:flow/tool",
        json!({ "flow_id": "flow", "tool_name": "tool", "created_at": "1970-01-01T00:00:02Z" }),
    )
    .await;
    seed(
        &docs,
        "paired_devices",
        "chan-1",
        json!({
            "label": "Phone", "device_pubkey": "pk", "core_session_token_hash": "h",
            "created_at": "1970-01-01T00:00:03Z", "last_seen_at": null, "revoked": false,
        }),
    )
    .await;
    seed(
        &docs,
        "integration_notifications",
        "n1",
        json!({
            "provider": "slack", "title": "t", "body": "b", "raw_payload": "{}",
            "status": "unread", "received_at": "1970-01-01T00:00:04Z", "received_ms": 4000,
        }),
    )
    .await;
    seed(&docs, "notification_dedup", "abc", json!({ "last_ms": 9 })).await;
    seed(
        &docs,
        "notification_settings",
        "slack",
        json!({ "enabled": true, "importance_threshold": 0.5, "route_to_orchestrator": true }),
    )
    .await;
    seed(
        &docs,
        "core_notifications",
        "5:local/e1",
        json!({
            "workspace": "local", "payload": "{}", "timestamp_ms": 6, "read": false,
            "created_at": "1970-01-01T00:00:05Z",
        }),
    )
    .await;
    seed(
        &docs,
        "task_sources",
        "src",
        json!({
            "provider": "linear", "enabled": true, "filter": "{}", "interval_secs": 60,
            "target": "{}", "max_tasks_per_fetch": 5,
            "created_at": "1970-01-01T00:00:06Z", "created_ms": 6000,
            "last_fetch_at": "1970-01-01T00:00:07Z",
        }),
    )
    .await;
    seed(
        &docs,
        "ingested_tasks",
        "3:src/ext",
        json!({
            "source_id": "src", "external_id": "ext", "content_hash": "h",
            "ingested_at": "1970-01-01T00:00:08Z", "ingested_ms": 8000, "ingested_ns": 8_000_000_000_i64,
        }),
    )
    .await;
    seed(
        &docs,
        "artifacts",
        "art-1",
        json!({
            "id": "art-1", "kind": "document", "title": "T", "path": "p", "size_bytes": 1,
            "status": "ready", "created_at": "1970-01-01T00:00:09Z", "created_ms": 9000,
        }),
    )
    .await;
    seed(
        &docs,
        "artifact_args",
        "art-1",
        json!({ "args": { "prompt": "x" } }),
    )
    .await;
    seed(
        &docs,
        "artifact_versions",
        "art-1",
        json!({
            "id": "art-1", "updatedAtMillis": 10, "updated_ms": 10,
            "versions": [{ "authorId": "a", "createdAtMillis": 1 }],
        }),
    )
    .await;
    seed(
        &docs,
        "cost_records",
        "c1",
        json!({
            "id": "c1", "session_id": "s", "ts_ms": 11000,
            "usage": { "model": "m", "timestamp": "1970-01-01T00:00:11Z" },
        }),
    )
    .await;
    seed(&docs, "config", "local", json!({ "toml_enc": "enc2:00" })).await;
    seed(
        &docs,
        "subagent_sessions",
        "sub-1",
        json!({
            "subagentSessionId": "sub-1", "parentSession": "p", "agentId": "a",
            "sandboxMode": "workspace", "taskKey": "k", "taskTitle": "T", "status": "idle",
            "reusable": true, "createdAt": "1970-01-01T00:00:12Z",
            "updatedAt": "1970-01-01T00:00:13Z", "lastUsedAt": "1970-01-01T00:00:13Z",
        }),
    )
    .await;
    seed(
        &docs,
        "threads_transcript_keys",
        "thread-1",
        json!({ "key": "agent" }),
    )
    .await;
    seed(&docs, "storage_agents", "asha", json!({})).await;
    seed(
        &docs,
        "composio_state",
        "composio_identities.json",
        json!({ "value": { "gmail": 3 } }),
    )
    .await;
    seed(
        &docs,
        "storage_import",
        "approval@abcd",
        json!({ "version": 1 }),
    )
    .await;
    seed(
        &docs,
        "threads_import",
        "legacy-conversations:k",
        json!({ "threads": 1 }),
    )
    .await;

    let report = prepare_and_migrate(&backend).await.unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);

    let request = get(&docs, "approvals.requests", "req-1").await;
    assert_eq!(request["created_ms"], 1000);
    assert_eq!(request["deadline_ms"], 5000);
    assert_eq!(request["pending"], true);
    assert_eq!(
        get(&docs, "approvals.flow_trust", "flow/tool").await,
        json!({ "flow_id": "flow", "tool_name": "tool", "created_ms": 2000 })
    );
    assert_eq!(
        get(&docs, "devices.paired", "chan-1").await["created_ms"],
        3000
    );
    assert_eq!(
        get(&docs, "notifications.integration", "n1").await["received_ms"],
        4000
    );
    assert_eq!(get(&docs, "notifications.dedup", "abc").await["last_ms"], 9);
    assert_eq!(
        get(&docs, "notifications.settings", "slack").await["enabled"],
        true
    );
    assert_eq!(
        get(&docs, "notifications.core", "local/e1").await["timestamp_ms"],
        6
    );
    assert_eq!(
        get(&docs, "task_sources.sources", "src").await["last_fetch_ms"],
        7000
    );
    assert_eq!(
        get(&docs, "task_sources.ingested", "src/ext").await["ingested_ms"],
        8000
    );
    assert!(get(&docs, "artifacts.records", "art-1")
        .await
        .get("created_at")
        .is_none());
    assert_eq!(
        get(&docs, "artifacts.args", "art-1").await,
        json!({ "prompt": "x" })
    );
    assert_eq!(
        get(&docs, "artifacts.versions", "art-1").await["versions"][0]["created_ms"],
        1
    );
    assert!(get(&docs, "cost.records", "c1").await["usage"]
        .get("timestamp")
        .is_none());
    assert_eq!(
        get(&docs, "config.documents", "local").await,
        json!({ "toml_enc": "enc2:00" })
    );
    assert_eq!(
        get(&docs, "subagents.sessions", "sub-1").await["created_ms"],
        12000
    );
    assert_eq!(
        get(&docs, "threads.transcript_keys", "thread-1").await,
        json!({ "key": "agent" })
    );
    assert_eq!(
        get(&docs, "agents.registry", "asha").await,
        json!({ "agent_id": "asha" })
    );
    // `composio_state` moved into tsd.kv, unwrapped.
    let kv = Kv::ns(&docs, "composio", "state").unwrap();
    assert_eq!(
        kv.get::<Value>("composio_identities.json").await.unwrap(),
        Some(json!({ "gmail": 3 }))
    );

    // The v1 collections are empty.
    for old in [
        "approvals",
        "approval_flow_trust",
        "paired_devices",
        "integration_notifications",
        "core_notifications",
        "task_sources",
        "ingested_tasks",
        "artifacts",
        "artifact_args",
        "artifact_versions",
        "cost_records",
        "config",
        "subagent_sessions",
        "threads_transcript_keys",
        "storage_agents",
        "composio_state",
    ] {
        assert_eq!(rows(&docs, old).await, 0, "{old} still holds records");
    }

    // The older builds' markers are finished imports now.
    let migrations = Migrations::new(&scoped);
    assert!(migrations
        .done("storage", &step_id("storage_import", "approval@abcd"))
        .await
        .unwrap());
    assert!(migrations
        .done(
            "threads",
            &step_id("threads_import", "legacy-conversations:k")
        )
        .await
        .unwrap());

    // A second boot has nothing left to do.
    let again = prepare_and_migrate(&backend).await.unwrap();
    assert!(again.failures.is_empty(), "{:?}", again.failures);
    assert_eq!(again.steps_run, 0);
    assert_eq!(migrate_legacy_markers(&scoped).await.unwrap(), 0);
}
