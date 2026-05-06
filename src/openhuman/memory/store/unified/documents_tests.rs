//! Tests for the `documents` module — upsert / list / delete / clear-namespace.

use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use crate::openhuman::embeddings::NoopEmbedding;
use crate::openhuman::memory::{NamespaceDocumentInput, UnifiedMemory};

fn make_doc_input(
    namespace: &str,
    key: &str,
    title: &str,
    content: &str,
) -> NamespaceDocumentInput {
    NamespaceDocumentInput {
        namespace: namespace.to_string(),
        key: key.to_string(),
        title: title.to_string(),
        content: content.to_string(),
        source_type: "doc".to_string(),
        priority: "medium".to_string(),
        tags: vec![],
        metadata: json!({}),
        category: "core".to_string(),
        session_id: None,
        document_id: None,
    }
}

#[tokio::test]
async fn clear_namespace_removes_all_data_and_preserves_other_namespaces() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    // --- Populate "test:cleanup" namespace ---

    // 3 documents
    memory
        .upsert_document(make_doc_input(
            "test:cleanup",
            "doc-a",
            "Document A",
            "Content of document A for cleanup.",
        ))
        .await
        .unwrap();
    memory
        .upsert_document(make_doc_input(
            "test:cleanup",
            "doc-b",
            "Document B",
            "Content of document B for cleanup.",
        ))
        .await
        .unwrap();
    memory
        .upsert_document(make_doc_input(
            "test:cleanup",
            "doc-c",
            "Document C",
            "Content of document C for cleanup.",
        ))
        .await
        .unwrap();

    // 2 KV entries
    memory
        .kv_set_namespace("test:cleanup", "pref-1", &json!({"theme": "dark"}))
        .await
        .unwrap();
    memory
        .kv_set_namespace("test:cleanup", "pref-2", &json!({"lang": "en"}))
        .await
        .unwrap();

    // 2 graph relations
    memory
        .graph_upsert_namespace(
            "test:cleanup",
            "Alice",
            "knows",
            "Bob",
            &json!({"source": "test"}),
        )
        .await
        .unwrap();
    memory
        .graph_upsert_namespace(
            "test:cleanup",
            "Bob",
            "works_at",
            "Acme",
            &json!({"source": "test"}),
        )
        .await
        .unwrap();

    // --- Populate "test:other" namespace (control) ---

    memory
        .upsert_document(make_doc_input(
            "test:other",
            "other-doc",
            "Other Document",
            "Content of document in the other namespace.",
        ))
        .await
        .unwrap();
    memory
        .kv_set_namespace("test:other", "other-key", &json!({"value": true}))
        .await
        .unwrap();
    memory
        .graph_upsert_namespace(
            "test:other",
            "X",
            "relates_to",
            "Y",
            &json!({"source": "other"}),
        )
        .await
        .unwrap();

    // --- Verify pre-conditions ---

    let cleanup_docs = memory.list_documents(Some("test:cleanup")).await.unwrap();
    assert_eq!(
        cleanup_docs["count"].as_u64().unwrap(),
        3,
        "test:cleanup should have 3 documents before clear"
    );

    let cleanup_kv = memory.kv_list_namespace("test:cleanup").await.unwrap();
    assert_eq!(
        cleanup_kv.len(),
        2,
        "test:cleanup should have 2 KV entries before clear"
    );

    let cleanup_graph = memory
        .graph_relations_namespace("test:cleanup", None, None)
        .await
        .unwrap();
    assert_eq!(
        cleanup_graph.len(),
        2,
        "test:cleanup should have 2 graph relations before clear"
    );

    let other_docs = memory.list_documents(Some("test:other")).await.unwrap();
    assert_eq!(
        other_docs["count"].as_u64().unwrap(),
        1,
        "test:other should have 1 document before clear"
    );

    // --- Execute clear_namespace ---

    memory.clear_namespace("test:cleanup").await.unwrap();

    // --- Assert: "test:cleanup" is empty ---

    let cleanup_docs_after = memory.list_documents(Some("test:cleanup")).await.unwrap();
    assert_eq!(
        cleanup_docs_after["count"].as_u64().unwrap(),
        0,
        "test:cleanup documents should be empty after clear"
    );

    let cleanup_kv_after = memory.kv_list_namespace("test:cleanup").await.unwrap();
    assert!(
        cleanup_kv_after.is_empty(),
        "test:cleanup KV entries should be empty after clear"
    );

    let cleanup_graph_after = memory
        .graph_relations_namespace("test:cleanup", None, None)
        .await
        .unwrap();
    assert!(
        cleanup_graph_after.is_empty(),
        "test:cleanup graph relations should be empty after clear"
    );

    // --- Assert: "test:other" is untouched (critical) ---

    let other_docs_after = memory.list_documents(Some("test:other")).await.unwrap();
    assert_eq!(
        other_docs_after["count"].as_u64().unwrap(),
        1,
        "test:other document must still exist after clearing test:cleanup"
    );

    let other_kv_after = memory.kv_list_namespace("test:other").await.unwrap();
    assert_eq!(
        other_kv_after.len(),
        1,
        "test:other KV entry must still exist after clearing test:cleanup"
    );

    let other_graph_after = memory
        .graph_relations_namespace("test:other", None, None)
        .await
        .unwrap();
    assert_eq!(
        other_graph_after.len(),
        1,
        "test:other graph relation must still exist after clearing test:cleanup"
    );
}

