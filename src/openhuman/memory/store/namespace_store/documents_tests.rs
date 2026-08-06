//! Tests for the `documents` module — upsert / list / delete / clear-namespace.

use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use crate::openhuman::inference::embeddings::NoopEmbedding;
use crate::openhuman::memory::store::{NamespaceDocumentInput, UnifiedMemory};

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
        taint: crate::openhuman::memory::MemoryTaint::Internal,
    }
}

fn count_vector_chunks(memory: &UnifiedMemory, namespace: &str, document_id: &str) -> i64 {
    let conn = memory.conn.lock();
    conn.query_row(
        "SELECT COUNT(*) FROM vector_chunks WHERE namespace = ?1 AND document_id = ?2",
        rusqlite::params![UnifiedMemory::sanitize_namespace(namespace), document_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[tokio::test]
async fn list_documents_without_namespace_returns_all_docs_across_namespaces() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(make_doc_input("test:one", "doc-a", "Doc A", "A body"))
        .await
        .unwrap();
    memory
        .upsert_document(make_doc_input("test:two", "doc-b", "Doc B", "B body"))
        .await
        .unwrap();

    let docs = memory.list_documents(None).await.unwrap();
    assert_eq!(docs["count"].as_u64().unwrap(), 2);
    let namespaces: std::collections::BTreeSet<_> = docs["documents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|doc| doc["namespace"].as_str())
        .collect();
    assert!(namespaces.contains("test_one"));
    assert!(namespaces.contains("test_two"));
}

#[tokio::test]
async fn list_namespaces_returns_distinct_sorted_sanitized_namespaces() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(make_doc_input("team alpha/#1", "doc-a", "Doc A", "A body"))
        .await
        .unwrap();
    memory
        .upsert_document(make_doc_input("team alpha/#1", "doc-b", "Doc B", "B body"))
        .await
        .unwrap();
    memory
        .upsert_document(make_doc_input("zeta", "doc-c", "Doc C", "C body"))
        .await
        .unwrap();

    let namespaces = memory.list_namespaces().await.unwrap();
    assert_eq!(
        namespaces,
        vec!["team_alpha/_1".to_string(), "zeta".to_string()]
    );
}

#[tokio::test]
async fn list_documents_with_namespace_filters_by_sanitized_namespace_and_orders_newest_first() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(make_doc_input(
            "team alpha/#1",
            "doc-a",
            "Older Doc",
            "A body",
        ))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    memory
        .upsert_document(make_doc_input(
            "team alpha/#1",
            "doc-b",
            "Newer Doc",
            "B body",
        ))
        .await
        .unwrap();
    memory
        .upsert_document(make_doc_input("other", "doc-c", "Other Doc", "C body"))
        .await
        .unwrap();

    let docs = memory.list_documents(Some("team alpha/#1")).await.unwrap();
    let documents = docs["documents"].as_array().unwrap();

    assert_eq!(docs["count"].as_u64().unwrap(), 2);
    assert_eq!(documents[0]["namespace"], json!("team_alpha/_1"));
    assert_eq!(documents[0]["key"], json!("doc-b"));
    assert_eq!(documents[1]["key"], json!("doc-a"));
}

#[tokio::test]
async fn load_documents_for_scope_defaults_invalid_json_fields_from_persisted_rows() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
    let namespace = UnifiedMemory::sanitize_namespace("broken/json");

    {
        let conn = memory.conn.lock();
        conn.execute(
            "INSERT INTO memory_docs
              (document_id, namespace, key, title, content, source_type, priority, tags_json, metadata_json, category, session_id, created_at, updated_at, markdown_rel_path)
             VALUES
              (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                "doc-invalid-json",
                namespace,
                "doc-a",
                "Doc A",
                "Body",
                "doc",
                "medium",
                "{not json",
                "also not json",
                "core",
                Option::<String>::None,
                10.0_f64,
                20.0_f64,
                "memory/namespaces/broken_json/docs/doc-invalid-json.md"
            ],
        )
        .unwrap();
    }

    let docs = memory
        .load_documents_for_scope("broken/json")
        .await
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert!(
        docs[0].tags.is_empty(),
        "invalid tags_json should fall back to []"
    );
    assert_eq!(
        docs[0].metadata,
        json!({}),
        "invalid metadata_json should fall back to an empty object"
    );
}

