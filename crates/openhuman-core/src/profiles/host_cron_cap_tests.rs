#[cfg(feature = "storage-sqlite")]
use crate::storage::test_support::sqlite_backend;
#[cfg(feature = "storage-sqlite")]
use crate::{
    core::runtime::{CoreContext, DispatchCtx, DomainSet, SaasConfig},
    profiles::{ProfileHost, ProfileId, ProfileLayout},
};
#[cfg(feature = "storage-sqlite")]
use std::sync::Arc;

#[cfg(feature = "storage-sqlite")]
fn profile(name: &str) -> ProfileId {
    ProfileId::parse(name).unwrap()
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn the_operators_cron_limits_bind_every_profile_whatever_its_config_says() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = sqlite_backend().unwrap();
    let mut saas = SaasConfig::new(tmp.path());
    saas.cron.agent_jobs_only = true;
    saas.cron.min_interval_secs = 900;
    saas.cron.max_active_jobs = 3;
    let host = ProfileHost::with_backend(
        saas,
        CoreContext::for_test(DomainSet::full(), None),
        Some(Arc::clone(&fixture.backend)),
    )
    .unwrap();
    let id = profile("alice");
    host.provision(&id).await.unwrap();
    // The user's own file tries to switch the mode off.
    let layout = ProfileLayout::new(tmp.path(), &id);
    std::fs::write(
        &layout.config_path,
        "[cron]\nagent_jobs_only = false\nmin_interval_secs = 1\nmax_active_jobs = 999\n",
    )
    .unwrap();
    let forced = host.provisioned_config(&id).await.unwrap();
    let opened = host.open(&id).await.unwrap();
    let loaded =
        crate::config::ops::load_current_or_init(&DispatchCtx::new(Arc::clone(opened.context())))
            .await
            .unwrap();
    for config in [&forced, &opened.config, &loaded] {
        assert!(config.cron.agent_jobs_only);
        assert_eq!(config.cron.min_interval_secs, 900);
        assert_eq!(config.cron.max_active_jobs, 3);
    }
}
