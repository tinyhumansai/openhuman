//! Core traits and data structures for the OpenHuman memory system.
//!
//! This module defines the foundational `Memory` trait that all storage backends
//! must implement, as well as the standard `MemoryEntry` and `MemoryCategory`
//! types used for representing and organizing memories.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Represents a single stored memory entry with associated metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique identifier for the memory entry (usually a UUID).
    pub id: String,
    /// The key or title associated with this memory.
    pub key: String,
    /// The actual content or value of the memory.
    pub content: String,
    /// Optional namespace for logical separation of memories.
    #[serde(default)]
    pub namespace: Option<String>,
    /// The organizational category this memory belongs to.
    pub category: MemoryCategory,
    /// ISO 8601 formatted timestamp of when the memory was created or last updated.
    pub timestamp: String,
    /// Optional session ID if this memory is scoped to a specific interaction.
    pub session_id: Option<String>,
    /// Optional relevance or confidence score, typically from 0.0 to 1.0.
    pub score: Option<f64>,
}

/// Categories used to organize and filter memories by their nature and lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// Long-term foundational facts, user preferences, and permanent decisions.
    Core,
    /// Temporal logs reflecting daily activities or ephemeral state.
    Daily,
    /// Contextual information derived from and relevant to active conversations.
    Conversation,
    /// A user-defined or system-defined custom category.
    Custom(String),
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core => write!(f, "core"),
            Self::Daily => write!(f, "daily"),
            Self::Conversation => write!(f, "conversation"),
            Self::Custom(name) => write!(f, "{name}"),
        }
    }
}

/// The core trait for memory storage and retrieval.
///
/// Any persistence backend (SQLite, Postgres, Vector DB, etc.) should implement
/// this trait to be used within the OpenHuman ecosystem.
#[async_trait]
pub trait Memory: Send + Sync {
    /// Returns the name of the memory backend (e.g., "sqlite", "vector").
    fn name(&self) -> &str;

    /// Stores a new memory entry or updates an existing one.
    ///
    /// # Arguments
    /// * `key` - The lookup key for the memory.
    /// * `content` - The actual data to store.
    /// * `category` - The organizational category.
    /// * `session_id` - Optional session scope.
    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()>;

    /// Recalls memories matching a query string using keyword or semantic search.
    ///
    /// # Arguments
    /// * `query` - The search term or natural language query.
    /// * `limit` - Maximum number of results to return.
    /// * `session_id` - Optional filter to scope search to a specific session.
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>>;

    /// Retrieves a specific memory entry by its exact key.
    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>>;

    /// Lists memory entries, optionally filtered by category and/or session.
    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>>;

    /// Deletes a memory entry associated with the given key.
    ///
    /// Returns `Ok(true)` if the entry was found and deleted, `Ok(false)` if not found.
    async fn forget(&self, key: &str) -> anyhow::Result<bool>;

    /// Returns the total count of all memory entries in the backend.
    async fn count(&self) -> anyhow::Result<usize>;

    /// Performs a health check on the underlying storage system.
    async fn health_check(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_category_display_outputs_expected_values() {
        assert_eq!(MemoryCategory::Core.to_string(), "core");
        assert_eq!(MemoryCategory::Daily.to_string(), "daily");
        assert_eq!(MemoryCategory::Conversation.to_string(), "conversation");
        assert_eq!(
            MemoryCategory::Custom("project_notes".into()).to_string(),
            "project_notes"
        );
    }

    #[test]
    fn memory_category_serde_uses_snake_case() {
        let core = serde_json::to_string(&MemoryCategory::Core).unwrap();
        let daily = serde_json::to_string(&MemoryCategory::Daily).unwrap();
        let conversation = serde_json::to_string(&MemoryCategory::Conversation).unwrap();

        assert_eq!(core, "\"core\"");
        assert_eq!(daily, "\"daily\"");
        assert_eq!(conversation, "\"conversation\"");
    }

    #[test]
    fn memory_entry_roundtrip_preserves_optional_fields() {
        let entry = MemoryEntry {
            id: "id-1".into(),
            key: "favorite_language".into(),
            content: "Rust".into(),
            namespace: Some("global".into()),
            category: MemoryCategory::Core,
            timestamp: "2026-02-16T00:00:00Z".into(),
            session_id: Some("session-abc".into()),
            score: Some(0.98),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: MemoryEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "id-1");
        assert_eq!(parsed.key, "favorite_language");
        assert_eq!(parsed.content, "Rust");
        assert_eq!(parsed.namespace.as_deref(), Some("global"));
        assert_eq!(parsed.category, MemoryCategory::Core);
        assert_eq!(parsed.session_id.as_deref(), Some("session-abc"));
        assert_eq!(parsed.score, Some(0.98));
    }
}
