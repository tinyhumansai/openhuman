use crate::openhuman::config::Config;
use crate::openhuman::cron::{self, DeliveryConfig, Schedule, SessionTarget};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
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
                    "description": "Shell command to execute. Use 'command' for shell jobs OR 'prompt' for agent jobs."
                },
                "prompt": {
                    "type": "string",
                    "description": "Agent prompt for recurring agent tasks (e.g. reminders, briefings). Use this instead of 'command' for user-facing notifications."
                },
                "name": {
                    "type": "string",
                    "description": "Short human-readable name for the job (e.g. 'drink_water_reminder'). Always provide a name."
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
            other => Ok(ToolResult::error(format!(
                    "Unknown action '{other}'. Use create/add/once/list/get/cancel/remove/pause/resume."
                ))),
        }
    }
}

impl ScheduleTool {
    fn enforce_mutation_allowed(&self, action: &str) -> Option<ToolResult> {
        if !self.security.can_act() {
            return Some(ToolResult::error(format!(
                "Security policy: read-only mode, cannot perform '{action}'"
            )));
        }

        if !self.security.record_action() {
            return Some(ToolResult::error(
                "Rate limit exceeded: action budget exhausted".to_string(),
            ));
        }

        None
    }

    fn handle_list(&self) -> Result<ToolResult> {
        let jobs = cron::list_jobs(&self.config)?;
        if jobs.is_empty() {
            return Ok(ToolResult::success("No scheduled jobs.".to_string()));
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

        Ok(ToolResult::success(format!(
            "Scheduled jobs ({}):\n{}",
            lines.len(),
            lines.join("\n")
        )))
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
                Ok(ToolResult::success(serde_json::to_string_pretty(&detail)?))
            }
            Err(_) => Ok(ToolResult::error(format!("Job '{id}' not found"))),
        }
    }

    fn handle_create_like(&self, action: &str, args: &serde_json::Value) -> Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty());
        let prompt = args
            .get("prompt")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty());

        // If the LLM passed a "command" that isn't a real shell command,
        // treat it as an agent prompt instead. This handles the common case
        // where the LLM puts "remind me to drink water" in the command field.
        let (command, prompt) = match (command, prompt) {
            (Some(cmd), None) if !looks_like_shell_command(cmd) => (None, Some(cmd)),
            other => other,
        };

        // Must have either command (shell) or prompt (agent).
        if command.is_none() && prompt.is_none() {
            return Ok(ToolResult::error(
                "Provide 'command' for shell jobs or 'prompt' for agent jobs.".to_string(),
            ));
        }

        let expression = args.get("expression").and_then(|value| value.as_str());
        let delay = args.get("delay").and_then(|value| value.as_str());
        let run_at = args.get("run_at").and_then(|value| value.as_str());

        match action {
            "add" => {
                if expression.is_none() || delay.is_some() || run_at.is_some() {
                    return Ok(ToolResult::error(
                        "'add' requires 'expression' and forbids delay/run_at",
                    ));
                }
            }
            "once" => {
                if expression.is_some() || (delay.is_none() && run_at.is_none()) {
                    return Ok(ToolResult::error(
                        "'once' requires exactly one of 'delay' or 'run_at'",
                    ));
                }
                if delay.is_some() && run_at.is_some() {
                    return Ok(ToolResult::error(
                        "'once' supports either delay or run_at, not both",
                    ));
                }
            }
            _ => {
                let count = [expression.is_some(), delay.is_some(), run_at.is_some()]
                    .into_iter()
                    .filter(|value| *value)
                    .count();
                if count != 1 {
                    return Ok(ToolResult::error(
                        "Exactly one of 'expression', 'delay', or 'run_at' must be provided",
                    ));
                }
            }
        }

        // ── Agent job (prompt provided) ──────────────────────────────
        if let Some(prompt_text) = prompt {
            let schedule = if let Some(expr) = expression {
                Schedule::Cron {
                    expr: expr.to_string(),
                    tz: None,
                }
            } else if let Some(delay_str) = delay {
                let at = Utc::now() + cron::parse_human_delay(delay_str)?;
                Schedule::At { at }
            } else if let Some(at_str) = run_at {
                let at: DateTime<Utc> = DateTime::parse_from_rfc3339(at_str)
                    .map_err(|e| anyhow::anyhow!("Invalid run_at timestamp: {e}"))?
                    .with_timezone(&Utc);
                Schedule::At { at }
            } else {
                return Ok(ToolResult::error("Missing scheduling parameters"));
            };

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| {
                    // Derive a slug from the prompt so jobs are never unnamed.
                    Some(
                        prompt_text
                            .chars()
                            .map(|c| {
                                if c.is_alphanumeric() {
                                    c.to_ascii_lowercase()
                                } else {
                                    '_'
                                }
                            })
                            .take(48)
                            .collect::<String>()
                            .trim_matches('_')
                            .to_string(),
                    )
                    .filter(|s| !s.is_empty())
                });

            let delete_after_run = matches!(schedule, Schedule::At { .. });
            let delivery = Some(DeliveryConfig {
                mode: "proactive".to_string(),
                channel: None,
                to: None,
                best_effort: true,
            });

            let job = cron::add_agent_job(
                &self.config,
                name,
                schedule,
                prompt_text,
                SessionTarget::Isolated,
                None,
                delivery,
                delete_after_run,
            )?;

            let job_name = job.name.as_deref().unwrap_or("(unnamed)");
            return Ok(ToolResult::success(format!(
                "Created agent job '{}' (id: {}, next: {})",
                job_name, job.id, job.next_run,
            )));
        }

        // ── Shell job (command provided) ─────────────────────────────
        let command = command.unwrap();

        if let Some(value) = expression {
            let job = cron::add_job(&self.config, value, command)?;
            return Ok(ToolResult::success(format!(
                "Created recurring job {} (expr: {}, next: {}, cmd: {})",
                job.id,
                job.expression,
                job.next_run.to_rfc3339(),
                job.command
            )));
        }

        if let Some(value) = delay {
            let job = cron::add_once(&self.config, value, command)?;
            return Ok(ToolResult::success(format!(
                "Created one-shot job {} (runs at: {}, cmd: {})",
                job.id,
                job.next_run.to_rfc3339(),
                job.command
            )));
        }

        let run_at_raw = run_at.ok_or_else(|| anyhow::anyhow!("Missing scheduling parameters"))?;
        let run_at_parsed: DateTime<Utc> = DateTime::parse_from_rfc3339(run_at_raw)
            .map_err(|error| anyhow::anyhow!("Invalid run_at timestamp: {error}"))?
            .with_timezone(&Utc);

        let job = cron::add_once_at(&self.config, run_at_parsed, command)?;
        Ok(ToolResult::success(format!(
            "Created one-shot job {} (runs at: {}, cmd: {})",
            job.id,
            job.next_run.to_rfc3339(),
            job.command
        )))
    }

    fn handle_cancel(&self, id: &str) -> ToolResult {
        match cron::remove_job(&self.config, id) {
            Ok(()) => ToolResult::success(format!("Cancelled job {id}")),
            Err(error) => ToolResult::error(error.to_string()),
        }
    }

    fn handle_pause_resume(&self, id: &str, pause: bool) -> ToolResult {
        let operation = if pause {
            cron::pause_job(&self.config, id)
        } else {
            cron::resume_job(&self.config, id)
        };

        match operation {
            Ok(_) => ToolResult::success(if pause {
                format!("Paused job {id}")
            } else {
                format!("Resumed job {id}")
            }),
            Err(error) => ToolResult::error(error.to_string()),
        }
    }
}

