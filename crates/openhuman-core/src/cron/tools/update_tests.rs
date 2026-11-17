use super::*;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::config::Config;
use crate::core::runtime::turn_origin::AgentTurnOrigin;
use tempfile::TempDir;
use tinyagents_harness::{context::RunConfig, ids::CallId, tool::ToolExecutionContext};
use tinyflows_bus::schedule::types::{DeliveryConfig, JobOrigin, Schedule, SessionTarget};
use tinyflows_bus::schedule::AgentJobSpec;

/// The trusted durable-session identity a channel turn carries. Jobs created
/// from a channel conversation record it; a turn without one has no
/// conversation to bind a job to.
fn channel_session(reply_target: &str) -> tinyflows_bus::schedule::ChannelSessionIdentity {
    tinyflows_bus::schedule::ChannelSessionIdentity {
        tenant_key: "profile:alice".into(),
        agent_id: "orchestrator".into(),
        session_key: format!("main:telegram:bot-a:dm:{reply_target}"),
        provider: "telegram".into(),
        account_id: Some("telegram:bot-a".into()),
        conversation: tinyflows_bus::schedule::ChannelConversationIdentity {
            kind: "dm".into(),
            id: reply_target.into(),
            scope_id: None,
            parent_id: None,
            thread_id: None,
            topic_id: None,
        },
    }
}

fn channel_origin(reply_target: &str) -> JobOrigin {
    JobOrigin::Channel {
        channel: "telegram".into(),
        reply_target: reply_target.into(),
        history_key: format!("telegram-user-{reply_target}"),
        sender: Some("alice".into()),
        thread_id: None,
        channel_session: Some(Box::new(channel_session(reply_target))),
    }
}

fn trusted_channel_context(reply_target: &str) -> (impl Sized, ToolExecutionContext) {
    let mut run_context = OpenHumanRunContext::new();
    run_context.origin = Some(AgentTurnOrigin::ExternalChannel {
        sender_name: None,
        channel: "telegram".into(),
        sender: Some("alice".into()),
        reply_target: reply_target.into(),
        message_id: "message-1".into(),
        history_key: Some(format!("telegram-user-{reply_target}")),
        channel_session: Some(channel_session(reply_target)),
    });
    let run = run_context.into_tinyagents(RunConfig::new("cron-update-context"));
    let tool_context =
        ToolExecutionContext::from_run_context(&run, CallId::new("cron-update-call"));
    (run, tool_context)
}

fn authorized_channel_config(tmp: &TempDir) -> Arc<Config> {
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    config.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: "test-token".into(),
        chat_id: None,
        allowed_users: vec!["alice".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    Arc::new(config)
}

fn channel_agent_job(
    config: &Config,
    origin: JobOrigin,
) -> tinyflows_bus::schedule::types::CronJob {
    let mut spec = AgentJobSpec::new(Schedule::Every { every_ms: 300_000 }, "check inbox");
    spec.session_target = SessionTarget::Isolated;
    spec.delivery = Some(DeliveryConfig::default());
    spec.origin = Some(origin);
    cron::add_agent_job_from_spec(config, spec).unwrap()
}

async fn test_config(tmp: &TempDir) -> Arc<Config> {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
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
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    // The allowlist only exists when the policy is on, and the shipped config
    // default is off (`[autonomy] enabled = false`).
    config.autonomy.enabled = true;
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

// ── GHSA-f46p-6vf9-64mm: approval gate must fire for cron_update ─

#[test]
fn cron_update_is_external_effect() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let cfg = Arc::new(config);
    let tool = CronUpdateTool::new(cfg.clone(), test_security(&cfg));
    assert!(
        tool.external_effect(),
        "cron_update must declare external_effect=true so ApprovalGate is consulted"
    );
}

#[test]
fn cron_update_permission_level_is_execute() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let cfg = Arc::new(config);
    let tool = CronUpdateTool::new(cfg.clone(), test_security(&cfg));
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
}