#[tokio::test]
async fn upsert_document_metadata_only_reuses_document_id_for_same_namespace_and_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let first_id = memory
        .upsert_document_metadata_only(make_doc_input(
            "test:meta",
            "doc-a",
            "Doc A",
            "Initial body",
        ))
        .await
        .unwrap();
    let second_id = memory
        .upsert_document_metadata_only(make_doc_input(
            "test:meta",
            "doc-a",
            "Doc A v2",
            "Updated body",
        ))
        .await
        .unwrap();

    assert_eq!(
        first_id, second_id,
        "metadata-only upsert should reuse the document id"
    );
    let docs = memory.load_documents_for_scope("test:meta").await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].document_id, first_id);
    assert_eq!(docs[0].title, "Doc A v2");
    assert_eq!(docs[0].content, "Updated body");
    assert_eq!(
        count_vector_chunks(&memory, "test:meta", &first_id),
        0,
        "metadata-only writes must not enqueue vector chunks"
    );
}

#[tokio::test]
async fn upsert_document_metadata_only_preserves_created_at_and_rewrites_sidecar() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let first_id = memory
        .upsert_document_metadata_only(make_doc_input(
            "test:meta-sidecar",
            "doc-a",
            "Doc A",
            "Initial body",
        ))
        .await
        .unwrap();
    let first_doc = memory
        .load_documents_for_scope("test:meta-sidecar")
        .await
        .unwrap()[0]
        .clone();
    let sidecar = tmp.path().join(&first_doc.markdown_rel_path);
    let first_markdown = std::fs::read_to_string(&sidecar).unwrap();
    assert!(first_markdown.contains("Initial body"));

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let second_id = memory
        .upsert_document_metadata_only(make_doc_input(
            "test:meta-sidecar",
            "doc-a",
            "Doc A v2",
            "Updated body",
        ))
        .await
        .unwrap();

    assert_eq!(first_id, second_id);
    let updated_doc = memory
        .load_documents_for_scope("test:meta-sidecar")
        .await
        .unwrap()[0]
        .clone();
    assert_eq!(updated_doc.created_at, first_doc.created_at);
    assert!(updated_doc.updated_at >= first_doc.updated_at);
    let updated_markdown = std::fs::read_to_string(sidecar).unwrap();
    assert!(updated_markdown.contains("Updated body"));
    assert!(updated_markdown.contains("Doc A v2"));
}

#[tokio::test]
async fn upsert_document_metadata_only_over_existing_document_preserves_vector_chunks() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let document_id = memory
        .upsert_document(make_doc_input(
            "test:meta-preserve-chunks",
            "doc-a",
            "Doc A",
            &"alpha ".repeat(400),
        ))
        .await
        .unwrap();
    let original_chunk_count =
        count_vector_chunks(&memory, "test:meta-preserve-chunks", &document_id);
    assert!(original_chunk_count > 0);

    let updated_id = memory
        .upsert_document_metadata_only(make_doc_input(
            "test:meta-preserve-chunks",
            "doc-a",
            "Doc A v2",
            "Updated body without re-embedding",
        ))
        .await
        .unwrap();

    assert_eq!(updated_id, document_id);
    let docs = memory
        .load_documents_for_scope("test:meta-preserve-chunks")
        .await
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].content, "Updated body without re-embedding");
    assert_eq!(
        count_vector_chunks(&memory, "test:meta-preserve-chunks", &document_id),
        original_chunk_count,
        "metadata-only writes should not delete existing semantic chunks"
    );
}

