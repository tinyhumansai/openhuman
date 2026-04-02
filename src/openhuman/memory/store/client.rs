use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::memory::embeddings::{self, EmbeddingProvider};
use crate::openhuman::memory::ingestion::{MemoryIngestionRequest, MemoryIngestionResult};
use crate::openhuman::memory::store::types::{
    NamespaceDocumentInput, NamespaceMemoryHit, NamespaceRetrievalContext,
};
use crate::openhuman::memory::store::unified::UnifiedMemory;
pub type MemoryClientRef = Arc<MemoryClient>;

pub struct MemoryState(pub std::sync::Mutex<Option<MemoryClientRef>>);

#[derive(Clone)]
pub struct MemoryClient {
    inner: Arc<UnifiedMemory>,
}

impl MemoryClient {
    pub fn from_token(_jwt_token: String) -> Option<Self> {
        Self::new_local().ok()
    }

    pub fn new_local() -> Result<Self, String> {
        let workspace_dir = dirs::home_dir()
            .ok_or_else(|| "Failed to resolve home directory".to_string())?
            .join(".openhuman")
            .join("workspace");
        Self::from_workspace_dir(workspace_dir)
    }

    pub fn from_workspace_dir(workspace_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&workspace_dir)
            .map_err(|e| format!("Create workspace dir {}: {e}", workspace_dir.display()))?;
        let embedder: Arc<dyn EmbeddingProvider> = embeddings::default_local_embedding_provider();
        let memory =
            UnifiedMemory::new(&workspace_dir, embedder, None).map_err(|e| format!("{e}"))?;
        Ok(Self {
            inner: Arc::new(memory),
        })
    }

    pub async fn put_doc(&self, input: NamespaceDocumentInput) -> Result<String, String> {
        self.inner.upsert_document(input).await
    }

    pub async fn ingest_doc(
        &self,
        request: MemoryIngestionRequest,
    ) -> Result<MemoryIngestionResult, String> {
        self.inner.ingest_document(request).await
    }

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
        self.inner
            .upsert_document(NamespaceDocumentInput {
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
            })
            .await
            .map(|_| ())
    }

    pub async fn list_documents(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.inner.list_documents(namespace).await
    }

    pub async fn list_namespaces(&self) -> Result<Vec<String>, String> {
        self.inner.list_namespaces().await
    }

    pub async fn delete_document(
        &self,
        namespace: &str,
        document_id: &str,
    ) -> Result<serde_json::Value, String> {
        self.inner.delete_document(namespace, document_id).await
    }

    pub async fn clear_namespace(&self, namespace: &str) -> Result<(), String> {
        self.inner.clear_namespace(namespace).await
    }

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

    pub async fn recall_namespace(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<Option<String>, String> {
        self.inner
            .recall_namespace_context(namespace, max_chunks)
            .await
    }

    pub async fn recall_namespace_context_data(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<NamespaceRetrievalContext, String> {
        self.inner
            .recall_namespace_context_data(namespace, max_chunks)
            .await
    }

    pub async fn recall_namespace_memories(
        &self,
        namespace: &str,
        limit: u32,
    ) -> Result<Vec<NamespaceMemoryHit>, String> {
        self.inner.recall_namespace_memories(namespace, limit).await
    }

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

    pub async fn kv_delete(&self, namespace: Option<&str>, key: &str) -> Result<bool, String> {
        match namespace {
            Some(ns) => self.inner.kv_delete_namespace(ns, key).await,
            None => self.inner.kv_delete_global(key).await,
        }
    }

    pub async fn kv_list_namespace(
        &self,
        namespace: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        self.inner.kv_list_namespace(namespace).await
    }

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
            None => self.inner.graph_query_global(subject, predicate).await,
        }
    }
}
