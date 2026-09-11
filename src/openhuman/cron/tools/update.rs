use crate::openhuman::config::Config;
use crate::openhuman::cron::{self, CronJobPatch};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

pub struct CronUpdateTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

impl CronUpdateTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }
}

#[async_trait]
impl Tool for CronUpdateTool {
    fn name(&self) -> &str {
        "cron_update"
    }

    fn description(&self) -> &str {
        "Patch an existing cron job (schedule, command, prompt, enabled, delivery, model, etc.)"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string" },
                "patch": { "type": "object" }
            },
            "required": ["job_id", "patch"]
        })
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        // Patching a job can change the stored command or re-enable a
        // disabled job.  Require approval via the gate (GHSA-f46p-6vf9-64mm).
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        if !self.config.cron.enabled {
            return Ok(ToolResult::error(
                "cron is disabled by config (cron.enabled=false)".to_string(),
            ));
        }

        let job_id = match args.get("job_id").and_then(serde_json::Value::as_str) {
            Some(v) if !v.trim().is_empty() => v,
            _ => {
                return Ok(ToolResult::error("Missing 'job_id' parameter".to_string()));
            }
        };

        let patch_val = match args.get("patch") {
            Some(v) => v.clone(),
            None => {
                return Ok(ToolResult::error("Missing 'patch' parameter".to_string()));
            }
        };

        let patch = match serde_json::from_value::<CronJobPatch>(patch_val) {
            Ok(patch) => patch,
            Err(e) => {
                return Ok(ToolResult::error(format!("Invalid patch payload: {e}")));
            }
        };

        if let Some(command) = &patch.command {
            if !self.security.is_command_allowed(command) {
                return Ok(ToolResult::error(format!(
                    "Command blocked by security policy: {command}"
                )));
            }
        }

        match cron::update_job(&self.config, job_id, patch) {
            Ok(job) => {
                let mut tr = ToolResult::success(serde_json::to_string_pretty(&job)?);
                if options.prefer_markdown {
                    tr.markdown_formatted = Some(format!(
                        "Updated **{}** (`{}`).\n- **schedule**: `{}`\n- **enabled**: {}\n- **next_run**: {}",
                        job.name.as_deref().unwrap_or(&job.id),
                        job.id,
                        job.expression,
                        job.enabled,
                        job.next_run.format("%Y-%m-%d %H:%M:%S UTC"),
                    ));
                }
                Ok(tr)
            }
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
