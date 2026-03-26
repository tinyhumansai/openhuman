use super::traits::{Tool, ToolResult};
use crate::openhuman::config::Config;
use crate::openhuman::cron;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use std::sync::Arc;

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

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if !self.config.cron.enabled {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("cron is disabled by config (cron.enabled=false)".to_string()),
            });
        }

        let job_id = match args.get("job_id").and_then(serde_json::Value::as_str) {
            Some(v) if !v.trim().is_empty() => v,
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing 'job_id' parameter".to_string()),
                });
            }
        };

        let job = match cron::get_job(&self.config, job_id) {
            Ok(job) => job,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e.to_string()),
                });
            }
        };

        let started_at = Utc::now();
        let (success, output) = cron::scheduler::execute_job_now(&self.config, &job).await;
        let finished_at = Utc::now();
        let duration_ms = (finished_at - started_at).num_milliseconds();
        let status = if success { "ok" } else { "error" };

        let _ = cron::record_run(
            &self.config,
            &job.id,
            started_at,
            finished_at,
            status,
            Some(&output),
            duration_ms,
        );
        let _ = cron::record_last_run(&self.config, &job.id, finished_at, success, &output);

        Ok(ToolResult {
            success,
            output: serde_json::to_string_pretty(&json!({
                "job_id": job.id,
                "status": status,
                "duration_ms": duration_ms,
                "output": output
            }))?,
            error: if success {
                None
            } else {
                Some("cron job execution failed".to_string())
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use tempfile::TempDir;

    async fn test_config(tmp: &TempDir) -> Arc<Config> {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        tokio::fs::create_dir_all(&config.workspace_dir)
            .await
            .unwrap();
        Arc::new(config)
    }

    #[tokio::test]
    async fn force_runs_job_and_records_history() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let job = cron::add_job(&cfg, "*/5 * * * *", "echo run-now").unwrap();
        let tool = CronRunTool::new(cfg.clone());

        let result = tool.execute(json!({ "job_id": job.id })).await.unwrap();
        assert!(result.success, "{:?}", result.error);

        let runs = cron::list_runs(&cfg, &job.id, 10).unwrap();
        assert_eq!(runs.len(), 1);
    }

    #[tokio::test]
    async fn errors_for_missing_job() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let tool = CronRunTool::new(cfg);

        let result = tool
            .execute(json!({ "job_id": "missing-job-id" }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("not found"));
    }
}
