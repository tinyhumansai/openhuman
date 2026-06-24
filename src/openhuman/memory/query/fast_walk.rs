//! Deterministic replacement for the former agentic `walk` / `smart_walk`
//! tool modes.
//!
//! Both modes now resolve to [`fast_retrieve`] — the E2GraphRAG, LLM-free
//! retriever. It returns a structured [`QueryResponse`] of ranked evidence
//! (no synthesized prose); a higher-level context agent composes the answer.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory_tree::retrieval::{fast_retrieve, FastRetrieveOptions};
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

    let cfg = config_rpc::load_config_with_timeout()
        .await
        .map_err(|e| anyhow::anyhow!("memory_tree walk: load config failed: {e}"))?;

    let opts = FastRetrieveOptions {
        limit,
        max_hops,
        time_window_days,
    };
    let resp = fast_retrieve(&cfg, &query, opts).await?;
    log::debug!(
        "[tool][memory_tree] walk returning hits={} total={}",
        resp.hits.len(),
        resp.total
    );
    let json = serde_json::to_string(&resp)?;
    Ok(ToolResult::success(json))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn missing_query_errors() {
        let err = run_fast_walk(json!({})).await.unwrap_err();
        assert!(err.to_string().contains("`query` is required"));
    }

    #[tokio::test]
    async fn blank_query_errors() {
        let err = run_fast_walk(json!({"query": "   "})).await.unwrap_err();
        assert!(err.to_string().contains("`query` is required"));
    }
}