/// Heuristic: does this look like a shell command rather than a natural
/// language prompt? Shell commands typically start with an executable name
/// or path and contain shell metacharacters.
fn looks_like_shell_command(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Starts with a path or known shell built-in
    if trimmed.starts_with('/') || trimmed.starts_with("./") || trimmed.starts_with("~/") {
        return true;
    }
    // Contains shell operators
    if trimmed.contains('|') || trimmed.contains("&&") || trimmed.contains(">>") {
        return true;
    }
    // First word is a common CLI executable
    let first_word = trimmed.split_whitespace().next().unwrap_or("");
    const SHELL_COMMANDS: &[&str] = &[
        "echo", "cat", "ls", "cd", "cp", "mv", "rm", "mkdir", "grep", "sed", "awk", "curl", "wget",
        "python", "python3", "node", "npm", "yarn", "cargo", "bash", "sh", "zsh", "git", "docker",
        "kubectl", "make", "env", "export", "source", "test", "find", "sort", "head", "tail", "wc",
        "tar", "zip", "unzip",
    ];
    SHELL_COMMANDS.contains(&first_word)
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
        assert!(!result.is_error);
        assert!(result.output().contains("No scheduled jobs"));
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
        assert!(!create.is_error);
        assert!(create.output().contains("Created recurring job"));

        let list = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(!list.is_error);
        assert!(list.output().contains("echo hello"));

        let create_output = create.output();
        let id = create_output.split_whitespace().nth(3).unwrap();

        let get = tool
            .execute(json!({"action": "get", "id": id}))
            .await
            .unwrap();
        assert!(!get.is_error);
        assert!(get.output().contains("echo hello"));

        let cancel = tool
            .execute(json!({"action": "cancel", "id": id}))
            .await
            .unwrap();
        assert!(!cancel.is_error);
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
        assert!(!once.is_error);

        let add = tool
            .execute(json!({
                "action": "add",
                "expression": "*/10 * * * *",
                "command": "echo recurring"
            }))
            .await
            .unwrap();
        assert!(!add.is_error);

        let add_output = add.output();
        let id = add_output.split_whitespace().nth(3).unwrap();
        let pause = tool
            .execute(json!({"action": "pause", "id": id}))
            .await
            .unwrap();
        assert!(!pause.is_error);

        let resume = tool
            .execute(json!({"action": "resume", "id": id}))
            .await
            .unwrap();
        assert!(!resume.is_error);
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
        assert!(blocked.is_error);
        assert!(blocked.output().contains("read-only"));

        let list = tool.execute(json!({"action": "list"})).await.unwrap();
        assert!(!list.is_error);
    }

    #[tokio::test]
    async fn unknown_action_returns_failure() {
        let (_tmp, config, security) = test_setup().await;
        let tool = ScheduleTool::new(security, config);

        let result = tool.execute(json!({"action": "explode"})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Unknown action"));
    }
}
