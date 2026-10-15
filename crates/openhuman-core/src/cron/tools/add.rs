use crate::config::Config;
use crate::cron::job_builder::{create_agent_job, AgentJobInput};
use crate::cron::origin::{current_job_origin, current_turn_may_skip_approval};
use crate::security::SecurityPolicy;
use crate::tools::schema_cache::static_schema;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolExposure, ToolResult};
use {
    crate::cron, tinyflows_schedule::types::DeliveryConfig, tinyflows_schedule::types::JobType,
    tinyflows_schedule::types::Schedule, tinyflows_schedule::types::SessionTarget,
};

pub struct CronAddTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
}

impl CronAddTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>) -> Self {
        Self { config, security }
    }
}

#[async_trait]
impl Tool for CronAddTool {
    /// Superseded by the `cron` tool, which dispatches every scheduler
    /// operation on one `action` field. Kept registered and dispatchable so a
    /// replayed transcript or a saved skill naming `cron_*` keeps working;
    /// hidden from the wire so six schemas do not ship where one does.
    fn exposure(&self) -> ToolExposure {
        ToolExposure::Hidden
    }

    fn name(&self) -> &str {
        "cron_add"
    }

    fn description(&self) -> &str {
        "Create a scheduled cron job (shell or agent) with cron/at/every schedules. \
         Standardizes on device-local timezone unless 'tz' is set. The scheduler polls on an \
         interval (default 15s, minimum 5s) and does not 'catch up' missed runs.\n\
         Delivery: an agent job created inside a conversation (web chat or a channel such as \
         Telegram, Discord, Slack) sends each run's reply back to that same conversation \
         automatically; do not set `delivery` or copy a reply target for that. Only set \
         `delivery` to override it (`proactive` for an in-app notification, `none` for silent). \
         A job created with no conversation defaults to `proactive`. In its scheduled run the \
         agent's final reply is what gets delivered; `NO_REPLY` skips the delivery.\n\
         Agent jobs must be scheduled at least 5 minutes apart; a tighter cron expression or \
         every_ms is rejected."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        static_schema!(include_str!("parameters/cron_add.json"))
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    fn permission_level(&self) -> PermissionLevel {
        // Scheduling a job persists a command or agent prompt that will
        // execute on the host.  Treat it as Execute so channel-level
        // permission caps are honoured and the approval gate is consulted.
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        // Creating a cron job is a durable, persistent side-effect: the
        // scheduler will later run the stored command or agent prompt on the
        // host.  Marking this true ensures ApprovalGate::intercept is called
        // before the job is written to disk, even when the turn originated
        // from an inbound channel message (GHSA-f46p-6vf9-64mm).
        true
    }

    fn external_effect_with_args(&self, args: &serde_json::Value) -> bool {
        // An agent job that replies only into the channel conversation that
        // asked for it needs no approval: the asker is the sole recipient and
        // its runs stay gated as that channel's turns. Everything else keeps
        // the unconditional gate above.
        !current_turn_may_skip_approval(args)
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

        let schedule = match args.get("schedule") {
            Some(v) => match serde_json::from_value::<Schedule>(v.clone()) {
                Ok(schedule) => schedule,
                Err(e) => {
                    return Ok(ToolResult::error(format!("Invalid schedule: {e}")));
                }
            },
            None => {
                return Ok(ToolResult::error(
                    "Missing 'schedule' parameter".to_string(),
                ));
            }
        };

        let name = args
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                // Derive a name from the prompt so cron jobs are never unnamed.
                args.get("prompt")
                    .and_then(serde_json::Value::as_str)
                    .map(|p| {
                        let slug: String = p
                            .chars()
                            .map(|c| {
                                if c.is_alphanumeric() {
                                    c.to_ascii_lowercase()
                                } else {
                                    '_'
                                }
                            })
                            .take(48)
                            .collect();
                        slug.trim_matches('_').to_string()
                    })
                    .filter(|s| !s.is_empty())
            });

