use crate::openhuman::memory::Memory;
use crate::openhuman::util::provenance_tag;
use std::collections::HashSet;
use std::fmt::Write;

pub(crate) const WORKING_MEMORY_KEY_PREFIX: &str = "working.user.";
pub(crate) const WORKING_MEMORY_LIMIT: usize = 3;

/// Maximum number of `[Cross-chat context]` lines surfaced into the
/// working prompt. Tight cap on purpose: cross-chat hits are a recovery
/// signal for "I told you in another window" continuity, not a dump of
/// unrelated chats. See issue #1505.
pub(crate) const CROSS_CHAT_LIMIT: usize = 3;

/// Maximum characters of any one cross-chat snippet rendered into the
/// prompt. Keeps the block bounded even if a prior chat had long turns.
/// Shared across the harness path here and the loader-side path in
/// `agent::memory_loader` so the same content renders at the same
/// length regardless of which code path emitted it.
pub(crate) const CROSS_CHAT_SNIPPET_CHARS: usize = 240;

/// Trim a cross-chat snippet to a bounded preview without panicking on
/// UTF-8 codepoint boundaries. Reuses the project-wide ellipsis helper
/// so the suffix accounting stays consistent with other prompt blocks.
fn shorten_for_cross_chat(content: &str) -> String {
    if content.chars().count() > CROSS_CHAT_SNIPPET_CHARS {
        crate::openhuman::util::truncate_with_ellipsis(content, CROSS_CHAT_SNIPPET_CHARS)
    } else {
        content.to_string()
    }
}

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
                seen_keys.insert(entry.key.clone());
                let _ = writeln!(context, "- {}: {}", entry.key, entry.content);
            }
            context.push('\n');
        }
    }

    // ── Cross-chat context (#1505) ───────────────────────────────────────
    //
    // Same user, multiple chats. Pull conversational hits from OTHER
    // sessions so context the user shared in chat A is recoverable when
    // they ask a dependent question in chat B. Workspace/user scope is
    // enforced at the SQLite layer (one DB per workspace == one user);
    // the current chat is excluded by passing `session_id` so the block
    // never duplicates the same-chat history.
    let current_thread_id =
        crate::openhuman::inference::provider::thread_context::current_thread_id();
    let cross_session_opts = crate::openhuman::memory::RecallOpts {
        session_id: current_thread_id.as_deref(),
        cross_session: true,
        min_score: Some(min_relevance_score),
        ..Default::default()
    };
    if let Ok(entries) = mem
        .recall(user_msg, CROSS_CHAT_LIMIT * 3, cross_session_opts)
        .await
    {
        let cross: Vec<_> = entries
            .iter()
            .filter(|e| e.id.starts_with("episodic-cross:"))
            .filter(|e| !seen_keys.contains(&e.key))
            .filter(|e| match e.score {
                Some(score) => score >= min_relevance_score,
                None => true,
            })
            .take(CROSS_CHAT_LIMIT)
            .collect();

        tracing::debug!(
            "[memory-context] cross-chat recall returned {} entries, {} after filtering (exclude={:?})",
            entries.len(),
            cross.len(),
            current_thread_id
        );

        if !cross.is_empty() {
            context.push_str("[Cross-chat context]\n");
            for entry in &cross {
                let prov = entry
                    .session_id
                    .as_deref()
                    .map(provenance_tag)
                    .unwrap_or_else(|| "chat:unknown".to_string());
                let snippet = shorten_for_cross_chat(&entry.content);
                let _ = writeln!(context, "- [{prov}] {snippet}");
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
        cross_chat: Vec<MemoryEntry>,
        fail_primary: bool,
    }

    impl MockMemory {
        fn new(primary: Vec<MemoryEntry>, working: Vec<MemoryEntry>, fail_primary: bool) -> Self {
            Self {
                primary,
                working,
                cross_chat: Vec::new(),
                fail_primary,
            }
        }
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
            opts: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            if opts.cross_session {
                // Mirror the production exclusion contract: when an
                // active thread id is threaded through RecallOpts, drop
                // hits whose `session_id` matches it. Without this, the
                // privacy/scope guard at `build_context` time can
                // silently regress and tests still pass.
                let exclude = opts.session_id;
                return Ok(self
                    .cross_chat
                    .clone()
                    .into_iter()
                    .filter(|e| match (exclude, e.session_id.as_deref()) {
                        (Some(tid), Some(sid)) => sid != tid,
                        _ => true,
                    })
                    .collect());
            }
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

    fn cross_chat_entry(
        cross_id: &str,
        session_id: &str,
        content: &str,
        score: Option<f64>,
    ) -> MemoryEntry {
        MemoryEntry {
            id: format!("episodic-cross:{cross_id}"),
            key: format!("{session_id}:user"),
            content: content.into(),
            namespace: None,
            category: MemoryCategory::Conversation,
            timestamp: "now".into(),
            session_id: Some(session_id.into()),
            score,
        }
    }

    #[tokio::test]
    async fn build_context_filters_scores_and_deduplicates_working_memory() {
        let mem = MockMemory::new(
            vec![
                entry("task", "primary entry", Some(0.9)),
                entry("low", "too low", Some(0.1)),
                entry("working.user.profile", "already present", Some(0.9)),
            ],
            vec![
                entry("working.user.profile", "already present", Some(0.95)),
                entry("working.user.timezone", "PST", Some(0.95)),
            ],
            false,
        );

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
        let mem = MockMemory::new(
            Vec::new(),
            vec![entry("working.user.pref", "Use Rust", None)],
            true,
        );

        let context = build_context(&mem, "hello", 0.4).await;
        assert!(!context.contains("[Memory context]"));
        assert!(context.contains("[User working memory]"));
        assert!(context.contains("Use Rust"));
    }

    #[tokio::test]
    async fn build_context_returns_empty_when_nothing_relevant_is_found() {
        let mem = MockMemory::new(
            vec![entry("low", "too low", Some(0.1))],
            vec![entry("not_working", "ignored", Some(0.9))],
            false,
        );

        assert!(build_context(&mem, "hello", 0.4).await.is_empty());
    }

    // ── Cross-chat context (#1505) ───────────────────────────────────────

    #[tokio::test]
    async fn build_context_surfaces_cross_chat_block_with_provenance() {
        let mut mem = MockMemory::new(Vec::new(), Vec::new(), false);
        mem.cross_chat = vec![cross_chat_entry(
            "1",
            "thread-source",
            "I prefer Postgres for new services",
            Some(0.9),
        )];

        let context = build_context(&mem, "what database should I use?", 0.4).await;
        assert!(
            context.contains("[Cross-chat context]"),
            "expected cross-chat header, got:\n{context}"
        );
        assert!(
            context.contains("Postgres"),
            "expected the cross-chat fact in the context, got:\n{context}"
        );
        assert!(
            context.contains("[chat:"),
            "expected provenance tag in cross-chat block, got:\n{context}"
        );
        assert!(
            !context.contains("thread-source"),
            "raw session id MUST NOT leak into the prompt — render only the hashed tag, got:\n{context}"
        );
    }

    #[tokio::test]
    async fn build_context_drops_low_score_cross_chat_entries() {
        let mut mem = MockMemory::new(Vec::new(), Vec::new(), false);
        mem.cross_chat = vec![
            cross_chat_entry("1", "thread-a", "Postgres preference", Some(0.05)),
            cross_chat_entry("2", "thread-b", "Postgres timezone fact", Some(0.9)),
        ];

        let context = build_context(&mem, "Postgres", 0.4).await;
        assert!(
            context.contains("Postgres timezone fact"),
            "high-score cross-chat must surface, got:\n{context}"
        );
        assert!(
            !context.contains("Postgres preference"),
            "low-score cross-chat must be filtered, got:\n{context}"
        );
    }

    #[tokio::test]
    async fn build_context_caps_cross_chat_block_length() {
        let mut mem = MockMemory::new(Vec::new(), Vec::new(), false);
        mem.cross_chat = (0..10)
            .map(|i| {
                cross_chat_entry(
                    &format!("{i}"),
                    &format!("thread-{i}"),
                    &format!("Cross-chat fact #{i}"),
                    Some(0.9),
                )
            })
            .collect();

        let context = build_context(&mem, "Cross-chat fact", 0.4).await;
        let cross_lines = context
            .lines()
            .filter(|l| l.starts_with("- [chat:"))
            .count();
        assert!(
            cross_lines <= CROSS_CHAT_LIMIT,
            "cross-chat block must be capped at {CROSS_CHAT_LIMIT}, saw {cross_lines} lines"
        );
    }

    #[tokio::test]
    async fn build_context_skips_cross_chat_block_when_no_other_chats_match() {
        let mem = MockMemory::new(Vec::new(), Vec::new(), false);
        let context = build_context(&mem, "Postgres", 0.4).await;
        assert!(
            !context.contains("[Cross-chat context]"),
            "no cross-chat hits must produce no header, got:\n{context}"
        );
    }
}
