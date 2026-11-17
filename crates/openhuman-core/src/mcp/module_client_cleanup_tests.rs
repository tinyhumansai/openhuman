use super::*;

#[tokio::test]
async fn delayed_agent_close_cannot_evict_a_replacement_cache_entry() {
    let workspace = tempfile::tempdir().expect("workspace");
    let workspace_dir = workspace
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    let mut config = Config::default();
    config.workspace_dir = workspace_dir.as_path().to_path_buf();
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
    let reservation = lifecycle::table().reservation_with_action(
        workspace_dir.as_path(),
        Arc::new(move || {
            callback_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            true
        }),
    );
    let captured = lifecycle::table()
        .retire(workspace_dir.as_path())
        .expect("retirement captures the exact old client");

    let replacement_port = FakePort::default();
    let replacement_closed = Arc::clone(&replacement_port.closed);
    let replacement = Arc::new(McpConfiguredClient::with_port(
        workspace_dir.as_path().to_string_lossy().into_owned(),
        Box::new(replacement_port),
    ));
    lifecycle::table().insert(
        workspace_dir.as_path().to_path_buf(),
        Arc::clone(&replacement),
    );

    assert!(
        !McpConfiguredClient::close_retired_client(
            workspace_dir.as_path().to_path_buf(),
            captured,
            Arc::clone(&reservation),
        )
        .await,
        "a stale close completion cannot claim the replacement entry"
    );
    assert!(!cleanup_ran.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(old_closed.lock().unwrap().len(), 1);
    assert!(replacement_closed.lock().unwrap().is_empty());
    let current = lifecycle::table()
        .snapshot()
        .into_iter()
        .find(|(path, _)| path == workspace_dir.as_path())
        .unwrap()
        .1;
    assert!(Arc::ptr_eq(&current, &replacement));
    lifecycle::table().evict(workspace_dir.as_path(), &replacement);
    lifecycle::table().finish_cleanup(workspace_dir.as_path(), None, &reservation);
}

#[tokio::test]
async fn failed_cleanup_actions_retain_and_retry_cached_and_orphaned_paths() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let workspace = tempfile::tempdir().expect("workspace");
    let workspace_dir = workspace
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    let agent_dir = workspace_dir.as_path().join("agents").join("alpha");
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
    let reservation = lifecycle::table().reservation_with_action(&agent_dir, action);
    let captured = lifecycle::table().retire(&agent_dir).unwrap();
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

    let orphan_dir = workspace_dir.as_path().join("agents").join("orphan");
    std::fs::create_dir_all(&orphan_dir).unwrap();
    let orphan_attempts = Arc::new(AtomicUsize::new(0));
    let callback_attempts = Arc::clone(&orphan_attempts);
    let orphan_home = orphan_dir.clone();
    let reservation = lifecycle::table().reservation_with_action(
        &orphan_dir,
        Arc::new(move || {
            if callback_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return false;
            }
            std::fs::remove_dir_all(&orphan_home).is_ok() || !orphan_home.exists()
        }),
    );
    assert!(!lifecycle::table().finish_cleanup(&orphan_dir, None, &reservation,));
    assert!(McpConfiguredClient::agent_close_pending(&orphan_dir));
    assert!(orphan_dir.exists());
    lifecycle::table().retry_orphaned();
    assert!(!McpConfiguredClient::agent_close_pending(&orphan_dir));
    assert!(!orphan_dir.exists());
}
