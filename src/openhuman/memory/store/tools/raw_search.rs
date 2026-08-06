//! `memory_store_raw_search` — free-text search over the entity index.
//!
//! Thin wrapper around `memory_tree::retrieval::search_entities`. Returns canonical
//! entity ids ranked by mention count. This is the rawest of the raw search
//! paths: no narrative, no scoring beyond aggregate occurrence, no rerank.
//! Use it when an agent needs to discover what entities exist in the store
//! before drilling into trees.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::retrieval::search::search_entities;
use crate::openhuman::memory::tree::score::extract::EntityKind;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryStoreRawSearchTool;

#[derive(Debug, Deserialize)]
struct Args {
    query: String,
    #[serde(default)]
    kinds: Option<Vec<String>>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    5
}

#[async_trait]
impl Tool for MemoryStoreRawSearchTool {
    fn name(&self) -> &str {
        "memory_store_raw_search"
    }

    fn description(&self) -> &str {
        "Free-text LIKE search over the canonical entity index. Returns \
         entity ids ranked by total mention count across every tree. Use to \
         discover what entities (people, channels, threads) exist in the \
         memory store before drilling into a tree with the memory_tree_* \
         tools. Pass `kinds` to narrow the result set (e.g. only people)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring matched against canonical entity id and surface form (case-insensitive)."
                },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional entity kind filter (e.g. [\"person\", \"channel\"]). Empty/absent = all kinds."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Max matches to return (default 5, clamped 100)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_store_raw_search: {e}"))?;
        log::debug!(
            "[tool][memory_store] raw_search q_len={} kinds={:?} limit={}",
            parsed.query.len(),
            parsed.kinds,
            parsed.limit
        );
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_store_raw_search: load config failed: {e}"))?;
        let kinds = match parsed.kinds {
            Some(ks) if !ks.is_empty() => {
                let mut out = Vec::with_capacity(ks.len());
                for k in ks {
                    out.push(
                        EntityKind::parse(&k)
                            .map_err(|e| anyhow::anyhow!("memory_store_raw_search: {e}"))?,
                    );
                }
                Some(out)
            }
            _ => None,
        };
        let hits = search_entities(&cfg, &parsed.query, kinds, parsed.limit).await?;
        log::debug!(
            "[tool][memory_store] raw_search returning hits={}",
            hits.len()
        );
        let json = serde_json::to_string(&hits)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    use tempfile::TempDir;

    use crate::openhuman::config::{Config, TEST_ENV_LOCK};
    use crate::openhuman::tools::traits::Tool;
    use serde_json::json;

    struct WorkspaceEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<OsString>,
    }

    impl WorkspaceEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
            unsafe {
                std::env::set_var("OPENHUMAN_WORKSPACE", path);
            }
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for WorkspaceEnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(previous) = self.previous.as_ref() {
                    std::env::set_var("OPENHUMAN_WORKSPACE", previous);
                } else {
                    std::env::remove_var("OPENHUMAN_WORKSPACE");
                }
            }
        }
    }

    async fn isolated_config(tmp: &TempDir) -> (WorkspaceEnvGuard, Config) {
        let guard = WorkspaceEnvGuard::set(tmp.path());
        let config = Config::load_or_init().await.expect("load config");
        (guard, config)
    }

    #[test]
    fn default_limit_is_five() {
        assert_eq!(default_limit(), 5);
    }

    #[test]
    fn args_deserialize_with_default_limit() {
        let args: Args = serde_json::from_value(json!({ "query": "alice" })).unwrap();
        assert_eq!(args.query, "alice");
        assert_eq!(args.limit, 5);
        assert!(args.kinds.is_none());
    }

    #[test]
    fn parameters_schema_describes_required_query() {
        let tool = MemoryStoreRawSearchTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["query"]));
        assert_eq!(schema["properties"]["limit"]["maximum"], 100);
    }

    #[tokio::test]
    async fn execute_rejects_missing_query() {
        let tool = MemoryStoreRawSearchTool;
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("missing query should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_store_raw_search"));
    }

    #[tokio::test]
    async fn execute_rejects_invalid_kind() {
        let tool = MemoryStoreRawSearchTool;
        let err = tool
            .execute(json!({
                "query": "alice",
                "kinds": ["not-a-kind"]
            }))
            .await
            .expect_err("invalid kind should fail");
        assert!(err.to_string().contains("memory_store_raw_search:"));
    }

    #[tokio::test]
    async fn execute_success_path_returns_json_array() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _config) = isolated_config(&tmp).await;
        let tool = MemoryStoreRawSearchTool;
        let result = tool
            .execute(json!({
                "query": "alice",
                "limit": 3
            }))
            .await
            .expect("valid raw_search request should succeed");
        assert!(!result.is_error);
        let parsed: serde_json::Value =
            serde_json::from_str(&result.text()).expect("tool result should be json");
        assert!(
            parsed.is_array(),
            "raw_search should serialize a JSON array"
        );
    }
}
