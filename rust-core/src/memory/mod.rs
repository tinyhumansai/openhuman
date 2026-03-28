//! Local persistent memory layer.
//!
//! Replaces cloud-backed memory sync with a fully local SQLite implementation.

mod db;
mod extraction;
mod ingestion;

use crate::memory::db::{default_db_path, init_schema};
use std::path::PathBuf;
use std::sync::Arc;

/// Shared, cloneable handle to the memory client.
pub type MemoryClientRef = Arc<MemoryClient>;

/// Shared app-state slot for the memory client.
pub struct MemoryState(pub std::sync::Mutex<Option<MemoryClientRef>>);

#[derive(Debug, Clone, Copy)]
pub enum SourceType {
    Doc,
    Chat,
    Email,
}

impl SourceType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::Chat => "chat",
            Self::Email => "email",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Priority {
    High,
    Medium,
    Low,
}

impl Priority {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

#[derive(Clone)]
pub struct MemoryClient {
    pub(crate) db_path: PathBuf,
}

impl MemoryClient {
    /// Construct a local memory client. Token is accepted for compatibility but ignored.
    pub fn from_token(_jwt_token: String) -> Option<Self> {
        Self::new_local().ok()
    }

    /// Construct local client without token.
    pub fn new_local() -> Result<Self, String> {
        let db_path = default_db_path()?;
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Create memory directory {}: {e}", parent.display()))?;
        }

        let conn = rusqlite::Connection::open(&db_path)
            .map_err(|e| format!("Open memory db {}: {e}", db_path.display()))?;
        init_schema(&conn)?;

        Ok(Self { db_path })
    }

    /// Store a skill data-sync result locally and atomically refresh its chunks.
    #[allow(clippy::too_many_arguments)]
    pub async fn store_skill_sync(
        &self,
        skill_id: &str,
        _integration_id: &str,
        title: &str,
        content: &str,
        source_type: Option<SourceType>,
        metadata: Option<serde_json::Value>,
        priority: Option<Priority>,
        created_at: Option<f64>,
        updated_at: Option<f64>,
        document_id: Option<String>,
    ) -> Result<(), String> {
        ingestion::store_skill_sync(
            self,
            skill_id,
            title,
            content,
            source_type,
            metadata,
            priority,
            created_at,
            updated_at,
            document_id,
        )
        .await
    }

    /// Query relevant context for a skill namespace.
    pub async fn query_skill_context(
        &self,
        skill_id: &str,
        _integration_id: &str,
        query: &str,
        max_chunks: u32,
    ) -> Result<String, String> {
        self.query_namespace_context(skill_id, query, max_chunks).await
    }

    /// Recall most recent context from a namespace.
    pub async fn recall_skill_context(
        &self,
        skill_id: &str,
        _integration_id: &str,
        max_chunks: u32,
    ) -> Result<Option<serde_json::Value>, String> {
        let ctx = self.recall_namespace_context(skill_id, max_chunks).await?;
        Ok(ctx.map(serde_json::Value::String))
    }

    /// List all ingested memory documents.
    pub async fn list_documents(&self) -> Result<serde_json::Value, String> {
        extraction::list_documents(self).await
    }

    /// Delete a specific document from a namespace.
    pub async fn delete_document(
        &self,
        document_id: &str,
        namespace: &str,
    ) -> Result<serde_json::Value, String> {
        extraction::delete_document(self, document_id, namespace).await
    }

    /// Query memory context by namespace.
    pub async fn query_namespace_context(
        &self,
        namespace: &str,
        query: &str,
        max_chunks: u32,
    ) -> Result<String, String> {
        extraction::query_namespace_context(self, namespace, query, max_chunks).await
    }

    /// Recall memory context by namespace directly.
    pub async fn recall_namespace_context(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<Option<String>, String> {
        extraction::recall_namespace_context(self, namespace, max_chunks).await
    }

    /// Delete all memories for a skill namespace.
    pub async fn clear_skill_memory(
        &self,
        skill_id: &str,
        _integration_id: &str,
    ) -> Result<(), String> {
        ingestion::clear_skill_memory(self, skill_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn local_memory_store_and_query() {
        let client = MemoryClient::new_local().expect("local client");
        client
            .store_skill_sync(
                "gmail",
                "gmail",
                "Test",
                "Alice sent a portfolio update. Bob sent meeting notes.",
                Some(SourceType::Email),
                None,
                Some(Priority::Medium),
                None,
                None,
                Some(format!("test-{}", Uuid::new_v4())),
            )
            .await
            .expect("store");

        let q = client
            .query_skill_context("gmail", "gmail", "portfolio", 5)
            .await
            .expect("query");
        assert!(q.to_ascii_lowercase().contains("portfolio"));

        let recall = client
            .recall_skill_context("gmail", "gmail", 5)
            .await
            .expect("recall");
        assert!(recall.is_some());
    }
}
