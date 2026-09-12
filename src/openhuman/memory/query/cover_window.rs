use crate::openhuman::memory::api::chunks::SourceKind;
use crate::openhuman::memory::api::provider::{CoverWindowQuery, MemoryProvider};
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::memory::tree::retrieval::rpc::CoverWindowRequest;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

/// Agent-facing wrapper for the windowed minimum-cover retrieval. Returns the
/// smallest set of nodes (summaries + raw chunks) covering all memory in
/// `[since_ms, until_ms]`. Built for time-bounded recaps like the morning
/// brief's "last 24h" — see `memory_tree::retrieval::cover`.
pub struct MemoryTreeCoverWindowTool;

#[async_trait]
impl Tool for MemoryTreeCoverWindowTool {
    fn name(&self) -> &str {
        "memory_tree_cover_window"
    }

    fn description(&self) -> &str {
        "Return the MINIMUM set of memory nodes covering a time window \
         [since_ms, until_ms] (epoch-milliseconds): condensed summaries where a \
         whole stretch is in-window, raw recent chunks otherwise. Grouped by \
         source, ordered oldest→newest. Use for time-bounded recaps (e.g. a \
         last-24h morning brief) instead of `query_source` (which is all-time)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "since_ms": {
                    "type": "integer",
                    "description": "Inclusive window start, epoch-milliseconds."
                },
                "until_ms": {
                    "type": "integer",
                    "description": "Inclusive window end, epoch-milliseconds."
                },
                "source_id": {
                    "type": "string",
                    "description": "Exact source id (e.g. `slack:#eng`, `gmail:abc`)."
                },
                "source_kind": {
                    "type": "string",
                    "enum": ["chat", "email", "document"],
                    "description": "Source kind filter when no exact id is known."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Max hits to return (default 200)."
                }
            },
            "required": ["since_ms", "until_ms"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] cover_window invoked");
        let req: CoverWindowRequest = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tree_cover_window: {e}"))?;
        // Correlation fields only — source_id can carry PII, so log its presence,
        // not its value.
        log::debug!(
            "[tool][memory_tree] cover_window parsed since_ms={} until_ms={} has_source_id={} has_source_kind={} has_limit={}",
            req.since_ms,
            req.until_ms,
            req.source_id.is_some(),
            req.source_kind.is_some(),
            req.limit.is_some()
        );
        // Validate arguments before touching config/disk — `SourceKind::parse`
        // is pure, so a bad `source_kind` must fail with the parse error
        // regardless of workspace state.
        let source_kind = match req.source_kind.as_deref() {
            Some(s) => {
                log::trace!("[tool][memory_tree] cover_window parse_source_kind");
                Some(
                    SourceKind::parse(s)
                        .map_err(|e| anyhow::anyhow!("memory_tree_cover_window: {e}"))?,
                )
            }
            None => None,
        };
        log::trace!(
            "[tool][memory_tree] cover_window dispatch limit={}",
            req.limit.unwrap_or(0)
        );
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_cover_window: {e}"))?;
        let window = CoverWindowQuery {
            since_ms: req.since_ms,
            until_ms: req.until_ms,
            source_id: req.source_id.clone(),
            source_kind,
            limit: req.limit,
        };
        let resp = guard
            .as_retrieval()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "memory_tree_cover_window: memory driver does not support the retrieval family"
                )
            })?
            .cover_window(&window, None)
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_cover_window: {e}"))?;
        log::debug!(
            "[tool][memory_tree] cover_window returning hits={} total={}",
            resp.hits.len(),
            resp.total
        );
        let json = serde_json::to_string(&resp)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
#[path = "cover_window_tests.rs"]
mod tests;