#[tokio::test]
async fn rejects_tightening_an_agent_job_below_five_minutes() {
    use {tinyflows_bus::schedule::types::Schedule, tinyflows_bus::schedule::types::SessionTarget};

    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let mut spec = AgentJobSpec::new(Schedule::Every { every_ms: 600_000 }, "check my inbox");
    spec.name = Some("inbox".into());
    spec.session_target = SessionTarget::Isolated;
    let job = cron::add_agent_job_from_spec(&cfg, spec).unwrap();
    let tool = CronUpdateTool::new(cfg.clone(), test_security(&cfg));

    let result = tool
        .execute(json!({
            "job_id": job.id,
            "patch": { "schedule": { "kind": "every", "every_ms": 60_000 } }
        }))
        .await
        .unwrap();
    assert!(result.is_error, "{:?}", result.output());
    assert!(
        result
            .output()
            .contains("agent jobs must run at least 5 minutes apart"),
        "{:?}",
        result.output()
    );
    // A rejected patch must not have been written before the error came back.
    assert_eq!(
        cron::get_job(&cfg, &job.id).unwrap().schedule,
        Schedule::Every { every_ms: 600_000 }
    );

    let result = tool
        .execute(json!({
            "job_id": job.id,
            "patch": { "schedule": { "kind": "every", "every_ms": 300_000 } }
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{:?}", result.output());
}

#[tokio::test]
async fn trusted_current_channel_can_update_delivery_to_its_own_target() {
    let tmp = TempDir::new().unwrap();
    let cfg = authorized_channel_config(&tmp);
    let job = channel_agent_job(&cfg, channel_origin("42"));
    let (_run, context) = trusted_channel_context("42");
    let tool = CronUpdateTool::new(cfg.clone(), test_security(&cfg));

    let result = tool
        .execute_with_context(
            json!({
                "job_id": job.id,
                "patch": {"delivery":{"mode":"origin"}}
            }),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let saved = cron::get_job(&cfg, &job.id).unwrap();
    assert_eq!(saved.delivery.mode, "origin");
    assert_eq!(saved.origin, Some(channel_origin("42")));
}

#[tokio::test]
async fn trusted_current_channel_cannot_authorize_spoofed_origin_update_target() {
    let tmp = TempDir::new().unwrap();
    let cfg = authorized_channel_config(&tmp);
    let original_origin = channel_origin("42");
    let job = channel_agent_job(&cfg, original_origin.clone());
    let (_run, context) = trusted_channel_context("42");
    let tool = CronUpdateTool::new(cfg.clone(), test_security(&cfg));

    let result = tool
        .execute_with_context(
            json!({
                "job_id": job.id,
                "patch": {
                    "delivery":{"mode":"origin"},
                    "origin": channel_origin("999")
                }
            }),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();

    assert!(result.is_error, "{}", result.output());
    assert!(result.output().contains("allowed"), "{}", result.output());
    let saved = cron::get_job(&cfg, &job.id).unwrap();
    assert_eq!(saved.origin, Some(original_origin));
    assert_eq!(saved.delivery.mode, "none");
}

#[tokio::test]
async fn clearing_origin_and_selecting_origin_delivery_is_rejected_without_persisting() {
    let tmp = TempDir::new().unwrap();
    let cfg = authorized_channel_config(&tmp);
    let original_origin = channel_origin("42");
    let job = channel_agent_job(&cfg, original_origin.clone());
    let tool = CronUpdateTool::new(cfg.clone(), test_security(&cfg));

    let result = tool
        .execute(json!({
            "job_id": job.id,
            "patch": {"delivery":{"mode":"origin"}, "origin":null}
        }))
        .await
        .unwrap();

    assert!(result.is_error, "{}", result.output());
    assert!(result.output().contains("origin"), "{}", result.output());
    let saved = cron::get_job(&cfg, &job.id).unwrap();
    assert_eq!(saved.delivery.mode, "none");
    assert_eq!(saved.origin, Some(original_origin));
}
