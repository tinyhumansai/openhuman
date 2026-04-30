use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use crate::openhuman::memory::{embeddings::NoopEmbedding, NamespaceDocumentInput, UnifiedMemory};

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
