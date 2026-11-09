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
    statuses: Arc<Mutex<Vec<tinymcp_bus::ConnStatus>>>,
    fail_status: Arc<std::sync::atomic::AtomicBool>,
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
        if member == names::methods::STATUS {
            if self
                .fail_status
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(ModuleCallError::TransportFailed);
            }
            return serde_json::to_value(self.statuses.lock().unwrap().clone())
                .map_err(|_| ModuleCallError::ModuleFault);
        }
        Ok(Value::Bool(true))
    }

    async fn call_object_if_ready(
        &self,
        object_path: &str,
        member: &str,
        args: Value,
        confidential: bool,
    ) -> Result<Value, ModuleCallError> {
        self.call_object(object_path, member, args, confidential)
            .await
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
        annotations: Some(serde_json::json!({"readOnlyHint": true, "openWorldHint": false})),
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
    assert_eq!(
        calls[0].2[0]["tools"][0]["annotations"],
        serde_json::json!({"readOnlyHint": true, "openWorldHint": false})
    );
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
async fn doctor_status_reads_only_an_open_client_for_the_exact_config() {
    let scratch = tempfile::tempdir().expect("create isolated client workspace");
    let mut host_config = Config::default();
    host_config.workspace_dir = scratch.path().to_path_buf();
    let client_config = crate::mcp::host::client_config(&host_config).await;
    let port = FakePort::default();
    let opened = Arc::clone(&port.opened);
    let calls = Arc::clone(&port.calls);
    let expected = tinymcp_bus::ConnStatus {
        server_id: "docs".to_owned(),
        qualified_name: "example/docs".to_owned(),
        display_name: "Docs".to_owned(),
        status: tinymcp_bus::ServerStatus::Error,
        tool_count: 0,
        last_error: Some("private response body".to_owned()),
        auth_hint: None,
    };
    port.statuses.lock().unwrap().push(expected.clone());
    let client = McpConfiguredClient::install_test_port(&host_config, Box::new(port))
        .expect("install the test client without opening its module");
    let mut state = ConfiguredLifecycle::new();
    state.open_step(&client_config);
    state.open_replied(Ok(OpenConfiguredResponse {
        object_path: "/workspace/open".to_owned(),
        transition_id: "fixture-open".to_owned(),
    }));
    state.open_polled(TransitionStatus::Complete);
    *client.state.lock().await = state;

    let statuses = McpConfiguredClient::existing_status_for_config(&host_config)
        .await
        .expect("status call succeeds")
        .expect("the exact client is already open");

    assert_eq!(statuses, vec![expected]);
    assert!(
        opened.lock().unwrap().is_empty(),
        "doctor must not reopen MCP"
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, names::methods::STATUS);
    assert!(
        !calls[0].3,
        "status contains no credentials and is read-only"
    );
}

#[tokio::test]
async fn doctor_status_skips_mismatched_retiring_and_absent_clients_without_opening() {
    let scratch = tempfile::tempdir().expect("create isolated client workspace");
    let mut selected = Config::default();
    selected.workspace_dir = scratch.path().to_path_buf();
    selected.mcp_client.servers.push(
        tinymcp_bus::McpServerConfig {
            name: "selected-b".into(),
            ..Default::default()
        }
        .into(),
    );
    let mut stale = selected.clone();
    stale.mcp_client.servers[0].name = "ambient-a".into();
    let port = FakePort::default();
    let opened = Arc::clone(&port.opened);
    let calls = Arc::clone(&port.calls);
    let client = McpConfiguredClient::install_test_port(&selected, Box::new(port.clone()))
        .expect("install the selected workspace client");
    let mut state = ConfiguredLifecycle::new();
    state.open_step(&crate::mcp::host::client_config(&stale).await);
    state.open_replied(Ok(OpenConfiguredResponse {
        object_path: "/workspace/ambient-a".to_owned(),
        transition_id: "fixture-open".to_owned(),
    }));
    state.open_polled(TransitionStatus::Complete);
    *client.state.lock().await = state;

    assert!(McpConfiguredClient::existing_status_for_config(&selected)
        .await
        .expect("stale configuration is not a transport error")
        .is_none());
    assert!(calls.lock().unwrap().is_empty());

    client.retiring.store(true, Ordering::Release);
    assert_eq!(
        McpConfiguredClient::existing_status_for_config(&selected).await,
        Err(ModuleCallError::Unavailable)
    );
    assert!(calls.lock().unwrap().is_empty());
    assert!(opened.lock().unwrap().is_empty());

    let absent_root = tempfile::tempdir().expect("create absent-client workspace");
    let mut absent = Config::default();
    absent.workspace_dir = absent_root.path().to_path_buf();
    assert!(McpConfiguredClient::existing_status_for_config(&absent)
        .await
        .expect("absent client is a safe unavailable result")
        .is_none());
    let absent_key = lifecycle::client_key(absent_root.path());
    assert!(McpConfiguredClient::open_clients()
        .iter()
        .all(|(path, _)| path != &absent_key));
}

#[tokio::test]
async fn existing_status_preserves_read_failures_without_reopening_the_client() {
    let scratch = tempfile::tempdir().expect("create isolated client workspace");
    let mut config = Config::default();
    config.workspace_dir = scratch.path().to_path_buf();
    let client_config = crate::mcp::host::client_config(&config).await;
    let port = FakePort::default();
    port.fail_status
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let opened = Arc::clone(&port.opened);
    let calls = Arc::clone(&port.calls);
    let client = McpConfiguredClient::install_test_port(&config, Box::new(port))
        .expect("install fake existing client");
    let mut state = ConfiguredLifecycle::new();
    state.open_step(&client_config);
    state.open_replied(Ok(OpenConfiguredResponse {
        object_path: "/workspace/open".to_owned(),
        transition_id: "fixture-open".to_owned(),
    }));
    state.open_polled(TransitionStatus::Complete);
    *client.state.lock().await = state;

    assert_eq!(
        McpConfiguredClient::existing_status_for_config(&config).await,
        Err(ModuleCallError::TransportFailed)
    );
    assert!(opened.lock().unwrap().is_empty());
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, names::methods::STATUS);
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

#[path = "module_client/lifecycle_gate_tests.rs"]
mod lifecycle_gate_tests;

#[path = "module_client_cleanup_tests.rs"]
mod captured_cleanup_tests;
