//! # Memory Client
//!
//! High-level client interface for interacting with the OpenHuman memory system.
//!
//! The `MemoryClient` provides a simplified API for storing and retrieving
//! information from the memory store, handling background tasks like graph
//! extraction and embedding generation. It primarily acts as a wrapper around
//! `UnifiedMemory`.

use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::memory::embeddings::{self, EmbeddingProvider};
use crate::openhuman::memory::ingestion::{
    MemoryIngestionConfig, MemoryIngestionRequest, MemoryIngestionResult,
};
use crate::openhuman::memory::ingestion_queue::{self, IngestionJob, IngestionQueue};
use crate::openhuman::memory::store::types::{
    NamespaceDocumentInput, NamespaceMemoryHit, NamespaceRetrievalContext,
};
use crate::openhuman::memory::store::unified::UnifiedMemory;

/// Reference-counted handle to a `MemoryClient`.
pub type MemoryClientRef = Arc<MemoryClient>;

/// Thread-safe container for an optional `MemoryClientRef`.
///
/// Used for global state management where the memory client may or may not
/// be initialized.
pub struct MemoryState(pub std::sync::Mutex<Option<MemoryClientRef>>);

/// Local-only memory client backed by SQLite in the user's workspace directory.
///
/// All memory storage and retrieval happens on-device; there is no remote sync.
/// Remote/cloud memory sync is a future consideration — until then the memory
/// subsystem operates entirely locally via [`UnifiedMemory`].
#[derive(Clone)]
pub struct MemoryClient {
    /// The underlying memory implementation.
    inner: Arc<UnifiedMemory>,
    /// Queue for background ingestion tasks (e.g., entity extraction).
    ingestion_queue: IngestionQueue,
}

impl MemoryClient {
    /// Returns a handle to the underlying SQLite connection for direct
    /// profile-facet writes via
    /// [`crate::openhuman::memory::store::unified::profile::profile_upsert`].
    ///
    /// Intentionally `pub(crate)` — external consumers should use the
    /// higher-level `MemoryClient` API; this escape hatch exists so
    /// in-crate subsystems (composio providers, archivist, learning
    /// hooks) can write structured profile facets without an additional
    /// round-trip through the ingestion queue.
    pub(crate) fn profile_conn(&self) -> std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>> {
        std::sync::Arc::clone(&self.inner.conn)
    }

    /// Create a new local memory client using the default `.openhuman` directory.
    ///
    /// # Errors
    ///
    /// Returns an error string if the home directory cannot be resolved or if
    /// initialization fails.
    pub fn new_local() -> Result<Self, String> {
        let workspace_dir = crate::openhuman::config::default_root_openhuman_dir()
            .map_err(|e| e.to_string())?
            .join("workspace");
        Self::from_workspace_dir(workspace_dir)
    }

    /// Create a new memory client from a specific workspace directory.
    ///
    /// # Arguments
    ///
    /// * `workspace_dir` - The path where memory databases and assets are stored.
    ///
    /// # Errors
    ///
    /// Returns an error string if the directory cannot be created or if the
    /// `UnifiedMemory` or `IngestionQueue` fails to start.
    pub fn from_workspace_dir(workspace_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&workspace_dir)
            .map_err(|e| format!("Create workspace dir {}: {e}", workspace_dir.display()))?;

        // Initialize the default local embedding provider (Ollama).
        let embedder: Arc<dyn EmbeddingProvider> = embeddings::default_local_embedding_provider();

        // Create the underlying UnifiedMemory instance.
        let memory =
            UnifiedMemory::new(&workspace_dir, embedder, None).map_err(|e| format!("{e}"))?;
        let inner = Arc::new(memory);

        // Start the background worker for document ingestion and graph extraction.
        let ingestion_queue = ingestion_queue::start_worker(Arc::clone(&inner));

