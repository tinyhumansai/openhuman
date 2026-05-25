//! `memory_store_raw_chunks` — structured chunk filter.
//!
//! Bypasses ranking entirely. Returns chunks (timestamp DESC) matching the
//! supplied source/owner/time/tag filters. Use when the agent knows the
//! exact subset of memory it wants to inspect.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory_store::chunks::store::{list_chunks, ListChunksQuery};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryStoreRawChunksTool;

#[derive(Debug, Deserialize)]
struct Args {
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    since_ms: Option<i64>,
    #[serde(default)]
    until_ms: Option<i64>,
    #[serde(default)]
    tags_all_of: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for MemoryStoreRawChunksTool {
    fn name(&self) -> &str {
        "memory_store_raw_chunks"
    }

    fn description(&self) -> &str {
        "List raw memory_store chunks (timestamp DESC) matching structured \
         filters: source kind, source id, owner, time range, required tags. \
         No scoring or rerank — use for exact-subset inspection, not search. \
         Returns full Chunk rows with metadata and content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "source_kind": { "type": "string", "enum": ["chat", "email", "document"] },
                "source_id":   { "type": "string", "description": "Exact source id." },
                "owner":       { "type": "string", "description": "Owner / account filter." },
                "since_ms":    { "type": "integer", "description": "Inclusive lower bound on timestamp_ms." },
                "until_ms":    { "type": "integer", "description": "Inclusive upper bound on timestamp_ms." },
                "tags_all_of": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Post-filter: chunk.metadata.tags must contain every tag listed."
                },
                "limit":       { "type": "integer", "minimum": 1, "maximum": 1000, "description": "Default 100." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_store_raw_chunks: {e}"))?;
        log::debug!(
            "[tool][memory_store] raw_chunks source_kind={:?} owner={:?} tags={:?} limit={:?}",
            parsed.source_kind,
            parsed.owner,
            parsed.tags_all_of,
            parsed.limit
        );
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_store_raw_chunks: load config failed: {e}"))?;
        let source_kind = match parsed.source_kind.as_deref() {
            Some(s) => Some(
                SourceKind::parse(s)
                    .map_err(|e| anyhow::anyhow!("memory_store_raw_chunks: {e}"))?,
            ),
            None => None,
        };
        if let Some(limit) = parsed.limit {
            if !(1..=1000).contains(&limit) {
                return Err(anyhow::anyhow!(
                    "memory_store_raw_chunks: limit must be between 1 and 1000"
                ));
            }
        }
        let query = ListChunksQuery {
            source_kind,
            source_id: parsed.source_id,
            owner: parsed.owner,
            since_ms: parsed.since_ms,
            until_ms: parsed.until_ms,
            limit: parsed.limit,
        };
        let mut rows = list_chunks(&cfg, &query)?;
        if let Some(required) = parsed.tags_all_of.as_ref() {
            if !required.is_empty() {
                rows.retain(|c| {
                    required
                        .iter()
                        .all(|t| c.metadata.tags.iter().any(|ct| ct == t))
                });
            }
        }
        log::debug!(
            "[tool][memory_store] raw_chunks returning rows={}",
            rows.len()
        );
        let json = serde_json::to_string(&rows)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::Tool;
    use serde_json::json;

    #[test]
    fn args_deserialize_optional_filters() {
        let args: Args = serde_json::from_value(json!({
            "source_kind": "chat",
            "source_id": "slack:#eng",
            "owner": "alice",
            "since_ms": 10,
            "until_ms": 20,
            "tags_all_of": ["person:alice"],
            "limit": 25
        }))
        .unwrap();

        assert_eq!(args.source_kind.as_deref(), Some("chat"));
        assert_eq!(args.source_id.as_deref(), Some("slack:#eng"));
        assert_eq!(args.owner.as_deref(), Some("alice"));
        assert_eq!(args.since_ms, Some(10));
        assert_eq!(args.until_ms, Some(20));
        assert_eq!(args.tags_all_of, Some(vec!["person:alice".to_string()]));
        assert_eq!(args.limit, Some(25));
    }

    #[test]
    fn parameters_schema_exposes_supported_source_kinds() {
        let tool = MemoryStoreRawChunksTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(
            schema["properties"]["source_kind"]["enum"],
            json!(["chat", "email", "document"])
        );
        assert_eq!(schema["properties"]["limit"]["maximum"], 1000);
    }

    #[tokio::test]
    async fn execute_rejects_invalid_source_kind() {
        let tool = MemoryStoreRawChunksTool;
        let err = tool
            .execute(json!({
                "source_kind": "not-real"
            }))
            .await
            .expect_err("invalid source kind should fail");
        assert!(err.to_string().contains("memory_store_raw_chunks:"));
    }

    #[tokio::test]
    async fn execute_rejects_wrong_type_for_limit() {
        let tool = MemoryStoreRawChunksTool;
        let err = tool
            .execute(json!({
                "limit": "ten"
            }))
            .await
            .expect_err("wrong limit type should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_store_raw_chunks"));
    }

    #[tokio::test]
    async fn execute_success_path_returns_json_array() {
        let tool = MemoryStoreRawChunksTool;
        let result = tool
            .execute(json!({
                "source_kind": "document",
                "limit": 2
            }))
            .await
            .expect("valid raw_chunks request should succeed");
        assert!(!result.is_error);
        let parsed: serde_json::Value =
            serde_json::from_str(&result.text()).expect("tool result should be json");
        assert!(
            parsed.is_array(),
            "raw_chunks should serialize a JSON array"
        );
    }
}
