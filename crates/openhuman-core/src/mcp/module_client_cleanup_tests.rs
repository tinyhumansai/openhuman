use super::*;

#[tokio::test]
async fn agent_cleanup_closes_only_the_cached_agent_and_keeps_siblings_open() {
    let target_workspace = tempfile::tempdir().expect("target workspace");
    let sibling_workspace = tempfile::tempdir().expect("sibling workspace");
    let mut target_config = Config::default();
    target_config.workspace_dir = target_workspace.path().to_path_buf();
    target_config.modules.enabled = true;
    let mut sibling_config = Config::default();
    sibling_config.workspace_dir = sibling_workspace.path().to_path_buf();
    sibling_config.modules.enabled = true;

    let target_port = FakePort::default();
    let target_closed = Arc::clone(&target_port.closed);
    let target = McpConfiguredClient::install_test_port(&target_config, Box::new(target_port))
        .expect("install target client");
    let sibling_port = FakePort::default();
    let sibling_closed = Arc::clone(&sibling_port.closed);
    let sibling = McpConfiguredClient::install_test_port(&sibling_config, Box::new(sibling_port))
        .expect("install sibling client");

    target
        .call::<bool>(&configured("target"), "ConnectedOverview", ())
        .await
        .unwrap();
    sibling
        .call::<bool>(&configured("sibling"), "ConnectedOverview", ())
        .await
        .unwrap();

    assert!(McpConfiguredClient::close_agent_data_dir(target_workspace.path()).await);
    assert_eq!(
        target_closed.lock().unwrap().as_slice(),
        [target_workspace.path().to_str().unwrap()]
    );
    assert!(sibling_closed.lock().unwrap().is_empty());
    assert!(McpConfiguredClient::for_config(&sibling_config).is_ok());
    assert_eq!(
        target
            .call::<bool>(&configured("target"), "ConnectedOverview", ())
            .await,
        Err(ModuleCallError::Unavailable),
        "an old client cannot reopen after acknowledged retirement"
    );
}

#[tokio::test]
async fn failed_agent_close_stays_retired_and_can_be_retried() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.modules.enabled = true;

    let port = FakePort::default();
    port.fail_first_close
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let closed = Arc::clone(&port.closed);
    let client =
        McpConfiguredClient::install_test_port(&config, Box::new(port)).expect("install client");
    client
        .call::<bool>(&configured("server"), "ConnectedOverview", ())
        .await
        .unwrap();

    assert!(!McpConfiguredClient::close_agent_data_dir(workspace.path()).await);
    assert!(McpConfiguredClient::agent_close_pending(workspace.path()));
    assert!(matches!(
        McpConfiguredClient::for_config(&config),
        Err(ModuleCallError::Unavailable)
    ));
    assert_eq!(closed.lock().unwrap().len(), 1);

    assert!(McpConfiguredClient::close_agent_data_dir(workspace.path()).await);
    assert!(!McpConfiguredClient::agent_close_pending(workspace.path()));
    assert_eq!(closed.lock().unwrap().len(), 2);
    assert!(McpConfiguredClient::for_config(&config).is_ok());
}

#[tokio::test]
async fn agent_id_stays_unavailable_until_its_cached_module_close_is_acknowledged() {
    let workspace = tempfile::tempdir().expect("workspace");
    let alpha_dir = workspace.path().join("agents").join("alpha");
    let beta_dir = workspace.path().join("agents").join("beta");
    let mut alpha_config = Config::default();
    alpha_config.workspace_dir = alpha_dir;
    alpha_config.modules.enabled = true;
    let mut beta_config = Config::default();
    beta_config.workspace_dir = beta_dir;
    beta_config.modules.enabled = true;

    let alpha_port = FakePort::default();
    alpha_port
        .block_first_close
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let alpha = McpConfiguredClient::install_test_port(&alpha_config, Box::new(alpha_port.clone()))
        .expect("install alpha client");
    alpha
        .call::<bool>(&configured("alpha"), "ConnectedOverview", ())
        .await
        .unwrap();
    let beta = McpConfiguredClient::install_test_port(&beta_config, Box::new(FakePort::default()))
        .expect("install beta client");
    beta.call::<bool>(&configured("beta"), "ConnectedOverview", ())
        .await
        .unwrap();

    let workspace_path = workspace.path().to_path_buf();
    let closing =
        tokio::spawn(async move { crate::mcp::host::close_agent(&workspace_path, "alpha").await });
    alpha_port.close_started.notified().await;
    assert!(crate::mcp::host::agent_close_pending(
        workspace.path(),
        "alpha"
    ));
    assert!(!crate::mcp::host::agent_close_pending(
        workspace.path(),
        "beta"
    ));
    assert!(matches!(
        McpConfiguredClient::for_config(&alpha_config),
        Err(ModuleCallError::Unavailable)
    ));
    assert!(McpConfiguredClient::for_config(&beta_config).is_ok());

    alpha_port.release_close.notify_one();
    assert!(closing.await.unwrap());
    assert!(!crate::mcp::host::agent_close_pending(
        workspace.path(),
        "alpha"
    ));
    assert!(McpConfiguredClient::for_config(&alpha_config).is_ok());
}