#[tokio::test]
async fn upsert_document_after_metadata_only_reuses_document_id_and_adds_vector_chunks() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let metadata_only_id = memory
        .upsert_document_metadata_only(make_doc_input(
            "test:meta-then-full",
            "doc-a",
            "Doc A",
            "Short body",
        ))
        .await
        .unwrap();
    assert_eq!(
        count_vector_chunks(&memory, "test:meta-then-full", &metadata_only_id),
        0
    );

    let full_id = memory
        .upsert_document(make_doc_input(
            "test:meta-then-full",
            "doc-a",
            "Doc A Embedded",
            &"beta ".repeat(400),
        ))
        .await
        .unwrap();

    assert_eq!(full_id, metadata_only_id);
    let docs = memory
        .load_documents_for_scope("test:meta-then-full")
        .await
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].title, "Doc A Embedded");
    assert!(
        count_vector_chunks(&memory, "test:meta-then-full", &full_id) > 0,
        "full upsert should backfill chunks for a metadata-only document"
    );
}

#[tokio::test]
async fn upsert_document_writes_vector_chunks_for_chunked_content() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let long_body = "alpha ".repeat(400);
    let document_id = memory
        .upsert_document(make_doc_input("test:vector", "doc-a", "Doc A", &long_body))
        .await
        .unwrap();

    assert!(
        count_vector_chunks(&memory, "test:vector", &document_id) > 0,
        "full document upsert should replace vector chunks for semantic retrieval"
    );
}

/// Embedder that records how many times `embed` is invoked and returns one
/// fixed-dimension vector per input text. Used to prove `upsert_document`
/// embeds all chunks in a SINGLE batch call rather than one call per chunk.
struct CountingEmbedder {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl crate::openhuman::inference::embeddings::EmbeddingProvider for CountingEmbedder {
    fn name(&self) -> &str {
        "counting"
    }

    fn model_id(&self) -> &str {
        "counting-test"
    }

    fn dimensions(&self) -> usize {
        3
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(texts.iter().map(|_| vec![0.1, 0.2, 0.3]).collect())
    }
}

#[tokio::test]
async fn upsert_document_batch_embeds_all_chunks_in_one_call() {
    let tmp = TempDir::new().unwrap();
    let embedder = Arc::new(CountingEmbedder {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let memory = UnifiedMemory::new(tmp.path(), embedder.clone(), None).unwrap();

    // Long enough to chunk into several pieces (chunk size is 225 chars).
    let long_body = "alpha ".repeat(400);
    let document_id = memory
        .upsert_document(make_doc_input("test:batch", "doc-a", "Doc A", &long_body))
        .await
        .unwrap();

    let chunk_count = count_vector_chunks(&memory, "test:batch", &document_id);
    assert!(
        chunk_count >= 3,
        "test body should chunk into >=3 pieces, got {chunk_count}"
    );
    assert_eq!(
        embedder.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "all chunks must be embedded in a single batch call, not one call per chunk"
    );
}

#[tokio::test]
async fn upsert_document_reuses_document_id_preserves_created_at_and_replaces_vector_chunks() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let first_id = memory
        .upsert_document(make_doc_input(
            "test:update",
            "doc-a",
            "Doc A",
            &"alpha ".repeat(400),
        ))
        .await
        .unwrap();
    let first_doc = memory
        .load_documents_for_scope("test:update")
        .await
        .unwrap()[0]
        .clone();
    let first_chunk_count = count_vector_chunks(&memory, "test:update", &first_id);
    assert!(first_chunk_count > 0);

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let second_id = memory
        .upsert_document(make_doc_input(
            "test:update",
            "doc-a",
            "Doc A v2",
            &"beta ".repeat(40),
        ))
        .await
        .unwrap();

    assert_eq!(
        first_id, second_id,
        "upsert should reuse the existing document id"
    );
    let updated_doc = memory
        .load_documents_for_scope("test:update")
        .await
        .unwrap()[0]
        .clone();
    assert_eq!(updated_doc.document_id, first_id);
    assert_eq!(updated_doc.created_at, first_doc.created_at);
    assert!(updated_doc.updated_at >= first_doc.updated_at);
    assert_eq!(updated_doc.title, "Doc A v2");
    assert_eq!(updated_doc.content, "beta ".repeat(40));
    let second_chunk_count = count_vector_chunks(&memory, "test:update", &second_id);
    assert!(second_chunk_count > 0);
    assert!(
        second_chunk_count <= first_chunk_count,
        "replacing with shorter content should not leave stale vector chunks behind"
    );
}

#[tokio::test]
async fn delete_document_removes_doc_sidecar_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let document_id = memory
        .upsert_document(make_doc_input("test:delete", "doc-a", "Doc A", "Delete me"))
        .await
        .unwrap();

    let docs = memory
        .load_documents_for_scope("test:delete")
        .await
        .unwrap();
    assert_eq!(docs.len(), 1);
    let sidecar = tmp.path().join(&docs[0].markdown_rel_path);
    assert!(sidecar.exists(), "sidecar should exist before delete");

    memory
        .graph_upsert_namespace(
            "test:delete",
            "Alice",
            "OWNS",
            "Phoenix",
            &json!({
                "document_id": document_id.clone(),
                "chunk_id": format!("{document_id}:0")
            }),
        )
        .await
        .unwrap();

    let deleted = memory
        .delete_document("test:delete", &document_id)
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], json!(true));
    assert_eq!(deleted["documentId"], json!(document_id.clone()));
    assert!(!sidecar.exists(), "sidecar should be removed on delete");
    assert!(memory
        .load_documents_for_scope("test:delete")
        .await
        .unwrap()
        .is_empty());
    assert!(
        memory
            .graph_relations_namespace("test:delete", None, None)
            .await
            .unwrap()
            .is_empty(),
        "document-linked graph relations should be pruned"
    );

    let second = memory
        .delete_document("test:delete", &document_id)
        .await
        .unwrap();
    assert_eq!(second["deleted"], json!(false));
    assert_eq!(second["documentId"], json!(document_id));
}

