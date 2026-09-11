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
