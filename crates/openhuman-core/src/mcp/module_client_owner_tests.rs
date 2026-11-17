use super::*;
use std::sync::atomic::AtomicUsize;
use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};

#[derive(Clone, Default)]
struct LifecycleFixture {
    closes: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    block_close_poll: Arc<AtomicBool>,
    close_poll_started: Arc<tokio::sync::Notify>,
    release_close_poll: Arc<tokio::sync::Notify>,
    fail_close: Arc<AtomicBool>,
    block_close: Arc<AtomicBool>,
    close_started: Arc<tokio::sync::Notify>,
    release_close: Arc<tokio::sync::Notify>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinymcp.Mcp")]
impl LifecycleFixture {
    async fn close(&self, _data_dir: String) -> tinybus::Result<Option<String>> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        if self.block_close.swap(false, Ordering::SeqCst) {
            self.close_started.notify_one();
            self.release_close.notified().await;
        }
        if self.fail_close.load(Ordering::SeqCst) {
            return Err(tinybus::Error::failed("fixture cleanup unavailable"));
        }
        Ok(Some("original-close".into()))
    }

    async fn transition_status(&self, receipt: String) -> tinybus::Result<TransitionStatus> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if receipt == "original-close" && self.block_close_poll.swap(false, Ordering::SeqCst) {
            self.close_poll_started.notify_one();
            self.release_close_poll.notified().await;
        }
        Ok(TransitionStatus::Complete)
    }
}

#[tokio::test]
async fn failed_agent_cleanup_retains_the_retired_owner_and_can_retry() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch_dir = scratch
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    let directory = scratch_dir.as_path().join("agents/worker");
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let fixture = LifecycleFixture::default();
    service
        .serve_at(
            ObjectPath::new(names::OBJECT_PATH).unwrap(),
            fixture.clone(),
        )
        .await
        .unwrap();
    service.request_name(names::INTERFACE).await.unwrap();
    let port = TinyMcpPort::new(crate::modules::ops::fixture_client(
        caller
            .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
            .unwrap(),
    ));
    port.transition_status("original-open").await.unwrap();
    let client = Arc::new(McpConfiguredClient::with_port(
        directory.to_str().unwrap().into(),
        Box::new(port),
    ));
    *client.state.lock().await = opened_state(tinymcp_bus::client_lifecycle::OpenObject {
        config: McpClientConfig::default(),
        object_path: "/test/original-child".into(),
    });
    lifecycle::table().insert(directory.clone(), Arc::clone(&client));
    let cleanup = agent_cleanup::take_agent_cleanup(scratch_dir.as_path(), "worker").unwrap();
    fixture.fail_close.store(true, Ordering::SeqCst);
    assert_eq!(cleanup.finish().await, Err(ModuleCallError::ModuleFault));
    assert!(Arc::ptr_eq(cached(&directory).as_ref().unwrap(), &client));
    assert!(matches!(
        client.state.lock().await.close_step(),
        Step::CallClose
    ));
    assert_eq!(
        client
            .call::<Value>(&McpClientConfig::default(), "NeverReopen", ())
            .await,
        Err(ModuleCallError::Unavailable)
    );
    fixture.fail_close.store(false, Ordering::SeqCst);
    cleanup.finish().await.unwrap();
    assert!(cached(&directory).is_none());
    // Retrying the old completed handle cannot evict a later owner.
    let successor = Arc::new(McpConfiguredClient::new(&directory, &Config::default()).unwrap());
    lifecycle::table().insert(directory.clone(), Arc::clone(&successor));
    assert!(cleanup.finish_if_idle());
    assert!(Arc::ptr_eq(
        cached(&directory).as_ref().unwrap(),
        &successor
    ));
    lifecycle::table().evict(&directory, cached(&directory).as_ref().unwrap());
}

#[test]
fn an_absent_agent_cleanup_does_not_create_a_configured_client() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch_dir = scratch
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    assert!(agent_cleanup::take_agent_cleanup(scratch_dir.as_path(), "never-opened").is_none());
    assert!(cached(&scratch_dir.as_path().join("agents/never-opened")).is_none());
}

