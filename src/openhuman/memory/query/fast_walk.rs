//! Deterministic replacement for the former agentic `walk` / `smart_walk`
//! tool modes.
//!
//! Both modes now resolve to [`fast_retrieve`] — the E2GraphRAG, LLM-free
//! retriever. It returns a structured [`QueryResponse`] of ranked evidence
//! (no synthesized prose); a higher-level context agent composes the answer.

use crate::openhuman::memory::api::provider::{FastRetrieveQuery, MemoryProvider};
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::ToolResult;

/// Parse the shared `memory_tree` args and run deterministic retrieval.
/// Accepts `query` (required), `limit`, `time_window_days`, and `max_hops`.
pub async fn run_fast_walk(args: serde_json::Value) -> anyhow::Result<ToolResult> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if query.trim().is_empty() {
        return Err(anyhow::anyhow!("memory_tree walk: `query` is required"));
    }

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(10);
    let time_window_days = args
        .get("time_window_days")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let max_hops = args
        .get("max_hops")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(2);

    log::debug!(
        "[tool][memory_tree] walk (deterministic) query_len={} limit={} max_hops={} window={:?}",
        query.len(),
        limit,
        max_hops,
        time_window_days
    );

    // Routed through the bound driver. `None` for the scope is not
    // "unrestricted": the guard intersects it with the ambient per-turn
    // allowlist, so the source gate still applies.
    let guard = active_memory_guard()
        .await
        .map_err(|e| anyhow::anyhow!("memory_tree walk: {e}"))?;
    let opts = FastRetrieveQuery {
        limit,
        max_hops,
        time_window_days,
    };
    let resp = guard
        .as_retrieval()
        .ok_or_else(|| {
            anyhow::anyhow!("memory_tree walk: memory driver does not support the retrieval family")
        })?
        .fast_retrieve(&query, opts, None)
        .await?;
    log::debug!(
        "[tool][memory_tree] walk returning hits={} total={}",
        resp.hits.len(),
        resp.total
    );
    let json = serde_json::to_string(&resp)?;
    Ok(ToolResult::success(json))
}

#[cfg(test)]
#[path = "fast_walk_tests.rs"]
mod tests;
