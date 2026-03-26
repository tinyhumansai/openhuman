use super::traits::{Tool, ToolResult};
use crate::openhuman::config::Config;
use crate::openhuman::cron;
use crate::openhuman::security::SecurityPolicy;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;

/// Tool that lets the agent manage recurring and one-shot scheduled tasks.
pub struct ScheduleTool {
    security: Arc<SecurityPolicy>,
    config: Config,
}

impl ScheduleTool {
    pub fn new(security: Arc<SecurityPolicy>, config: Config) -> Self {
        Self { security, config }
    }
}

#[async_trait]
impl Tool for ScheduleTool {
    fn name(&self) -> &str {
        "schedule"
    }

    fn description(&self) -> &str {
        "Manage scheduled tasks. Actions: create/add/once/list/get/cancel/remove/pause/resume"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "add", "once", "list", "get", "cancel", "remove", "pause", "resume"],
                    "description": "Action to perform"
                },
                "expression": {
                    "type": "string",
                    "description": "Cron expression for recurring tasks (e.g. '*/5 * * * *')."
                },
                "delay": {
                    "type": "string",
                    "description": "Delay for one-shot tasks (e.g. '30m', '2h', '1d')."
                },
                "run_at": {
                    "type": "string",
                    "description": "Absolute RFC3339 time for one-shot tasks (e.g. '2030-01-01T00:00:00Z')."
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to execute. Required for create/add/once."
                },
                "id": {
                    "type": "string",
                    "description": "Task ID. Required for get/cancel/remove/pause/resume."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "list" => self.handle_list(),
            "get" => {
                let id = args
                    .get("id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter for get action"))?;
                self.handle_get(id)
            }
            "create" | "add" | "once" => {
                if let Some(blocked) = self.enforce_mutation_allowed(action) {
                    return Ok(blocked);
                }
                self.handle_create_like(action, &args)
            }
            "cancel" | "remove" => {
                if let Some(blocked) = self.enforce_mutation_allowed(action) {
                    return Ok(blocked);
                }
                let id = args
                    .get("id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter for cancel action"))?;
                Ok(self.handle_cancel(id))
            }
            "pause" => {
                if let Some(blocked) = self.enforce_mutation_allowed(action) {
                    return Ok(blocked);
                }
                let id = args
                    .get("id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter for pause action"))?;
                Ok(self.handle_pause_resume(id, true))
            }
            "resume" => {
                if let Some(blocked) = self.enforce_mutation_allowed(action) {
                    return Ok(blocked);
                }
                let id = args
                    .get("id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'id' parameter for resume action"))?;
                Ok(self.handle_pause_resume(id, false))
            }
            other => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Unknown action '{other}'. Use create/add/once/list/get/cancel/remove/pause/resume."
                )),
            }),
        }
    }
}