#[tokio::test]
async fn delete_document_succeeds_when_sidecar_is_already_missing() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let document_id = memory
        .upsert_document(make_doc_input(
            "test:delete-missing-sidecar",
            "doc-a",
            "Doc A",
            "Delete me",
        ))
        .await
        .unwrap();

    let docs = memory
        .load_documents_for_scope("test:delete-missing-sidecar")
        .await
        .unwrap();
    assert_eq!(docs.len(), 1);
    let sidecar = tmp.path().join(&docs[0].markdown_rel_path);
    assert!(sidecar.exists());
    std::fs::remove_file(&sidecar).unwrap();
    assert!(!sidecar.exists());

    let deleted = memory
        .delete_document("test:delete-missing-sidecar", &document_id)
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], json!(true));
    assert!(memory
        .load_documents_for_scope("test:delete-missing-sidecar")
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn delete_document_accepts_unsanitized_namespace_and_removes_chunks() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let document_id = memory
        .upsert_document(make_doc_input(
            "Team Alpha/#1",
            "doc-a",
            "Doc A",
            &"delete ".repeat(300),
        ))
        .await
        .unwrap();
    assert!(count_vector_chunks(&memory, "Team Alpha/#1", &document_id) > 0);

    let deleted = memory
        .delete_document("Team Alpha/#1", &document_id)
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], json!(true));
    assert_eq!(deleted["namespace"], json!("Team_Alpha/_1"));
    assert_eq!(
        count_vector_chunks(&memory, "Team Alpha/#1", &document_id),
        0
    );
    assert!(memory
        .load_documents_for_scope("Team Alpha/#1")
        .await
        .unwrap()
        .is_empty());
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
async fn clear_namespace_accepts_unsanitized_namespace_and_removes_sanitized_docs_dir() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(make_doc_input(
            "Team Alpha/#1",
            "doc-a",
            "Doc A",
            "Namespace cleanup body",
        ))
        .await
        .unwrap();
    memory
        .kv_set_namespace("Team Alpha/#1", "pref-1", &json!({"theme": "dark"}))
        .await
        .unwrap();

    let docs_dir = tmp
        .path()
        .join("memory")
        .join("namespaces")
        .join("Team_Alpha/_1")
        .join("docs");
    assert!(docs_dir.exists());

    memory.clear_namespace("Team Alpha/#1").await.unwrap();

    assert!(memory
        .load_documents_for_scope("Team Alpha/#1")
        .await
        .unwrap()
        .is_empty());
    assert!(memory
        .kv_list_namespace("Team Alpha/#1")
        .await
        .unwrap()
        .is_empty());
    assert!(!docs_dir.exists());
}

