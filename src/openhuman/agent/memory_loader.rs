use crate::openhuman::memory::Memory;
use async_trait::async_trait;
use std::collections::HashSet;

const WORKING_MEMORY_KEY_PREFIX: &str = "working.user.";
const WORKING_MEMORY_LIMIT: usize = 3;

#[async_trait]
pub trait MemoryLoader: Send + Sync {
    async fn load_context(&self, memory: &dyn Memory, user_message: &str)
        -> anyhow::Result<String>;
}

pub struct DefaultMemoryLoader {
    limit: usize,
    min_relevance_score: f64,
    /// Maximum characters of memory context to inject (0 = unlimited).
    max_context_chars: usize,
}

impl Default for DefaultMemoryLoader {
    fn default() -> Self {
        Self {
            limit: 5,
            min_relevance_score: 0.4,
            max_context_chars: 2000,
        }
    }
}

impl DefaultMemoryLoader {
    pub fn new(limit: usize, min_relevance_score: f64) -> Self {
        Self {
            limit: limit.max(1),
            min_relevance_score,
            max_context_chars: 2000,
        }
    }

    pub fn with_max_chars(mut self, max_chars: usize) -> Self {
        self.max_context_chars = max_chars;
        self
    }
}

#[async_trait]
impl MemoryLoader for DefaultMemoryLoader {
    async fn load_context(
        &self,
        memory: &dyn Memory,
        user_message: &str,
    ) -> anyhow::Result<String> {
        let entries = memory.recall(user_message, self.limit, None).await?;
        let mut context = String::new();
        let budget = if self.max_context_chars > 0 {
            self.max_context_chars
        } else {
            usize::MAX
        };
        let mut seen_keys = HashSet::new();

        let header = "[Memory context]\n";
        for entry in entries {
            if let Some(score) = entry.score {
                if score < self.min_relevance_score {
                    continue;
                }
            }
            let line = format!("- {}: {}\n", entry.key, entry.content);
            if context.is_empty() {
                if header.len() >= budget {
                    return Ok(String::new());
                }
                context.push_str(header);
            }
            if context.len() + line.len() > budget {
                tracing::debug!(
                    budget,
                    current_len = context.len(),
                    skipped_line_len = line.len(),
                    "[memory_loader] context budget reached, skipping remaining entries"
                );
                break;
            }
            seen_keys.insert(entry.key);
            context.push_str(&line);
        }

        // Explicit bounded recall for sync-derived user working memory.
        let working_query = format!("working.user {user_message}");
        let working_entries = memory
            .recall(&working_query, WORKING_MEMORY_LIMIT + 2, None)
            .await
            .unwrap_or_default();
        let mut appended_working_header = false;
        for entry in working_entries
            .into_iter()
            .filter(|entry| entry.key.starts_with(WORKING_MEMORY_KEY_PREFIX))
            .filter(|entry| !seen_keys.contains(&entry.key))
            .filter(|entry| match entry.score {
                Some(score) => score >= self.min_relevance_score,
                None => true,
            })
            .take(WORKING_MEMORY_LIMIT)
        {
            if !appended_working_header {
                let section = "[User working memory]\n";
                if context.len() + section.len() > budget {
                    break;
                }
                context.push_str(section);
                appended_working_header = true;
            }
            let line = format!("- {}: {}\n", entry.key, entry.content);
            if context.len() + line.len() > budget {
                tracing::debug!(
                    budget,
                    current_len = context.len(),
                    skipped_line_len = line.len(),
                    "[memory_loader] context budget reached while appending working memory"
                );
                break;
            }
            context.push_str(&line);
        }

        if context.is_empty() {
            return Ok(String::new());
        }
        context.push('\n');
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};

    struct MockMemory;

    #[async_trait]
    impl Memory for MockMemory {
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
            query: &str,
            limit: usize,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            if limit == 0 {
                return Ok(vec![]);
            }
            if query.contains("working.user") {
                return Ok(vec![MemoryEntry {
                    id: "2".into(),
                    key: "working.user.gmail.summary".into(),
                    content: "User prefers concise updates.".into(),
                    namespace: Some("global".into()),
                    category: MemoryCategory::Core,
                    timestamp: "now".into(),
                    session_id: None,
                    score: Some(0.95),
                }]);
            }
            Ok(vec![MemoryEntry {
                id: "1".into(),
                key: "k".into(),
                content: "v".into(),
                namespace: None,
                category: MemoryCategory::Conversation,
                timestamp: "now".into(),
                session_id: None,
                score: None,
            }])
        }

        async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(vec![])
        }

        async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
            Ok(true)
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }

        fn name(&self) -> &str {
            "mock"
        }
    }

    #[tokio::test]
    async fn default_loader_formats_context() {
        let loader = DefaultMemoryLoader::default();
        let context = loader.load_context(&MockMemory, "hello").await.unwrap();
        assert!(context.contains("[Memory context]"));
        assert!(context.contains("- k: v"));
        assert!(context.contains("[User working memory]"));
        assert!(context.contains("working.user.gmail.summary"));
    }
}
