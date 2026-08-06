//! LLM-callable tools for the skill runtime domain.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops::{resolve_runtimes, RuntimeRequirement};

pub struct SkillRuntimeResolveRuntimesTool;

impl SkillRuntimeResolveRuntimesTool {
    pub fn new(_config: Arc<Config>) -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SkillRuntimeResolveRuntimesTool {
    fn name(&self) -> &str {
        "skill_runtime_resolve_runtimes"
    }

    fn description(&self) -> &str {
        "Resolve OpenHuman's reusable Node/Python runtimes for skill execution. \
         Use before running skills that reference node, npm, npx, python, or \
         bundled .js/.py scripts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "runtime": {
                    "type": "string",
                    "enum": ["all", "node", "python"],
                    "description": "Runtime to resolve. Defaults to all."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let config = Config::load_or_init()
            .await
            .map_err(|error| anyhow::anyhow!("load config: {error:#}"))?;
        let requirement = RuntimeRequirement::from_optional(
            args.get("runtime").and_then(serde_json::Value::as_str),
        )
        .map_err(|error| anyhow::anyhow!(error))?;
        tracing::debug!(
            requirement = ?requirement,
            "[tool][skill_runtime] resolve_runtimes"
        );
        let outcome = resolve_runtimes(&config, requirement).await;
        Ok(ToolResult::success(serde_json::to_string(&outcome)?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}
