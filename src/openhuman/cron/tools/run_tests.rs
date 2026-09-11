use super::*;
use crate::openhuman::config::Config;
use tempfile::TempDir;

async fn test_config(tmp: &TempDir) -> Arc<Config> {
    let ws = tmp.path().join("workspace");
    let config = Config {
        workspace_dir: ws.clone(),
        action_dir: ws.clone(),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    tokio::fs::create_dir_all(&config.workspace_dir)
        .await
        .unwrap();
    Arc::new(config)
}

#[tokio::test]
async fn force_runs_job_and_records_history() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let job = cron::add_job(&cfg, "*/5 * * * *", "echo run-now").unwrap();
    let tool = CronRunTool::new(cfg.clone());

    let result = tool.execute(json!({ "job_id": job.id })).await.unwrap();
    if cfg!(windows) {
        // Windows is platform-dependent for `echo`: cmd.exe treats it
        // as a shell built-in (no standalone executable), but a dev
        // box with Git Bash on PATH exposes a real `echo.exe` that
        // succeeds. Both outcomes are valid; assert only that we
        // get a deterministic ToolResult and that the runs ledger
        // matches the success/failure decision.
        if result.is_error {
            assert!(
                result.output().contains("spawn error"),
                "expected spawn-error explanation on Windows failure path: {:?}",
                result.output()
            );
            let runs = cron::list_runs(&cfg, &job.id, 10).unwrap();
            assert_eq!(runs.len(), 0, "spawn failure must not persist a run");
        } else {
            let runs = cron::list_runs(&cfg, &job.id, 10).unwrap();
            assert_eq!(
                runs.len(),
                1,
                "successful run must persist exactly one entry"
            );
        }
    } else {
        assert!(!result.is_error, "{:?}", result.output());
        let runs = cron::list_runs(&cfg, &job.id, 10).unwrap();
        assert_eq!(runs.len(), 1);
    }
}

#[tokio::test]
async fn errors_for_missing_job() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let tool = CronRunTool::new(cfg);

    let result = tool
        .execute(json!({ "job_id": "missing-job-id" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not found"));
}

// ── GHSA-f46p-6vf9-64mm: approval gate must fire for cron_run ────

#[test]
fn cron_run_is_external_effect() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let cfg = Arc::new(config);
    let tool = CronRunTool::new(cfg);
    assert!(
        tool.external_effect(),
        "cron_run must declare external_effect=true so ApprovalGate is consulted"
    );
}

#[test]
fn cron_run_permission_level_is_execute() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let cfg = Arc::new(config);
    let tool = CronRunTool::new(cfg);
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
}
