use super::*;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

#[derive(Clone, Default)]
struct FakePort {
    opened: Arc<Mutex<Vec<(String, McpClientConfig)>>>,
    closed: Arc<Mutex<Vec<String>>>,
    calls: Arc<Mutex<Vec<(String, String, Value, bool)>>>,
    open_started: Arc<Notify>,
    close_started: Arc<Notify>,
    call_started: Arc<Notify>,
    release_call: Arc<Notify>,
    block_first_open: Arc<std::sync::atomic::AtomicBool>,
    block_first_close: Arc<std::sync::atomic::AtomicBool>,
    block_calls: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl McpModulePort for FakePort {
    async fn open_configured(
        &self,
        data_dir: &str,
        config: &McpClientConfig,
    ) -> Result<OpenConfiguredResponse, ModuleCallError> {
        let index = {
            let mut opened = self.opened.lock().unwrap();
            let prior = opened
                .iter()
                .position(|(path, existing)| path == data_dir && existing == config);
            opened.push((data_dir.to_owned(), config.clone()));
            prior.map_or(opened.len(), |index| index + 1)
        };
        self.open_started.notify_one();
        if index == 1
            && self
                .block_first_open
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            std::future::pending::<()>().await;
        }
        Ok(OpenConfiguredResponse {
            object_path: format!("/workspace/{index}"),
            transition_id: format!("open-{index}"),
        })
    }

    async fn close(&self, data_dir: &str) -> Result<Option<String>, ModuleCallError> {
        let index = {
            let mut closed = self.closed.lock().unwrap();
            closed.push(data_dir.to_owned());
            closed.len()
        };
        self.close_started.notify_one();
        if index == 1
            && self
                .block_first_close
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            std::future::pending::<()>().await;
        }
        Ok(Some("close-1".to_owned()))
    }

    async fn transition_status(
        &self,
        _transition_id: &str,
    ) -> Result<TransitionStatus, ModuleCallError> {
        Ok(TransitionStatus::Complete)
    }

    async fn call_object(
        &self,
        object_path: &str,
        member: &str,
        args: Value,
        confidential: bool,
    ) -> Result<Value, ModuleCallError> {
        self.calls.lock().unwrap().push((
            object_path.to_owned(),
            member.to_owned(),
            args,
            confidential,
        ));
        self.call_started.notify_one();
        if self.block_calls.load(std::sync::atomic::Ordering::SeqCst) {
            self.release_call.notified().await;
        }
        Ok(Value::Bool(true))
    }
}

fn client(port: FakePort) -> McpConfiguredClient {
    McpConfiguredClient::with_port("/private/workspace".to_owned(), Box::new(port))
}

fn configured(name: &str) -> McpClientConfig {
    McpClientConfig {
        servers: vec![tinymcp_bus::McpServerConfig {
            name: name.to_owned(),
            ..tinymcp_bus::McpServerConfig::default()
        }],
        ..McpClientConfig::default()
    }
}

#[tokio::test]
async fn opening_sends_configuration_confidentially_and_reuses_the_ready_object() {
    let port = FakePort::default();
    let opened = Arc::clone(&port.opened);
    let calls = Arc::clone(&port.calls);
    let client = client(port);
    let config = configured("private-server");

    assert!(client
        .call::<bool>(&config, "ConnectedOverview", ())
        .await
        .unwrap());
    assert!(client
        .call::<bool>(&config, "ConnectedOverview", ())
        .await
        .unwrap());

    assert_eq!(opened.lock().unwrap().len(), 1);
    assert_eq!(opened.lock().unwrap()[0].0, "/private/workspace");
    assert_eq!(opened.lock().unwrap()[0].1, config);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "/workspace/1");
    assert_eq!(calls[0].1, "ConnectedOverview");
    assert!(!calls[0].3);
}