#[tokio::test]
async fn an_old_closed_context_cannot_create_the_reused_agents_first_client() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    let scratch = tempfile::tempdir().unwrap();
    let scratch_dir = scratch
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    let mut config = Config::default();
    config.workspace_dir = scratch_dir.as_path().into();
    let parent = CoreContext::for_test(DomainSet::full(), Some(config.workspace_dir.clone()));
    let old_scope = Arc::new(crate::security::approval::ApprovalScope::default());
    let mut old_overrides = crate::agent::host_overrides::HostOverrides::default();
    old_overrides.approval_scope = Some(Arc::clone(&old_scope));
    let old_overrides = Arc::new(old_overrides);
    let mut overlay = ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::none())
        .session_agent("worker");
    overlay.host_overrides = Some(old_overrides);
    let old_context = parent.derive_with(overlay);
    old_scope.close("agent_removed");
    CoreContext::scope(old_context, async {
        assert!(matches!(
            McpConfiguredClient::for_config(&config),
            Err(ModuleCallError::Unavailable)
        ));
    })
    .await;
    let directory = config.workspace_dir.join("agents/worker");
    assert!(cached(&directory).is_none());
    let mut new_overlay =
        ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::none())
            .session_agent("worker");
    let mut new_overrides = crate::agent::host_overrides::HostOverrides::default();
    new_overrides.approval_scope =
        Some(Arc::new(crate::security::approval::ApprovalScope::default()));
    new_overlay.host_overrides = Some(Arc::new(new_overrides));
    CoreContext::scope(parent.derive_with(new_overlay), async {
        McpConfiguredClient::for_config(&config)
            .expect("the new instance has an open lifecycle scope");
    })
    .await;
    assert!(
        agent_cleanup::take_agent_cleanup(&config.workspace_dir, "worker")
            .unwrap()
            .finish_if_idle()
    );
}

