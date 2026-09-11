use super::*;
use crate::openhuman::config::Config;
use tempfile::TempDir;

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
    use crate::openhuman::cron::{Schedule, SessionTarget};

    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let job = cron::add_agent_job(
        &cfg,
        Some("inbox".into()),
        Schedule::Every { every_ms: 600_000 },
        "check my inbox",
        SessionTarget::Isolated,
        None,
        None,
        false,
    )
    .unwrap();
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
