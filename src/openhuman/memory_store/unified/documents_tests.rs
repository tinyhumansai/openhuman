//! Tests for the `documents` module — upsert / list / delete / clear-namespace.

use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use crate::openhuman::embeddings::NoopEmbedding;
use crate::openhuman::memory_store::{NamespaceDocumentInput, UnifiedMemory};

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

// ---------------------------------------------------------------------------
// Personal-identifier (PII) rejection at the namespace/key boundary.
//
// Mirrors the secret-like rejection tests above, exercising the
// `safety::pii::has_likely_pii` early-return branches added to
// `kv_set_global`, `kv_set_namespace`, `upsert_document`, and
// `upsert_document_metadata_only`. Each branch returns the
// `"cannot contain personal identifiers"` error.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn kv_set_global_rejects_pii_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .kv_set_global("ssn-123-45-6789", &json!({"value": "ok"}))
        .await
        .expect_err("PII-like global key should be rejected");
    assert!(
        err.contains("cannot contain personal identifiers"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn kv_set_namespace_rejects_pii_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .kv_set_namespace("safe", "ssn-123-45-6789", &json!({"value": "ok"}))
        .await
        .expect_err("PII-like key should be rejected");
    assert!(
        err.contains("cannot contain personal identifiers"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn kv_set_namespace_rejects_pii_like_namespace() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
        .kv_set_namespace("user/111.444.777-35", "safe-key", &json!({"value": "ok"}))
        .await
        .expect_err("PII-like namespace should be rejected");
    assert!(
        err.contains("cannot contain personal identifiers"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn upsert_document_rejects_pii_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
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
        })
        .await
        .expect_err("PII-like key should be rejected");
    assert!(
        err.contains("cannot contain personal identifiers"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn upsert_document_rejects_pii_like_namespace() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
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
        })
        .await
        .expect_err("PII-like namespace should be rejected");
    assert!(
        err.contains("cannot contain personal identifiers"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn upsert_document_metadata_only_rejects_pii_like_key() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
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
        })
        .await
        .expect_err("PII-like key should be rejected");
    assert!(
        err.contains("cannot contain personal identifiers"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn upsert_document_metadata_only_rejects_pii_like_namespace() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let err = memory
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
        })
        .await
        .expect_err("PII-like namespace should be rejected");
    assert!(
        err.contains("cannot contain personal identifiers"),
        "unexpected error: {err}"
    );
}
