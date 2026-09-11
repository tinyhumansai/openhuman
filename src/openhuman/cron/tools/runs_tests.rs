use super::*;
use crate::openhuman::config::Config;
use chrono::{Duration as ChronoDuration, Utc};
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
async fn lists_runs_with_truncation() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let job = cron::add_job(&cfg, "*/5 * * * *", "echo ok").unwrap();

    let long_output = "x".repeat(1000);
    let now = Utc::now();
    cron::record_run(
        &cfg,
        &job.id,
        now,
        now + ChronoDuration::milliseconds(1),
        "ok",
        Some(&long_output),
        1,
    )
    .unwrap();

    let tool = CronRunsTool::new(cfg.clone());
    let result = tool
        .execute(json!({ "job_id": job.id, "limit": 5 }))
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.output().contains("..."));
}

#[tokio::test]
async fn errors_when_job_id_missing() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let tool = CronRunsTool::new(cfg);
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'job_id'"));
}
