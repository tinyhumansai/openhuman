use super::*;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

type CallRecord = (String, String, Value, bool);

#[derive(Clone, Default)]
struct FakePort {
    opened: Arc<Mutex<Vec<(String, McpClientConfig)>>>,
    closed: Arc<Mutex<Vec<String>>>,
    calls: Arc<Mutex<Vec<CallRecord>>>,
    prepared_requests: Arc<Mutex<Vec<PrepareAgentToolsRequest>>>,
    validation_requests: Arc<Mutex<Vec<ValidateServerArgumentsRequest>>>,
    transform_requests: Arc<Mutex<Vec<tinymcp_bus::TransformTextRequest>>>,
    open_started: Arc<Notify>,
    close_started: Arc<Notify>,
    release_close: Arc<Notify>,
    call_started: Arc<Notify>,
    release_call: Arc<Notify>,
    block_first_open: Arc<std::sync::atomic::AtomicBool>,
    block_first_close: Arc<std::sync::atomic::AtomicBool>,
    fail_first_close: Arc<std::sync::atomic::AtomicBool>,
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
        if self
            .fail_first_close
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(ModuleCallError::TransportFailed);
        }
        if index == 1
            && self
                .block_first_close
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.release_close.notified().await;
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
        if member == names::methods::DRAIN_SUPERVISOR_EVENTS {
            self.calls.lock().unwrap().push((
                object_path.to_owned(),
                member.to_owned(),
                args.clone(),
                confidential,
            ));
            self.call_started.notify_one();
            return serde_json::to_value(SupervisorBatch::default())
                .map_err(|_| ModuleCallError::ModuleFault);
        }
        self.calls.lock().unwrap().push((
            object_path.to_owned(),
            member.to_owned(),
            args.clone(),
            confidential,
        ));
        self.call_started.notify_one();
        if self.block_calls.load(std::sync::atomic::Ordering::SeqCst) {
            self.release_call.notified().await;
        }
        if member == names::methods::STATIC_SCRUB_TOOLS {
            let tools = args
                .get(0)
                .and_then(|request| request.get("tools"))
                .cloned()
                .ok_or(ModuleCallError::ModuleFault)?;
            return Ok(tools);
        }
        if member == names::methods::STATIC_SERVER_SUMMARIES {
            return serde_json::to_value(vec![tinymcp_bus::StaticServerSummary {
                name: "docs".to_owned(),
                endpoint: "https://docs.example.test/mcp".to_owned(),
                description: Some("Documentation".to_owned()),
                timeout_secs: 19,
                allowed_tools: vec!["search".to_owned()],
                disallowed_tools: Vec::new(),
                auth_configured: true,
                auth_kind: "bearer_token".to_owned(),
                source: "config".to_owned(),
            }])
            .map_err(|_| ModuleCallError::ModuleFault);
        }
        Ok(Value::Bool(true))
    }

    async fn prepare_agent_tools(
        &self,
        request: PrepareAgentToolsRequest,
    ) -> Result<Vec<McpToolPresentation>, ModuleCallError> {
        self.prepared_requests.lock().unwrap().push(request.clone());
        Ok(request
            .sources
            .into_iter()
            .flat_map(|source| {
                source
                    .tools
                    .into_iter()
                    .map(move |tool| McpToolPresentation {
                        name: format!("module:{}:{}", source.server_id, tool.name),
                        legacy_name: format!("legacy:{}:{}", source.server_id, tool.name),
                        server_id: source.server_id.clone(),
                        remote_name: tool.name,
                        family: source.family.clone(),
                        description: String::new(),
                        parameters: Value::Null,
                        deferred: false,
                        tags: vec![format!("mcp.server_id:{}", source.server_id)],
                    })
            })
            .collect())
    }

    async fn validate_server_arguments(
        &self,
        request: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError> {
        let arguments = request.arguments.as_object().cloned().unwrap_or_default();
        self.validation_requests.lock().unwrap().push(request);
        Ok(ServerArgumentsValidation::Valid(arguments))
    }

    async fn transform_text(
        &self,
        request: tinymcp_bus::TransformTextRequest,
    ) -> Result<String, ModuleCallError> {
        self.transform_requests
            .lock()
            .unwrap()
            .push(request.clone());
        Ok(request.text)
    }
}

