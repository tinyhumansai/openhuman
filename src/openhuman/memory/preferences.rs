//! Two-lane explicit user preferences — namespaces + read helpers.
//!
//! Preferences written by the `save_preference` tool live in one of two
//! namespaces depending on their relevance scope:
//!
//! - [`USER_PREF_GENERAL_NAMESPACE`] — always-on; injected into the system
//!   prompt at thread start (Lane A).
//! - [`USER_PREF_SITUATIONAL_NAMESPACE`] — topic-scoped; recalled per-turn by
//!   semantic similarity to the user's message (Lane B).
//!
//! Keeping the namespace constants and read helpers here (rather than in the
//! tool module) lets the write path, the system-prompt builder, and the
//! per-turn recall path all share one definition.

use std::sync::Arc;

use super::Memory;

/// Always-on preferences — injected into the system prompt every thread.
pub const USER_PREF_GENERAL_NAMESPACE: &str = "user_pref_general";

/// Topic-scoped preferences — recalled per query against the user's message.
pub const USER_PREF_SITUATIONAL_NAMESPACE: &str = "user_pref_situational";

/// Default cap on general preferences injected into the system prompt. Keeps
/// the always-on block bounded so it can't blow a small model's context window
/// (see the legacy `gpt-4` 8K overflow).
pub const STANDING_PREFS_LIMIT: usize = 10;

/// Load the latest-`limit` general preferences as plain-language strings,
/// newest-first (by `updated_at`). This is the Lane-A system-prompt block.
///
/// `list()` returns entries ordered newest-first but with `content` set to the
/// title (= topic key), so the body value is fetched via `get()`.
pub async fn load_general_preferences(memory: &Arc<dyn Memory>, limit: usize) -> Vec<String> {
    let entries = memory
        .list(Some(USER_PREF_GENERAL_NAMESPACE), None, None)
        .await
        .unwrap_or_default();

    let mut out = Vec::new();
    for entry in entries.into_iter().take(limit) {
        if let Ok(Some(full)) = memory.get(USER_PREF_GENERAL_NAMESPACE, &entry.key).await {
            let value = full.content.trim();
            if !value.is_empty() {
                out.push(value.to_string());
            }
        }
    }
    out
}

/// Top-K situational preferences to recall per turn (Lane B).
pub const SITUATIONAL_RECALL_LIMIT: usize = 5;

/// Minimum query↔preference vector similarity for a situational preference to be
/// injected. Below this the current message isn't considered relevant to the
/// preference, so nothing is injected (the "unrelated query → no block"
/// behaviour). Tunable against live data.
pub const SITUATIONAL_MIN_SIMILARITY: f64 = 0.35;

/// Recall situational preferences semantically relevant to `query` (Lane B).
///
/// Returns only preferences whose vector similarity to the message clears
/// [`SITUATIONAL_MIN_SIMILARITY`], so an unrelated message yields an empty list
/// (and no injected block). Uses the model-aware embedding recall, so a stale
/// embedding-model signature is excluded rather than mis-scored.
pub async fn recall_situational_preferences(memory: &Arc<dyn Memory>, query: &str) -> Vec<String> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    memory
        .recall_relevant_by_vector(
            USER_PREF_SITUATIONAL_NAMESPACE,
            query,
            SITUATIONAL_RECALL_LIMIT,
            SITUATIONAL_MIN_SIMILARITY,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(_topic, value)| value)
        .collect()
}

/// Minimum similarity for an existing preference to be flagged as a possible
/// contradiction of a newly-saved one. Higher than the Lane-B recall floor — we
/// only surface genuinely-close matches as contradiction candidates. Tunable.
pub const CONTRADICTION_SIMILARITY: f64 = 0.6;

/// Find existing preferences (across both lanes) semantically close to `value`,
/// excluding `exclude_topic` (the just-saved one). Returns `(topic, value)`
/// pairs so the chat agent — which captured the preference in the first place —
/// can resolve a contradiction itself: overwrite the conflicting topic or remove
/// it. No separate model call; the conversation affirms it.
pub async fn recall_related_preferences(
    memory: &Arc<dyn Memory>,
    value: &str,
    exclude_topic: &str,
    limit: usize,
) -> Vec<(String, String)> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    // `limit` is a global cap across *both* lanes, not per-namespace — spend a
    // shared budget so the total surfaced for one contradiction check can never
    // exceed what the caller asked for.
    let mut remaining = limit;
    for ns in [USER_PREF_GENERAL_NAMESPACE, USER_PREF_SITUATIONAL_NAMESPACE] {
        if remaining == 0 {
            break;
        }
        if let Ok(hits) = memory
            .recall_relevant_by_vector(ns, value, remaining, CONTRADICTION_SIMILARITY)
            .await
        {
            for (topic, val) in hits {
                if topic != exclude_topic {
                    out.push((topic, val));
                    remaining = remaining.saturating_sub(1);
                    if remaining == 0 {
                        break;
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::embeddings::NoopEmbedding;
    use crate::openhuman::memory::MemoryCategory;
    use crate::openhuman::memory_store::UnifiedMemory;
    use tempfile::TempDir;

    #[tokio::test]
    async fn load_general_preferences_returns_values_newest_first_capped() {
        let tmp = TempDir::new().unwrap();
        let mem: Arc<dyn Memory> =
            Arc::new(UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap());

        mem.store(
            USER_PREF_GENERAL_NAMESPACE,
            "reply_language",
            "Reply in British English.",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();
        mem.store(
            USER_PREF_GENERAL_NAMESPACE,
            "tone",
            "Be terse.",
            MemoryCategory::Core,
            None,
        )
        .await
        .unwrap();

        let general = load_general_preferences(&mem, 10).await;
        // Returns the values (bodies), not the topic keys.
        assert!(general.iter().any(|v| v.contains("British English")));
        assert!(general.iter().any(|v| v.contains("Be terse")));
        assert!(!general.iter().any(|v| v == "reply_language"));

        // The limit caps the block.
        assert_eq!(load_general_preferences(&mem, 1).await.len(), 1);
    }
}
