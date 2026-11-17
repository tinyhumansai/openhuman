use super::*;
use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
use crate::memory::lifecycle::jobs::enqueue;
use crate::profiles::layout::profile_config;
use crate::profiles::ProfileId;
use tinymemory_api::{ConsolidateRequest, ItemKind, Namespace, Reach};
use tinymemory_tools::BackgroundJob;

fn host(tmp: &tempfile::TempDir, idle_secs: u64) -> ProfileHost {
    let mut saas = SaasConfig::new(tmp.path());
    saas.idle_evict_secs = idle_secs;
    let backend_path = tmp.path().join("profile-storage");
    let backend_url = format!("sqlite:{}", backend_path.display());
    let backend = crate::storage::block_on_anyhow(async move {
        crate::storage::open(&backend_url)
            .await
            .map_err(anyhow::Error::from)
    })
    .expect("profile test SQLite backend opens");
    ProfileHost::with_backend(
        saas,
        CoreContext::for_test(DomainSet::full(), None),
        Some(backend),
    )
    .expect("profile test host uses conditional SQLite leases")
}

fn build_job() -> BackgroundJob {
    BackgroundJob::BuildBeliefs {
        request: ConsolidateRequest::new(Reach::exact(Namespace::agent("a")))
            .kinds([ItemKind::Conversation]),
    }
}

#[tokio::test]
async fn nothing_to_do_does_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 3600);
    assert_eq!(tick(&host).await, TickReport::default());
    let id = ProfileId::for_user(
        &format!("idle-{}", uuid::Uuid::new_v4()),
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    host.provision(&id).await.unwrap();
    assert_eq!(tick(&host).await, TickReport::default());
    assert!(
        !host.is_open(&id),
        "an agent with no queued work is not opened"
    );
}

#[tokio::test]
async fn only_profiles_with_queued_memory_jobs_are_run() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 3600);
    let busy = ProfileId::for_user(
        &format!("busy-{}", uuid::Uuid::new_v4()),
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    let quiet = ProfileId::for_user(
        &format!("quiet-{}", uuid::Uuid::new_v4()),
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    host.provision(&busy).await.unwrap();
    host.provision(&quiet).await.unwrap();

    let config = profile_config(&host.layout_of(&busy), &busy);
    crate::memory::test_fixtures::bind_reference(&config);
    enqueue(&config, &Namespace::ROOT, vec![build_job()]).await;
    assert!(jobs::has_pending(&config.workspace_dir));

    let report = tick(&host).await;
    assert_eq!(report.ran, 1, "{report:?}");
    assert!(
        host.is_open(&busy),
        "the busy agent was opened to run its jobs"
    );
    assert!(!host.is_open(&quiet), "the quiet agent was left closed");
}

#[tokio::test]
async fn a_tick_sweeps_idle_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 0);
    let id = ProfileId::for_user(
        &format!("sweep-{}", uuid::Uuid::new_v4()),
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    host.provision(&id).await.unwrap();
    drop(host.open(&id).await.unwrap());
    assert!(host.is_open(&id));
    tick(&host).await;
    assert!(!host.is_open(&id));
}
