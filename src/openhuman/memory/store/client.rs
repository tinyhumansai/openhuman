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
use crate::openhuman::memory::store::unified::profile::{self, FacetType, ProfileFacet};
use crate::openhuman::memory::store::unified::UnifiedMemory;

/// Canonical namespace for everything we know about the owner of this
/// OpenHuman instance. Skills, the discovery agent, and the learning hook
/// all read and write here so the system prompt has a single source of
/// truth for "who is the user".
pub const OWNER_NAMESPACE: &str = "owner";

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

        // Initialize the default local embedding provider (e.g., FastEmbed).
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

    // ========================================================================
    // Owner identity helpers
    //
    // These back the `memory.updateOwner` skill host function and the
    // owner-discovery agent. Both write paths funnel through here so that
    // structured facts always land in the `user_profile` table and rich
    // documents always land in the dedicated `owner` namespace.
    // ========================================================================

    /// Upsert a single structured fact about the owner into the
    /// `user_profile` SQLite table.
    ///
    /// `origin` is a grep-friendly provenance tag that is appended to the
    /// `source_segment_ids` column so we can later tell which skill or agent
    /// asserted the fact (e.g. `"skill-owner-gmail"`, `"discovery-apify"`).
    ///
    /// Default confidence when the caller does not supply one is `0.8` —
    /// high enough to beat loose conversation-derived guesses but low
    /// enough that a later, higher-confidence write can override it.
    pub fn profile_upsert_owner(
        &self,
        facet_type: FacetType,
        key: &str,
        value: &str,
        confidence: Option<f64>,
        origin: &str,
    ) -> Result<(), String> {
        if key.trim().is_empty() {
            return Err("profile_upsert_owner: key must be non-empty".to_string());
        }
        if value.trim().is_empty() {
            return Err("profile_upsert_owner: value must be non-empty".to_string());
        }

        let facet_id = format!("owner.{}.{}", facet_type.as_str(), key);
        let confidence = confidence.unwrap_or(0.8).clamp(0.0, 1.0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        profile::profile_upsert(
            &self.inner.conn,
            &facet_id,
            &facet_type,
            key,
            value,
            confidence,
            Some(origin),
            now,
        )
        .map_err(|e| format!("profile_upsert_owner: {e}"))?;

        log::debug!(
            "[owner] upsert facet type={} key={} origin={} confidence={:.2}",
            facet_type.as_str(),
            key,
            origin,
            confidence
        );
        Ok(())
    }

    /// Load every profile facet currently stored — used by context assembly
    /// to populate the `## Owner` / `## User Profile` sections of the system
    /// prompt.
    pub fn profile_load_all(&self) -> Result<Vec<ProfileFacet>, String> {
        profile::profile_load_all(&self.inner.conn).map_err(|e| format!("profile_load_all: {e}"))
    }

    /// Store a rich document about the owner in the dedicated `owner`
    /// namespace. Used for content that doesn't fit the flat
    /// `key = value` shape of the profile table — bios, email signature
    /// blocks, discovery-agent summaries, etc.
    ///
    /// `origin` is stored in the document's `metadata.origin` field so
    /// downstream inspectors can tell where the blob came from.
    pub async fn store_owner_doc(
        &self,
        title: &str,
        content: &str,
        source_type: Option<String>,
        origin: &str,
    ) -> Result<String, String> {
        if title.trim().is_empty() {
            return Err("store_owner_doc: title must be non-empty".to_string());
        }
        if content.trim().is_empty() {
            return Err("store_owner_doc: content must be non-empty".to_string());
        }

        let source_type = source_type.unwrap_or_else(|| "doc".to_string());
        // Deterministic key so re-pushing the same title overwrites rather
        // than duplicating (e.g. a skill re-running OAuth for the same
        // integration should update, not spam, its bio document).
        let key = format!("{}.{}", origin, slugify(title));

        let input = NamespaceDocumentInput {
            namespace: OWNER_NAMESPACE.to_string(),
            key: key.clone(),
            title: title.to_string(),
            content: content.to_string(),
            source_type,
            priority: "high".to_string(),
            tags: vec!["owner".to_string()],
            metadata: json!({ "origin": origin }),
            category: "owner".to_string(),
            session_id: None,
            document_id: None,
        };

        let doc_id = self.put_doc(input).await?;
        log::debug!(
            "[owner] stored doc title='{}' origin={} key={} id={}",
            title,
            origin,
            key,
            doc_id
        );
        Ok(doc_id)
    }

    /// Recall the raw documents currently stored in the `owner` namespace.
    ///
    /// Used by the context assembler to render a `## Owner` section and by
    /// the discovery agent to avoid re-researching known facts.
    pub async fn recall_owner_docs(&self, max_chunks: u32) -> Result<Option<String>, String> {
        self.recall_namespace(OWNER_NAMESPACE, max_chunks).await
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

/// Turn a free-form title into a deterministic, filesystem-safe key
/// segment. Used when composing owner-document keys so re-pushing the same
/// title updates the existing document rather than duplicating it.
fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_dash = false;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("untitled");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_handles_common_cases() {
        assert_eq!(slugify("Gmail signature bio"), "gmail-signature-bio");
        assert_eq!(slugify("  Spaces   collapse "), "spaces-collapse");
        assert_eq!(slugify("Weird!!!Chars???"), "weird-chars");
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("---"), "untitled");
    }
}