impl ScheduleTool {
    fn enforce_mutation_allowed(&self, action: &str) -> Option<ToolResult> {
        if !self.security.can_act() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Security policy: read-only mode, cannot perform '{action}'"
                )),
            });
        }

        if !self.security.record_action() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Rate limit exceeded: action budget exhausted".to_string()),
            });
        }

        None
    }

    fn handle_list(&self) -> Result<ToolResult> {
        let jobs = cron::list_jobs(&self.config)?;
        if jobs.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: "No scheduled jobs.".to_string(),
                error: None,
            });
        }

        let mut lines = Vec::with_capacity(jobs.len());
        for job in jobs {
            let paused = !job.enabled;
            let one_shot = matches!(job.schedule, cron::Schedule::At { .. });
            let flags = match (paused, one_shot) {
                (true, true) => " [disabled, one-shot]",
                (true, false) => " [disabled]",
                (false, true) => " [one-shot]",
                (false, false) => "",
            };
            let last_run = job
                .last_run
                .map_or_else(|| "never".to_string(), |value| value.to_rfc3339());
            let last_status = job.last_status.unwrap_or_else(|| "n/a".to_string());
            lines.push(format!(
                "- {} | {} | next={} | last={} ({}){} | cmd: {}",
                job.id,
                job.expression,
                job.next_run.to_rfc3339(),
                last_run,
                last_status,
                flags,
                job.command
            ));
        }

        Ok(ToolResult {
            success: true,
            output: format!("Scheduled jobs ({}):\n{}", lines.len(), lines.join("\n")),
            error: None,
        })
    }

    fn handle_get(&self, id: &str) -> Result<ToolResult> {
        match cron::get_job(&self.config, id) {
            Ok(job) => {
                let detail = json!({
                    "id": job.id,
                    "expression": job.expression,
                    "command": job.command,
                    "next_run": job.next_run.to_rfc3339(),
                    "last_run": job.last_run.map(|value| value.to_rfc3339()),
                    "last_status": job.last_status,
                    "enabled": job.enabled,
                    "one_shot": matches!(job.schedule, cron::Schedule::At { .. }),
                });
                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&detail)?,
                    error: None,
                })
            }
            Err(_) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Job '{id}' not found")),
            }),
        }
    }

    fn handle_create_like(&self, action: &str, args: &serde_json::Value) -> Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("Missing or empty 'command' parameter"))?;

        let expression = args.get("expression").and_then(|value| value.as_str());
        let delay = args.get("delay").and_then(|value| value.as_str());
        let run_at = args.get("run_at").and_then(|value| value.as_str());

        match action {
            "add" => {
                if expression.is_none() || delay.is_some() || run_at.is_some() {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("'add' requires 'expression' and forbids delay/run_at".into()),
                    });
                }
            }
            "once" => {
                if expression.is_some() || (delay.is_none() && run_at.is_none()) {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("'once' requires exactly one of 'delay' or 'run_at'".into()),
                    });
                }
                if delay.is_some() && run_at.is_some() {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("'once' supports either delay or run_at, not both".into()),
                    });
                }
            }
            _ => {
                let count = [expression.is_some(), delay.is_some(), run_at.is_some()]
                    .into_iter()
                    .filter(|value| *value)
                    .count();
                if count != 1 {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(
                            "Exactly one of 'expression', 'delay', or 'run_at' must be provided"
                                .into(),
                        ),
                    });
                }
            }
        }

        if let Some(value) = expression {
            let job = cron::add_job(&self.config, value, command)?;
            return Ok(ToolResult {
                success: true,
                output: format!(
                    "Created recurring job {} (expr: {}, next: {}, cmd: {})",
                    job.id,
                    job.expression,
                    job.next_run.to_rfc3339(),
                    job.command
                ),
                error: None,
            });
        }

        if let Some(value) = delay {
            let job = cron::add_once(&self.config, value, command)?;
            return Ok(ToolResult {
                success: true,
                output: format!(
                    "Created one-shot job {} (runs at: {}, cmd: {})",
                    job.id,
                    job.next_run.to_rfc3339(),
                    job.command
                ),
                error: None,
            });
        }

        let run_at_raw = run_at.ok_or_else(|| anyhow::anyhow!("Missing scheduling parameters"))?;
        let run_at_parsed: DateTime<Utc> = DateTime::parse_from_rfc3339(run_at_raw)
            .map_err(|error| anyhow::anyhow!("Invalid run_at timestamp: {error}"))?
            .with_timezone(&Utc);

        let job = cron::add_once_at(&self.config, run_at_parsed, command)?;
        Ok(ToolResult {
            success: true,
            output: format!(
                "Created one-shot job {} (runs at: {}, cmd: {})",
                job.id,
                job.next_run.to_rfc3339(),
                job.command
            ),
            error: None,
        })
    }

    fn handle_cancel(&self, id: &str) -> ToolResult {
        match cron::remove_job(&self.config, id) {
            Ok(()) => ToolResult {
                success: true,
                output: format!("Cancelled job {id}"),
                error: None,
            },
            Err(error) => ToolResult {
                success: false,
                output: String::new(),
                error: Some(error.to_string()),
            },
        }
    }

    fn handle_pause_resume(&self, id: &str, pause: bool) -> ToolResult {
        let operation = if pause {
            cron::pause_job(&self.config, id)
        } else {
            cron::resume_job(&self.config, id)
        };

        match operation {
            Ok(_) => ToolResult {
                success: true,
                output: if pause {
                    format!("Paused job {id}")
                } else {
                    format!("Resumed job {id}")
                },
                error: None,
            },
            Err(error) => ToolResult {
                success: false,
                output: String::new(),
                error: Some(error.to_string()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::AutonomyLevel;
    use tempfile::TempDir;

    async fn test_setup() -> (TempDir, Config, Arc<SecurityPolicy>) {
        let tmp = TempDir::new().unwrap();
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        tokio::fs::create_dir_all(&config.workspace_dir)
            .await
            .unwrap();
        let security = Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
        ));
        (tmp, config, security)
    }

    #[tokio::test]
    async fn tool_name_and_schema() {
        let (_tmp, config, security) = test_setup().await;
        let tool = ScheduleTool::new(security, config);
        assert_eq!(tool.name(), "schedule");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["action"].is_object());
    }

    #[tokio::test]
    async fn list_empty() {
        let (_tmp, config, security) = test_setup().await;
        let tool = ScheduleTool::new(security, config);

        let result = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No scheduled jobs"));
    }

    #[tokio::test]
    async fn create_get_and_cancel_roundtrip() {
        let (_tmp, config, security) = test_setup().await;
        let tool = ScheduleTool::new(security, config);

        let create = tool
            .execute(json!({
                "action": "create",
                "expression": "*/5 * * * *",
                "command": "echo hello"
            }))
            .await
            .unwrap();
        assert!(create.success);
        assert!(create.output.contains("Created recurring job"));

        let list = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(list.success);
        assert!(list.output.contains("echo hello"));

        let id = create.output.split_whitespace().nth(3).unwrap();

        let get = tool
            .execute(json!({"action": "get", "id": id}))
            .await
            .unwrap();
        assert!(get.success);
        assert!(get.output.contains("echo hello"));

        let cancel = tool
            .execute(json!({"action": "cancel", "id": id}))
            .await
            .unwrap();
        assert!(cancel.success);
    }

    #[tokio::test]
    async fn once_and_pause_resume_aliases_work() {
        let (_tmp, config, security) = test_setup().await;
        let tool = ScheduleTool::new(security, config);

        let once = tool
            .execute(json!({
                "action": "once",
                "delay": "30m",
                "command": "echo delayed"
            }))
            .await
            .unwrap();
        assert!(once.success);

        let add = tool
            .execute(json!({
                "action": "add",
                "expression": "*/10 * * * *",
                "command": "echo recurring"
            }))
            .await
            .unwrap();
        assert!(add.success);

        let id = add.output.split_whitespace().nth(3).unwrap();
        let pause = tool
            .execute(json!({"action": "pause", "id": id}))
            .await
            .unwrap();
        assert!(pause.success);

        let resume = tool
            .execute(json!({"action": "resume", "id": id}))
            .await
            .unwrap();
        assert!(resume.success);
    }

    #[tokio::test]
    async fn readonly_blocks_mutating_actions() {
        let tmp = TempDir::new().unwrap();
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            autonomy: crate::openhuman::config::AutonomyConfig {
                level: AutonomyLevel::ReadOnly,
                ..Default::default()
            },
            ..Config::default()
        };
        tokio::fs::create_dir_all(&config.workspace_dir)
            .await
            .unwrap();
        let security = Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
        ));

        let tool = ScheduleTool::new(security, config);

        let blocked = tool
            .execute(json!({
                "action": "create",
                "expression": "* * * * *",
                "command": "echo blocked"
            }))
            .await
            .unwrap();
        assert!(!blocked.success);
        assert!(blocked.error.as_deref().unwrap().contains("read-only"));

        let list = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(list.success);
    }

    #[tokio::test]
    async fn unknown_action_returns_failure() {
        let (_tmp, config, security) = test_setup().await;
        let tool = ScheduleTool::new(security, config);

        let result = tool.execute(json!({"action": "explode"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("Unknown action"));
    }
}
