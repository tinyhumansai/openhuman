//! OpenHuman's **host-side** per-turn memory-source allowlist.
//!
//! # Why this is here and not in a tinymemory crate
//!
//! The engine has a task-local of the same shape
//! (`crate::openhuman::memory::source_scope`), and for a while this host simply used it.
//! That only worked because the host linked the engine. Once memory is reached
//! through the loadable TinyMemory module, it stops working *silently*: the
//! module is a separately compiled `cdylib` with its own statics, so a
//! task-local set on this side of the bus is invisible on the other. The turn
//! would run with the scope apparently applied and the engine would see
//! `None` — unrestricted.
//!
//! So the scope crosses the bus as a value instead. `SourceScope` is a
//! `tinymemory-bus` payload type and every scoped `MemoryProvider` method takes
//! it as an argument. What stays host-side is the part that was always the
//! host's: gathering the allowlist off the agent profile and making it ambient
//! for the duration of a turn, so the dozens of call sites between the channel
//! and the provider do not each have to thread it.
//!
//! [`as_bus_scope`] is the join between the two — it reads the ambient scope and
//! renders it in the vocabulary the provider call expects.
//!
//! # Semantics
//!
//! Agent profiles can restrict which memory sources a flavour recalls (the
//! `AgentProfile::memory_sources` allowlist). Threading that allowlist through
//! every memory tool and the deep `select_trees` retrieval layer would touch
//! dozens of call sites, so the channel sets a [`tokio::task_local`] around the
//! agent turn and the retrieval path reads it.
//!
//! - `None` scope (outside any [`with_source_scope`], or `with_source_scope(None, …)`)
//!   means **unrestricted** — every source tree is visible. This is the default
//!   for profile-less cron, sub-agents, the CLI, and any profile that left
//!   `memory_sources` unset.
//! - `Some(set)` restricts recall to source trees whose `scope` string is in the
//!   set. An empty set surfaces nothing (the profile selected no sources).
//!
//! The allowlist entries are matched against tree `scope` strings — the same
//! identifiers the `memory_tree_query_source` tool accepts as `source_id`.
//!
//!
//! ```ignore
//! use openhuman::openhuman::memory::source_scope::{with_source_scope, current_source_scope};
//!
//! with_source_scope(Some(vec!["slack:#eng".into()]), async {
//!     assert!(current_source_scope().unwrap().contains("slack:#eng"));
//! }).await;
//! ```

use std::collections::HashSet;
use std::future::Future;

use tinymemory_api::provider::types::SourceScope;

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

/// Render the ambient scope in the vocabulary a `MemoryProvider` call takes.
///
/// This is the whole point of keeping the task-local host-side: the ambient
/// value is gathered once per turn, and every provider call that accepts a
/// scope passes the result of this function rather than relying on the driver
/// to read a task-local it cannot see.
///
/// `None` means unrestricted and must stay `None` — [`SourceScope`] treats an
/// **empty** allowlist as denying every source-attributed item, so mapping
/// "no restriction" onto `Some(SourceScope::new([]))` would invert the policy
/// and silently blank out recall.
#[must_use]
pub fn as_bus_scope() -> Option<SourceScope> {
    current_source_scope().map(SourceScope::new)
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
    tinymemory_api::sync_events::extract_mem_src_id(source_id).is_some_and(|id| set.contains(id))
}

#[cfg(test)]
#[path = "source_scope_tests.rs"]
mod tests;
