use super::*;

#[tokio::test]
async fn disabled_archive_operations_leave_the_host_filesystem_untouched() {
    let workspace = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_owned();
    config.modules.enabled = false;
    let owner = TriggerHistory {
        initial_workspace: config.workspace_dir.clone(),
        lease: Arc::new(Mutex::new(None)),
        #[cfg(feature = "modules")]
        fixture: None,
    };
    assert!(owner
        .read(&config, Some(10))
        .await
        .unwrap_err()
        .contains("module unavailable"));
    assert!(!workspace.path().join("state").exists());
    assert!(owner.lease.lock().await.is_none());
    owner.close().await.unwrap();
}

#[cfg(feature = "modules")]
#[derive(Clone, Default)]
struct Fixture {
    calls: Arc<Mutex<Vec<String>>>,
    open_started: Arc<tokio::sync::Notify>,
    open_continue: Arc<tokio::sync::Notify>,
    pause_open: bool,
}

#[cfg(feature = "modules")]
#[tinybus::interface(name = "test.ConnectorArchive")]
impl Fixture {
    async fn open_archive(&self, request: OpenArchiveRequest) -> tinybus::Result<ArchiveHandle> {
        self.calls
            .lock()
            .await
            .push(format!("open:{}", request.state_dir));
        if self.pause_open {
            self.open_started.notify_one();
            self.open_continue.notified().await;
        }
        Ok(ArchiveHandle(request.state_dir))
    }
    async fn read_archive(
        &self,
        request: ReadArchiveRequest,
    ) -> tinybus::Result<ComposioTriggerHistoryResult> {
        self.calls
            .lock()
            .await
            .push(format!("read:{}", request.handle.0));
        Ok(ComposioTriggerHistoryResult {
            archive_dir: "fixture".into(),
            current_day_file: "fixture-day".into(),
            entries: Vec::new(),
        })
    }
    async fn close_archive(&self, handle: ArchiveHandle) -> tinybus::Result<()> {
        self.calls.lock().await.push(format!("close:{}", handle.0));
        Ok(())
    }
}

#[cfg(feature = "modules")]
async fn connected_owner(fixture: Fixture) -> (Arc<TriggerHistory>, tinybus::Connection) {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    service
        .serve_at(ObjectPath::new("/test/archive").unwrap(), fixture)
        .await
        .unwrap();
    service.request_name("test.ConnectorArchive").await.unwrap();
    let client = ModuleClient::fixture(
        caller
            .proxy(
                "test.ConnectorArchive",
                "/test/archive",
                "test.ConnectorArchive",
            )
            .unwrap(),
    );
    (
        Arc::new(TriggerHistory {
            initial_workspace: PathBuf::from("first"),
            lease: Arc::new(Mutex::new(None)),
            fixture: Some(client),
        }),
        service,
    )
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn changing_users_releases_the_old_archive_before_opening_the_next_scope() {
    let fixture = Fixture::default();
    let calls = fixture.calls.clone();
    let (owner, _service) = connected_owner(fixture).await;
    assert!(
        calls.lock().await.is_empty(),
        "constructing an owner must remain lazy"
    );
    let mut config = Config::default();
    config.workspace_dir = PathBuf::from("first");
    owner.read(&config, Some(2)).await.unwrap();
    config.workspace_dir = PathBuf::from("second");
    owner.read(&config, Some(2)).await.unwrap();
    owner.close().await.unwrap();
    owner.close().await.unwrap();
    assert_eq!(
        *calls.lock().await,
        vec![
            format!("open:{}", PathBuf::from("first").join("state").display()),
            format!("read:{}", PathBuf::from("first").join("state").display()),
            format!("close:{}", PathBuf::from("first").join("state").display()),
            format!("open:{}", PathBuf::from("second").join("state").display()),
            format!("read:{}", PathBuf::from("second").join("state").display()),
            format!("close:{}", PathBuf::from("second").join("state").display())
        ]
    );
    assert!(owner.lease.lock().await.is_none());
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn cancelling_a_pending_open_retains_a_handle_that_shutdown_can_release() {
    let fixture = Fixture {
        pause_open: true,
        ..Default::default()
    };
    let observed = fixture.clone();
    let (owner, _service) = connected_owner(fixture).await;
    let mut config = Config::default();
    config.workspace_dir = PathBuf::from("cancelled");
    let opening = tokio::spawn({
        let owner = owner.clone();
        async move { owner.read(&config, Some(1)).await }
    });
    observed.open_started.notified().await;
    opening.abort();
    assert!(opening.await.unwrap_err().is_cancelled());
    observed.open_continue.notify_one();
    owner.close().await.unwrap();
    assert_eq!(
        *observed.calls.lock().await,
        vec!["open:cancelled/state", "close:cancelled/state"]
    );
    assert!(owner.lease.lock().await.is_none());
}
