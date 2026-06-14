//! Ambient per-turn allowlist of memory-source scopes an agent may recall from.
//!
//! Agent profiles can restrict which memory sources a flavour recalls (the
//! `AgentProfile::memory_sources` allowlist). Threading that allowlist through
//! every memory tool and the deep `select_trees` retrieval layer would touch
//! dozens of call sites, so — mirroring [`thread_context`] — the channel sets a
//! [`tokio::task_local`] around the agent turn and the source-tree retrieval
//! reads it.
//!
//! Semantics:
//! - `None` scope (outside any [`with_source_scope`], or `with_source_scope(None, …)`)
//!   means **unrestricted** — every source tree is visible. This is the default
//!   for cron, sub-agents, the CLI, and any profile that left `memory_sources`
//!   unset.
//! - `Some(set)` restricts recall to source trees whose `scope` string is in the
//!   set. An empty set surfaces nothing (the profile selected no sources).
//!
//! The allowlist entries are matched against tree `scope` strings — the same
//! identifiers the `memory_tree_query_source` tool accepts as `source_id`.
//!
//! [`thread_context`]: crate::openhuman::inference::provider::thread_context
//!
//! ```ignore
//! use crate::openhuman::memory::source_scope::{with_source_scope, current_source_scope};
//!
//! with_source_scope(Some(vec!["slack:#eng".into()]), async {
//!     assert!(current_source_scope().unwrap().contains("slack:#eng"));
//! }).await;
//! ```

use std::collections::HashSet;
use std::future::Future;

tokio::task_local! {
    static SOURCE_SCOPE: Option<HashSet<String>>;
}

/// Normalize a raw allowlist into the task-local representation. Trims entries
/// and drops empties. `None` → unrestricted; `Some(vec)` → restricted (an empty
/// vec stays `Some(empty)` = "no sources").
fn normalize(allowlist: Option<Vec<String>>) -> Option<HashSet<String>> {
    allowlist.map(|items| {
        items
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<HashSet<String>>()
    })
}

/// Run `fut` with `allowlist` available to any descendant call to
/// [`current_source_scope`]. `None` leaves recall unrestricted.
pub async fn with_source_scope<F, T>(allowlist: Option<Vec<String>>, fut: F) -> T
where
    F: Future<Output = T>,
{
    let value = normalize(allowlist);
    log::debug!(
        "[memory:source_scope] entering scope: {}",
        match &value {
            None => "unrestricted".to_string(),
            Some(set) => format!("{} source(s)", set.len()),
        }
    );
    SOURCE_SCOPE.scope(value, fut).await
}

/// Return the ambient source-scope allowlist set by an enclosing
/// [`with_source_scope`], or `None` (unrestricted) when called outside one.
pub fn current_source_scope() -> Option<HashSet<String>> {
    SOURCE_SCOPE.try_with(|v| v.clone()).ok().flatten()
}

/// Whether `scope` is recallable under the ambient allowlist. `true` when there
/// is no active scope (unrestricted) or when the scope is explicitly allowed.
pub fn scope_allowed(scope: &str) -> bool {
    match current_source_scope() {
        None => true,
        Some(set) => set.contains(scope),
    }
}

/// The tag every memory-source–ingested chunk carries (set by
/// `memory_sources::sync` and the github reader). Used as the discriminator so
/// the chunk-level gate only touches memory-SOURCE chunks and never working /
/// conversation / internal chunks.
const MEMORY_SOURCE_TAG: &str = "memory_sources";

/// Whether a memory-store chunk is recallable under the ambient allowlist,
/// given its `tags` and `source_id`.
///
/// Fail-open for everything that is NOT a memory-source chunk: a chunk without
/// the `memory_sources` tag (working memory, conversation transcripts, internal
/// chunks) always passes. A tagged memory-source chunk passes iff its source
/// identifier is allowed — matched flexibly against either the raw `source_id`
/// (Composio / channel scopes like `slack:#eng`) or the registry id extracted
/// from a `mem_src:<id>:<item>` composite (reader-based sources). `None` scope
/// is unrestricted.
pub fn chunk_source_allowed(tags: &[String], source_id: &str) -> bool {
    match current_source_scope() {
        None => true,
        Some(set) => chunk_source_allowed_in(&set, tags, source_id),
    }
}

