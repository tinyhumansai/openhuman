use super::*;

fn ensure_memory_client() {
    crate::openhuman::memory::ops::shared_memory_test_workspace();
}

fn unique_namespace(prefix: &str) -> String {
    let short = &uuid::Uuid::new_v4().as_simple().to_string()[..12];
    format!("{prefix}{short}")
}

#[tokio::test]
async fn kv_handlers_roundtrip_scoped_values() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    ensure_memory_client();
    let namespace = unique_namespace("kv-graph-kv");
    let key = format!(
        "state{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    );

    let set = kv_set(KvSetParams {
        namespace: Some(namespace.clone()),
        key: key.clone(),
        value: serde_json::json!({"open": true}),
    })
    .await
    .expect("kv set");
    assert!(set.value);

    let get = kv_get(KvGetDeleteParams {
        namespace: Some(namespace.clone()),
        key: key.clone(),
    })
    .await
    .expect("kv get");
    assert_eq!(get.value, Some(serde_json::json!({"open": true})));

    let listed = kv_list_namespace(super::super::documents::NamespaceOnlyParams {
        namespace: namespace.clone(),
    })
    .await
    .expect("kv list namespace");
    assert!(listed
        .value
        .iter()
        .any(|row| row["key"] == key && row["value"] == serde_json::json!({"open": true})));

    let deleted = kv_delete(KvGetDeleteParams {
        namespace: Some(namespace.clone()),
        key: key.clone(),
    })
    .await
    .expect("kv delete");
    assert!(deleted.value);

    let after = kv_get(KvGetDeleteParams {
        namespace: Some(namespace),
        key,
    })
    .await
    .expect("kv get after delete");
    assert!(after.value.is_none());
}

#[tokio::test]
async fn graph_handlers_roundtrip_relation_rows() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    ensure_memory_client();
    let namespace = unique_namespace("kv-graph-rel");
    let subject = format!(
        "alice{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    );

    let upsert = graph_upsert(GraphUpsertParams {
        namespace: Some(namespace.clone()),
        subject: subject.clone(),
        predicate: "OWNS".into(),
        object: "Atlas".into(),
        attrs: serde_json::json!({"source": "test", "confidence": 0.9}),
    })
    .await
    .expect("graph upsert");
    assert!(upsert.value);

    let queried = graph_query(GraphQueryParams {
        namespace: Some(namespace),
        subject: Some(subject.clone()),
        predicate: Some("OWNS".into()),
    })
    .await
    .expect("graph query");

    assert_eq!(queried.logs, vec!["memory graph queried".to_string()]);
    assert_eq!(queried.value.len(), 1);
    assert_eq!(queried.value[0]["predicate"], "OWNS");
    // Case-insensitively, because entity-name casing is the *driver's* policy
    // and not the contract's. `MemoryGraph::put_relation` keys a relation on
    // `(namespace, subject, predicate, object)` and says nothing about
    // normalising any of them; TinyCortex upper-cases entity names, and this
    // assertion used to spell `subject.to_uppercase()` / `"ATLAS"` — pinning
    // one engine's normalisation from the host, where a second driver that
    // preserved case would fail a test about handler wiring.
    //
    // What is worth pinning here is that the handler round-trips the edge it
    // was given into the right fields, and that is what this now checks.
    assert_eq!(
        queried.value[0]["subject"]
            .as_str()
            .expect("subject is a string")
            .to_ascii_lowercase(),
        subject.to_ascii_lowercase()
    );
    assert_eq!(
        queried.value[0]["object"]
            .as_str()
            .expect("object is a string")
            .to_ascii_lowercase(),
        "atlas"
    );
}

/// The guarded `kv_set` must land in the **same** module-backed provider
/// returned by the shared memory API. This is the failure a re-point can
/// hide: routing through a binding over a different workspace still
/// returns `Ok`, it just writes somewhere nobody reads.
#[tokio::test]
async fn kv_set_through_the_guard_is_visible_to_the_memory_api() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    ensure_memory_client();
    let namespace = unique_namespace("kv-guard");
    let key = format!(
        "guarded{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    );

    kv_set(KvSetParams {
        namespace: Some(namespace.clone()),
        key: key.clone(),
        value: serde_json::json!({"via": "guard"}),
    })
    .await
    .expect("guarded kv set");

    let guard = active_memory_guard().await.expect("guard");
    let graph = guard.inner().as_graph().expect("graph family");
    let raw = graph
        .kv_get(Some(namespace.as_str()), &key)
        .await
        .expect("module-backed kv get");
    assert_eq!(
        raw.map(|record| record.value),
        Some(serde_json::json!({"via": "guard"}))
    );
}
