//! Namespace scope for per-turn automatic recall.
//!
//! `RecallOpts::default()` resolves to the `global` namespace, so the automatic
//! turn context has never searched connector memories: a synced email can sit
//! in `skill-gmail` and still be unreachable from the very turn that asks about
//! it. That scoping is not a decision — it is what an unfinished namespace
//! migration left behind, and connector sync landed on top of it.
//!
//! Recall is namespace-scoped in the store, so widening the scope means fanning
//! out: one recall per namespace, run concurrently, merged by score. The fan-out
//! is deliberately bounded — each namespace recall embeds the query and scans
//! that namespace's chunks, so unbounded connector growth would otherwise turn
//! every single turn into a full-store scan.

use futures::future::join_all;

use crate::openhuman::memory::{Memory, MemoryEntry, RecallOpts};

/// Namespace prefix connector-synced documents are stored under
/// (`skill-gmail`, `skill-slack`, …).
const SKILL_NAMESPACE_PREFIX: &str = "skill-";

/// Connector namespaces searched per turn, largest first.
///
/// Every extra namespace costs one query embedding plus a scan of that
/// namespace's chunks. Four keeps the added turn latency to roughly one
/// concurrent recall while still covering the connectors a user actually syncs;
/// the rest stay reachable through the explicit `memory_recall` tool, which
/// takes a namespace.
pub(crate) const MAX_CONNECTOR_NAMESPACES: usize = 4;

/// Recall for the automatic turn context: the global namespace plus the
/// busiest connector namespaces, merged and ranked as one result set.
///
/// Never fails the turn — a namespace whose recall errors is skipped, and a
/// store that cannot enumerate namespaces degrades to global-only, which is
/// exactly the previous behaviour.
pub(crate) async fn recall_with_connectors(
    mem: &dyn Memory,
    query: &str,
    limit: usize,
) -> Vec<MemoryEntry> {
    recall_across(mem, query, limit, Some(MAX_CONNECTOR_NAMESPACES)).await
}

/// Recall for the **explicit** `memory_recall` tool called without a namespace.
///
/// Searches every non-empty connector namespace, not the busiest few. The tool's
/// schema tells the model that omitting the namespace searches everywhere, and
/// the model is instructed to omit it; borrowing the per-turn cap here would
/// make that instruction quietly false for anyone with more connectors than the
/// cap, and the memory it was told to look for would simply not be looked at.
/// The turn-context path keeps the cap — there the cost is paid on every turn,
/// not on a call the model chose to make.
pub(crate) async fn recall_every_namespace(
    mem: &dyn Memory,
    query: &str,
    limit: usize,
) -> Vec<MemoryEntry> {
    recall_across(mem, query, limit, None).await
}

async fn recall_across(
    mem: &dyn Memory,
    query: &str,
    limit: usize,
    max_namespaces: Option<usize>,
) -> Vec<MemoryEntry> {
    let mut entries = match crate::openhuman::agent::tinyagents::retriever::recall_through_facade(
        mem,
        query,
        limit,
        RecallOpts::default(),
    )
    .await
    {
        Ok(entries) => entries,
        Err(error) => {
            tracing::debug!("[memory::auto-recall] global recall failed: {error}");
            Vec::new()
        }
    };

    let namespaces = connector_namespaces(mem, max_namespaces).await;
    if !namespaces.is_empty() {
        tracing::debug!(
            "[memory::auto-recall] fanning out to connector namespaces: {}",
            namespaces.join(", ")
        );
    }
    let recalls = namespaces.iter().map(|namespace| async move {
        mem.recall(
            query,
            limit,
            RecallOpts {
                namespace: Some(namespace),
                ..Default::default()
            },
        )
        .await
    });
    for (namespace, result) in namespaces.iter().zip(join_all(recalls).await) {
        match result {
            Ok(hits) => entries.extend(hits),
            Err(error) => {
                tracing::debug!("[memory::auto-recall] recall failed for {namespace}: {error}")
            }
        }
    }

    rank_and_truncate(entries, limit)
}

/// The connector namespaces worth searching: non-empty, busiest first, capped at
/// `max_namespaces` when the caller sets one. `None` searches all of them.
async fn connector_namespaces(mem: &dyn Memory, max_namespaces: Option<usize>) -> Vec<String> {
    let mut summaries = match mem.namespace_summaries().await {
        Ok(summaries) => summaries,
        Err(error) => {
            tracing::debug!("[memory::auto-recall] namespace listing failed: {error}");
            return Vec::new();
        }
    };
    summaries.retain(|summary| {
        summary.count > 0 && summary.namespace.starts_with(SKILL_NAMESPACE_PREFIX)
    });
    // Busiest first, name as the tie-break so the selected set is stable across
    // turns rather than reshuffling with map iteration order.
    summaries.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.namespace.cmp(&b.namespace))
    });
    if let Some(cap) = max_namespaces {
        summaries.truncate(cap);
    }
    summaries
        .into_iter()
        .map(|summary| summary.namespace)
        .collect()
}

/// Merge hits from several namespaces into one ranked list.
///
/// Scores are comparable across namespaces (the same hybrid scorer produced
/// them), so a connector hit outranks a weak global one on merit rather than on
/// which namespace it came from. Unscored entries sort last instead of being
/// dropped: non-vector backends return them and they were previously kept.
fn rank_and_truncate(mut entries: Vec<MemoryEntry>, limit: usize) -> Vec<MemoryEntry> {
    entries.sort_by(|a, b| {
        b.score
            .unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&a.score.unwrap_or(f64::NEG_INFINITY))
    });
    // One document reachable from two namespaces must not occupy two slots.
    let mut seen = std::collections::HashSet::new();
    entries.retain(|entry| seen.insert((entry.namespace.clone(), entry.key.clone())));
    entries.truncate(limit);
    entries
}

#[cfg(test)]
#[path = "auto_recall_tests.rs"]
mod tests;
