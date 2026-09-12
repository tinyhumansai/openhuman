use crate::openhuman::memory::Memory;
use serde::{Deserialize, Serialize};

/// Maximum number of `[Prior conversations]` lines surfaced into the prompt
/// at the start of a fresh chat. Tight cap on purpose: this block is meant
/// to recover continuity for high-importance facts, not to dump session
/// history into context. See issue #1399.
const PRIOR_CONVERSATION_LIMIT: usize = 3;
/// Only the importance prefix `high.` survives into the prompt block.
/// Medium/low entries stay queryable via the on-demand memory tool but
/// do not auto-pollute every fresh chat.
const PRIOR_CONVERSATION_KEY_PREFIX: &str = "high.";

/// Canonical header for the `[Cross-chat context]` block injected on
/// every turn that has FTS-surfaced hits from other threads.
///
/// The "historical" / "capabilities may have changed since" suffix is
/// deliberate: it tells the model these snippets are snapshots from
/// earlier moments and that capability claims (e.g. "I can't delete
/// emails") may be stale because the tool surface or per-toolkit scope
/// toggles can change between chats.
///
/// Single source of truth — all three call sites bind to this constant
/// so a wording tweak doesn't drift between (a) `memory_loader.rs`'s
/// primary JSONL path, (b) `harness/memory_context.rs`'s fallback
/// recall path, and (c) the orchestrator's "Capability questions"
/// prompt section that names the header verbatim. Tests assert on this
/// constant too — see `memory_loader::tests` and
/// `harness::memory_context::tests`.
pub const CROSS_CHAT_HEADER: &str =
    "[Cross-chat context — historical; capabilities may have changed since]\n";

/// Lightweight citation object derived from recalled memory entries.
///
/// These citations are attached to agent responses so the UI can show
/// provenance for memory-informed answers without exposing full raw memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCitation {
    pub id: String,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub timestamp: String,
    pub snippet: String,
}

/// Collect citation metadata from semantic memory recall for a user turn.
///
/// This mirrors the primary recall path used by `DefaultMemoryLoader` so the
/// UI can display trusted sources whenever memory context influenced a reply.
pub async fn collect_recall_citations(
    memory: &dyn Memory,
    user_message: &str,
    limit: usize,
    min_relevance_score: f64,
) -> anyhow::Result<Vec<MemoryCitation>> {
    // Routed through the tinyagents retrieval facade (issue #4249, 09.2): the
    // facade wraps `Memory::recall` verbatim (ranking engine unchanged) so the
    // citation set stays byte-identical, while making retrieval swappable and
    // emitting `MemoryLoaded`.
    let entries = crate::openhuman::agent::tinyagents::retriever::recall_through_facade(
        memory,
        user_message,
        limit.max(1),
        crate::openhuman::memory::RecallOpts::default(),
    )
    .await?;

    let citations = entries
        .into_iter()
        .filter(|entry| match entry.score {
            Some(score) => score >= min_relevance_score,
            None => true,
        })
        .map(|entry| {
            let snippet = if entry.content.chars().count() > 280 {
                crate::openhuman::util::truncate_with_ellipsis(&entry.content, 280)
            } else {
                entry.content
            };
            MemoryCitation {
                id: entry.id,
                key: entry.key,
                namespace: entry.namespace,
                score: entry.score,
                timestamp: entry.timestamp,
                snippet,
            }
        })
        .collect();

    Ok(citations)
}

#[cfg(test)]
#[path = "memory_loader_tests.rs"]
mod tests;