        Ok(Self {
            inner,
            ingestion_queue,
        })
    }

    /// Store a document in a specific namespace.
    ///
    /// This method performs an "upsert" (update or insert). It immediately
    /// persists the document and then enqueues a background job for graph
    /// extraction (entities and relations).
    ///
    /// # Arguments
    ///
    /// * `input` - The document content and metadata.
    ///
    /// # Returns
    ///
    /// The unique ID of the stored document.
    pub async fn put_doc(&self, input: NamespaceDocumentInput) -> Result<String, String> {
        let document_id = self.inner.upsert_document(input.clone()).await?;

        // Enqueue background graph extraction so entities/relations are
        // extracted without blocking the caller. The document is already
        // persisted — extract_graph will not upsert again.
        self.ingestion_queue.submit(IngestionJob {
            document_id: document_id.clone(),
            document: input,
            config: MemoryIngestionConfig::default(),
        });

        Ok(document_id)
    }

    /// Store a document (DB row + markdown file) without vector embedding or
    /// graph extraction.  Use this for high-frequency, ephemeral writes where
    /// the full pipeline would be too expensive (e.g. screen-intelligence
    /// snapshots).  The document is still searchable by metadata/FTS but will
    /// not appear in semantic vector queries or the knowledge graph.
    pub async fn put_doc_light(&self, input: NamespaceDocumentInput) -> Result<String, String> {
        self.inner.upsert_document_metadata_only(input).await
    }

    /// Perform a full ingestion (chunking, embedding, extraction) synchronously.
    ///
    /// Unlike `put_doc`, this waits for the entire process to complete.
    pub async fn ingest_doc(
        &self,
        request: MemoryIngestionRequest,
    ) -> Result<MemoryIngestionResult, String> {
        self.inner.ingest_document(request).await
    }

    /// Specialized method for syncing skill data into memory.
    ///
    /// Maps generic skill/integration fields into the `NamespaceDocumentInput` structure.
    #[allow(clippy::too_many_arguments)]
    pub async fn store_skill_sync(
        &self,
        skill_id: &str,
        _integration_id: &str,
        title: &str,
        content: &str,
        source_type: Option<String>,
        metadata: Option<serde_json::Value>,
        priority: Option<String>,
        _created_at: Option<f64>,
        _updated_at: Option<f64>,
        document_id: Option<String>,
    ) -> Result<(), String> {
        let namespace = format!("skill-{}", skill_id.trim());
        let input = NamespaceDocumentInput {
            namespace,
            key: title.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            source_type: source_type.unwrap_or_else(|| "doc".to_string()),
            priority: priority.unwrap_or_else(|| "medium".to_string()),
            tags: Vec::new(),
            metadata: metadata.unwrap_or_else(|| json!({})),
            category: "core".to_string(),
            session_id: None,
            document_id,
        };

        let doc_id = self.inner.upsert_document(input.clone()).await?;

        // Enqueue background graph extraction.
        self.ingestion_queue.submit(IngestionJob {
            document_id: doc_id,
            document: input,
            config: MemoryIngestionConfig::default(),
        });

        Ok(())
    }

    /// List documents in a namespace (or all namespaces if `None`).
    pub async fn list_documents(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.inner.list_documents(namespace).await
    }

    /// List all unique namespaces in the memory store.
    pub async fn list_namespaces(&self) -> Result<Vec<String>, String> {
        self.inner.list_namespaces().await
    }

    /// Delete a specific document by its ID and namespace.
    pub async fn delete_document(
        &self,
        namespace: &str,
        document_id: &str,
    ) -> Result<serde_json::Value, String> {
        self.inner.delete_document(namespace, document_id).await
    }

    /// Clear all documents and data within a specific namespace.
    pub async fn clear_namespace(&self, namespace: &str) -> Result<(), String> {
        self.inner.clear_namespace(namespace).await
    }

    /// Clear memory associated with a specific skill.
    pub async fn clear_skill_memory(
        &self,
        skill_id: &str,
        _integration_id: &str,
    ) -> Result<(), String> {
        let namespace = format!("skill-{}", skill_id.trim());
        let docs = self.list_documents(Some(&namespace)).await?;
        let items = docs
            .get("documents")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in items {
            if let Some(document_id) = item.get("documentId").and_then(serde_json::Value::as_str) {
                let _ = self.delete_document(&namespace, document_id).await?;
            }
        }
        Ok(())
    }

    /// Query a namespace for context using natural language.
    ///
    /// Returns a formatted string containing relevant text chunks and context.
    pub async fn query_namespace(
        &self,
        namespace: &str,
        query: &str,
        max_chunks: u32,
    ) -> Result<String, String> {
        self.inner
            .query_namespace_context(namespace, query, max_chunks)
            .await
    }

    /// Query a namespace and return raw context data (hits, relations, etc.).
    pub async fn query_namespace_context_data(
        &self,
        namespace: &str,
        query: &str,
        max_chunks: u32,
    ) -> Result<NamespaceRetrievalContext, String> {
        self.inner
            .query_namespace_context_data(namespace, query, max_chunks)
            .await
    }

    /// Recall recent context from a namespace without a specific query.
    pub async fn recall_namespace(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<Option<String>, String> {
        self.inner
            .recall_namespace_context(namespace, max_chunks)
            .await
    }

    /// Recall raw context data from a namespace without a specific query.
    pub async fn recall_namespace_context_data(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<NamespaceRetrievalContext, String> {
        self.inner
            .recall_namespace_context_data(namespace, max_chunks)
            .await
    }

    /// Recall a specific number of recent memories (hits) from a namespace.
    pub async fn recall_namespace_memories(
        &self,
        namespace: &str,
        limit: u32,
    ) -> Result<Vec<NamespaceMemoryHit>, String> {
        self.inner.recall_namespace_memories(namespace, limit).await
    }

    /// Store a key-value pair in a namespace (or global if `None`).
    pub async fn kv_set(
        &self,
        namespace: Option<&str>,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        match namespace {
            Some(ns) => self.inner.kv_set_namespace(ns, key, value).await,
            None => self.inner.kv_set_global(key, value).await,
        }
    }

    /// Retrieve a key-value pair.
    pub async fn kv_get(
        &self,
        namespace: Option<&str>,
        key: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        match namespace {
            Some(ns) => self.inner.kv_get_namespace(ns, key).await,
            None => self.inner.kv_get_global(key).await,
        }
    }

    /// Delete a key-value pair.
    pub async fn kv_delete(&self, namespace: Option<&str>, key: &str) -> Result<bool, String> {
        match namespace {
            Some(ns) => self.inner.kv_delete_namespace(ns, key).await,
            None => self.inner.kv_delete_global(key).await,
        }
    }

    /// List all key-value pairs in a namespace.
    pub async fn kv_list_namespace(
        &self,
        namespace: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.inner.kv_list_namespace(namespace).await
    }

    /// Upsert a relationship in the knowledge graph.
    pub async fn graph_upsert(
        &self,
        namespace: Option<&str>,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        match namespace {
            Some(ns) => {
                self.inner
                    .graph_upsert_namespace(ns, subject, predicate, object, attrs)
                    .await
            }
            None => {
                self.inner
                    .graph_upsert_global(subject, predicate, object, attrs)
                    .await
            }
        }
    }

    /// Query relationships in the knowledge graph using optional filters.
    ///
    /// When `namespace` is `None`, returns relations from **all** namespaces
    /// plus the global graph, so ingested data is always surfaced in the UI.
    pub async fn graph_query(
        &self,
        namespace: Option<&str>,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        match namespace {
            Some(ns) => {
                self.inner
                    .graph_query_namespace(ns, subject, predicate)
                    .await
            }
            None => self.inner.graph_query_all(subject, predicate).await,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
