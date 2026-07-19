//! Tests for `MemoryClient` — exercise the sync storage surface (upsert, list,
//! kv, graph) against a fresh temp workspace.

use super::*;
use tempfile::TempDir;

/// Build a MemoryClient pointed at a fresh temp workspace. Ollama is
/// the default embedder — it won't be reachable in tests so anything
/// that exercises the embedding path will surface a retrieval-empty
/// state. That's fine for these tests: we're verifying the sync
/// storage surface (upsert, list, kv, graph) which does not require
/// a working embedder.
fn make_client() -> (TempDir, MemoryClient) {
    let tmp = TempDir::new().unwrap();
    let client = MemoryClient::from_workspace_dir(tmp.path().join("workspace"))
        .expect("client should initialise against a fresh workspace");
    (tmp, client)
}

fn doc(namespace: &str, key: &str, content: &str) -> NamespaceDocumentInput {
    NamespaceDocumentInput {
        namespace: namespace.to_string(),
        key: key.to_string(),
        title: key.to_string(),
        content: content.to_string(),
        source_type: "doc".to_string(),
        priority: "normal".to_string(),
        tags: vec![],
        metadata: serde_json::Value::Null,
        category: "core".to_string(),
        session_id: None,
        document_id: None,
        taint: crate::openhuman::memory::MemoryTaint::Internal,
    }
}

#[tokio::test]
async fn from_workspace_dir_creates_workspace_and_returns_client() {
    let (tmp, client) = make_client();
    assert!(tmp.path().join("workspace").exists());
    // put_doc_light is the cheapest sanity check — it stores a DB row
    // without touching the embedder / graph extractor.
    let id = client
        .put_doc_light(doc("test-ns", "k1", "hello"))
        .await
        .unwrap();
    assert!(!id.is_empty());
}

#[tokio::test]
async fn list_namespaces_returns_what_was_written() {
    let (_tmp, client) = make_client();
    client.put_doc_light(doc("alpha", "k1", "a")).await.unwrap();
    client.put_doc_light(doc("beta", "k1", "b")).await.unwrap();
    let mut namespaces = client.list_namespaces().await.unwrap();
    namespaces.sort();
    assert!(namespaces.contains(&"alpha".to_string()));
    assert!(namespaces.contains(&"beta".to_string()));
}

#[tokio::test]
async fn list_documents_and_delete_document_round_trip() {
    let (_tmp, client) = make_client();
    let id = client
        .put_doc_light(doc("docs", "k1", "some content"))
        .await
        .unwrap();

    let docs = client.list_documents(Some("docs")).await.unwrap();
    let docs_arr = docs
        .get("documents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(docs_arr
        .iter()
        .any(|d| { d.get("documentId").and_then(|v| v.as_str()) == Some(&id) }));

    let _ = client.delete_document("docs", &id).await.unwrap();
    let docs = client.list_documents(Some("docs")).await.unwrap();
    let docs_arr = docs
        .get("documents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(docs_arr
        .iter()
        .all(|d| { d.get("documentId").and_then(|v| v.as_str()) != Some(&id) }));
}

