use crate::openhuman::config::Config;
use crate::openhuman::cron;
use crate::openhuman::cron::CronJob;
use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write as _;
use std::sync::Arc;

fn render_jobs_markdown(jobs: &[CronJob]) -> String {
    if jobs.is_empty() {
        return "_No scheduled cron jobs._".to_string();
    }
    let mut out = format!("# Cron jobs ({})\n", jobs.len());
    for job in jobs {
        let label = job.name.as_deref().unwrap_or(&job.id);
        let _ = writeln!(out, "\n## {label}");
        let _ = writeln!(out, "- **id**: `{}`", job.id);
        let _ = writeln!(out, "- **schedule**: `{}`", job.expression);
        let _ = writeln!(out, "- **enabled**: {}", job.enabled);
        let _ = writeln!(
            out,
            "- **next_run**: {}",
            job.next_run.format("%Y-%m-%d %H:%M:%S UTC")
        );
        if let Some(last) = job.last_run {
            let _ = writeln!(
                out,
                "- **last_run**: {} ({})",
                last.format("%Y-%m-%d %H:%M:%S UTC"),
                job.last_status.as_deref().unwrap_or("unknown")
            );
        }
        if let Some(agent) = &job.agent_id {
            let _ = writeln!(out, "- **agent**: `{agent}`");
        }
        let _ = writeln!(out, "- **command**: `{}`", job.command);
        if let Some(prompt) = &job.prompt {
            let trimmed = prompt.trim();
            if !trimmed.is_empty() {
                let preview = if trimmed.chars().count() > 200 {
                    let snippet: String = trimmed.chars().take(200).collect();
                    format!("{snippet}…")
                } else {
                    trimmed.to_string()
                };
                let _ = writeln!(out, "- **prompt**: {preview}");
            }
        }
    }
    out
}

pub struct CronListTool {
    config: Arc<Config>,
}

impl CronListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for CronListTool {
    fn name(&self) -> &str {
        "cron_list"
    }

    fn description(&self) -> &str {
        "List all scheduled cron jobs"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        _args: serde_json::Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        if !self.config.cron.enabled {
            return Ok(ToolResult::error(
                "cron is disabled by config (cron.enabled=false)".to_string(),
            ));
        }

        match cron::list_jobs(&self.config) {
            Ok(jobs) => {
                let json_str = serde_json::to_string_pretty(&jobs)?;
                let mut result = ToolResult::success(json_str);
                if options.prefer_markdown {
                    result.markdown_formatted = Some(render_jobs_markdown(&jobs));
                }
                Ok(result)
            }
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }

    fn supports_markdown(&self) -> bool {
        true
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
    async fn returns_empty_list_when_no_jobs() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let tool = CronListTool::new(cfg);

        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output().trim(), "[]");
    }

    #[tokio::test]
    async fn errors_when_cron_disabled() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = (*test_config(&tmp).await).clone();
        cfg.cron.enabled = false;
        let tool = CronListTool::new(Arc::new(cfg));

        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("cron is disabled"));
    }
}