#[tokio::test]
async fn list_namespaces_skips_blank_rows_inserted_outside_normal_writes() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    {
        let conn = memory.conn.lock();
        conn.execute(
            "INSERT INTO memory_docs
              (document_id, namespace, key, title, content, source_type, priority, tags_json, metadata_json, category, session_id, created_at, updated_at, markdown_rel_path)
             VALUES
              (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                "doc-blank-ns",
                "   ",
                "doc-a",
                "Doc A",
                "Body",
                "doc",
                "medium",
                "[]",
                "{}",
                "core",
                Option::<String>::None,
                10.0_f64,
                20.0_f64,
                "memory/namespaces/blank/docs/doc-blank-ns.md"
            ],
        )
        .unwrap();
    }
    memory
        .upsert_document(make_doc_input("valid/ns", "doc-b", "Doc B", "Body"))
        .await
        .unwrap();

    let namespaces = memory.list_namespaces().await.unwrap();
    assert_eq!(namespaces, vec!["valid/ns".to_string()]);
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
            taint: crate::openhuman::memory::MemoryTaint::Internal,
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
            taint: crate::openhuman::memory::MemoryTaint::Internal,
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
            taint: crate::openhuman::memory::MemoryTaint::Internal,
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
            taint: crate::openhuman::memory::MemoryTaint::Internal,
        })
        .await
        .expect_err("secret-like key should be rejected");
    assert!(err.contains("cannot contain secrets"));
}

// ---------------------------------------------------------------------------
// Personal-identifier (PII) at the namespace/key boundary — auto-sanitize.
//
// Rather than rejecting writes with PII-like keys/namespaces (which caused
// unthrottled retry loops, see #5164), the store now auto-sanitizes the
// namespace and key using `redact_pii` before persisting. These tests verify
// the upsert succeeds and the stored key/namespace contains redacted tokens.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn kv_set_global_auto_sanitizes_pii_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    // SSN-like key should be auto-sanitized, not rejected.
    memory
        .kv_set_global("ssn-123-45-6789", &json!({"value": "ok"}))
        .await
        .expect("PII-like global key should be auto-sanitized, not rejected");

    // ... and the caller must be able to read it back with the identifier it
    // wrote. The canonicalization is a storage-address transform, so the read
    // path applies the same one (#5164). A miss here is what made the caller
    // write again, which is the loop the issue was reported for.
    let stored = memory.kv_get_global("ssn-123-45-6789").await.unwrap();
    assert_eq!(
        stored,
        Some(json!({"value": "ok"})),
        "a canonicalized KV key must stay readable by its original identifier"
    );
    assert!(
        memory.kv_delete_global("ssn-123-45-6789").await.unwrap(),
        "delete must address the same canonicalized row the write created"
    );
}

#[tokio::test]
async fn kv_set_namespace_auto_sanitizes_pii_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .kv_set_namespace("safe", "ssn-123-45-6789", &json!({"value": "ok"}))
        .await
        .expect("PII-like key should be auto-sanitized, not rejected");

    let records = memory.kv_records_namespace("safe").await.unwrap();
    // The record should still exist; the key gets redacted internally.
    assert_eq!(records.len(), 1);
    assert_eq!(
        memory
            .kv_get_namespace("safe", "ssn-123-45-6789")
            .await
            .unwrap(),
        Some(json!({"value": "ok"})),
        "a canonicalized KV key must stay readable by its original identifier"
    );
}

#[tokio::test]
async fn kv_set_namespace_auto_sanitizes_pii_like_namespace() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .kv_set_namespace("user/111.444.777-35", "safe-key", &json!({"value": "ok"}))
        .await
        .expect("PII-like namespace should be auto-sanitized, not rejected");

    assert_eq!(
        memory
            .kv_get_namespace("user/111.444.777-35", "safe-key")
            .await
            .unwrap(),
        Some(json!({"value": "ok"})),
        "a canonicalized KV namespace must stay readable by its original value"
    );
}