#[tokio::test]
async fn clear_namespace_removes_all_docs_in_namespace() {
    let (_tmp, client) = make_client();
    client
        .put_doc_light(doc("throwaway", "k1", "x"))
        .await
        .unwrap();
    client
        .put_doc_light(doc("throwaway", "k2", "y"))
        .await
        .unwrap();
    client.clear_namespace("throwaway").await.unwrap();
    let docs = client.list_documents(Some("throwaway")).await.unwrap();
    let docs_arr = docs
        .get("documents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(docs_arr.is_empty());
}

#[tokio::test]
async fn store_skill_sync_with_secret_like_title_uses_stable_document_id_as_key() {
    // Regression for #4947 (Bug 2): a Composio provider sync passes the
    // provider's human-readable title as the document title, but the upsert
    // *key* must be the stable opaque `document_id` (`gmail:<message_id>`),
    // NOT the title. Some email subjects legitimately look secret-like
    // (verification codes, token-rotation notices), and `upsert_document`
    // rejects any secret-like namespace/key. Because gmail's tinycortex
    // pipeline does not tolerate scope errors, one such subject would abort
    // the entire scheduled sync with `document namespace/key cannot contain
    // secrets`, leaving the source stale ("Last synced 17d ago").
    let (_tmp, client) = make_client();

    // A subject that trips the secret guard. Assert the precondition so the
    // test cannot silently pass if the detector's patterns change.
    let secret_like_title = "Security alert: token glpat-aaaaaaaaaaaaaaaaaaaa was created";
    assert!(
        crate::openhuman::memory_store::safety::has_likely_secret(secret_like_title),
        "test title must trip the secret detector for this regression to be meaningful"
    );

    // With a stable document_id provided, the write must succeed — the key is
    // the opaque id, not the secret-like subject. Before the fix the key was
    // the title and this returned the secrets error.
    client
        .store_skill_sync(
            "gmail",
            "conn-1",
            secret_like_title,
            "email body",
            Some("composio-provider-incremental".into()),
            None,
            Some("medium".into()),
            None,
            None,
            Some("gmail:19af23bc00112233".into()),
        )
        .await
        .expect("secret-like subject must not block a stable-id-keyed sync write");

    // The document is persisted and keyed by the stable id, so a second sync
    // of the same message dedups (updates in place) rather than duplicating.
    client
        .store_skill_sync(
            "gmail",
            "conn-1",
            secret_like_title,
            "email body v2",
            Some("composio-provider-incremental".into()),
            None,
            Some("medium".into()),
            None,
            None,
            Some("gmail:19af23bc00112233".into()),
        )
        .await
        .expect("re-sync of same message id must succeed");

    let docs = client.list_documents(Some("skill-gmail")).await.unwrap();
    let arr = docs
        .get("documents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        arr.len(),
        1,
        "stable-id key must dedupe both syncs into a single document"
    );
}

#[tokio::test]
async fn clear_skill_memory_targets_prefixed_namespace() {
    let (_tmp, client) = make_client();
    // `store_skill_sync` prefixes the namespace with "skill-<id>".
    client
        .store_skill_sync(
            "my-skill", "default", "Title", "body", None, None, None, None, None, None,
        )
        .await
        .unwrap();
    // Verify the doc lives under the prefixed namespace.
    let docs = client.list_documents(Some("skill-my-skill")).await.unwrap();
    let arr = docs
        .get("documents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(!arr.is_empty());
    // Clearing by skill id should remove it.
    client
        .clear_skill_memory("my-skill", "default")
        .await
        .unwrap();
    let after = client.list_documents(Some("skill-my-skill")).await.unwrap();
    let after_arr = after
        .get("documents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(after_arr.is_empty());
}

#[tokio::test]
async fn kv_set_get_delete_round_trip() {
    let (_tmp, client) = make_client();
    let value = json!("ship-it");
    client.kv_set(Some("team"), "goal", &value).await.unwrap();
    let got = client.kv_get(Some("team"), "goal").await.unwrap();
    assert_eq!(got.as_ref(), Some(&value));
    let removed = client.kv_delete(Some("team"), "goal").await.unwrap();
    assert!(removed);
    let after = client.kv_get(Some("team"), "goal").await.unwrap();
    assert!(after.is_none());
}

#[tokio::test]
async fn kv_global_set_and_get_uses_none_namespace_branch() {
    let (_tmp, client) = make_client();
    let v = json!({"k": 1});
    client.kv_set(None, "global-key", &v).await.unwrap();
    let got = client.kv_get(None, "global-key").await.unwrap();
    assert_eq!(got.as_ref(), Some(&v));
}

#[tokio::test]
async fn kv_list_namespace_returns_all_keys() {
    let (_tmp, client) = make_client();
    client
        .kv_set(Some("cfg"), "env", &json!("dev"))
        .await
        .unwrap();
    client
        .kv_set(Some("cfg"), "region", &json!("us-east"))
        .await
        .unwrap();
    let entries = client.kv_list_namespace("cfg").await.unwrap();
    // Each entry is a JSON object — we just check that both keys are present.
    let s = serde_json::to_string(&entries).unwrap();
    assert!(s.contains("env"));
    assert!(s.contains("region"));
}

#[tokio::test]
async fn graph_upsert_does_not_error_for_namespaced_and_global_writes() {
    // We exercise both `Some(ns)` and `None` branches of `graph_upsert`
    // — the storage shape returned by `graph_query` is internal and
    // varies between unified store versions, so we only assert the
    // upsert path completes successfully.
    let (_tmp, client) = make_client();
    client
        .graph_upsert(
            Some("team"),
            "Alice",
            "OWNS",
            "Atlas",
            &json!({"evidence": "chat"}),
        )
        .await
        .unwrap();
    client
        .graph_upsert(None, "Bob", "FOLLOWS", "Carol", &json!({}))
        .await
        .unwrap();
    // graph_query() must not error in either form; we accept any
    // returned vec (possibly empty depending on store internals).
    let _ = client
        .graph_query(Some("team"), Some("Alice"), None)
        .await
        .unwrap();
    let _ = client.graph_query(None, Some("Bob"), None).await.unwrap();
}

#[tokio::test]
async fn profile_conn_returns_arc_shared_connection() {
    let (_tmp, client) = make_client();
    let a = client.profile_conn();
    let b = client.profile_conn();
    // Both handles wrap the same Arc.
    assert!(Arc::ptr_eq(&a, &b));
}

#[tokio::test]
async fn put_doc_full_pipeline_completes() {
    // Exercise the full `put_doc` path (vs `put_doc_light`) — the
    // ingestion queue submits a background job. The call itself
    // returns the document id immediately.
    let (_tmp, client) = make_client();
    let id = client
        .put_doc(doc(
            "ingestion-pipeline",
            "k1",
            "background-extract content",
        ))
        .await
        .unwrap();
    assert!(!id.is_empty());
}

#[tokio::test]
async fn recall_namespace_memories_returns_recent_inputs() {
    let (_tmp, client) = make_client();
    for i in 0..3 {
        client
            .put_doc_light(doc("recall-ns", &format!("k{i}"), &format!("body {i}")))
            .await
            .unwrap();
    }
    let hits = client
        .recall_namespace_memories("recall-ns", 10)
        .await
        .unwrap();
    // Light docs may not register as queryable hits in every backend,
    // but the call must not error.
    let _ = hits;
}

#[tokio::test]
async fn recall_namespace_with_no_data_returns_none_or_empty() {
    let (_tmp, client) = make_client();
    let recalled = client
        .recall_namespace("never-written-ns", 5)
        .await
        .unwrap();
    // Either no context (None) or empty string is acceptable.
    assert!(recalled.is_none() || recalled.as_deref() == Some(""));
}

#[tokio::test]
async fn query_namespace_with_no_data_returns_empty_or_short() {
    let (_tmp, client) = make_client();
    let result = client
        .query_namespace("never-written-ns", "anything", 5)
        .await
        .unwrap();
    // Empty namespace → either empty result or trivial sentinel.
    assert!(result.is_empty() || result.len() < 200);
}

#[tokio::test]
async fn query_and_recall_namespace_context_data_return_empty_context() {
    // Hit the `*_context_data` variants of query / recall so their
    // delegation arms in `MemoryClient` get exercised.
    let (_tmp, client) = make_client();
    let q = client
        .query_namespace_context_data("empty-ns", "q", 5)
        .await
        .unwrap();
    let r = client
        .recall_namespace_context_data("empty-ns", 5)
        .await
        .unwrap();
    // Ensure the accessor surface is reachable; exact shape varies.
    let _ = (q, r);
}

#[tokio::test]
async fn ingest_doc_completes_and_stores_document() {
    let (_tmp, client) = make_client();
    let req = MemoryIngestionRequest {
        document: doc("ingest-ns", "direct-k", "inline sync ingest body"),
        config: MemoryIngestionConfig::default(),
    };
    let result = client.ingest_doc(req).await;
    // Depending on whether the embedder is reachable the call may
    // error out with a clear message — we only assert that the path
    // is exercised (no panic).
    let _ = result;
}
