use super::*;

#[cfg(feature = "flows")]
#[tokio::test]
async fn flows_boot_reconcile_uses_the_runtime_captured_config() {
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join("not-a-workspace-directory");
    std::fs::write(&blocker, b"not a directory").unwrap();
    let config = Config {
        workspace_dir: blocker.clone(),
        action_dir: blocker.clone(),
        config_path: tmp.path().join("runtime-config.toml"),
        ..Config::default()
    };
    crate::storage::local::use_persistent_default_for_test(&config.workspace_dir);
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let dispatch = crate::core::runtime::DispatchCtx::new(core);

    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("install test tracing subscriber");

    let swept = reconcile_flows_on_boot(&dispatch)
        .await
        .expect("captured runtime configuration loads");
    assert_eq!(swept, 0);
    let warning = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(
        warning.contains(&blocker.display().to_string()),
        "BootSweep must use the workspace captured by this runtime; got {warning:?}"
    );
}

/// desktop() must enable every bootstrap job.
#[test]
fn desktop_plan_enables_every_job() {
    let plan = bootstrap_job_plan(&ServiceSet::desktop());
    assert_eq!(
        plan,
        BootstrapJobPlan {
            memory_jobs: true,
            task_source_pollers: true,
            module_preload: true,
        }
    );
}

/// none() / headless_api() run no bootstrap job at all.
#[test]
fn job_free_presets_enable_nothing() {
    let empty = BootstrapJobPlan {
        memory_jobs: false,
        task_source_pollers: false,
        module_preload: false,
    };
    assert_eq!(bootstrap_job_plan(&ServiceSet::none()), empty);
    assert_eq!(bootstrap_job_plan(&ServiceSet::headless_api()), empty);
}

/// From none(), flipping exactly one concern flag enables exactly its job.
#[test]
fn each_concern_flag_enables_exactly_its_job() {
    let mut integrations = ServiceSet::none();
    integrations.integrations = true;
    let plan = bootstrap_job_plan(&integrations);
    assert!(plan.module_preload);
    assert!(!plan.memory_jobs);
    assert!(!plan.task_source_pollers);

    let mut memory_sync = ServiceSet::none();
    memory_sync.memory_sync = true;
    let plan = bootstrap_job_plan(&memory_sync);
    assert!(plan.memory_jobs);
    assert!(!plan.module_preload);
    assert!(!plan.task_source_pollers);
}

/// From desktop(), disabling exactly one concern flag disables only its job.
#[test]
fn disabling_one_concern_disables_only_its_job() {
    let mut services = ServiceSet::desktop();
    services.integrations = false;
    let plan = bootstrap_job_plan(&services);
    assert!(!plan.module_preload);
    assert!(plan.memory_jobs);
    assert!(plan.task_source_pollers);

    let mut services = ServiceSet::desktop();
    services.memory_sync = false;
    let plan = bootstrap_job_plan(&services);
    assert!(!plan.memory_jobs);
    assert!(plan.module_preload);
}

/// `channels` gates NO bootstrap job.
#[test]
fn channels_flag_gates_no_bootstrap_job() {
    let mut channels_only = ServiceSet::none();
    channels_only.channels = true;
    assert_eq!(
        bootstrap_job_plan(&channels_only),
        bootstrap_job_plan(&ServiceSet::none()),
        "channels alone must enable no bootstrap job"
    );

    let mut channels_off = ServiceSet::desktop();
    channels_off.channels = false;
    assert_eq!(
        bootstrap_job_plan(&channels_off),
        bootstrap_job_plan(&ServiceSet::desktop()),
        "dropping channels must not drop any bootstrap job"
    );
}

// ── ServiceTasks: background services a runtime can stop ─────────

#[tokio::test]
async fn stopping_service_tasks_aborts_every_tracked_task() {
    let tasks = ServiceTasks::default();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _keep = tx;
        std::future::pending::<()>().await;
    });
    tasks.track("pending", handle);
    assert_eq!(tasks.len(), 1);
    tasks.stop();
    assert_eq!(tasks.len(), 0);
    // The sender drops when the aborted task is torn down.
    assert!(tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .expect("the aborted task is torn down")
        .is_err());
}

#[tokio::test]
async fn dropping_service_tasks_aborts_them() {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    {
        let tasks = ServiceTasks::default();
        tasks.track(
            "pending",
            tokio::spawn(async move {
                let _keep = tx;
                std::future::pending::<()>().await;
            }),
        );
    }
    assert!(tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .expect("the aborted task is torn down")
        .is_err());
}

#[test]
fn service_tasks_start_only_once() {
    let tasks = ServiceTasks::default();
    assert!(tasks.begin(), "the first start claims the services");
    assert!(!tasks.begin(), "a second start is a no-op");
    tasks.stop();
    assert!(tasks.begin(), "after a stop the services may start again");
}
