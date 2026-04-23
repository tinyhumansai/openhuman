use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::memory_loader::{DefaultMemoryLoader, MemoryLoader};
use openhuman_core::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use std::sync::Arc;

struct ScriptedMemory {
    primary: Vec<MemoryEntry>,
    working: Vec<MemoryEntry>,
}

#[async_trait]
impl Memory for ScriptedMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        _limit: usize,
        _opts: openhuman_core::openhuman::memory::RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        if query.contains("working.user") {
            Ok(self.working.clone())
        } else {
            Ok(self.primary.clone())
        }
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(
        &self,
    ) -> Result<Vec<openhuman_core::openhuman::memory::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "scripted"
    }
}

fn entry(key: &str, content: &str, score: Option<f64>) -> MemoryEntry {
    MemoryEntry {
        id: key.into(),
        key: key.into(),
        content: content.into(),
        namespace: None,
        category: MemoryCategory::Conversation,
        timestamp: "now".into(),
        session_id: None,
        score,
    }
}

#[tokio::test]
async fn loader_merges_primary_and_working_memory_with_filters() -> Result<()> {
    let memory: Arc<dyn Memory> = Arc::new(ScriptedMemory {
        primary: vec![
            entry("high", "keep me", Some(0.9)),
            entry("low", "drop me", Some(0.1)),
        ],
        working: vec![
            entry("working.user.pref", "concise", Some(0.95)),
            entry("high", "duplicate", Some(0.95)),
        ],
    });

    let context = DefaultMemoryLoader::new(5, 0.4)
        .with_max_chars(200)
        .load_context(memory.as_ref(), "hello")
        .await?;

    assert!(context.contains("[Memory context]"));
    assert!(context.contains("- high: keep me"));
    assert!(!context.contains("drop me"));
    assert!(context.contains("[User working memory]"));
    assert!(context.contains("working.user.pref"));
    assert!(!context.contains("duplicate"));
    Ok(())
}

#[tokio::test]
async fn loader_can_return_only_working_memory_when_primary_is_empty() -> Result<()> {
    let memory: Arc<dyn Memory> = Arc::new(ScriptedMemory {
        primary: Vec::new(),
        working: vec![entry("working.user.todo", "ship it", None)],
    });

    let context = DefaultMemoryLoader::default()
        .load_context(memory.as_ref(), "hello")
        .await?;

    assert!(!context.contains("[Memory context]"));
    assert!(context.contains("[User working memory]"));
    assert!(context.contains("working.user.todo"));
    Ok(())
}

#[tokio::test]
async fn loader_respects_tight_budgets() -> Result<()> {
    let memory: Arc<dyn Memory> = Arc::new(ScriptedMemory {
        primary: vec![entry("main", "1234567890", Some(0.9))],
        working: vec![entry("working.user.tip", "include me", Some(0.9))],
    });

    let header_len = "[Memory context]\n".len();
    let empty = DefaultMemoryLoader::new(1, 0.4)
        .with_max_chars(header_len)
        .load_context(memory.as_ref(), "hello")
        .await?;
    assert!(empty.is_empty());

    let bounded = DefaultMemoryLoader::new(1, 0.4)
        .with_max_chars("[Memory context]\n- main: 1234567890\n".len() + 1)
        .load_context(memory.as_ref(), "hello")
        .await?;
    assert!(bounded.contains("- main: 1234567890"));
    assert!(!bounded.contains("working.user.tip"));
    Ok(())
}
