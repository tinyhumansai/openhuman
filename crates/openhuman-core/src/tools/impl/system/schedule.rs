use crate::config::Config;
use crate::cron::job_builder::{create_agent_job, AgentJobInput};
use crate::cron::origin::{current_job_origin, current_turn_may_skip_approval};
use crate::security::SecurityPolicy;
use crate::tools::schema_cache::static_schema;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool, ToolResult};
use {crate::cron, tinyflows_schedule::types::Schedule};

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
        "Manage scheduled tasks. Actions: create/add/once/list/get/cancel/remove/pause/resume. \
         A task with a `prompt` created inside a conversation replies back into that same \
         conversation automatically on every run; no delivery target is needed. In the \
         scheduled run your final reply is what gets delivered (`NO_REPLY` skips it)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        static_schema!(include_str!("parameters/schedule.json"))
    }

    fn permission_level(&self) -> PermissionLevel {
        // Minimum level across all actions — list/get need only ReadOnly.
        // Mutating actions enforce Execute via permission_level_with_args.
        PermissionLevel::ReadOnly
    }

    fn permission_level_with_args(&self, args: &serde_json::Value) -> PermissionLevel {
        match args.get("action").and_then(|v| v.as_str()) {
            Some("list") | Some("get") => PermissionLevel::ReadOnly,
            _ => PermissionLevel::Execute,
        }
    }

    fn external_effect_with_args(&self, args: &serde_json::Value) -> bool {
        // Read-only actions (list/get) need no approval; all mutating actions
        // (create/add/once/cancel/remove/pause/resume) persist or remove a
        // scheduled job and must go through the ApprovalGate
        // (GHSA-f46p-6vf9-64mm).
        //
        // Exception: an agent task (`prompt`, no shell `command`) created from a
        // channel conversation replies only into that conversation and runs as
        // that channel's turn, so it skips the gate (see `cron::origin`).
        match args.get("action").and_then(|v| v.as_str()) {
            Some("list") | Some("get") => false,
            Some("create") | Some("add") | Some("once") => {
                !current_turn_may_skip_approval(&schedule_args_as_cron_add(args))
            }
            _ => true,
        }
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
                self.handle_create_like(action, &args).await
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

/// The `cron` add-shaped view of a `schedule` create call, for the shared
/// approval-exemption check: an agent task has a `prompt` and no real shell
/// `command`; anything else is treated as a shell job.
fn schedule_args_as_cron_add(args: &serde_json::Value) -> serde_json::Value {
    let non_empty = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    match (non_empty("command"), non_empty("prompt")) {
        // `execute` re-reads a non-shell-looking `command` as the prompt.
        (Some(cmd), None) if !looks_like_shell_command(&cmd) => {
            json!({ "job_type": "agent", "prompt": cmd })
        }
        (None, Some(prompt)) => json!({ "job_type": "agent", "prompt": prompt }),
        _ => json!({ "job_type": "shell" }),
    }
}

impl ScheduleTool {
    fn enforce_mutation_allowed(&self, action: &str) -> Option<ToolResult> {
        if !self.security.can_act() {
            return Some(ToolResult::error(format!(
                "[policy-blocked] Security policy: read-only mode, cannot perform '{action}'"
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

    async fn handle_create_like(
        &self,
        action: &str,
        args: &serde_json::Value,
    ) -> Result<ToolResult> {
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
            tracing::debug!(
                action = %action,
                prompt_len = prompt_text.len(),
                "[schedule] creating agent job"
            );
            let schedule = if let Some(expr) = expression {
                Schedule::Cron {
                    expr: expr.to_string(),
                    tz: None,
                    active_hours: None,
                }
            } else if let Some(delay_str) = delay {
                let at = Utc::now() + tinyflows_schedule::validation::parse_human_delay(delay_str)?;
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
            let job = create_agent_job(
                &self.config,
                AgentJobInput {
                    name,
                    schedule,
                    prompt: prompt_text.to_string(),
                    session_target: None,
                    model: None,
                    delivery: None,
                    delete_after_run,
                },
                current_job_origin(),
            )
            .map_err(anyhow::Error::msg)?;

            let job_name = job.name.as_deref().unwrap_or("(unnamed)");
            tracing::debug!(
                job_id = %job.id,
                job_name = %job_name,
                next_run = %job.next_run,
                "[schedule] agent job created"
            );
            return Ok(ToolResult::success(format!(
                "Created agent job '{}' (id: {}, next: {})",
                job_name, job.id, job.next_run,
            )));
        }

        // ── Shell job (command provided) ─────────────────────────────
        let command = command.unwrap();
        if let Err(reason) =
            crate::cron::scheduler::shell_job::authorize_shell_command(&self.security, command)
                .await
        {
            return Ok(ToolResult::error(reason));
        }

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
    // Exclude ambiguous words (test, find, make, source, head, sort) that
    // are common English verbs and would misclassify natural-language prompts.
    const SHELL_COMMANDS: &[&str] = &[
        "echo", "cat", "ls", "cd", "cp", "mv", "rm", "mkdir", "grep", "sed", "awk", "curl", "wget",
        "python", "python3", "node", "npm", "yarn", "cargo", "bash", "sh", "zsh", "git", "docker",
        "kubectl", "env", "export", "tail", "wc", "tar", "zip", "unzip",
    ];
    SHELL_COMMANDS.contains(&first_word)
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "schedule_schema_tests.rs"]
mod schema_tests;