#[tokio::test]
async fn identifier_normalization_uses_tinymcp_root_transform_operation() {
    let port = FakePort::default();
    let requests = Arc::clone(&port.transform_requests);
    let client = client(port);

    let normalized = client.normalize_identifier("**search!**").await.unwrap();

    assert_eq!(normalized, "**search!**");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].operation,
        tinymcp_bus::TextTransform::NormalizeIdentifier
    );
    assert_eq!(requests[0].text, "**search!**");
}

#[tokio::test]
async fn agent_tool_names_are_requested_from_the_module_root() {
    let port = FakePort::default();
    let requests = Arc::clone(&port.prepared_requests);
    let client = client(port);
    let request = PrepareAgentToolsRequest {
        sources: vec![tinymcp_bus::McpToolSourceSpec {
            server_id: "docs".to_owned(),
            label: "docs".to_owned(),
            family: "docs".to_owned(),
            display_name: "Docs".to_owned(),
            tools: vec![tinymcp_bus::McpTool::new("whoami")],
            default_exposure: tinymcp_bus::McpToolExposure::Direct,
            direct_tools: vec!["whoami".to_owned()],
        }],
    };

    let prepared = client.prepare_agent_tools(request.clone()).await.unwrap();

    assert_eq!(prepared[0].name, "module:docs:whoami");
    assert_eq!(requests.lock().unwrap().as_slice(), &[request]);
}

#[tokio::test]
async fn generic_server_argument_validation_uses_the_module_root() {
    let port = FakePort::default();
    let requests = Arc::clone(&port.validation_requests);
    let client = client(port);
    let request = ValidateServerArgumentsRequest {
        arguments: serde_json::json!({"query": " topic "}),
        allowed: vec!["query".to_owned()],
        rules: vec![tinymcp_bus::ServerArgumentRule::RequiredNonEmptyString {
            key: "query".to_owned(),
        }],
    };

    let result = client
        .validate_server_arguments(request.clone())
        .await
        .expect("module validation");

    assert_eq!(
        result,
        ServerArgumentsValidation::Valid(
            serde_json::json!({"query": " topic "})
                .as_object()
                .unwrap()
                .clone()
        )
    );
    assert_eq!(requests.lock().unwrap().as_slice(), &[request]);
}

#[tokio::test]
async fn static_tool_metadata_scrubbing_uses_confidential_module_delivery() {
    let port = FakePort::default();
    let calls = Arc::clone(&port.calls);
    let client = client(port);
    let config = configured("docs");
    let tool = McpRemoteTool {
        name: "search".to_owned(),
        title: Some("Docs search".to_owned()),
        description: Some("Search docs".to_owned()),
        input_schema: serde_json::json!({"type":"object"}),
        meta: Some(serde_json::json!({"ui":{"resourceUri":"ui://search"}})),
    };

    let returned = client
        .scrub_static_tools(&config, "docs", vec![tool.clone()])
        .await
        .unwrap();

    assert_eq!(returned, [tool]);
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, names::methods::STATIC_SCRUB_TOOLS);
    assert_eq!(calls[0].2[0]["server"], "docs");
    assert_eq!(calls[0].2[0]["tools"][0]["name"], "search");
    assert!(calls[0].3, "raw remote metadata must cross confidentially");
}

