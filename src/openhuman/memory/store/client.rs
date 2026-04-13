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
    /// Create a new local memory client using the default `.openhuman` directory.
    ///
    /// # Errors
    ///
    /// Returns an error string if the home directory cannot be resolved or if
    /// initialization fails.
    pub fn new_local() -> Result<Self, String> {
        let workspace_dir = dirs::home_dir()
            .ok_or_else(|| "Failed to resolve home directory".to_string())?
            .join(".openhuman")
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
