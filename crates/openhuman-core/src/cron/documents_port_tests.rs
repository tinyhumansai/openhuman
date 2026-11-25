use super::*;
use crate::storage::{test_support::memory_backend, Scope, StorageBackend};
use serde_json::json;
use tinyflows_bus::schedule::{DOCUMENT_INTERFACE, DOCUMENT_METHOD};

async fn port() -> (tinybus::Proxy, CallbackAddress) {
    let (proxy, address, _) = port_with_backend().await;
    (proxy, address)
}

async fn port_with_backend() -> (tinybus::Proxy, CallbackAddress, Arc<dyn StorageBackend>) {
    let config = Config::default();
    // Start the module runtime and attach the module before touching the bus.
    module::proxy(&config).await.expect("module attaches");
    let backend = memory_backend();
    let scope = DocScope::on_backend(&config, Arc::clone(&backend), Scope::local());
    let address = address(&scope).await.expect("port is served");
    let runtime = crate::modules::runtime::runtime().await.unwrap();
    let proxy = runtime
        .connection()
        .proxy(
            &address.destination,
            &address.object_path,
            DOCUMENT_INTERFACE,
        )
        .expect("the contract's document interface is the one served");
    (proxy, address, backend)
}

async fn ask(proxy: &tinybus::Proxy, request: Value) -> Value {
    proxy.call(DOCUMENT_METHOD, (request,)).await.unwrap()
}

#[tokio::test]
async fn the_port_serves_the_contracts_document_interface_and_reports_capabilities() {
    let (proxy, _) = port().await;
    let reply = ask(&proxy, json!({"op": "capabilities"})).await;
    assert!(reply["value"]["transactions"].is_boolean(), "{reply}");
    assert_eq!(reply["value"]["record_metadata"], true, "{reply}");
}

#[tokio::test]
async fn schedule_store_migration_moves_legacy_jobs_runs_and_counter_before_reads() {
    use tinyflows_bus::schedule::StoreOperation;
    use tinystoragedrivers::Precondition;

    let config = Config::default();
    let (_port, _callback, backend) = port_with_backend().await;
    let scoped = backend.for_scope(&Scope::local()).unwrap();
    let docs = scoped.documents();
    for collection in ["cron_jobs", "cron_runs", "cron_counters"] {
        docs.ensure_collection(&CollectionSpec::new(collection))
            .await
            .unwrap();
    }
    docs.put(
        "cron_jobs",
        "legacy-job",
        json!({
            "claim_generation": 3,
            "expression": "0 9 * * *",
            "command": "echo legacy",
            "schedule": "{\"kind\":\"every\",\"every_ms\":3600000}",
            "job_type": "shell",
            "session_target": "isolated",
            "enabled": true,
            "delivery": "{\"mode\":\"none\",\"channel\":null,\"to\":null,\"best_effort\":true}",
            "delete_after_run": false,
            "created_at": "2026-03-04T05:06:07.123Z",
            "created_ms": 1772600767123_i64,
            "next_run": "2026-03-05T09:00:00Z",
            "next_run_ms": 1772701200000_i64,
            "next_run_ns": 1772701200000000000_i64,
            "name": "legacy-job"
        }),
        Precondition::None,
    )
    .await
    .unwrap();
    docs.put(
        "cron_runs",
        "00000000000000000007",
        json!({
            "seq": 7,
            "job_id": "legacy-job",
            "started_at": "2026-03-04T09:00:00Z",
            "started_ns": 1772614800000000000_i64,
            "finished_at": "2026-03-04T09:00:02Z",
            "status": "success",
            "duration_ms": 2000,
            "output": "legacy output"
        }),
        Precondition::None,
    )
    .await
    .unwrap();
    docs.put(
        "cron_counters",
        "runs",
        json!({"next": 8}),
        Precondition::None,
    )
    .await
    .unwrap();

    let scope = DocScope::on_backend(&config, Arc::clone(&backend), Scope::local());
    let listed: Vec<tinyflows_bus::schedule::types::CronJob> =
        crate::cron::client::store_in(&config, &scope, StoreOperation::List)
            .await
            .expect("the first store read migrates the legacy schedule schema");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "legacy-job");
    assert_eq!(
        listed[0].schedule,
        tinyflows_bus::schedule::types::Schedule::Every {
            every_ms: 3_600_000
        }
    );

    let runs: Vec<tinyflows_bus::schedule::types::CronRun> = crate::cron::client::store_in(
        &config,
        &scope,
        StoreOperation::ListRuns {
            job_id: "legacy-job".into(),
            limit: 10,
        },
    )
    .await
    .expect("the migrated run remains readable through the module store");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].output.as_deref(), Some("legacy output"));

    // Schedule schema v2 keeps a job's creation time in its body
    // (`created_ms`), which every read prefers; the record's own metadata time
    // is the migration's write.
    let migrated_job = docs.get("cron.jobs", "legacy-job").await.unwrap().unwrap();
    assert_eq!(migrated_job.doc["created_ms"], 1772600767123_i64);
    assert_eq!(listed[0].created_at.timestamp_millis(), 1772600767123_i64);
    for (collection, id) in [
        ("cron_jobs", "legacy-job"),
        ("cron_runs", "00000000000000000007"),
        ("cron_counters", "runs"),
    ] {
        assert!(
            docs.get(collection, id).await.unwrap().is_none(),
            "successful migration retires {collection}/{id}"
        );
    }
    // v1 kept the next run number to hand out; the v2 counter (`{value}`)
    // holds the last one issued, so the next run is still number 8.
    assert_eq!(
        docs.get("tsd.counters", "cron/runs")
            .await
            .unwrap()
            .expect("the imported counter remains readable")
            .doc["value"],
        7
    );
    for progress in [
        "cron/cron.jobs/jobs_v2",
        "cron/cron.runs/runs_v2",
        "cron/cron.admission/admission_v2",
        "cron/counters_import",
    ] {
        assert!(
            docs.get("tsd.migrations", progress)
                .await
                .unwrap()
                .is_some(),
            "the module records migration progress at {progress}"
        );
    }
}

