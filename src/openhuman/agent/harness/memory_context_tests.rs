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
        taint: Default::default(),
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
        taint: Default::default(),
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
        context
            .contains(crate::openhuman::memory::agent::memory_loader::CROSS_CHAT_HEADER.trim_end()),
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
        !context
            .contains(crate::openhuman::memory::agent::memory_loader::CROSS_CHAT_HEADER.trim_end()),
        "no cross-chat hits must produce no header, got:\n{context}"
    );
}
