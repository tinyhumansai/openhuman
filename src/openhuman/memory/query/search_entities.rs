use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory_tree::retrieval;
use crate::openhuman::memory_tree::retrieval::rpc::SearchEntitiesRequest;
use crate::openhuman::memory_tree::score::extract::EntityKind;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct MemoryTreeSearchEntitiesTool;

#[async_trait]
impl Tool for MemoryTreeSearchEntitiesTool {
    fn name(&self) -> &str {
        "memory_tree_search_entities"
    }

    fn description(&self) -> &str {
        "Free-text LIKE search over the entity index — resolve a name or \
         handle to a canonical id (e.g. \"alice\" -> \
         `email:alice@example.com`). ALWAYS call this first when the user \
         mentions someone by name before calling `memory_tree_query_topic`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring to match (case-insensitive)."
                },
                "kinds": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": [
                            "email", "url", "handle", "hashtag", "person",
                            "organization", "location", "event", "product",
                            "misc", "topic"
                        ]
                    },
                    "description": "Optional kind filter — restrict to these entity kinds only."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Max matches (default 5, clamped to 100)."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] search_entities invoked");
        let req: SearchEntitiesRequest = serde_json::from_value(args).map_err(|e| {
            anyhow::anyhow!("invalid arguments for memory_tree_search_entities: {e}")
        })?;
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_search_entities: load config failed: {e}"))?;
        let kinds = match req.kinds {
            None => None,
            Some(list) => {
                let parsed: Result<Vec<EntityKind>, String> =
                    list.iter().map(|s| EntityKind::parse(s)).collect();
                Some(parsed.map_err(|e| {
                    anyhow::anyhow!("memory_tree_search_entities: invalid kind: {e}")
                })?)
            }
        };
        let limit = req.limit.unwrap_or(5).min(100);
        let matches = retrieval::search_entities(&cfg, &req.query, kinds, limit).await?;
        log::debug!(
            "[tool][memory_tree] search_entities returning matches={}",
            matches.len()
        );
        let json = serde_json::to_string(&matches)?;
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
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for WorkspaceEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("OPENHUMAN_WORKSPACE", previous);
            } else {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            }
        }
    }

    async fn isolated_config(tmp: &TempDir) -> (WorkspaceEnvGuard, Config) {
        let guard = WorkspaceEnvGuard::set(tmp.path());
        let config = Config::load_or_init().await.expect("load config");
        (guard, config)
    }

    #[test]
    fn parameters_schema_requires_query() {
        let tool = MemoryTreeSearchEntitiesTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"], json!(["query"]));
        assert_eq!(
            schema["properties"]["limit"]["description"].is_string(),
            true
        );
    }

    #[test]
    fn kind_enum_contains_expected_memory_entity_kinds() {
        let tool = MemoryTreeSearchEntitiesTool;
        let schema = tool.parameters_schema();
        let kinds = schema["properties"]["kinds"]["items"]["enum"]
            .as_array()
            .unwrap();
        for required in ["email", "person", "organization", "topic"] {
            assert!(
                kinds.iter().any(|v| v == required),
                "missing kind {required}"
            );
        }
    }

    #[tokio::test]
    async fn execute_rejects_missing_query() {
        let tool = MemoryTreeSearchEntitiesTool;
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("missing query should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tree_search_entities"));
    }

    #[tokio::test]
    async fn execute_rejects_invalid_kind_after_validation() {
        let tool = MemoryTreeSearchEntitiesTool;
        let err = tool
            .execute(json!({
                "query": "alice",
                "kinds": ["not-a-real-kind"]
            }))
            .await
            .expect_err("invalid kind should fail");
        assert!(err
            .to_string()
            .contains("memory_tree_search_entities: invalid kind:"));
    }

    #[tokio::test]
    async fn execute_success_path_returns_empty_json_array_for_isolated_workspace() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeSearchEntitiesTool;
        let result = tool
            .execute(json!({
                "query": "alice",
                "limit": 3
            }))
            .await
            .expect("valid search_entities request should succeed in isolated workspace");
        assert!(!result.is_error);
        let payload = result.text();
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("result should be valid json");
        assert!(
            parsed.is_array(),
            "search_entities should serialize a JSON array"
        );
        assert_eq!(parsed, json!([]));

        let direct = retrieval::search_entities(&cfg, "alice", None, 3)
            .await
            .expect("direct search_entities on empty workspace");
        assert!(direct.is_empty());
    }

    #[tokio::test]
    async fn execute_accepts_kind_filter_and_clamps_large_limit() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _cfg) = isolated_config(&tmp).await;
        let tool = MemoryTreeSearchEntitiesTool;
        let result = tool
            .execute(json!({
                "query": "alice",
                "kinds": ["email", "person"],
                "limit": 999
            }))
            .await
            .expect("filtered search_entities request should succeed");
        assert!(!result.is_error);
    }
}
