//! Agent tool: `tokenjuice_retrieve` — fetch the original of a compacted result.
//!
//! The content router may replace a large tool result with a compacted view and
//! a `⟦tj:<hash>⟧` marker, stashing the original in the CCR store
//! ([`crate::openhuman::inference::tokenjuice::cache::store`]). This tool hands the
//! original back on demand — fully or by a byte/line range — so even lossy
//! compaction stays reversible.
//!
//! Read-only, no side effects, no path/network access.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::inference::tokenjuice::types::{RangeUnit, RetrieveRange};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

pub struct TokenjuiceRetrieveTool;

impl TokenjuiceRetrieveTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TokenjuiceRetrieveTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TokenjuiceRetrieveTool {
    fn name(&self) -> &str {
        super::RETRIEVE_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Retrieve the full, original text of a tool result that was compacted to save \
         context. When output shows a marker like `⟦tj:a1b2c3d4⟧` (or a legacy \
         `retrieve_tool_output(\"…\")` footer), call this with that token to get the \
         complete original back. Optionally pass a `range` to fetch just a byte or line \
         slice. Use it only when you actually need the dropped detail — the compacted \
         view is usually enough."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "token": {
                    "type": "string",
                    "description": "The hash from a ⟦tj:…⟧ marker (or legacy retrieve footer)."
                },
                "range": {
                    "type": "object",
                    "description": "Optional slice of the original to return.",
                    "properties": {
                        "start": { "type": "integer", "minimum": 0 },
                        "end": { "type": "integer", "minimum": 0 },
                        "unit": { "type": "string", "enum": ["bytes", "lines"] }
                    },
                    "required": ["start", "end"]
                }
            },
            "required": ["token"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        // Accept `token` (canonical) or `hash` (legacy arg name).
        let token = args
            .get("token")
            .or_else(|| args.get("hash"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(token) = token else {
            return Ok(ToolResult::error(
                "tokenjuice_retrieve: missing required 'token' argument".to_string(),
            ));
        };

        // Optional range.
        if let Some(range) = args.get("range").filter(|v| v.is_object()) {
            let start = range.get("start").and_then(Value::as_u64).unwrap_or(0) as usize;
            let end = range.get("end").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
            let unit = match range.get("unit").and_then(Value::as_str) {
                Some("bytes") => RangeUnit::Bytes,
                _ => RangeUnit::Lines,
            };
            return match super::retrieve(
                token.to_string(),
                Some(RetrieveRange { start, end, unit }),
            )
            .await
            {
                Ok(Some(slice)) => {
                    log::debug!(
                        "[tokenjuice][ccr] retrieved range token={token} {start}..{end} {} bytes",
                        slice.len()
                    );
                    Ok(ToolResult::success(slice))
                }
                Ok(None) => Ok(ToolResult::error(miss_message(token))),
                Err(error) => Ok(ToolResult::error(error)),
            };
        }

        match super::retrieve(token.to_string(), None).await {
            Ok(Some(original)) => {
                log::debug!(
                    "[tokenjuice][ccr] retrieved token={token} bytes={}",
                    original.len()
                );
                Ok(ToolResult::success(original))
            }
            Ok(None) => Ok(ToolResult::error(miss_message(token))),
            Err(error) => Ok(ToolResult::error(error)),
        }
    }
}

fn miss_message(token: &str) -> String {
    // Deliberately does NOT say "re-run the tool". Re-running regenerates the
    // same oversized result, which is compacted and offloaded again under a new
    // token that can be evicted just as fast — so a blind re-run turns one cache
    // miss into an unbounded compact→retrieve→re-run loop (observed live: a
    // parent agent re-delegated forever on an evicted subagent result). Tell the
    // model to proceed with the compacted summary it already has instead.
    format!(
        "tokenjuice_retrieve: the full original for token '{token}' is no longer cached \
         (evicted, or from an earlier session). Do NOT re-run the same tool call to \
         regenerate it — that will produce the same oversized result and be compacted \
         again. Proceed using the compacted summary already shown above; only if a \
         specific missing detail is essential, retry with narrower arguments (a tighter \
         query, filter, or smaller limit) so the result is small enough to keep in full."
    )
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
