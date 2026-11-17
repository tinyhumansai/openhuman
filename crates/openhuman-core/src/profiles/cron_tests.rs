use super::*;
use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
use crate::cron::host_delivery::{CronDelivery, CronReply};
use std::sync::atomic::{AtomicUsize, Ordering};

struct NoopDelivery;

#[async_trait::async_trait]
impl CronDelivery for NoopDelivery {
    async fn deliver(&self, _: CronReply) -> Result<(), String> {
        Ok(())
    }
}

struct RestoreInstalledStorage(Option<Arc<dyn crate::storage::StorageBackend>>);

impl Drop for RestoreInstalledStorage {
    fn drop(&mut self) {
        crate::storage::clear();
        if let Some(previous) = self.0.take() {
            crate::storage::install(previous);
        }
    }
}

struct RestoreCronDelivery(Option<Arc<dyn CronDelivery>>);

impl Drop for RestoreCronDelivery {
    fn drop(&mut self) {
        crate::cron::host_delivery::clear();
        if let Some(previous) = self.0.take() {
            crate::cron::host_delivery::install(previous);
        }
    }
}

#[tokio::test]
async fn a_host_without_a_shared_backend_sweeps_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let host = ProfileHost::new(
        SaasConfig::new(tmp.path()),
        CoreContext::for_test(DomainSet::full(), None),
    );
    assert_eq!(sweep(&host, Utc::now()).await, SweepReport::default());
}

#[tokio::test]
async fn hosted_cron_sweep_runs_trusted_memory_jobs_in_agent_only_profiles() {
    const CHILD: &str = "OPENHUMAN_HOSTED_MEMORY_CRON_SWEEP_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "profiles::cron::tests::hosted_cron_sweep_runs_trusted_memory_jobs_in_agent_only_profiles",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "isolated sweep child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("running 1 test"),
            "child selected no test: {stdout}"
        );
        assert!(
            stdout.contains("test profiles::cron::tests::hosted_cron_sweep_runs_trusted_memory_jobs_in_agent_only_profiles"),
            "expected child test did not run: {stdout}"
        );
        assert!(
            stdout.contains("test result: ok. 1 passed"),
            "child failed: {stdout}"
        );
        return;
    }

    hosted_memory_cron_sweep_child().await;
}

#[allow(clippy::await_holding_lock)] // Serializes access to the process-wide installed storage slot.
async fn hosted_memory_cron_sweep_child() {
    let _storage_slot = crate::storage::test_support::lock_storage_slot();
    let registry = crate::storage::fence::registry();
    let backend: Arc<dyn crate::storage::StorageBackend> =
        Arc::new(crate::storage::fenced_backend::FencedBackend::new(
            crate::storage::test_support::memory_backend(),
            Arc::clone(&registry),
        ));
    let _storage = RestoreInstalledStorage(crate::storage::install(backend));
    let _delivery =
        RestoreCronDelivery(crate::cron::host_delivery::install(Arc::new(NoopDelivery)));

    let tmp = tempfile::tempdir().unwrap();
    let id = crate::profiles::ProfileId::for_user(
        "memory-cron-hosted-sweep",
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    let context = CoreContext::for_test(DomainSet::full(), None);
    let seed_host = ProfileHost::new(SaasConfig::new(tmp.path()), Arc::clone(&context));
    seed_host.provision(&id).await.unwrap();
    let opened = seed_host.open(&id).await.unwrap();
    let config = opened.config.clone();
    CoreContext::scope(Arc::clone(opened.context()), async {
        crate::cron::system_jobs::ensure_memory_jobs(&config).unwrap();
        let due_at = Utc::now() - chrono::Duration::seconds(1);
        for job in crate::cron::list_jobs(&config).unwrap() {
            if crate::cron::system_jobs::system_job_name(&job).is_some_and(|name| {
                name == crate::memory::lifecycle::jobs::BACKGROUND_JOB
                    || name == crate::memory::bus::SOURCES_SYNC_JOB
            }) {
                crate::cron::store::tests::force_next_run_for_test(&config, &job.id, due_at)
                    .unwrap();
            }
        }
    })
    .await;
    drop(opened);
    seed_host.release(&id).await.unwrap();

    let mut saas = SaasConfig::new(tmp.path());
    saas.cron.agent_jobs_only = true;
    saas.cron.max_active_jobs = 2;
    let host = ProfileHost::new(saas, context);
    assert!(!host.provision(&id).await.unwrap());

    let calls = Arc::new(AtomicUsize::new(0));
    let background_calls = Arc::clone(&calls);
    let _background = crate::cron::system_job_handlers::register(
        crate::memory::lifecycle::jobs::BACKGROUND_JOB,
        Arc::new(move |_| {
            let calls = Arc::clone(&background_calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }),
    );
    let sync_calls = Arc::clone(&calls);
    let _sync = crate::cron::system_job_handlers::register(
        crate::memory::bus::SOURCES_SYNC_JOB,
        Arc::new(move |_| {
            let calls = Arc::clone(&sync_calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }),
    );

    let report = sweep(&host, Utc::now()).await;
    assert_eq!(
        report.ran, 2,
        "both trusted memory jobs should run: {report:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