#[tokio::test]
async fn static_server_summaries_use_the_confidential_module_member() {
    let port = FakePort::default();
    let calls = Arc::clone(&port.calls);
    let client = client(port);

    let summaries: Vec<tinymcp_bus::StaticServerSummary> = client
        .call_confidential(
            &configured("docs"),
            names::methods::STATIC_SERVER_SUMMARIES,
            (),
        )
        .await
        .unwrap();

    assert_eq!(summaries[0].auth_kind, "bearer_token");
    assert_eq!(summaries[0].endpoint, "https://docs.example.test/mcp");
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, names::methods::STATIC_SERVER_SUMMARIES);
    assert!(calls[0].3, "server metadata must use confidential delivery");
}

#[tokio::test]
async fn supervisor_drain_only_uses_an_open_object_and_keeps_the_bounded_limit() {
    let port = FakePort::default();
    let calls = Arc::clone(&port.calls);
    let client = client(port);

    assert_eq!(client.drain_supervisor_events(256).await.unwrap(), None);
    assert!(calls.lock().unwrap().is_empty());

    client
        .call::<bool>(&configured("server"), "ConnectedOverview", ())
        .await
        .unwrap();
    let batch = client
        .drain_supervisor_events(256)
        .await
        .unwrap()
        .expect("open child is polled");
    assert!(batch.events.is_empty());
    let calls = calls.lock().unwrap();
    assert_eq!(calls[1].0, "/workspace/1");
    assert_eq!(calls[1].1, names::methods::DRAIN_SUPERVISOR_EVENTS);
    assert_eq!(calls[1].2, serde_json::json!([256]));
    assert!(!calls[1].3);
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

#[cfg(test)]
#[path = "module_client/cleanup_tests.rs"]
mod cleanup_tests;

/// A port that models TinyMCP's module-wide lifecycle gate: a second
/// `open_configured` while another workspace's transition is still pending is
/// refused instead of queued.
#[derive(Clone, Default)]
struct SingleTransitionPort {
    pending: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl McpModulePort for SingleTransitionPort {
    async fn open_configured(
        &self,
        data_dir: &str,
        _config: &McpClientConfig,
    ) -> Result<OpenConfiguredResponse, ModuleCallError> {
        if self.pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(ModuleCallError::ModuleFault);
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok(OpenConfiguredResponse {
            object_path: format!("/workspace{data_dir}"),
            transition_id: "open".to_owned(),
        })
    }

    async fn close(&self, _data_dir: &str) -> Result<Option<String>, ModuleCallError> {
        Ok(None)
    }

    async fn transition_status(
        &self,
        _transition_id: &str,
    ) -> Result<TransitionStatus, ModuleCallError> {
        tokio::time::sleep(Duration::from_millis(5)).await;
        self.pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(TransitionStatus::Complete)
    }

    async fn call_object(
        &self,
        _object_path: &str,
        _member: &str,
        _args: Value,
        _confidential: bool,
    ) -> Result<Value, ModuleCallError> {
        Ok(Value::Bool(true))
    }

    async fn prepare_agent_tools(
        &self,
        _request: PrepareAgentToolsRequest,
    ) -> Result<Vec<McpToolPresentation>, ModuleCallError> {
        Ok(Vec::new())
    }

    async fn validate_server_arguments(
        &self,
        _request: ValidateServerArgumentsRequest,
    ) -> Result<ServerArgumentsValidation, ModuleCallError> {
        Err(ModuleCallError::IncompatibleContract)
    }
}

#[tokio::test]
async fn concurrent_workspaces_open_one_at_a_time_through_the_module_lifecycle_gate() {
    let port = SingleTransitionPort::default();
    let first = McpConfiguredClient::with_port("/private/a".to_owned(), Box::new(port.clone()));
    let second = McpConfiguredClient::with_port("/private/b".to_owned(), Box::new(port));
    let config = configured("server");

    let (a, b) = tokio::join!(
        first.call::<bool>(&config, "ConnectedOverview", ()),
        second.call::<bool>(&config, "ConnectedOverview", ()),
    );

    assert!(a.expect("first workspace opens"));
    assert!(b.expect("second workspace opens"));
}