        let job_type = match args.get("job_type").and_then(serde_json::Value::as_str) {
            Some("agent") => JobType::Agent,
            Some("shell") => JobType::Shell,
            Some(other) => {
                return Ok(ToolResult::error(format!("Invalid job_type: {other}")));
            }
            None => {
                if args.get("prompt").is_some() {
                    JobType::Agent
                } else {
                    JobType::Shell
                }
            }
        };

        let default_delete_after_run = matches!(schedule, Schedule::At { .. });
        let delete_after_run = args
            .get("delete_after_run")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(default_delete_after_run);

        let result = match job_type {
            JobType::Shell => {
                let command = match args.get("command").and_then(serde_json::Value::as_str) {
                    Some(command) if !command.trim().is_empty() => command,
                    _ => {
                        return Ok(ToolResult::error(
                            "Missing 'command' for shell job".to_string(),
                        ));
                    }
                };

                if let Err(reason) = super::super::scheduler::shell_job::authorize_shell_command(
                    &self.security,
                    command,
                )
                .await
                {
                    return Ok(ToolResult::error(reason));
                }

                cron::add_shell_job(&self.config, name, schedule, command)
            }
            JobType::Agent => {
                let prompt = match args.get("prompt").and_then(serde_json::Value::as_str) {
                    Some(prompt) if !prompt.trim().is_empty() => prompt,
                    _ => {
                        return Ok(ToolResult::error(
                            "Missing 'prompt' for agent job".to_string(),
                        ));
                    }
                };

                let session_target = match args.get("session_target") {
                    Some(v) => match serde_json::from_value::<SessionTarget>(v.clone()) {
                        Ok(target) => Some(target),
                        Err(e) => {
                            return Ok(ToolResult::error(format!("Invalid session_target: {e}")));
                        }
                    },
                    None => None,
                };

                let model = args
                    .get("model")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);

                let delivery = match args.get("delivery") {
                    Some(v) => match serde_json::from_value::<DeliveryConfig>(v.clone()) {
                        Ok(cfg) => Some(cfg),
                        Err(e) => {
                            return Ok(ToolResult::error(format!("Invalid delivery config: {e}")));
                        }
                    },
                    None => None,
                };

                create_agent_job(
                    &self.config,
                    AgentJobInput {
                        name,
                        schedule,
                        prompt: prompt.to_string(),
                        session_target,
                        model,
                        delivery,
                        delete_after_run,
                    },
                    current_job_origin(),
                )
                .map_err(anyhow::Error::msg)
            }
            // `job_type` above is derived only from `Some("agent")`/`Some("shell")`/
            // the `prompt`-presence heuristic, so this arm is unreachable in
            // practice — `JobType::Flow` rows are created internally by
            // `flows::ops::flows_set_enabled` (via `cron::add_flow_schedule_job`),
            // never through this agent-facing tool. Kept as an explicit error
            // (not `unreachable!()`) so a future change to the heuristic above
            // fails loudly with a clear message instead of panicking.
            JobType::Flow => Err(anyhow::anyhow!(
                "flow-type cron jobs are managed by the Workflows feature and cannot be \
                 created via cron_add"
            )),
        };

        match result {
            Ok(job) => {
                let payload = json!({
                    "id": job.id,
                    "name": job.name,
                    "job_type": job.job_type,
                    "schedule": job.schedule,
                    "next_run": job.next_run,
                    "enabled": job.enabled,
                    "session_target": job.session_target,
                    "delivery_mode": job.delivery.mode
                });
                let mut tr = ToolResult::success(serde_json::to_string_pretty(&payload)?);
                if options.prefer_markdown {
                    let md = format!(
                        "Created cron job **{}** (`{}`).\n- **next_run**: {}\n- **enabled**: {}",
                        job.name.as_deref().unwrap_or(&job.id),
                        job.id,
                        job.next_run.format("%Y-%m-%d %H:%M:%S UTC"),
                        job.enabled,
                    );
                    tr.markdown_formatted = Some(md);
                }
                Ok(tr)
            }
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

#[cfg(test)]
#[path = "add_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "add_schema_tests.rs"]
mod schema_tests;
