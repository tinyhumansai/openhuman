use super::*;
use crate::config::Config;
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

    let runs = cron::list_runs(&cfg, &job.id, 10).unwrap();
    if let Some(run) = runs.first() {
        assert_eq!(run.status, if result.is_error { "error" } else { "ok" });
        assert_eq!(run.delivery_status, None);
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

#[tokio::test]
async fn cron_run_tool_pairs_start_and_completion_without_requesting_delivery() {
    use crate::core::events::DomainEvent;
    use tinybus::EventHandler;
    use tokio::sync::mpsc;

    struct Events(String, mpsc::UnboundedSender<&'static str>);
    #[async_trait::async_trait]
    impl EventHandler<DomainEvent> for Events {
        fn name(&self) -> &str {
            "test::cron::run_tool_event_pair"
        }
        async fn handle(&self, event: &DomainEvent) {
            let (job_id, label) = match event {
                DomainEvent::CronJobTriggered { job_id, .. } => (job_id, "triggered"),
                DomainEvent::CronJobCompleted { job_id, .. } => (job_id, "completed"),
                _ => return,
            };
            if job_id == &self.0 {
                let _ = self.1.send(label);
            }
        }
    }

    crate::core::bus::init().await.unwrap();
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp).await;
    let job = crate::cron::system_jobs::ensure_system_job(
        &cfg,
        "cron-run-tool-event-pair",
        tinyflows_bus::schedule::types::Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    let _handler = crate::cron::system_job_handlers::register(
        "cron-run-tool-event-pair",
        Arc::new(|_| Box::pin(async { Ok(()) })),
    );
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _subscription = crate::core::bus::BUS
        .subscribe(Arc::new(Events(job.id.clone(), tx)))
        .unwrap();

    let result = CronRunTool::new(cfg.clone())
        .execute(json!({ "job_id": job.id }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let pair = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut seen = Vec::new();
        loop {
            let event = rx.recv().await.expect("cron event subscriber remains open");
            seen.push(event);
            if event == "completed" {
                break;
            }
        }
        seen
    })
    .await
    .expect("cron_run completion event arrives");
    assert_eq!(pair, vec!["triggered", "completed"]);
    let runs = cron::list_runs(&cfg, &job.id, 10).unwrap();
    assert_eq!(runs[0].delivery_status, None);
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
