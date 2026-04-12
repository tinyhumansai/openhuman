use crate::openhuman::memory::Memory;
use async_trait::async_trait;
use std::collections::HashSet;

use super::harness::memory_context::{WORKING_MEMORY_KEY_PREFIX, WORKING_MEMORY_LIMIT};

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
