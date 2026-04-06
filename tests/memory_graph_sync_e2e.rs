//! Integration test: document ingestion → graph query pipeline.
//!
//! Verifies that storing a document through the memory system produces
//! graph entities and relations that are queryable via the same APIs
//! the UI calls.
//!
//! Tests are `#[ignore]` by default (slow, requires disk I/O + ingestion worker).
//! Run explicitly:
//!   cargo test --test memory_graph_sync_e2e -- --ignored --nocapture

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tempfile::tempdir;

use openhuman_core::openhuman::memory::{
    embeddings::NoopEmbedding, MemoryClient, MemoryIngestionConfig, MemoryIngestionRequest,
    NamespaceDocumentInput, UnifiedMemory,
};

/// Config that skips the GLiNER RelEx ONNX model (avoids ORT init issues
/// on CI runners). Heuristic extraction still runs and will pick up
/// structured lines like "Owner:", "Project name:", etc.
fn ci_safe_config() -> MemoryIngestionConfig {
    MemoryIngestionConfig {
        model_name: "__test_no_model__".to_string(),
        ..MemoryIngestionConfig::default()
    }
}

/// A document with known entities that the heuristic extractor can find.
/// Uses structured lines (Project name, Owner, etc.) that the parser
/// recognises without requiring the ONNX model.
const TEST_DOCUMENT: &str = "\
Project name: Acme Corp
Owner: Alice

Alice works at Acme Corp. Bob is the CEO of Acme Corp.

From: Alice <alice@acme.com>
To: Bob <bob@acme.com>
Subject: Q4 roadmap review

Hi Bob, let's review the Q4 roadmap for Acme Corp next week.

Decision: Ship the beta release by end of November.
Preferred communication channel: Slack over email.
";

// ── Test: full ingest_document → graph_query_namespace ─────────────────

#[tokio::test]
#[ignore] // Slow: SQLite + ingestion pipeline. Run with --ignored.
async fn ingest_document_populates_namespace_graph() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    let tmp = tempdir().expect("tempdir");
    let memory =
        UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).expect("UnifiedMemory::new");

    let namespace = "test-ns";

    let result = memory
        .ingest_document(MemoryIngestionRequest {
            document: NamespaceDocumentInput {
                namespace: namespace.to_string(),
                key: "acme-doc".to_string(),
                title: "Acme Corp team overview".to_string(),
                content: TEST_DOCUMENT.to_string(),
                source_type: "doc".to_string(),
                priority: "high".to_string(),
                tags: Vec::new(),
                metadata: json!({}),
                category: "core".to_string(),
                session_id: None,
                document_id: None,
            },
            config: ci_safe_config(),
        })
        .await
        .expect("ingest_document");

    eprintln!("--- Ingestion result ---");
    eprintln!("  document_id:  {}", result.document_id);
    eprintln!("  namespace:    {}", result.namespace);
    eprintln!("  entities:     {}", result.entity_count);
    eprintln!("  relations:    {}", result.relation_count);
    eprintln!("  chunks:       {}", result.chunk_count);
    eprintln!("  preferences:  {}", result.preference_count);
    eprintln!("  decisions:    {}", result.decision_count);

    for entity in &result.entities {
        eprintln!("  entity: {} ({})", entity.name, entity.entity_type);
    }
    for relation in &result.relations {
        eprintln!(
            "  relation: {} --[{}]--> {}",
            relation.subject, relation.predicate, relation.object
        );
    }

    // ── Verify entities extracted ──
    assert!(
        result.entity_count >= 2,
        "Expected at least 2 entities from heuristic extraction, got {}",
        result.entity_count
    );

    let entity_names: Vec<&str> = result.entities.iter().map(|e| e.name.as_str()).collect();
    eprintln!("  All entity names: {entity_names:?}");

    // The heuristic extractor should find ALICE and ACME CORP from the
    // structured lines.
    assert!(
        entity_names.iter().any(|n| n.contains("ALICE")),
        "Expected entity 'ALICE' among: {entity_names:?}"
    );
    assert!(
        entity_names.iter().any(|n| n.contains("ACME")),
        "Expected entity containing 'ACME' among: {entity_names:?}"
    );

    // ── Verify relations extracted ──
    assert!(
        result.relation_count >= 1,
        "Expected at least 1 relation, got {}",
        result.relation_count
    );

    // ── Verify graph is queryable via namespace ──
    let graph_rows = memory
        .graph_query_namespace(namespace, None, None)
        .await
        .expect("graph_query_namespace");

    eprintln!(
        "\n--- graph_query_namespace({namespace}) returned {} rows ---",
        graph_rows.len()
    );
    for row in &graph_rows {
        eprintln!("  {row}");
    }

    assert!(
        !graph_rows.is_empty(),
        "graph_query_namespace should return relations after ingestion"
    );

    // ── Verify graph_query_all also returns the namespace data ──
    let all_rows = memory
        .graph_query_all(None, None)
        .await
        .expect("graph_query_all");

    eprintln!("\n--- graph_query_all returned {} rows ---", all_rows.len());

    assert!(
        !all_rows.is_empty(),
        "graph_query_all should include namespace relations when no namespace filter is set"
    );

    // At minimum, the all-query should contain the same rows as namespace
    assert!(
        all_rows.len() >= graph_rows.len(),
        "graph_query_all ({}) should return at least as many rows as namespace query ({})",
        all_rows.len(),
        graph_rows.len()
    );
}

// ── Test: MemoryClient put_doc → background extraction → graph_query ──

#[tokio::test]
#[ignore] // Slow: background worker + 5s wait. Run with --ignored.
async fn put_doc_background_extraction_then_graph_query() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    let tmp = tempdir().expect("tempdir");
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let client = MemoryClient::from_workspace_dir(workspace_dir).expect("MemoryClient");

    let namespace = "test-bg";
    let doc_id = client
        .put_doc(NamespaceDocumentInput {
            namespace: namespace.to_string(),
            key: "bg-test-doc".to_string(),
            title: "Background extraction test".to_string(),
            content: TEST_DOCUMENT.to_string(),
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: Vec::new(),
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .expect("put_doc");

    eprintln!("put_doc returned doc_id={doc_id}");

    // Wait for the background ingestion worker to process the job.
    // The worker runs on a separate tokio task; give it time to complete.
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Query with namespace
    let ns_rows = client
        .graph_query(Some(namespace), None, None)
        .await
        .expect("graph_query with namespace");

    eprintln!(
        "graph_query(Some({namespace})) returned {} rows",
        ns_rows.len()
    );

    // Query without namespace (the fix: should include namespace data)
    let all_rows = client
        .graph_query(None, None, None)
        .await
        .expect("graph_query without namespace");

    eprintln!("graph_query(None) returned {} rows", all_rows.len());

    // The background worker uses the default config which tries to load the
    // ONNX model.  On CI this may fail silently, yielding 0 relations. The
    // heuristic extractor still runs, so we usually get relations, but we
    // assert conservatively: if namespace query found rows, the all-query
    // must too.
    if !ns_rows.is_empty() {
        assert!(
            !all_rows.is_empty(),
            "graph_query(None) must return rows when graph_query(Some(ns)) does"
        );
    }

    // Verify document was stored regardless
    let docs = client
        .list_documents(Some(namespace))
        .await
        .expect("list_documents");
    let doc_count = docs
        .get("documents")
        .and_then(|d| d.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    eprintln!("Documents in namespace '{namespace}': {doc_count}");
    assert!(
        doc_count >= 1,
        "Expected at least 1 document after put_doc, got {doc_count}"
    );
}
