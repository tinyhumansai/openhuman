//! Agent tool: `tokenjuice_retrieve` — fetch the original of a compacted result.
//!
//! The content router may replace a large tool result with a compacted view and
//! a `⟦tj:<hash>⟧` marker, stashing the original in the CCR store
//! ([`crate::openhuman::tokenjuice::cache::store`]). This tool hands the
//! original back on demand — fully or by a byte/line range — so even lossy
//! compaction stays reversible.
//!
//! Read-only, no side effects, no path/network access.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::tokenjuice::cache::{self, store::RangeUnit};
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
        cache::RETRIEVE_TOOL_NAME
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
            return match cache::retrieve_range(token, start, end, unit) {
                Some(slice) => {
                    log::debug!(
                        "[tokenjuice][ccr] retrieved range token={token} {start}..{end} {} bytes",
                        slice.len()
                    );
                    Ok(ToolResult::success(slice))
                }
                None => Ok(ToolResult::error(miss_message(token))),
            };
        }

        match cache::retrieve(token) {
            Some(original) => {
                log::debug!(
                    "[tokenjuice][ccr] retrieved token={token} bytes={}",
                    original.len()
                );
                Ok(ToolResult::success(original))
            }
            None => Ok(ToolResult::error(miss_message(token))),
        }
    }
}

fn miss_message(token: &str) -> String {
    format!(
        "tokenjuice_retrieve: no cached original for token '{token}' \
         (it may have been evicted; re-run the tool to regenerate it)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tokenjuice::cache::store;

    #[tokio::test]
    async fn retrieves_offloaded_original() {
        let original = "ORIGINAL TOKENJUICE PAYLOAD ".repeat(20);
        let hash = store::offload(&original);
        let tool = TokenjuiceRetrieveTool::new();
        let res = tool.execute(json!({ "token": hash })).await.unwrap();
        assert!(!res.is_error);
        assert_eq!(res.output(), original);
    }

    #[tokio::test]
    async fn retrieves_line_range() {
        let original = "r0\nr1\nr2\nr3\nr4";
        let hash = store::offload(original);
        let tool = TokenjuiceRetrieveTool::new();
        let res = tool
            .execute(json!({ "token": hash, "range": { "start": 1, "end": 3, "unit": "lines" } }))
            .await
            .unwrap();
        assert!(!res.is_error);
        assert_eq!(res.output(), "r1\nr2");
    }

    #[tokio::test]
    async fn missing_token_is_error() {
        let tool = TokenjuiceRetrieveTool::new();
        let res = tool
            .execute(json!({ "token": "deadbeefcafe" }))
            .await
            .unwrap();
        assert!(res.is_error);
        let res2 = tool.execute(json!({})).await.unwrap();
        assert!(res2.is_error);
    }
}