#[tokio::test]
async fn cancelled_close_retries_its_receipt_on_the_original_owner() {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let original = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let replacement = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let original_fixture = LifecycleFixture::default();
    let replacement_fixture = LifecycleFixture::default();
    for (connection, fixture) in [
        (&original, original_fixture.clone()),
        (&replacement, replacement_fixture.clone()),
    ] {
        connection
            .serve_at(ObjectPath::new(names::OBJECT_PATH).unwrap(), fixture)
            .await
            .unwrap();
    }
    original.request_name(names::INTERFACE).await.unwrap();
    let port = TinyMcpPort::new(crate::modules::ops::fixture_client(
        caller
            .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
            .unwrap(),
    ));
    assert_eq!(
        port.transition_status("original-open").await,
        Ok(TransitionStatus::Complete)
    );
    let client = Arc::new(McpConfiguredClient::with_port(
        "/test/agent".into(),
        Box::new(port),
    ));
    *client.state.lock().await = opened_state(tinymcp_bus::client_lifecycle::OpenObject {
        config: McpClientConfig::default(),
        object_path: "/test/original-child".into(),
    });
    original_fixture
        .block_close_poll
        .store(true, Ordering::SeqCst);
    let closing = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.close().await }
    });
    tokio::time::timeout(
        Duration::from_secs(10),
        original_fixture.close_poll_started.notified(),
    )
    .await
    .expect("close receipt reaches the original module");
    closing.abort();
    assert!(closing.await.unwrap_err().is_cancelled());
    original_fixture.release_close_poll.notify_one();
    assert_eq!(
        client.state.lock().await.close_step(),
        Step::PollClose("original-close".into())
    );
    original.release_name(names::INTERFACE).await.unwrap();
    replacement.request_name(names::INTERFACE).await.unwrap();
    client.close().await.unwrap();
    assert_eq!(original_fixture.closes.load(Ordering::SeqCst), 1);
    assert_eq!(
        original_fixture.polls.load(Ordering::SeqCst),
        3,
        "retry must poll the retained receipt on the original owner"
    );
    assert_eq!(replacement_fixture.closes.load(Ordering::SeqCst), 0);
    assert_eq!(replacement_fixture.polls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cleanup_receipts_stay_with_the_original_module_after_alias_replacement() {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let original = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let replacement = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let original_fixture = LifecycleFixture::default();
    let replacement_fixture = LifecycleFixture::default();
    for (connection, fixture) in [
        (&original, original_fixture.clone()),
        (&replacement, replacement_fixture.clone()),
    ] {
        connection
            .serve_at(ObjectPath::new(names::OBJECT_PATH).unwrap(), fixture)
            .await
            .unwrap();
    }
    original.request_name(names::INTERFACE).await.unwrap();
    let root = crate::modules::ops::fixture_client(
        caller
            .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
            .unwrap(),
    );
    let port = TinyMcpPort::new(root);
    // Resolve the original owner through a real bus call. No secret is sent
    // to these deliberately unattested lifecycle fixtures.
    assert_eq!(
        port.transition_status("original-open").await,
        Ok(TransitionStatus::Complete)
    );
    let client = McpConfiguredClient::with_port("/test/agent".into(), Box::new(port));
    *client.state.lock().await = opened_state(tinymcp_bus::client_lifecycle::OpenObject {
        config: McpClientConfig::default(),
        object_path: "/test/original-child".into(),
    });
    original.release_name(names::INTERFACE).await.unwrap();
    replacement.request_name(names::INTERFACE).await.unwrap();

    client.close().await.unwrap();
    assert_eq!(
        original_fixture.closes.load(Ordering::SeqCst),
        1,
        "cleanup must address the original module instance"
    );
    assert_eq!(
        original_fixture.polls.load(Ordering::SeqCst),
        2,
        "the close receipt must be polled on its original owner"
    );
    assert_eq!(replacement_fixture.closes.load(Ordering::SeqCst), 0);
    assert_eq!(replacement_fixture.polls.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_blocked_close_has_an_overall_cleanup_deadline_and_keeps_the_owner_for_retry() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch_dir = scratch
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    let directory = scratch_dir.as_path().join("agents/blocked-close");
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let fixture = LifecycleFixture::default();
    service
        .serve_at(
            ObjectPath::new(names::OBJECT_PATH).unwrap(),
            fixture.clone(),
        )
        .await
        .unwrap();
    service.request_name(names::INTERFACE).await.unwrap();
    let port = TinyMcpPort::new(
        crate::modules::ops::fixture_client(
            caller
                .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
                .unwrap(),
        )
        .with_timeout(Duration::from_secs(120)),
    );
    port.transition_status("original-open").await.unwrap();
    let client = Arc::new(McpConfiguredClient::with_port(
        directory.to_str().unwrap().into(),
        Box::new(port),
    ));
    // The acknowledged object isolates lifecycle routing from module admission.
    *client.state.lock().await = opened_state(tinymcp_bus::client_lifecycle::OpenObject {
        config: McpClientConfig::default(),
        object_path: "/test/original-child".into(),
    });
    lifecycle::table().insert(directory.clone(), Arc::clone(&client));
    let cleanup =
        agent_cleanup::take_agent_cleanup(scratch_dir.as_path(), "blocked-close").unwrap();
    fixture.block_close.store(true, Ordering::SeqCst);
    let attempt = cleanup.clone();
    let mut task = tokio::spawn(async move { attempt.finish().await });
    fixture.close_started.notified().await;
    let outcome = tokio::time::timeout(Duration::from_secs(31), &mut task).await;
    if outcome.is_err() {
        task.abort();
        let _ = task.await;
        fixture.release_close.notify_one();
        lifecycle::table().evict(&directory, cached(&directory).as_ref().unwrap());
    }
    let result = outcome.expect("cleanup must return within 30 seconds even when Close has a 120-second operation timeout")
        .expect("cleanup task must complete normally");
    assert_eq!(result, Err(ModuleCallError::TransportFailed));
    assert!(matches!(
        client.state.lock().await.close_step(),
        Step::CallClose
    ));
    assert!(Arc::ptr_eq(cached(&directory).as_ref().unwrap(), &client));
    assert!(client.retiring.load(Ordering::Acquire));
    // Retry the captured owner; timing out must not evict its close intent.
    fixture.release_close.notify_one();
    cleanup.finish().await.unwrap();
    assert_eq!(fixture.closes.load(Ordering::SeqCst), 2);
    assert!(cached(&directory).is_none());
}

#[test]
fn orphaned_shutdown_cleanup_keeps_a_host_directory_reservation_until_confirmed() {
    let scratch = tempfile::tempdir().unwrap();
    let scratch_dir = scratch
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    let directory = scratch_dir.as_path().join("agents/purge-pending");
    let token = crate::mcp::host::reserve_agent_directory(scratch_dir.as_path(), "purge-pending");
    lifecycle::table().retry_orphaned();
    assert!(McpConfiguredClient::agent_close_pending(&directory));
    let reservation = lifecycle::table().reservation_for(&directory);
    assert!(!lifecycle::table().finish_cleanup(&directory, None, &reservation));
    assert!(McpConfiguredClient::agent_close_pending(&directory));
    token.complete();
    assert!(!McpConfiguredClient::agent_close_pending(&directory));
}

fn cached(directory: &std::path::Path) -> Option<Arc<McpConfiguredClient>> {
    lifecycle::table()
        .snapshot()
        .into_iter()
        .find(|(path, _)| path == directory)
        .map(|(_, client)| client)
}
fn opened_state(object: tinymcp_bus::client_lifecycle::OpenObject) -> ConfiguredLifecycle {
    let mut state = ConfiguredLifecycle::new();
    state.open_step(&object.config);
    state.open_replied(Ok(OpenConfiguredResponse {
        object_path: object.object_path,
        transition_id: "fixture-open".into(),
    }));
    state.open_polled(TransitionStatus::Complete);
    state
}
