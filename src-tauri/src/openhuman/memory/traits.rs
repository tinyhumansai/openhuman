use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A single memory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub content: String,
    pub category: MemoryCategory,
    pub timestamp: String,
    pub session_id: Option<String>,
    pub score: Option<f64>,
}

/// Memory categories for organization
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// Long-term facts, preferences, decisions
    Core,
    /// Daily session logs
    Daily,
    /// Conversation context
    Conversation,
    /// User-defined custom category
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

/// Core memory trait — implement for any persistence backend
#[async_trait]
pub trait Memory: Send + Sync {
    /// Backend name
    fn name(&self) -> &str;

    /// Store a memory entry, optionally scoped to a session
    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()>;

    /// Recall memories matching a query (keyword search), optionally scoped to a session
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>>;

    /// Get a specific memory by key
    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>>;

    /// List all memory keys, optionally filtered by category and/or session
    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>>;

    /// Remove a memory by key
    async fn forget(&self, key: &str) -> anyhow::Result<bool>;

    /// Count total memories
    async fn count(&self) -> anyhow::Result<usize>;

    /// Health check
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
        assert_eq!(parsed.category, MemoryCategory::Core);
        assert_eq!(parsed.session_id.as_deref(), Some("session-abc"));
        assert_eq!(parsed.score, Some(0.98));
    }
}