#[tokio::test]
async fn clear_namespace_on_empty_namespace_is_noop() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    // Clearing a namespace that has never been used should succeed without error.
    memory.clear_namespace("nonexistent").await.unwrap();

    let docs = memory.list_documents(Some("nonexistent")).await.unwrap();
    assert_eq!(docs["count"].as_u64().unwrap(), 0);
}

#[tokio::test]
async fn clear_namespace_removes_on_disk_markdown_files() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(make_doc_input(
            "test:diskcheck",
            "disk-doc",
            "Disk Doc",
            "This doc has a markdown file on disk.",
        ))
        .await
        .unwrap();

    let docs_dir = tmp
        .path()
        .join("memory")
        .join("namespaces")
        .join("test_diskcheck")
        .join("docs");
    assert!(
        docs_dir.exists(),
        "docs directory should exist after upsert"
    );

    memory.clear_namespace("test:diskcheck").await.unwrap();

    assert!(
        !docs_dir.exists(),
        "docs directory should be removed after clear_namespace"
    );
}

#[tokio::test]
async fn upsert_document_redacts_secret_like_content_before_persisting() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "safe".to_string(),
            key: "secret-note".to_string(),
            title: "Bearer abcdefghijklmnop".to_string(),
            content: "token=abc123\n-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----"
                .to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: vec!["sk-1234567890123456789012345".to_string()],
            metadata: json!({
                "token": "raw",
                "notes": "api_key=really-secret"
            }),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .unwrap();

    let docs = memory.load_documents_for_scope("safe").await.unwrap();
    assert_eq!(docs.len(), 1);
    let doc = &docs[0];
    assert!(!doc.title.contains("abcdefghijklmnop"));
    assert!(doc.title.contains("[REDACTED]"));
    assert!(!doc.content.contains("BEGIN PRIVATE KEY"));
    assert!(doc.content.contains("[REDACTED_PRIVATE_KEY]"));
    assert_eq!(doc.metadata["token"], json!("[REDACTED_SECRET]"));
    assert_eq!(doc.metadata["notes"], json!("api_key=[REDACTED]"));
    assert_eq!(doc.tags[0], "[REDACTED]");

    let markdown = std::fs::read_to_string(tmp.path().join(&doc.markdown_rel_path)).unwrap();
    assert!(!markdown.contains("BEGIN PRIVATE KEY"));
    assert!(markdown.contains("[REDACTED_PRIVATE_KEY]"));
}

#[tokio::test]
async fn kv_set_namespace_redacts_secret_like_payloads() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .kv_set_namespace(
            "safe",
            "key-1",
            &json!({
                "token": "super-secret",
                "note": "Bearer abcdefghijklmnop"
            }),
        )
        .await
        .unwrap();

    let rows = memory.kv_list_namespace("safe").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["key"], json!("key-1"));
    assert_eq!(rows[0]["value"]["token"], json!("[REDACTED_SECRET]"));
    assert_eq!(rows[0]["value"]["note"], json!("Bearer [REDACTED]"));
}

#[tokio::test]
async fn kv_set_namespace_rejects_secret_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .kv_set_namespace(
            "safe",
            "api_key=sk-1234567890123456789012345",
            &json!({"value": "ok"}),
        )
        .await
        .expect_err("secret-like key should be rejected");
    assert!(err.contains("cannot contain secrets"));
}

#[tokio::test]
async fn kv_set_namespace_rejects_secret_like_namespace() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .kv_set_namespace(
            "Bearer abcdefghijklmnop",
            "safe-key",
            &json!({"value": "ok"}),
        )
        .await
        .expect_err("secret-like namespace should be rejected");
    assert!(err.contains("cannot contain secrets"));
}

#[tokio::test]
async fn kv_set_global_rejects_secret_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .kv_set_global(
            "authorization=Bearer abcdefghijklmnop",
            &json!({"value": "ok"}),
        )
        .await
        .expect_err("secret-like global key should be rejected");
    assert!(err.contains("cannot contain secrets"));
}

#[tokio::test]
async fn upsert_document_rejects_secret_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "safe".to_string(),
            key: "api_key=sk-1234567890123456789012345".to_string(),
            title: "Title".to_string(),
            content: "Body".to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .expect_err("secret-like key should be rejected");
    assert!(err.contains("cannot contain secrets"));
}

#[tokio::test]
async fn upsert_document_rejects_secret_like_namespace() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "Bearer abcdefghijklmnop".to_string(),
            key: "k1".to_string(),
            title: "Title".to_string(),
            content: "Body".to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .expect_err("secret-like namespace should be rejected");
    assert!(err.contains("cannot contain secrets"));
}

#[tokio::test]
async fn upsert_document_metadata_only_rejects_secret_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .upsert_document_metadata_only(NamespaceDocumentInput {
            namespace: "safe".to_string(),
            key: "refresh_token=abcdef".to_string(),
            title: "Title".to_string(),
            content: "Body".to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .expect_err("secret-like key should be rejected");
    assert!(err.contains("cannot contain secrets"));
}
