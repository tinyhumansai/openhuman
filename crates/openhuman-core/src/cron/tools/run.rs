use crate::config::Config;
use crate::cron;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult};

pub struct CronRunTool {
    config: Arc<Config>,
}

impl CronRunTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CronRunTool {
    fn name(&self) -> &str {
        "cron_run"
    }

    fn description(&self) -> &str {
        "Force-run a cron job immediately and record run history"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string" }
            },
            "required": ["job_id"]
        })
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        // Force-running a job immediately executes the stored command or
        // agent prompt on the host.  Require approval (GHSA-f46p-6vf9-64mm).
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        options: ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        if !self.config.cron.enabled {
            return Ok(ToolResult::error(
                "cron is disabled by config (cron.enabled=false)".to_string(),
            ));
        }

        let dispatch = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        let job_id = match args.get("job_id").and_then(serde_json::Value::as_str) {
            Some(v) if !v.trim().is_empty() => v,
            _ => {
                return Ok(ToolResult::error("Missing 'job_id' parameter".to_string()));
            }
        };

        let job = match cron::ops::load_job_for_run(&self.config, job_id) {
            Ok(job) => job,
            Err(e) => {
                return Ok(ToolResult::error(e.to_string()));
            }
        };

        if let Err(e) = cron::limits::check_run(&self.config, &job) {
            return Ok(ToolResult::error(e.to_string()));
        }
        let started_at = Utc::now();
        let run_id = uuid::Uuid::new_v4().to_string();
        let reply = match cron::scheduler::run_job_now(
            &dispatch,
            &self.config,
            job.clone(),
            run_id,
            true,
            tinyflows_bus::schedule::ManualDelivery::NotRequested,
        )
        .await
        {
            Ok(reply) => reply,
            Err(error) => return Ok(ToolResult::error(error.to_string())),
        };
        let (success, output) = match reply {
            tinyflows_bus::schedule::RunJobReply::Completed {
                success, attempt, ..
            } => (success, attempt.output),
            tinyflows_bus::schedule::RunJobReply::Refused { reason } => {
                return Ok(ToolResult::error(format!(
                    "cron job '{}' was not admitted: {reason:?}",
                    job.id
                )));
            }
            tinyflows_bus::schedule::RunJobReply::Accepted { .. } => {
                return Ok(ToolResult::error(
                    "cron module accepted a synchronous run without completing it".to_string(),
                ));
            }
        };
        let duration_ms = (Utc::now() - started_at).num_milliseconds();
        let status = if success { "ok" } else { "error" };

        let payload = json!({
            "job_id": job.id,
            "status": status,
            "duration_ms": duration_ms,
            "output": output
        });
        let result_output = serde_json::to_string_pretty(&payload)?;
        let md = if options.prefer_markdown {
            let trimmed = output.trim();
            let body = if trimmed.is_empty() {
                String::new()
            } else {
                format!("\n\n```\n{trimmed}\n```")
            };
            Some(format!(
                "**job**: `{}` — **status**: {} — **{}ms**{}",
                job.id, status, duration_ms, body
            ))
        } else {
            None
        };
        let mut tr = if success {
            ToolResult::success(result_output)
        } else {
            ToolResult::error(result_output)
        };
        tr.markdown_formatted = md;
        Ok(tr)
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