/// Pure form of [`chunk_source_allowed`] against an explicit allowlist `set`,
/// for callers that already hold the scope (e.g. `list_chunks`, which captures
/// it on the async side and filters DB rows before applying the row limit so a
/// disallowed-source-heavy prefix can't starve permitted rows).
pub fn chunk_source_allowed_in(set: &HashSet<String>, tags: &[String], source_id: &str) -> bool {
    let is_memory_source = tags.iter().any(|t| t == MEMORY_SOURCE_TAG);
    if !is_memory_source {
        return true;
    }
    if set.contains(source_id) {
        return true;
    }
    crate::openhuman::memory::sync::extract_mem_src_id(source_id).is_some_and(|id| set.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unrestricted_outside_scope() {
        assert!(current_source_scope().is_none());
        assert!(scope_allowed("anything"));
    }

    #[tokio::test]
    async fn restricts_to_allowlisted_scopes() {
        with_source_scope(
            Some(vec!["slack:#eng".into(), "  gmail:me  ".into()]),
            async {
                let set = current_source_scope().expect("scope set");
                assert_eq!(set.len(), 2);
                assert!(scope_allowed("slack:#eng"));
                assert!(scope_allowed("gmail:me")); // trimmed
                assert!(!scope_allowed("notion:team"));
            },
        )
        .await;
        // Must not leak past the scope.
        assert!(current_source_scope().is_none());
        assert!(scope_allowed("notion:team"));
    }

    #[tokio::test]
    async fn empty_allowlist_blocks_everything() {
        with_source_scope(Some(vec![]), async {
            assert!(current_source_scope().is_some());
            assert!(!scope_allowed("slack:#eng"));
        })
        .await;
    }

    #[tokio::test]
    async fn explicit_none_is_unrestricted() {
        with_source_scope(None, async {
            assert!(current_source_scope().is_none());
            assert!(scope_allowed("slack:#eng"));
        })
        .await;
    }

    #[tokio::test]
    async fn chunk_gate_passes_non_source_chunks_and_gates_tagged_ones() {
        let src_tags = vec!["memory_sources".to_string(), "document".to_string()];
        let other_tags = vec!["conversation".to_string()];

        with_source_scope(
            Some(vec!["slack:#eng".into(), "src-rss-42".into()]),
            async {
                // Non-source chunk (no memory_sources tag) always passes.
                assert!(chunk_source_allowed(&other_tags, "thr_123:user"));
                // Composio/channel source chunk: raw source_id == scope.
                assert!(chunk_source_allowed(&src_tags, "slack:#eng"));
                assert!(!chunk_source_allowed(&src_tags, "gmail:alice"));
                // Reader-based composite: extracted registry id matches.
                assert!(chunk_source_allowed(
                    &src_tags,
                    "mem_src:src-rss-42:https://example.com/item-7"
                ));
                assert!(!chunk_source_allowed(
                    &src_tags,
                    "mem_src:src-folder-9:/notes/a.md"
                ));
            },
        )
        .await;
    }

    #[tokio::test]
    async fn chunk_gate_unrestricted_without_scope() {
        let src_tags = vec!["memory_sources".to_string()];
        // Outside any scope, even tagged source chunks pass.
        assert!(chunk_source_allowed(&src_tags, "gmail:alice"));
    }

    #[tokio::test]
    async fn chunk_gate_empty_allowlist_blocks_tagged_sources_only() {
        let src_tags = vec!["memory_sources".to_string()];
        let other_tags: Vec<String> = vec![];
        with_source_scope(Some(vec![]), async {
            assert!(!chunk_source_allowed(&src_tags, "slack:#eng"));
            // Non-source chunks still pass even under an empty allowlist.
            assert!(chunk_source_allowed(&other_tags, "thr_1:user"));
        })
        .await;
    }
}
