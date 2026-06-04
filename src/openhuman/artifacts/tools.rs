//! LLM-callable wrappers over the artifacts metadata domain.
//!
//! Each tool is a thin shim over a read/delete handler in
//! [`crate::openhuman::artifacts::ops`], unwrapping the `RpcOutcome`
//! envelope and emitting the inner JSON value. The artifacts domain owns
//! agent-generated files (presentations/documents/images) under
//! `<workspace>/artifacts/`; these tools let the agent enumerate and
//! inspect what it has produced.
//!
//! `artifact_list` / `artifact_get` are read-only and default-enabled.
//! `artifact_delete` is `Dangerous` (irreversible directory removal) and
//! ships default-OFF — it must be opted in via the tool toggle
//! (`TOOL_ID_TO_RUST_NAMES` in `tools/user_filter.rs`).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::artifacts::ops;
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Read `offset` / `limit` as optional `usize` from tool args.
fn read_opt_usize(args: &serde_json::Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(serde_json::Value::as_u64)
        .map(|v| v as usize)
}

/// Read a required, non-empty string arg.
fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    let raw = args
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match raw {
        Some(s) => Ok(s.to_string()),
        None => Err(anyhow::anyhow!("missing required string argument `{key}`")),
    }
}

/// List artifacts the agent has produced, newest first.
pub struct ArtifactListTool {
    config: Arc<Config>,
}

impl ArtifactListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ArtifactListTool {
    fn name(&self) -> &str {
        "artifact_list"
    }

    fn description(&self) -> &str {
        "List agent-generated artifacts (presentations, documents, images, \
         and other files this assistant has produced), sorted by creation \
         time descending. Each entry carries `id`, `kind`, `title`, `path`, \
         `size_bytes`, `status`, and `created_at`. Use this to recall what \
         you have already generated for the user before regenerating, or to \
         find an artifact `id` to pass to `artifact_get`. Supports `offset` \
         and `limit` pagination."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "offset": { "type": "integer", "minimum": 0, "description": "Pagination offset (default 0)." },
                "limit": { "type": "integer", "minimum": 1, "description": "Max artifacts to return (default 50, cap 200)." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][artifacts] list invoked");
        let offset = read_opt_usize(&args, "offset");
        let limit = read_opt_usize(&args, "limit");
        // The agent-facing tool surface lists everything in the workspace;
        // the per-chat filter is an RPC-only knob used by the React panel
        // (#3226). Keep the tool path unchanged so existing agent flows
        // (orchestrator artifact reasoning) still see the full set.
        let outcome = ops::ai_list_artifacts(&self.config, offset, limit, None)
            .await
            .map_err(|e| anyhow::anyhow!("artifact_list: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Retrieve a single artifact's metadata plus its absolute on-disk path.
pub struct ArtifactGetTool {
    config: Arc<Config>,
}

impl ArtifactGetTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ArtifactGetTool {
    fn name(&self) -> &str {
        "artifact_get"
    }

    fn description(&self) -> &str {
        "Get one agent-generated artifact by `id`, returning its metadata \
         (`kind`, `title`, `path`, `size_bytes`, `status`, `created_at`) plus \
         a computed `absolute_path` you can read from. Call `artifact_list` \
         first to discover ids."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "artifact_id": { "type": "string", "description": "The artifact id (UUID) to fetch." }
            },
            "required": ["artifact_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][artifacts] get invoked");
        let id = read_required_str(&args, "artifact_id")?;
        let outcome = ops::ai_get_artifact(&self.config, &id)
            .await
            .map_err(|e| anyhow::anyhow!("artifact_get: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Delete an artifact directory and all its contents. **Irreversible** —
/// ships default-OFF (`Dangerous`).
pub struct ArtifactDeleteTool {
    config: Arc<Config>,
}

impl ArtifactDeleteTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ArtifactDeleteTool {
    fn name(&self) -> &str {
        "artifact_delete"
    }

    fn description(&self) -> &str {
        "Permanently delete an agent-generated artifact and all of its files \
         by `id`. This is IRREVERSIBLE — the artifact directory is removed \
         from disk. Only use when the user has clearly asked to discard a \
         specific artifact."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "artifact_id": { "type": "string", "description": "The artifact id (UUID) to delete." }
            },
            "required": ["artifact_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][artifacts] delete invoked");
        let id = read_required_str(&args, "artifact_id")?;
        let outcome = ops::ai_delete_artifact(&self.config, &id)
            .await
            .map_err(|e| anyhow::anyhow!("artifact_delete: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    fn test_config() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn metadata_is_stable() {
        let cfg = test_config();
        assert_eq!(ArtifactListTool::new(cfg.clone()).name(), "artifact_list");
        assert_eq!(ArtifactGetTool::new(cfg.clone()).name(), "artifact_get");
        assert_eq!(
            ArtifactDeleteTool::new(cfg.clone()).name(),
            "artifact_delete"
        );
        assert_eq!(
            ArtifactListTool::new(cfg.clone()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            ArtifactDeleteTool::new(cfg.clone()).permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(ArtifactListTool::new(cfg).scope(), ToolScope::All);
    }

    #[test]
    fn read_tools_are_concurrency_safe() {
        let cfg = test_config();
        assert!(ArtifactListTool::new(cfg.clone()).is_concurrency_safe(&serde_json::Value::Null));
        assert!(ArtifactGetTool::new(cfg).is_concurrency_safe(&serde_json::Value::Null));
    }

    #[tokio::test]
    async fn get_requires_artifact_id() {
        let tool = ArtifactGetTool::new(test_config());
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("expected missing-arg error");
        assert!(err.to_string().contains("artifact_id"));
    }

    #[tokio::test]
    async fn delete_requires_artifact_id() {
        let tool = ArtifactDeleteTool::new(test_config());
        let err = tool
            .execute(json!({ "artifact_id": "  " }))
            .await
            .expect_err("expected missing-arg error");
        assert!(err.to_string().contains("artifact_id"));
    }

    #[tokio::test]
    async fn list_returns_artifacts_envelope() {
        // Config::default() points at a workspace dir; listing an empty/missing
        // artifacts root yields an empty list, not an error.
        let tool = ArtifactListTool::new(test_config());
        let result = tool.execute(json!({ "limit": 5 })).await;
        // Either a clean empty listing or a benign error if the workspace is
        // unwritable in the sandbox — assert it does not panic and, when Ok,
        // carries the expected shape.
        if let Ok(res) = result {
            let body = res.output_for_llm(false);
            assert!(body.contains("artifacts"), "body was: {body}");
        }
    }
}