#[tokio::test]
async fn shared_schedule_records_are_owner_scoped_and_never_scanned() {
    let (proxy, _, backend) = port_with_backend().await;
    let scoped = backend.for_scope(&Scope::local()).unwrap();
    let docs = scoped.documents();
    for collection in ["tsd.counters", "tsd.leases", "tsd.migrations"] {
        let ensured = ask(&proxy, json!({"op":"ensure","spec":{"name":collection}})).await;
        assert!(ensured.get("error").is_none(), "{collection}: {ensured}");
    }

    let record = ask(
        &proxy,
        json!({
            "op":"put",
            "collection":"tsd.counters",
            "id":"cron/runs",
            "doc":{"next":7},
            "precondition":{"kind":"none"}
        }),
    )
    .await;
    assert!(record.get("error").is_none(), "{record}");
    assert!(docs
        .get("tsd.counters", "cron/runs")
        .await
        .unwrap()
        .is_some());

    let requests = [
        json!({"op":"get","collection":"tsd.counters","id":"other/runs"}),
        json!({"op":"put","collection":"tsd.leases","id":"other/lease","doc":{},"precondition":{"kind":"none"}}),
        json!({"op":"get","collection":"tsd.migrations","id":"cron/unrelated/step"}),
        json!({"op":"query","collection":"tsd.counters","query":{"filter":{"op":"all"}}}),
        json!({"op":"count","collection":"tsd.leases","filter":{"op":"all"}}),
        json!({"op":"delete_where","collection":"tsd.migrations","filter":{"op":"all"}}),
        json!({"op":"get","collection":"tsd.kv","id":"cron/anything"}),
        json!({"op":"atomic_batch","ops":[{"op":"delete","collection":"tsd.counters","id":"other/runs","precondition":{"kind":"none"}}]}),
    ];
    let mut unexpected = Vec::new();
    for request in requests {
        let reply = ask(&proxy, request.clone()).await;
        if reply["error"]["kind"] != "invalid_input" {
            unexpected.push(format!("{request}: {reply}"));
        }
    }

    let denied_batch = ask(
        &proxy,
        json!({"op":"atomic_batch","ops":[
            {"op":"put","collection":"tsd.counters","id":"cron/allowed","doc":{"next":1},"precondition":{"kind":"none"}},
            {"op":"put","collection":"tsd.counters","id":"other/protected","doc":{"next":2},"precondition":{"kind":"none"}}
        ]}),
    )
    .await;
    if denied_batch["error"]["kind"] != "invalid_input" {
        unexpected.push(format!("atomic shared write batch: {denied_batch}"));
    }
    if docs
        .get("tsd.counters", "cron/allowed")
        .await
        .unwrap()
        .is_some()
    {
        unexpected.push("rejected shared atomic batch was partially applied".into());
    }

    let kv_ensure = ask(&proxy, json!({"op":"ensure","spec":{"name":"tsd.kv"}})).await;
    if kv_ensure.get("error").is_some() {
        unexpected.push(format!("tsd.kv ensure-only: {kv_ensure}"));
    }

    let exact_progress = ask(
        &proxy,
        json!({"op":"put","collection":"tsd.migrations","id":"cron/cron.jobs/jobs_v2","doc":{"done":true},"precondition":{"kind":"none"}}),
    )
    .await;
    assert!(exact_progress.get("error").is_none(), "{exact_progress}");
    assert!(
        unexpected.is_empty(),
        "shared schedule port violations: {unexpected:#?}"
    );
}

