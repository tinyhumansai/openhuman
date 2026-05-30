//! Tool: vault_write_markdown — approved markdown/wiki writes into user vaults.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

pub struct VaultWriteMarkdownTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

impl VaultWriteMarkdownTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }
}

#[async_trait]
impl Tool for VaultWriteMarkdownTool {
    fn name(&self) -> &str {
        "vault_write_markdown"
    }

    fn description(&self) -> &str {
        "Create or update an approved .md/.markdown wiki note inside a registered knowledge vault. \
         Use only after the user has approved writing generated content into that vault."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["vault_id", "rel_path", "content"],
            "properties": {
                "vault_id": {
                    "type": "string",
                    "description": "Identifier of the registered vault to write into."
                },
                "rel_path": {
                    "type": "string",
                    "description": "Relative path under the vault root. Must end with .md or .markdown."
                },
                "content": {
                    "type": "string",
                    "description": "Markdown content to write."
                },
                "overwrite": {
                    "type": "boolean",
                    "description": "Set true to update an existing markdown file."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.security.gate_decision(CommandClass::Write) == GateDecision::Prompt
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only",
            ));
        }
        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }

        let vault_id = args
            .get("vault_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'vault_id' parameter"))?;
        let rel_path = args
            .get("rel_path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'rel_path' parameter"))?;
        let content = args
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;
        let overwrite = args
            .get("overwrite")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        tracing::debug!(
            vault_id = %vault_id,
            rel_path = %rel_path,
            overwrite,
            content_bytes = content.len(),
            "[vault_write_markdown] execute"
        );
        match crate::openhuman::vault::ops::vault_write_markdown(
            &self.config,
            vault_id,
            rel_path,
            content,
            overwrite,
            true,
        )
        .await
        {
            Ok(outcome) => Ok(ToolResult::json(json!(outcome.value))),
            Err(err) => Ok(ToolResult::error(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::AutonomyLevel;

    fn security(workspace: std::path::PathBuf, autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy,
            workspace_dir: workspace,
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn vault_write_markdown_tool_schema_requires_core_fields() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(Config::default());
        let tool = VaultWriteMarkdownTool::new(
            config,
            security(dir.path().to_path_buf(), AutonomyLevel::Supervised),
        );

        assert_eq!(tool.name(), "vault_write_markdown");
        assert_eq!(tool.permission_level(), PermissionLevel::Write);
        assert!(tool.external_effect_with_args(&json!({})));
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|value| value == "vault_id"));
        assert!(required.iter().any(|value| value == "rel_path"));
        assert!(required.iter().any(|value| value == "content"));
    }

    #[tokio::test]
    async fn vault_write_markdown_tool_blocks_read_only_autonomy() {
        let dir = tempfile::tempdir().unwrap();
        let config = Arc::new(Config::default());
        let tool = VaultWriteMarkdownTool::new(
            config,
            security(dir.path().to_path_buf(), AutonomyLevel::ReadOnly),
        );

        let result = tool
            .execute(json!({
                "vault_id": "v-1",
                "rel_path": "wiki/a.md",
                "content": "# A"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.text().contains("read-only"));
    }
}
