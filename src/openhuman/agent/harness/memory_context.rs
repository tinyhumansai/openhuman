use crate::openhuman::memory::Memory;
use std::collections::HashSet;
use std::fmt::Write;

pub(crate) const WORKING_MEMORY_KEY_PREFIX: &str = "working.user.";
pub(crate) const WORKING_MEMORY_LIMIT: usize = 3;

/// Build context preamble by searching memory for relevant entries.
/// Entries with a hybrid score below `min_relevance_score` are dropped to
/// prevent unrelated memories from bleeding into the conversation.
pub(crate) async fn build_context(
    mem: &dyn Memory,
    user_msg: &str,
    min_relevance_score: f64,
) -> String {
    let mut context = String::new();
    let mut seen_keys = HashSet::new();

    // Pull relevant memories for this message
    if let Ok(entries) = mem
        .recall(user_msg, 5, crate::openhuman::memory::RecallOpts::default())
        .await
    {
        let relevant: Vec<_> = entries
            .iter()
            .filter(|e| match e.score {
                Some(score) => score >= min_relevance_score,
                None => true,
            })
            .collect();

        if !relevant.is_empty() {
            context.push_str("[Memory context]\n");
            for entry in &relevant {
                seen_keys.insert(entry.key.clone());
                let _ = writeln!(context, "- {}: {}", entry.key, entry.content);
            }
            context.push('\n');
        }
    }

    // Explicitly load bounded user working memory entries so sync-derived profile
    // facts can influence the turn in a controlled way.
    let working_query = format!("working.user {user_msg}");
    if let Ok(entries) = mem
        .recall(
            &working_query,
            WORKING_MEMORY_LIMIT + 2,
            crate::openhuman::memory::RecallOpts::default(),
        )
        .await
    {
        let working: Vec<_> = entries
            .iter()
            .filter(|entry| entry.key.starts_with(WORKING_MEMORY_KEY_PREFIX))
            .filter(|entry| !seen_keys.contains(&entry.key))
            .filter(|entry| match entry.score {
                Some(score) => score >= min_relevance_score,
                None => true,
            })
            .take(WORKING_MEMORY_LIMIT)
            .collect();

        if !working.is_empty() {
            context.push_str("[User working memory]\n");
            for entry in &working {
                let _ = writeln!(context, "- {}: {}", entry.key, entry.content);
            }
            context.push('\n');
        }
    }

    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
    use async_trait::async_trait;

    struct MockMemory {
        primary: Vec<MemoryEntry>,
        working: Vec<MemoryEntry>,
        fail_primary: bool,
    }

    #[async_trait]
    impl Memory for MockMemory {
        fn name(&self) -> &str {
            "mock"
        }

        async fn store(
            &self,
            _namespace: &str,
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
            _limit: usize,
            _opts: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            if query.starts_with("working.user ") {
                return Ok(self.working.clone());
            }
            if self.fail_primary {
                anyhow::bail!("primary recall failed");
            }
            Ok(self.primary.clone())
        }

        async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
            Ok(Vec::new())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
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
    async fn build_context_filters_scores_and_deduplicates_working_memory() {
        let mem = MockMemory {
            primary: vec![
                entry("task", "primary entry", Some(0.9)),
                entry("low", "too low", Some(0.1)),
                entry("working.user.profile", "already present", Some(0.9)),
            ],
            working: vec![
                entry("working.user.profile", "already present", Some(0.95)),
                entry("working.user.timezone", "PST", Some(0.95)),
            ],
            fail_primary: false,
        };

        let context = build_context(&mem, "hello", 0.4).await;
        assert!(context.contains("[Memory context]"));
        assert!(context.contains("- task: primary entry"));
        assert!(!context.contains("too low"));
        assert!(context.contains("[User working memory]"));
        assert!(context.contains("- working.user.timezone: PST"));
        assert_eq!(context.matches("working.user.profile").count(), 1);
    }

    #[tokio::test]
    async fn build_context_uses_working_memory_even_if_primary_recall_fails() {
        let mem = MockMemory {
            primary: Vec::new(),
            working: vec![entry("working.user.pref", "Use Rust", None)],
            fail_primary: true,
        };

        let context = build_context(&mem, "hello", 0.4).await;
        assert!(!context.contains("[Memory context]"));
        assert!(context.contains("[User working memory]"));
        assert!(context.contains("Use Rust"));
    }

    #[tokio::test]
    async fn build_context_returns_empty_when_nothing_relevant_is_found() {
        let mem = MockMemory {
            primary: vec![entry("low", "too low", Some(0.1))],
            working: vec![entry("not_working", "ignored", Some(0.9))],
            fail_primary: false,
        };

        assert!(build_context(&mem, "hello", 0.4).await.is_empty());
    }
}
