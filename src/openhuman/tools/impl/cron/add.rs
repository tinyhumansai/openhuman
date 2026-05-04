use crate::openhuman::config::Config;
use crate::openhuman::cron::{self, DeliveryConfig, JobType, Schedule, SessionTarget};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Look up the configured `allowed_users` list for a channel by name.
/// Returns `None` if the channel is unknown or unconfigured. An empty
/// `Some(&[])` means the channel is configured but accepts any sender.
fn allowed_users_for_channel<'a>(config: &'a Config, channel: &str) -> Option<&'a [String]> {
    let ch = channel.trim().to_ascii_lowercase();
    let cc = &config.channels_config;
    match ch.as_str() {
        "telegram" => cc.telegram.as_ref().map(|c| c.allowed_users.as_slice()),
        "discord" => cc.discord.as_ref().map(|c| c.allowed_users.as_slice()),
        "slack" => cc.slack.as_ref().map(|c| c.allowed_users.as_slice()),
        "mattermost" => cc.mattermost.as_ref().map(|c| c.allowed_users.as_slice()),
        "matrix" => cc.matrix.as_ref().map(|c| c.allowed_users.as_slice()),
        "irc" => cc.irc.as_ref().map(|c| c.allowed_users.as_slice()),
        "lark" => cc.lark.as_ref().map(|c| c.allowed_users.as_slice()),
        "dingtalk" => cc.dingtalk.as_ref().map(|c| c.allowed_users.as_slice()),
        "qq" => cc.qq.as_ref().map(|c| c.allowed_users.as_slice()),
        _ => None,
    }
}

