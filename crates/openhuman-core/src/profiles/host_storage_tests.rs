#![cfg(feature = "storage-sqlite")]

use super::*;

fn profile_id(name: &str) -> ProfileId {
    ProfileId::for_user(name, super::super::ProfileIdMode::Raw).unwrap()
}

fn agent_add() -> tinyflows_bus::schedule::StoreOperation {
    tinyflows_bus::schedule::StoreOperation::AddAgent {
        spec: tinyflows_bus::schedule::AgentJobSpec::new(
            tinyflows_bus::schedule::Schedule::Every { every_ms: 600_000 },
            "profile quota probe",
        ),
    }
}

async fn assert_supplied_backend_owns_the_cap_and_fence(
    host: ProfileHost,
    id: &ProfileId,
    query_backend: Arc<dyn crate::storage::StorageBackend>,
) {
    host.provision(id).await.unwrap();
    let config = host.provisioned_config(id).await.unwrap();
    let profile_scope = crate::storage::scope_for_profile(id.as_str());
    let query_scope = crate::cron::documents_port::DocScope::on_backend(
        &config,
        Arc::clone(&query_backend),
        profile_scope.clone(),
    );

    let first =
        crate::cron::client::store_in::<crate::cron::CronJob>(&config, &query_scope, agent_add())
            .await
            .unwrap();
    let second =
        crate::cron::client::store_in::<crate::cron::CronJob>(&config, &query_scope, agent_add())
            .await;
    let error = second.expect_err("the cap must be installed in the supplied backend");
    assert!(error
        .to_string()
        .contains("Failed to insert cron agent job"));

    let _opened = host.open(id).await.unwrap();
    let lease_ttl = host.saas().lease_ttl();
    let thief = crate::storage::lease::DocumentLeases::cluster(
        query_backend.as_ref(),
        "cron-quota-thief",
        None,
        lease_ttl,
    )
    .unwrap();
    let takeover_time = crate::profiles::lease::now_ms().saturating_add(
        lease_ttl
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
            .saturating_mul(2),
    );
    thief.acquire(id.as_str(), takeover_time).await.unwrap();
    let guarded_scope = crate::cron::documents_port::DocScope::on_backend(
        &config,
        Arc::clone(host.cron_backend.as_ref().unwrap()),
        profile_scope,
    );
    let update = crate::cron::client::store_in::<crate::cron::CronJob>(
        &config,
        &guarded_scope,
        tinyflows_bus::schedule::StoreOperation::Update {
            job_id: first.id.clone(),
            patch: tinyflows_bus::schedule::types::CronJobPatch {
                name: Some("fenced rename".into()),
                ..Default::default()
            },
        },
    )
    .await;
    assert!(
        update.is_err(),
        "a superseded profile must refuse backend writes: {update:?}"
    );
}

#[tokio::test]
async fn a_backend_supplied_to_profile_host_owns_its_cron_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let id = profile_id("cron-cap-shared-backend");
    let mut saas = crate::core::runtime::SaasConfig::new(tmp.path());
    saas.cron.agent_jobs_only = true;
    saas.cron.max_active_jobs = 1;
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend = Arc::clone(&fixture.backend);
    let fences = Arc::new(crate::storage::fence::FenceRegistry::new(
        crate::profiles::lease::now_ms,
    ));
    let host = ProfileHost::with_backend(
        saas,
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None),
        Some(Arc::clone(&backend)),
    )
    .unwrap()
    .with_fences(fences);

    assert_supplied_backend_owns_the_cap_and_fence(host, &id, Arc::clone(&backend)).await;
}

#[tokio::test]
async fn an_already_fenced_backend_remains_compatible_with_the_public_constructor() {
    let tmp = tempfile::tempdir().unwrap();
    let id = profile_id("cron-cap-public-fenced-backend");
    let mut saas = crate::core::runtime::SaasConfig::new(tmp.path());
    saas.cron.agent_jobs_only = true;
    saas.cron.max_active_jobs = 1;
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend = Arc::clone(&fixture.backend);
    let fences = crate::storage::fence::registry();
    let supplied: Arc<dyn crate::storage::StorageBackend> =
        Arc::new(crate::storage::fenced_backend::FencedBackend::new(
            Arc::clone(&backend),
            Arc::clone(&fences),
        ));
    let host = ProfileHost::with_backend(
        saas,
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None),
        Some(supplied),
    )
    .unwrap()
    .with_fences(fences);

    assert_supplied_backend_owns_the_cap_and_fence(host, &id, backend).await;
}

#[tokio::test]
async fn the_installed_fenced_backend_is_reused_without_an_extra_wrapper() {
    let tmp = tempfile::tempdir().unwrap();
    let id = profile_id("cron-cap-installed-fenced-backend");
    let mut saas = crate::core::runtime::SaasConfig::new(tmp.path());
    saas.cron.agent_jobs_only = true;
    saas.cron.max_active_jobs = 1;
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend = Arc::clone(&fixture.backend);
    let fences = crate::storage::fence::registry();
    let supplied: Arc<dyn crate::storage::StorageBackend> =
        Arc::new(crate::storage::fenced_backend::FencedBackend::new(
            Arc::clone(&backend),
            Arc::clone(&fences),
        ));
    let host = ProfileHost::with_fenced_backend(
        saas,
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None),
        Some(Arc::clone(&supplied)),
    )
    .unwrap();
    assert!(
        Arc::ptr_eq(host.cron_backend.as_ref().unwrap(), &supplied),
        "the internal installed-backend path should reuse the existing fence layer"
    );

    assert_supplied_backend_owns_the_cap_and_fence(host, &id, backend).await;
}
