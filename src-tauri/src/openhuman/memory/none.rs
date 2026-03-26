use super::traits::{Memory, MemoryCategory, MemoryEntry};
use async_trait::async_trait;

/// Explicit no-op memory backend.
///
/// This backend is used when `memory.backend = "none"` to disable persistence
/// while keeping the runtime wiring stable.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoneMemory;

impl NoneMemory {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Memory for NoneMemory {
    fn name(&self) -> &str {
        "none"
    }

    async fn store(
        &self,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn none_memory_is_noop() {
        let memory = NoneMemory::new();

        memory
            .store("k", "v", MemoryCategory::Core, None)
            .await
            .unwrap();

        assert!(memory.get("k").await.unwrap().is_none());
        assert!(memory.recall("k", 10, None).await.unwrap().is_empty());
        assert!(memory.list(None, None).await.unwrap().is_empty());
        assert!(!memory.forget("k").await.unwrap());
        assert_eq!(memory.count().await.unwrap(), 0);
        assert!(memory.health_check().await);
    }
}