/// Validate a `DeliveryConfig` at cron-create time.
///
/// For `mode: "announce"` we require both `channel` and `to`, and we
/// reject `to` values that are not in the channel's configured
/// `allowed_users` list. This blocks an LLM (or RPC caller) from
/// scheduling a cron whose output gets sent to an arbitrary chat id —
/// see the "no cross-tenant `to`" acceptance criterion in #928.
///
/// `proactive` and `none` modes are not channel-targeted and are not
/// validated here.
fn validate_delivery(config: &Config, delivery: &DeliveryConfig) -> Result<(), String> {
    let mode = delivery.mode.trim().to_ascii_lowercase();
    if mode != "announce" {
        return Ok(());
    }

    let channel = delivery
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "delivery.channel is required for announce mode".to_string())?;
    let to = delivery
        .to
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "delivery.to is required for announce mode".to_string())?;

    // "web" announce is a degenerate case (web has no allowed_users
    // gate). Other unknown channels (e.g. "email") fall through to the
    // generic reject.
    if channel.eq_ignore_ascii_case("web") {
        return Ok(());
    }

    match allowed_users_for_channel(config, channel) {
        Some(list) if list.is_empty() => Ok(()),
        Some(list) => {
            if list.iter().any(|u| u == to) {
                Ok(())
            } else {
                Err(format!(
                    "delivery target '{to}' on channel '{channel}' is not in allowed_users \
                     for that channel; refusing to schedule cross-tenant delivery"
                ))
            }
        }
        None => Err(format!(
            "delivery channel '{channel}' is not configured; cannot validate target"
        )),
    }
}

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
    fn name(&self) -> &str {
        "cron_add"
    }

    fn description(&self) -> &str {
        "Create a scheduled cron job (shell or agent) with cron/at/every schedules. \
         Standardizes on device-local timezone unless 'tz' is set. The scheduler polls on an \
         interval (default 15s, minimum 5s) and does not 'catch up' missed runs.\n\
         Delivery: agent jobs default to `mode: \"proactive\"` which lands in the in-app/web \
         stream. When the current turn includes a `[Channel context]` block (e.g. Telegram, \
         Discord, Slack), set `delivery` to `{ \"mode\": \"announce\", \"channel\": <channel>, \
         \"to\": <reply target from the context block> }` so the reminder is delivered back to \
         the same chat instead of the desktop. Only use the default proactive mode when the \
         user explicitly asks for an in-app notification or when no channel context is present."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short human-readable name for the job (e.g. 'drink_water_reminder'). Always provide a name." },
                "schedule": {
                    "description": "Schedule: cron expression, one-shot at time, or fixed interval.",
                    "oneOf": [
                        {
                            "type": "object",
                            "description": "Repeating cron schedule. 'tz' is an IANA timezone (e.g. 'America/Los_Angeles'); defaults to device-local timezone.",
                            "properties": {
                                "kind": { "type": "string", "const": "cron" },
                                "expr": { "type": "string", "description": "Cron expression (5, 6, or 7 fields)" },
                                "tz": { "type": "string", "description": "Optional IANA timezone name" },
                                "active_hours": {
                                    "type": "object",
                                    "description": "Optional: only run during these local hours",
                                    "properties": {
                                        "start": { "type": "string", "description": "Start time HH:MM (e.g. '09:00')" },
                                        "end": { "type": "string", "description": "End time HH:MM (e.g. '17:00')" }
                                    },
                                    "required": ["start", "end"],
                                    "additionalProperties": false
                                }
                            },
                            "required": ["kind", "expr"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "description": "One-shot job that runs once at a specific UTC instant.",
                            "properties": {
                                "kind": { "type": "string", "const": "at" },
                                "at": { "type": "string", "description": "ISO-8601 UTC timestamp" }
                            },
                            "required": ["kind", "at"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "description": "Repeating job that fires every N milliseconds.",
                            "properties": {
                                "kind": { "type": "string", "const": "every" },
                                "every_ms": { "type": "integer", "description": "Interval in milliseconds (must be > 0)" }
                            },
                            "required": ["kind", "every_ms"],
                            "additionalProperties": false
                        }
                    ]
                },
                "job_type": { "type": "string", "enum": ["shell", "agent"] },
                "command": { "type": "string" },
                "prompt": { "type": "string" },
                "session_target": { "type": "string", "enum": ["isolated", "main"] },
                "model": { "type": "string" },
                "delivery": {
                    "type": "object",
                    "description": "Delivery config. Defaults to proactive (notifies user). Modes: proactive, announce (needs channel+to), none (silent).",
                    "properties": {
                        "mode": { "type": "string", "enum": ["proactive", "announce", "none"] },
                        "channel": { "type": "string", "description": "Required for announce mode" },
                        "to": { "type": "string", "description": "Required for announce mode" },
                        "best_effort": { "type": "boolean", "default": true }
                    }
                },
                "delete_after_run": { "type": "boolean" }
            },
            "required": ["name", "schedule"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
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

                if !self.security.is_command_allowed(command) {
                    return Ok(ToolResult::error(format!(
                        "Command blocked by security policy: {command}"
                    )));
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
                        Ok(target) => target,
                        Err(e) => {
                            return Ok(ToolResult::error(format!("Invalid session_target: {e}")));
                        }
                    },
                    None => SessionTarget::Isolated,
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
                    None => Some(DeliveryConfig {
                        mode: "proactive".to_string(),
                        channel: None,
                        to: None,
                        best_effort: true,
                    }),
                };

                if let Some(ref cfg) = delivery {
                    if let Err(msg) = validate_delivery(&self.config, cfg) {
                        return Ok(ToolResult::error(msg));
                    }
                }

                cron::add_agent_job(
                    &self.config,
                    name,
                    schedule,
                    prompt,
                    session_target,
                    model,
                    delivery,
                    delete_after_run,
                )
            }
        };

        match result {
            Ok(job) => Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                "id": job.id,
                "name": job.name,
                "job_type": job.job_type,
                "schedule": job.schedule,
                "next_run": job.next_run,
                "enabled": job.enabled
            }))?)),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use crate::openhuman::cron::ActiveHours;
    use crate::openhuman::security::AutonomyLevel;
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
    async fn adds_shell_job() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "schedule": { "kind": "cron", "expr": "*/5 * * * *" },
                "job_type": "shell",
                "command": "echo ok"
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "{:?}", result.output());
        assert!(result.output().contains("next_run"));
    }

    #[tokio::test]
    async fn adds_active_hours_shell_job_from_tool_payload() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "name": "work_hours_ping",
                "schedule": {
                    "kind": "cron",
                    "expr": "* * * * *",
                    "tz": "UTC",
                    "active_hours": {
                        "start": "09:00",
                        "end": "17:00"
                    }
                },
                "job_type": "shell",
                "command": "echo ok"
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "{:?}", result.output());
        let jobs = cron::list_jobs(&cfg).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].name.as_deref(), Some("work_hours_ping"));
        assert_eq!(
            jobs[0].schedule,
            Schedule::Cron {
                expr: "* * * * *".into(),
                tz: Some("UTC".into()),
                active_hours: Some(ActiveHours {
                    start: "09:00".into(),
                    end: "17:00".into(),
                }),
            }
        );
    }

    #[tokio::test]
    async fn blocks_disallowed_shell_command() {
        let tmp = TempDir::new().unwrap();
        let mut config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        config.autonomy.allowed_commands = vec!["echo".into()];
        config.autonomy.level = AutonomyLevel::Supervised;
        tokio::fs::create_dir_all(&config.workspace_dir)
            .await
            .unwrap();
        let cfg = Arc::new(config);
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));

        let result = tool
            .execute(json!({
                "schedule": { "kind": "cron", "expr": "*/5 * * * *" },
                "job_type": "shell",
                "command": "curl https://example.com"
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output().contains("blocked by security policy"));
    }

    #[tokio::test]
    async fn rejects_invalid_schedule() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));

        let result = tool
            .execute(json!({
                "schedule": { "kind": "every", "every_ms": 0 },
                "job_type": "shell",
                "command": "echo nope"
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output().contains("every_ms must be > 0"));
    }

    #[tokio::test]
    async fn agent_job_defaults_to_proactive_delivery() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "schedule": { "kind": "every", "every_ms": 300000 },
                "job_type": "agent",
                "prompt": "remind me to drink water"
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "{:?}", result.output());
        let jobs = cron::list_jobs(&cfg).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].delivery.mode, "proactive");
    }

    #[tokio::test]
    async fn agent_job_respects_explicit_none_delivery() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "schedule": { "kind": "every", "every_ms": 300000 },
                "job_type": "agent",
                "prompt": "silent background task",
                "delivery": { "mode": "none" }
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "{:?}", result.output());
        let jobs = cron::list_jobs(&cfg).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].delivery.mode, "none");
    }

    #[tokio::test]
    async fn agent_job_requires_prompt() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await;
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));

        let result = tool
            .execute(json!({
                "schedule": { "kind": "cron", "expr": "*/5 * * * *" },
                "job_type": "agent"
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Missing 'prompt'"));
    }

    // ── #928: announce-mode delivery validation ───────────────────

    use crate::openhuman::config::TelegramConfig;

    fn cfg_with_telegram(tmp: &TempDir, allowed: Vec<String>) -> Arc<Config> {
        let mut config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        config.channels_config.telegram = Some(TelegramConfig {
            bot_token: "test-token".into(),
            allowed_users: allowed,
            stream_mode: Default::default(),
            draft_update_interval_ms: 1000,
            silent_streaming: true,
            mention_only: false,
        });
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        Arc::new(config)
    }

    #[tokio::test]
    async fn agent_job_announce_telegram_authorized_chat_succeeds() {
        let tmp = TempDir::new().unwrap();
        let cfg = cfg_with_telegram(&tmp, vec!["123456".into()]);
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "schedule": { "kind": "every", "every_ms": 300000 },
                "job_type": "agent",
                "prompt": "remind me to drink water",
                "delivery": {
                    "mode": "announce",
                    "channel": "telegram",
                    "to": "123456"
                }
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "{:?}", result.output());
        let jobs = cron::list_jobs(&cfg).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].delivery.mode, "announce");
        assert_eq!(jobs[0].delivery.channel.as_deref(), Some("telegram"));
        assert_eq!(jobs[0].delivery.to.as_deref(), Some("123456"));
    }

    #[tokio::test]
    async fn agent_job_announce_telegram_open_bot_allows_any_chat() {
        // Empty allowed_users == "any sender ok". Mirrors the existing
        // channel runtime behavior: an open bot accepts cron targets too.
        let tmp = TempDir::new().unwrap();
        let cfg = cfg_with_telegram(&tmp, vec![]);
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "schedule": { "kind": "every", "every_ms": 300000 },
                "job_type": "agent",
                "prompt": "ping",
                "delivery": {
                    "mode": "announce",
                    "channel": "telegram",
                    "to": "999"
                }
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "{:?}", result.output());
    }

    #[tokio::test]
    async fn agent_job_announce_telegram_unauthorized_chat_rejected() {
        let tmp = TempDir::new().unwrap();
        let cfg = cfg_with_telegram(&tmp, vec!["alice".into()]);
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "schedule": { "kind": "every", "every_ms": 300000 },
                "job_type": "agent",
                "prompt": "ping",
                "delivery": {
                    "mode": "announce",
                    "channel": "telegram",
                    "to": "mallory"
                }
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output().contains("not in allowed_users"));
        // Job must not be persisted on rejection.
        assert!(cron::list_jobs(&cfg).unwrap().is_empty());
    }

    #[tokio::test]
    async fn agent_job_announce_unconfigured_channel_rejected() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp).await; // no telegram block
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "schedule": { "kind": "every", "every_ms": 300000 },
                "job_type": "agent",
                "prompt": "ping",
                "delivery": {
                    "mode": "announce",
                    "channel": "telegram",
                    "to": "123"
                }
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output().contains("not configured"));
    }

    #[tokio::test]
    async fn agent_job_announce_missing_target_rejected() {
        let tmp = TempDir::new().unwrap();
        let cfg = cfg_with_telegram(&tmp, vec![]);
        let tool = CronAddTool::new(cfg.clone(), test_security(&cfg));
        let result = tool
            .execute(json!({
                "schedule": { "kind": "every", "every_ms": 300000 },
                "job_type": "agent",
                "prompt": "ping",
                "delivery": {
                    "mode": "announce",
                    "channel": "telegram"
                }
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output().contains("delivery.to is required"));
    }

    #[test]
    fn validate_delivery_skips_proactive_and_none_modes() {
        let tmp = TempDir::new().unwrap();
        let cfg = cfg_with_telegram(&tmp, vec!["alice".into()]);

        let proactive = DeliveryConfig {
            mode: "proactive".into(),
            channel: None,
            to: None,
            best_effort: true,
        };
        assert!(validate_delivery(&cfg, &proactive).is_ok());

        let none = DeliveryConfig {
            mode: "none".into(),
            channel: None,
            to: None,
            best_effort: true,
        };
        assert!(validate_delivery(&cfg, &none).is_ok());
    }

    #[test]
    fn validate_delivery_announce_web_is_a_no_op() {
        // "web" doesn't have an allowed_users gate; announce to web is
        // a degenerate but valid configuration (in-app explicit).
        let tmp = TempDir::new().unwrap();
        let cfg = test_config_sync(&tmp);
        let cfg_unused = DeliveryConfig {
            mode: "announce".into(),
            channel: Some("web".into()),
            to: Some("any".into()),
            best_effort: true,
        };
        assert!(validate_delivery(&cfg, &cfg_unused).is_ok());
    }

    fn test_config_sync(tmp: &TempDir) -> Arc<Config> {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        Arc::new(config)
    }
}
