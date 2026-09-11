use super::memory_context_safety::{is_potentially_untrusted, wrap_untrusted_for_agent};
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
/// `agent_memory::memory_loader` so the same content renders at the same
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

    // Pull relevant memories for this message — routed through the tinyagents
    // retrieval facade (issue #4249, 09.2) so recall is swappable and emits
    // `MemoryLoaded`. The facade wraps `Memory::recall` verbatim, so the
    // rendered `[Memory context]` block stays byte-identical.
    if let Ok(entries) = crate::openhuman::agent::tinyagents::retriever::recall_through_facade(
        mem,
        user_msg,
        5,
        crate::openhuman::memory::RecallOpts::default(),
    )
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
                let rendered_content =
                    if is_potentially_untrusted(entry.namespace.as_deref(), &entry.key) {
                        let hint = entry.namespace.as_deref().unwrap_or("connector");
                        wrap_untrusted_for_agent(&entry.content, hint)
                    } else {
                        entry.content.clone()
                    };
                let _ = writeln!(context, "- {}: {}", entry.key, rendered_content);
            }
            context.push('\n');
        }
    }

    // Explicitly load bounded user working memory entries so sync-derived profile
    // facts can influence the turn in a controlled way.
    let working_query = format!("working.user {user_msg}");
    if let Ok(entries) = crate::openhuman::agent::tinyagents::retriever::recall_through_facade(
        mem,
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
        crate::openhuman::agent::tinyagents::thread_context::current_thread_id();
    let cross_session_opts = crate::openhuman::memory::RecallOpts {
        session_id: current_thread_id.as_deref(),
        cross_session: true,
        min_score: Some(min_relevance_score),
        ..Default::default()
    };
    if let Ok(entries) = crate::openhuman::agent::tinyagents::retriever::recall_through_facade(
        mem,
        user_msg,
        CROSS_CHAT_LIMIT * 3,
        cross_session_opts,
    )
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
            // Use the canonical CROSS_CHAT_HEADER from `memory_loader` so
            // this fallback recall path emits the same literal as the
            // primary JSONL path, and the orchestrator prompt's
            // "Capability questions" section that names this header stays
            // in sync. See CROSS_CHAT_HEADER's doc for the rationale.
            context.push_str(crate::openhuman::memory::agent::memory_loader::CROSS_CHAT_HEADER);
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
#[path = "memory_context_tests.rs"]
mod tests;