#[tokio::test]
async fn upsert_document_auto_sanitizes_pii_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let doc_id = memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "safe".to_string(),
            key: "cuit-20-11111111-2".to_string(),
            title: "Title".to_string(),
            content: "Body".to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
            taint: crate::openhuman::memory::MemoryTaint::Internal,
        })
        .await
        .expect("PII-like key should be auto-sanitized, not rejected");

    // The document was stored with a redacted key.
    let docs = memory.load_documents_for_scope("safe").await.unwrap();
    let doc = docs.iter().find(|d| d.document_id == doc_id).unwrap();
    assert!(!doc.key.contains("20-11111111-2"), "key should be redacted");
    assert!(
        doc.key.contains("REDACTED"), // matches [REDACTED_PII_CUIT]
        "key should contain a redaction token, got: {}",
        doc.key
    );
}

#[tokio::test]
async fn upsert_document_auto_sanitizes_pii_like_namespace() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let doc_id = memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "cliente-RFC-VECJ880326XK4".to_string(),
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
            taint: crate::openhuman::memory::MemoryTaint::Internal,
        })
        .await
        .expect("PII-like namespace should be auto-sanitized, not rejected");

    // Look up the document by sanitized namespace (note sanitize_namespace
    // normalises special chars, so `[` becomes `_`).
    let docs = memory
        .load_documents_for_scope("cliente-RFC-VECJ880326XK4")
        .await
        .unwrap();
    let doc = docs.iter().find(|d| d.document_id == doc_id).unwrap();
    assert!(
        doc.namespace.contains("REDACTED"), // [REDACTED_PII_RFC] after sanitize_namespace
        "namespace should contain a redaction token, got: {}",
        doc.namespace
    );
}

#[tokio::test]
async fn upsert_document_metadata_only_auto_sanitizes_pii_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let doc_id = memory
        .upsert_document_metadata_only(NamespaceDocumentInput {
            namespace: "safe".to_string(),
            key: "ssn-123-45-6789".to_string(),
            title: "Title".to_string(),
            content: "Body".to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
            taint: crate::openhuman::memory::MemoryTaint::Internal,
        })
        .await
        .expect("PII-like key should be auto-sanitized, not rejected");

    let docs = memory.load_documents_for_scope("safe").await.unwrap();
    let doc = docs.iter().find(|d| d.document_id == doc_id).unwrap();
    assert!(
        doc.key.contains("REDACTED"), // [REDACTED_PII_SSN]
        "key should contain a redaction token, got: {}",
        doc.key
    );
}

#[tokio::test]
async fn upsert_document_metadata_only_auto_sanitizes_pii_like_namespace() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let doc_id = memory
        .upsert_document_metadata_only(NamespaceDocumentInput {
            namespace: "user/111.444.777-35".to_string(),
            key: "safe-key".to_string(),
            title: "Title".to_string(),
            content: "Body".to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
            taint: crate::openhuman::memory::MemoryTaint::Internal,
        })
        .await
        .expect("PII-like namespace should be auto-sanitized, not rejected");

    let docs = memory
        .load_documents_for_scope("user/111.444.777-35")
        .await
        .unwrap();
    let doc = docs.iter().find(|d| d.document_id == doc_id).unwrap();
    assert!(
        doc.namespace.contains("REDACTED"), // [REDACTED_PII_CPF] after sanitize_namespace
        "namespace should contain a redaction token, got: {}",
        doc.namespace
    );
}