#[tokio::test]
async fn disabling_modules_rejects_a_cached_client_but_keeps_shutdown_cleanup() {
    let temporary = tempfile::tempdir().expect("temporary workspace");
    let mut enabled = Config::default();
    enabled.workspace_dir = temporary.path().to_path_buf();
    enabled.modules.enabled = true;

    let port = FakePort::default();
    let opened = Arc::clone(&port.opened);
    let closed = Arc::clone(&port.closed);
    let client = McpConfiguredClient::install_test_port(&enabled, Box::new(port))
        .expect("install test module port");
    client
        .call::<bool>(&configured("server"), "ConnectedOverview", ())
        .await
        .expect("open enabled module client");
    assert_eq!(opened.lock().unwrap().len(), 1);

    let mut disabled = enabled;
    disabled.modules.enabled = false;
    assert!(matches!(
        McpConfiguredClient::for_config(&disabled),
        Err(ModuleCallError::Unavailable)
    ));
    assert_eq!(opened.lock().unwrap().len(), 1);

    // Disabling new calls must not drop the prior owner before shutdown can
    // close the configured TinyMCP object.
    client.close().await.expect("cleanup cached client");
    assert_eq!(closed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn configuration_switches_replace_the_object_before_dispatch() {
    let port = FakePort::default();
    let opened = Arc::clone(&port.opened);
    let calls = Arc::clone(&port.calls);
    let client = client(port);
    let first = configured("first");
    let second = configured("second");

    client
        .call::<bool>(&first, "ConnectedOverview", ())
        .await
        .unwrap();
    client
        .call::<bool>(&second, "ConnectedOverview", ())
        .await
        .unwrap();

    assert_eq!(opened.lock().unwrap().len(), 2);
    assert_eq!(calls.lock().unwrap()[0].0, "/workspace/1");
    assert_eq!(calls.lock().unwrap()[1].0, "/workspace/2");
}

#[tokio::test]
async fn a_cancelled_open_retries_the_same_confidential_request_and_recovers_its_receipt() {
    let port = FakePort {
        block_first_open: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        ..FakePort::default()
    };
    let opened = Arc::clone(&port.opened);
    let calls = Arc::clone(&port.calls);
    let started = Arc::clone(&port.open_started);
    let client = Arc::new(client(port));
    let config = configured("private-server");
    let started_wait = started.notified();
    let task = tokio::spawn({
        let client = Arc::clone(&client);
        let config = config.clone();
        async move { client.call::<bool>(&config, "ConnectedOverview", ()).await }
    });
    started_wait.await;
    task.abort();
    let _ = task.await;

    let retry = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.call::<bool>(&config, "ConnectedOverview", ()).await }
    });
    assert_eq!(retry.await.unwrap(), Ok(true));
    let opened = opened.lock().unwrap();
    assert_eq!(opened.len(), 2);
    assert_eq!(opened[0].1, opened[1].1);
    assert_eq!(calls.lock().unwrap()[0].0, "/workspace/1");
}

#[tokio::test]
async fn close_waits_for_accepted_calls_and_confirms_cleanup() {
    let port = FakePort {
        block_calls: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        ..FakePort::default()
    };
    let close_count = Arc::clone(&port.closed);
    let call_started = Arc::clone(&port.call_started);
    let release_call = Arc::clone(&port.release_call);
    let client = Arc::new(client(port));
    let config = configured("server");
    let call_started_wait = call_started.notified();
    let call = tokio::spawn({
        let client = Arc::clone(&client);
        let config = config.clone();
        async move { client.call::<bool>(&config, "ConnectedOverview", ()).await }
    });
    call_started_wait.await;
    let close = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.close().await }
    });
    tokio::task::yield_now().await;
    assert!(close_count.lock().unwrap().is_empty());
    release_call.notify_one();
    assert_eq!(call.await.unwrap(), Ok(true));
    assert_eq!(close.await.unwrap(), Ok(()));
    assert_eq!(close_count.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_cancelled_close_keeps_the_cleanup_operation_retryable() {
    let port = FakePort {
        block_first_close: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        ..FakePort::default()
    };
    let close_count = Arc::clone(&port.closed);
    let started = Arc::clone(&port.close_started);
    let client = Arc::new(client(port));
    client
        .call::<bool>(&configured("server"), "ConnectedOverview", ())
        .await
        .unwrap();

    let started_wait = started.notified();
    let close = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.close().await }
    });
    started_wait.await;
    close.abort();
    let _ = close.await;

    let retry = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.close().await }
    });
    assert_eq!(retry.await.unwrap(), Ok(()));
    assert_eq!(close_count.lock().unwrap().len(), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn non_utf8_workspace_paths_are_rejected_before_opening() {
    use std::os::unix::ffi::OsStrExt;
    let path = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/workspace\xff"));
    let result = McpConfiguredClient::new(&path, &Config::default());
    assert!(matches!(result, Err(ModuleCallError::IncompatibleContract)));
}
