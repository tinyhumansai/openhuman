use crate::config::Config;
use crate::cron;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolExposure, ToolResult};

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
    /// Superseded by the `cron` tool, which dispatches every scheduler
    /// operation on one `action` field. Kept registered and dispatchable so a
    /// replayed transcript or a saved skill naming `cron_*` keeps working;
    /// hidden from the wire so six schemas do not ship where one does.
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Hidden
    }

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

        let job = match cron::get_job(&self.config, job_id) {
            Ok(job) => job,
            Err(e) => {
                return Ok(ToolResult::error(e.to_string()));
            }
        };

        if let Err(e) = cron::policy::get_policy(&self.config, &job.id) {
            return Ok(ToolResult::error(e.to_string()));
        }
        if let Err(e) = cron::limits::check_run(&self.config, &job) {
            return Ok(ToolResult::error(e.to_string()));
        }

        // Claim the job so this run cannot overlap a scheduled tick or a Run Now.
        let Some(_run_guard) = cron::ops::try_acquire_run(&job.id) else {
            return Ok(ToolResult::error(format!(
                "cron job '{}' is already running",
                job.id
            )));
        };

        let started_at = Utc::now();
        let run_id = uuid::Uuid::new_v4().to_string();
        let (success, output) = cron::scheduler::execute_job_now(&self.config, &job, &run_id).await;
        let finished_at = Utc::now();
        let duration_ms = (finished_at - started_at).num_milliseconds();
        let status = if success { "ok" } else { "error" };

        // The output is returned to the calling agent below, so nothing is
        // delivered here: an origin delivery would append to the transcript
        // of the very session this tool call is running inside, and wait on
        // that session's own turn lock.
        let _ = cron::record_run_with_delivery(
            &self.config,
            &job.id,
            started_at,
            finished_at,
            status,
            Some(&output),
            duration_ms,
            Some(cron::DeliveryStatus::NotRequested),
        );
        let _ = cron::record_last_run(&self.config, &job.id, finished_at, success, &output);

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
