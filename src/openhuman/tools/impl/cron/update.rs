use crate::openhuman::config::Config;
use crate::openhuman::cron::{self, CronJobPatch};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
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

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
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
            Ok(job) => Ok(ToolResult::success(serde_json::to_string_pretty(&job)?)),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
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

    fn test_security(cfg: &Config) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy::from_config(
            &cfg.autonomy,
            &cfg.workspace_dir,
        ))
    }

    #[tokio::test]
    async fn updates_enabled_flag() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let job = cron::add_job(&cfg, "*/5 * * * *", "echo ok").unwrap();
        let tool = CronUpdateTool::new(cfg.clone(), test_security(&cfg));

        let result = tool
            .execute(json!({
                "job_id": job.id,
                "patch": { "enabled": false }
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "{:?}", result.output());
        assert!(result.output().contains("\"enabled\": false"));
    }

    #[tokio::test]
    async fn blocks_disallowed_command_updates() {
        let tmp = TempDir::new().unwrap();
        let mut config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        config.autonomy.allowed_commands = vec!["echo".into()];
        tokio::fs::create_dir_all(&config.workspace_dir)
            .await
            .unwrap();
        let cfg = Arc::new(config);
        let job = cron::add_job(&cfg, "*/5 * * * *", "echo ok").unwrap();
        let tool = CronUpdateTool::new(cfg.clone(), test_security(&cfg));

        let result = tool
            .execute(json!({
                "job_id": job.id,
                "patch": { "command": "curl https://example.com" }
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("blocked by security policy"));
    }
}
