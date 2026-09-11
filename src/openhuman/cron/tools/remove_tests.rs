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

#[tokio::test]
async fn removes_existing_job() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let job = cron::add_job(&cfg, "*/5 * * * *", "echo ok").unwrap();
    let tool = CronRemoveTool::new(cfg.clone());

    let result = tool.execute(json!({"job_id": job.id})).await.unwrap();
    assert!(!result.is_error);
    assert!(cron::list_jobs(&cfg).unwrap().is_empty());
}

#[tokio::test]
async fn errors_when_job_id_missing() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let tool = CronRemoveTool::new(cfg);

    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'job_id'"));
}

// ── GHSA-f46p-6vf9-64mm: approval gate must fire for cron_remove ─

#[test]
fn cron_remove_is_external_effect() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let cfg = Arc::new(config);
    let tool = CronRemoveTool::new(cfg);
    assert!(
        tool.external_effect(),
        "cron_remove must declare external_effect=true so ApprovalGate is consulted"
    );
}

#[test]
fn cron_remove_permission_level_is_write() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let cfg = Arc::new(config);
    let tool = CronRemoveTool::new(cfg);
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}