#[tokio::test]
async fn deferred_agent_purge_waits_for_module_close_and_keeps_the_id_reserved() {
    let workspace = tempfile::tempdir().expect("workspace");
    let agent_dir = workspace.path().join("agents").join("alpha");
    std::fs::create_dir_all(&agent_dir).expect("agent home");
    std::fs::write(agent_dir.join("transcript.jsonl"), "kept until close").unwrap();
    let mut config = Config::default();
    config.workspace_dir = agent_dir.clone();
    config.modules.enabled = true;

    let port = FakePort::default();
    port.block_first_close
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let close_started = Arc::clone(&port.close_started);
    let release_close = Arc::clone(&port.release_close);
    let client = McpConfiguredClient::install_test_port(&config, Box::new(port))
        .expect("install module client");
    client
        .call::<bool>(&configured("agent"), "ConnectedOverview", ())
        .await
        .unwrap();

    McpConfiguredClient::schedule_agent_close_and_cleanup(
        &agent_dir,
        Arc::new({
            let agent_dir = agent_dir.clone();
            move || std::fs::remove_dir_all(&agent_dir).is_ok() || !agent_dir.exists()
        }),
    );
    close_started.notified().await;

    assert!(agent_dir.join("transcript.jsonl").exists());
    assert!(McpConfiguredClient::agent_close_pending(&agent_dir));
    assert!(matches!(
        McpConfiguredClient::for_config(&config),
        Err(ModuleCallError::Unavailable)
    ));

    release_close.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while McpConfiguredClient::agent_close_pending(&agent_dir) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("close and deferred cleanup finish");
    assert!(!agent_dir.exists());
    assert!(McpConfiguredClient::for_config(&config).is_ok());
}

#[test]
fn closed_client_cleanup_finishes_without_a_tokio_runtime() {
    let workspace = tempfile::tempdir().expect("workspace");
    let agent_dir = workspace.path().join("agents").join("closed");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let mut config = Config::default();
    config.workspace_dir = agent_dir.clone();
    config.modules.enabled = true;
    McpConfiguredClient::install_test_port(&config, Box::new(FakePort::default()))
        .expect("install closed client");

    McpConfiguredClient::schedule_agent_close_and_cleanup(
        &agent_dir,
        Arc::new({
            let agent_dir = agent_dir.clone();
            move || std::fs::remove_dir_all(&agent_dir).is_ok() || !agent_dir.exists()
        }),
    );

    assert!(!agent_dir.exists());
    assert!(!McpConfiguredClient::agent_close_pending(&agent_dir));
}

