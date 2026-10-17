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

/// A pending close fences the folder whatever path spells it: a runtime that
/// reopens the workspace through a symlink must not slip past the reservation
/// and have its agent home purged by the old runtime's deferred cleanup.
#[cfg(unix)]
#[tokio::test]
async fn a_pending_close_fences_the_folder_under_another_spelling() {
    let workspace = tempfile::tempdir().expect("workspace");
    let agent_dir = workspace.path().join("agents").join("alpha");
    std::fs::create_dir_all(&agent_dir).expect("agent home");
    let links = tempfile::tempdir().expect("links");
    let alias = links.path().join("alias");
    std::os::unix::fs::symlink(workspace.path(), &alias).expect("symlink");
    let aliased_dir = alias.join("agents").join("alpha");

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

    McpConfiguredClient::schedule_agent_close_and_cleanup(&agent_dir, Arc::new(|| true));
    close_started.notified().await;

    assert!(McpConfiguredClient::agent_close_pending(&aliased_dir));
    let mut aliased = config.clone();
    aliased.workspace_dir = aliased_dir.clone();
    assert!(matches!(
        McpConfiguredClient::for_config(&aliased),
        Err(ModuleCallError::Unavailable)
    ));

    release_close.notify_one();
    tokio::time::timeout(Duration::from_secs(1), async {
        while McpConfiguredClient::agent_close_pending(&aliased_dir) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("close finishes");
}