#[tokio::test]
async fn document_port_preserves_schedule_record_metadata_options() {
    let (proxy, _, backend) = port_with_backend().await;
    let scoped = backend.for_scope(&Scope::local()).unwrap();
    let docs = scoped.documents();
    let ensured = ask(&proxy, json!({"op":"ensure","spec":{"name":"cron.jobs"}})).await;
    assert!(ensured.get("error").is_none(), "{ensured}");

    for (id, created_ms, schema) in [("zero", 0_i64, 2_u64), ("negative", -1_i64, 3_u64)] {
        let reply = ask(
            &proxy,
            json!({
                "op":"put",
                "collection":"cron.jobs",
                "id":id,
                "doc":{"id":id},
                "precondition":{"kind":"none"},
                "schema":schema,
                "created_ms":created_ms,
                "expires_ms":9999999999999_i64
            }),
        )
        .await;
        assert!(reply.get("error").is_none(), "{id}: {reply}");
        let stored = docs.get("cron.jobs", id).await.unwrap().unwrap();
        assert_eq!(stored.meta.schema, schema as u32, "{id}");
        assert_eq!(stored.meta.created_ms, created_ms, "{id}");
        assert_eq!(stored.meta.expires_ms, Some(9999999999999_i64), "{id}");
        let read_back = ask(&proxy, json!({"op":"get","collection":"cron.jobs","id":id})).await;
        assert_eq!(
            read_back["value"]["meta"]["schema"], schema,
            "{id}: {read_back}"
        );
        assert_eq!(
            read_back["value"]["meta"]["created_ms"], created_ms,
            "{id}: {read_back}"
        );
        assert_eq!(
            read_back["value"]["meta"]["expires_ms"], 9999999999999_i64,
            "{id}: {read_back}"
        );
    }
}

#[tokio::test]
async fn a_second_absent_write_comes_back_as_a_conflict_kind() {
    let (proxy, _) = port().await;
    let absent = serde_json::to_value(tinystoragedrivers::Precondition::Absent).unwrap();
    let put =
        json!({"op":"put","collection":"cron.jobs","id":"dup","doc":{"a":1},"precondition":absent});
    // The collection must exist first; the module declares it, the port relays.
    let ensured = ask(&proxy, json!({"op":"ensure","spec":{"name":"cron.jobs"}})).await;
    assert!(ensured.get("error").is_none(), "{ensured}");
    let first = ask(&proxy, put.clone()).await;
    assert!(first.get("error").is_none(), "{first}");
    let second = ask(&proxy, put).await;
    assert_eq!(second["error"]["kind"], "conflict", "{second}");
}

#[tokio::test]
async fn a_collection_outside_the_schedule_port_is_refused() {
    let (proxy, _) = port().await;
    let none = serde_json::to_value(tinystoragedrivers::Precondition::None).unwrap();
    for request in [
        json!({"op":"get","collection":"memory_items","id":"x"}),
        json!({"op":"count","collection":"flows","filter":{}}),
        json!({"op":"ensure","spec":{"name":"secrets"}}),
        json!({"op":"atomic_batch","ops":[
            {"op":"put","collection":"cron.jobs","id":"ok","doc":{},"precondition":none},
            {"op":"delete","collection":"approvals","id":"x","precondition":none}
        ]}),
    ] {
        let reply = ask(&proxy, request.clone()).await;
        assert_eq!(
            reply["error"]["kind"], "invalid_input",
            "{request} -> {reply}"
        );
    }
}

#[tokio::test]
async fn an_unknown_operation_is_refused_not_guessed() {
    let (proxy, _) = port().await;
    let reply = ask(
        &proxy,
        json!({"op":"drop_collection","collection":"cron.jobs"}),
    )
    .await;
    assert_eq!(reply["error"]["kind"], "invalid_input", "{reply}");
}

#[tokio::test]
async fn one_object_is_served_per_scope_and_reused() {
    let config = Config::default();
    module::proxy(&config).await.unwrap();
    let backend: Arc<dyn StorageBackend> = crate::storage::test_support::memory_backend();
    let a = DocScope::on_backend(&config, Arc::clone(&backend), Scope::local());
    let b = DocScope::on_backend(&config, backend, crate::storage::scope_for_agent("other"));
    let (first, again, other) = (
        address(&a).await.unwrap(),
        address(&a).await.unwrap(),
        address(&b).await.unwrap(),
    );
    assert_eq!(first.object_path, again.object_path);
    assert_ne!(first.object_path, other.object_path);
    assert_eq!(first.destination, module::HOST_BUS_NAME);
}