// ---------------------------------------------------------------------------
// #5164 — the identifier canonicalization has to be symmetric, and it has to
// leave scanner-built identifiers alone.
//
// Canonicalizing a namespace/key rewrites the row's *address*. Two failure
// modes follow, and both re-create the unthrottled write loop the issue was
// filed for (silently, this time):
//
//   1. a read path that addresses the raw caller identifier never finds the
//      canonicalized row, so the caller writes it again;
//   2. canonicalizing with the lenient *content* scrubber maps every
//      phone-shaped identifier onto one placeholder, so distinct chats collapse
//      onto one `(namespace, key)` and the upsert's `ON CONFLICT … DO UPDATE`
//      has one contact's document overwrite another's.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pii_like_document_key_round_trips_through_get_and_forget() {
    use crate::openhuman::memory::traits::Memory;

    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(make_doc_input(
            "clients",
            "ssn-123-45-6789",
            "Title",
            "Body",
        ))
        .await
        .expect("PII-like key should be canonicalized, not rejected");

    let entry = memory
        .get("clients", "ssn-123-45-6789")
        .await
        .unwrap()
        .expect("a canonicalized key must stay readable by its original identifier");
    assert_eq!(entry.content, "Body");
    assert!(
        !entry.key.contains("123-45-6789"),
        "the SSN must not be persisted as the storage address, got: {}",
        entry.key
    );

    assert!(
        memory.forget("clients", "ssn-123-45-6789").await.unwrap(),
        "forget must address the same canonicalized row the write created"
    );
    assert!(memory
        .get("clients", "ssn-123-45-6789")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn pii_like_namespace_round_trips_through_get_and_list() {
    use crate::openhuman::memory::traits::Memory;

    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(make_doc_input(
            "cliente-RFC-VECJ880326XK4",
            "notes",
            "Title",
            "Body",
        ))
        .await
        .expect("PII-like namespace should be canonicalized, not rejected");

    assert!(
        memory
            .get("cliente-RFC-VECJ880326XK4", "notes")
            .await
            .unwrap()
            .is_some(),
        "a canonicalized namespace must stay readable by its original value"
    );
    let listed = memory
        .list(Some("cliente-RFC-VECJ880326XK4"), None, None)
        .await
        .unwrap();
    assert_eq!(
        listed.len(),
        1,
        "list() must canonicalize its namespace the same way the write did"
    );
}

#[tokio::test]
async fn scanner_built_phone_shaped_keys_stay_distinct_documents() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    // Two different WhatsApp contacts, same day. The digit runs differ only in
    // the phone number — the lenient content scrubber replaces both with one
    // `[REDACTED_PII_PHONE]` token, which would collapse them onto a single row.
    for (key, content) in [
        ("12025551234@c.us:2026-05-30", "alice thread"),
        ("12025559999@c.us:2026-05-30", "bob thread"),
    ] {
        memory
            .upsert_document(make_doc_input("whatsapp-web", key, "Chat", content))
            .await
            .unwrap();
    }

    let docs = memory
        .load_documents_for_scope("whatsapp-web")
        .await
        .unwrap();
    assert_eq!(
        docs.len(),
        2,
        "scanner-built phone-shaped keys must stay distinct documents, got: {:?}",
        docs.iter().map(|d| d.key.clone()).collect::<Vec<_>>()
    );
    for key in ["12025551234@c.us:2026-05-30", "12025559999@c.us:2026-05-30"] {
        assert!(
            docs.iter().any(|d| d.key == key),
            "key {key} must be stored verbatim, got: {:?}",
            docs.iter().map(|d| d.key.clone()).collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn scanner_built_identifiers_are_preserved_verbatim() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    // The strict boundary predicate deliberately tolerates these shapes
    // (WhatsApp group JID, iMessage E.164 chat id, padded ms timestamp); the
    // content scrubber does not. Canonicalization must follow the strict set,
    // or every scanner rewrites its own storage addresses.
    for key in [
        "12025551234-1543890267@g.us:2026-05-30",
        "imessage:+12025551234:2026-05-30",
        "accepted:000001747729035001",
    ] {
        let doc_id = memory
            .upsert_document(make_doc_input("scanner", key, "Title", "Body"))
            .await
            .unwrap();
        let docs = memory.load_documents_for_scope("scanner").await.unwrap();
        let doc = docs.iter().find(|d| d.document_id == doc_id).unwrap();
        assert_eq!(
            doc.key, key,
            "scanner-built identifier must not be rewritten"
        );
    }
}

#[tokio::test]
async fn metadata_only_write_round_trips_through_pii_like_key() {
    use crate::openhuman::memory::traits::Memory;

    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document_metadata_only(make_doc_input(
            "clients",
            "cuit-20-11111111-2",
            "Title",
            "Body",
        ))
        .await
        .expect("PII-like key should be canonicalized, not rejected");

    assert!(
        memory
            .get("clients", "cuit-20-11111111-2")
            .await
            .unwrap()
            .is_some(),
        "the metadata-only path must canonicalize keys the same way the full upsert does"
    );
}
