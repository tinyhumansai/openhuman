use super::*;

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn provisioning_with_a_new_root_replaces_a_dead_higher_epoch_fence() {
    const CHILD: &str = "OPENHUMAN_PROFILE_DEAD_HIGHER_FENCE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "profiles::host::tests::fence_tests::provisioning_with_a_new_root_replaces_a_dead_higher_epoch_fence",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated profile fence child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "child selected no test: {stdout}"
        );
        assert!(stdout.contains("test profiles::host::tests::fence_tests::provisioning_with_a_new_root_replaces_a_dead_higher_epoch_fence"), "expected child test did not run: {stdout}");
        assert!(
            stdout.contains("test result: ok. 1 passed"),
            "child did not pass exactly one test: {stdout}"
        );
        return;
    }

    let first_root = tempfile::tempdir().unwrap();
    let second_root = tempfile::tempdir().unwrap();
    let id = profile(&format!("cron-fence-{}", uuid::Uuid::new_v4().simple()));
    let registry = crate::storage::fence::registry();
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend: Arc<dyn crate::storage::StorageBackend> =
        Arc::new(crate::storage::fenced_backend::FencedBackend::new(
            Arc::clone(&fixture.backend),
            Arc::clone(&registry),
        ));
    let _storage = RestoreInstalledStorage(crate::storage::install(backend));

    let mut first_saas = SaasConfig::new(first_root.path());
    first_saas.cron.agent_jobs_only = true;
    first_saas.cron.max_active_jobs = 1;
    let first = ProfileHost::new(first_saas, CoreContext::for_test(DomainSet::full(), None));
    first.provision(&id).await.unwrap();
    let opened = first.open(&id).await.unwrap();
    let old_epoch = opened.lease_fence().epoch();
    let _retire_stale_fence = RetireProfileFence {
        registry: Arc::clone(&registry),
        profile_id: id.as_str().to_string(),
        epoch: old_epoch,
    };
    drop(opened);
    drop(first);

    let mut second_saas = SaasConfig::new(second_root.path());
    second_saas.cron.agent_jobs_only = true;
    second_saas.cron.max_active_jobs = 1;
    let second = ProfileHost::new(second_saas, CoreContext::for_test(DomainSet::full(), None));
    let provisioned = second.provision(&id).await;
    assert!(
        provisioned.unwrap(),
        "the new root must be able to register its own profile fence"
    );
}

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn opening_replaces_a_dropped_hosts_fence_before_cron_reconciliation() {
    const CHILD: &str = "OPENHUMAN_PROFILE_REOPEN_DROPPED_FENCE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "profiles::host::tests::fence_tests::opening_replaces_a_dropped_hosts_fence_before_cron_reconciliation",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated profile reopen child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("running 1 test"),
            "child selected no test: {stdout}"
        );
        assert!(stdout.contains("test profiles::host::tests::fence_tests::opening_replaces_a_dropped_hosts_fence_before_cron_reconciliation"), "expected child test did not run: {stdout}");
        assert!(
            stdout.contains("test result: ok. 1 passed"),
            "child did not pass exactly one test: {stdout}"
        );
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let id = profile(&format!("cron-fence-{}", uuid::Uuid::new_v4().simple()));
    let registry = crate::storage::fence::registry();
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend: Arc<dyn crate::storage::StorageBackend> =
        Arc::new(crate::storage::fenced_backend::FencedBackend::new(
            Arc::clone(&fixture.backend),
            Arc::clone(&registry),
        ));
    let _storage = RestoreInstalledStorage(crate::storage::install(backend));

    let mut first_saas = SaasConfig::new(tmp.path());
    first_saas.cron.agent_jobs_only = true;
    first_saas.cron.max_active_jobs = 1;
    let first = ProfileHost::new(first_saas, CoreContext::for_test(DomainSet::full(), None));
    first.provision(&id).await.unwrap();
    let opened = first.open(&id).await.unwrap();
    let old_epoch = opened.lease_fence().epoch();
    let _retire_stale_fence = RetireProfileFence {
        registry: Arc::clone(&registry),
        profile_id: id.as_str().to_string(),
        epoch: old_epoch,
    };
    drop(opened);
    drop(first);

    let mut second_saas = SaasConfig::new(tmp.path());
    second_saas.cron.agent_jobs_only = true;
    second_saas.cron.max_active_jobs = 1;
    let second = ProfileHost::new(second_saas, CoreContext::for_test(DomainSet::full(), None));
    assert!(!second.provision(&id).await.unwrap());
    let reopened = second.open(&id).await;
    assert!(
        reopened.is_ok(),
        "the later lease in this root must replace the dropped host's profile fence"
    );
}

#[tokio::test]
async fn disabling_profile_cron_limits_clears_the_persisted_store_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let id = profile("cron-cap-toggle-off");
    let mut saas = SaasConfig::new(tmp.path());
    saas.cron.agent_jobs_only = true;
    saas.cron.max_active_jobs = 1;
    let enforced = ProfileHost::new(saas, CoreContext::for_test(DomainSet::full(), None));
    enforced.provision(&id).await.unwrap();
    let config = enforced.provisioned_config(&id).await.unwrap();
    let scope = crate::cron::documents_port::DocScope::for_scope(
        &config,
        crate::storage::scope_for_profile(id.as_str()),
    );
    crate::cron::client::store_in::<crate::cron::CronJob>(&config, &scope, raw_profile_agent_add())
        .await
        .unwrap();

    let unenforced = ProfileHost::new(
        SaasConfig::new(tmp.path()),
        CoreContext::for_test(DomainSet::full(), None),
    );
    unenforced.provision(&id).await.unwrap();
    let _profile = unenforced.open(&id).await.unwrap();
    let config = unenforced.provisioned_config(&id).await.unwrap();
    let scope = crate::cron::documents_port::DocScope::for_scope(
        &config,
        crate::storage::scope_for_profile(id.as_str()),
    );

    crate::cron::client::store_in::<crate::cron::CronJob>(&config, &scope, raw_profile_agent_add())
        .await
        .expect("disabling operator policy should clear its persisted cap");
}