#[tokio::test]
async fn a_removed_agent_context_cannot_reopen_its_evicted_client() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.modules.enabled = true;
    let scope = Arc::new(crate::security::approval::ApprovalScope::default());
    let mut overrides = crate::agent::host_overrides::HostOverrides::default();
    overrides.approval_scope = Some(Arc::clone(&scope));
    let overrides = Arc::new(overrides);
    let context = CoreContext::for_test(DomainSet::full(), Some(workspace.path().to_path_buf()))
        .derive_with(
            ContextOverlay::new(config.clone(), DomainSet::kernel(), ToolGroups::none())
                .session_agent("agent")
                .host_overrides(overrides),
        );
    scope.close("agent_removed");

    CoreContext::scope(context, async {
        assert!(matches!(
            McpConfiguredClient::for_config(&config),
            Err(ModuleCallError::Unavailable)
        ));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_removed_context_waiting_for_the_client_cache_cannot_reopen_it() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let workspace = tempfile::tempdir().expect("workspace");
    let scoped_dir = workspace.path().join("agents").join("agent");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.modules.enabled = true;
    let scope = Arc::new(crate::security::approval::ApprovalScope::default());
    let mut overrides = crate::agent::host_overrides::HostOverrides::default();
    overrides.approval_scope = Some(Arc::clone(&scope));
    let context = CoreContext::for_test(DomainSet::full(), Some(workspace.path().to_path_buf()))
        .derive_with(
            ContextOverlay::new(config.clone(), DomainSet::kernel(), ToolGroups::none())
                .session_agent("agent")
                .host_overrides(Arc::new(overrides)),
        );

    let clients = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let clients_lock = clients.lock().unwrap();
    let (checked, checked_rx) = tokio::sync::oneshot::channel();
    *AFTER_CONTEXT_CHECK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() = Some((scoped_dir.clone(), checked));
    let request = tokio::spawn(CoreContext::scope(context, async move {
        McpConfiguredClient::for_config(&config)
    }));

    tokio::time::timeout(Duration::from_secs(10), checked_rx)
        .await
        .expect("request passed the initial context check")
        .unwrap();
    scope.close("agent_removed");
    drop(clients_lock);

    let admitted = request.await.unwrap().is_ok();
    if admitted {
        clients.lock().unwrap().remove(&scoped_dir);
    }
    assert!(!admitted, "removed context must not create a cached client");
    assert!(!clients.lock().unwrap().contains_key(&scoped_dir));
}

#[tokio::test]
async fn delayed_agent_close_cannot_evict_a_replacement_cache_entry() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.modules.enabled = true;

    let old_port = FakePort::default();
    let old_closed = Arc::clone(&old_port.closed);
    let old = McpConfiguredClient::install_test_port(&config, Box::new(old_port))
        .expect("install old client");
    old.call::<bool>(&configured("server"), "ConnectedOverview", ())
        .await
        .unwrap();
    let cleanup_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback_flag = Arc::clone(&cleanup_ran);
    let reservation = McpConfiguredClient::reservation_with_action(
        workspace.path(),
        Arc::new(move || {
            callback_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            true
        }),
    );
    let captured = McpConfiguredClient::retire_cached_client(workspace.path())
        .expect("retirement captures the exact old client");

    let replacement_port = FakePort::default();
    let replacement_closed = Arc::clone(&replacement_port.closed);
    let replacement = Arc::new(McpConfiguredClient::with_port(
        workspace.path().to_string_lossy().into_owned(),
        Box::new(replacement_port),
    ));
    CLIENTS
        .get()
        .expect("old client initialized the cache")
        .lock()
        .unwrap()
        .insert(workspace.path().to_path_buf(), Arc::clone(&replacement));

    assert!(
        !McpConfiguredClient::close_retired_client(
            workspace.path().to_path_buf(),
            captured,
            Arc::clone(&reservation),
        )
        .await,
        "a stale close completion cannot claim the replacement entry"
    );
    assert!(!cleanup_ran.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(old_closed.lock().unwrap().len(), 1);
    assert!(replacement_closed.lock().unwrap().is_empty());
    let current = CLIENTS
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .get(workspace.path())
        .cloned()
        .expect("replacement remains cached");
    assert!(Arc::ptr_eq(&current, &replacement));
    CLEANUP_RESERVATIONS
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .remove(workspace.path());
}

#[tokio::test]
async fn failed_cleanup_actions_retain_and_retry_cached_and_orphaned_paths() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let workspace = tempfile::tempdir().expect("workspace");
    let agent_dir = workspace.path().join("agents").join("alpha");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::write(agent_dir.join("state.json"), "retained until cleanup").unwrap();
    let mut config = Config::default();
    config.workspace_dir = agent_dir.clone();
    config.modules.enabled = true;
    let client = McpConfiguredClient::install_test_port(&config, Box::new(FakePort::default()))
        .expect("install client");
    client
        .call::<bool>(&configured("alpha"), "ConnectedOverview", ())
        .await
        .unwrap();

    let attempts = Arc::new(AtomicUsize::new(0));
    let callback_attempts = Arc::clone(&attempts);
    let home = agent_dir.clone();
    let action: CleanupAction = Arc::new(move || {
        if callback_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return false;
        }
        std::fs::remove_dir_all(&home).is_ok() || !home.exists()
    });
    let reservation = McpConfiguredClient::reservation_with_action(&agent_dir, action);
    let captured = McpConfiguredClient::retire_cached_client(&agent_dir).unwrap();
    assert!(
        !McpConfiguredClient::close_retired_client(agent_dir.clone(), captured, reservation,).await
    );
    assert!(agent_dir.join("state.json").exists());
    assert!(McpConfiguredClient::agent_close_pending(&agent_dir));
    assert!(matches!(
        McpConfiguredClient::for_config(&config),
        Err(ModuleCallError::Unavailable)
    ));
    assert!(McpConfiguredClient::close_agent_data_dir(&agent_dir).await);
    assert!(!agent_dir.exists());
    assert!(McpConfiguredClient::for_config(&config).is_ok());

    let orphan_dir = workspace.path().join("agents").join("orphan");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    let orphan_attempts = Arc::new(AtomicUsize::new(0));
    let callback_attempts = Arc::clone(&orphan_attempts);
    let orphan_home = orphan_dir.clone();
    let reservation = McpConfiguredClient::reservation_with_action(
        &orphan_dir,
        Arc::new(move || {
            if callback_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return false;
            }
            std::fs::remove_dir_all(&orphan_home).is_ok() || !orphan_home.exists()
        }),
    );
    assert!(!McpConfiguredClient::finish_cleanup(
        &orphan_dir,
        None,
        reservation,
    ));
    assert!(McpConfiguredClient::agent_close_pending(&orphan_dir));
    assert!(orphan_dir.exists());
    McpConfiguredClient::retry_orphaned_cleanups();
    assert!(!McpConfiguredClient::agent_close_pending(&orphan_dir));
    assert!(!orphan_dir.exists());
}
