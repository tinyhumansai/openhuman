use super::*;
use std::sync::Mutex;
use tinymcp_bus::ServerHostCall;

struct CallbackDrop(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for CallbackDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[derive(Default)]
struct PortState {
    opened: Option<ServerSessionConfig>,
    submitted: Option<ServerInput>,
    submit_count: usize,
    completed: Option<ServerHostReply>,
    complete_count: usize,
    lose_first_complete_reply: bool,
    cancelled: bool,
    unavailable: bool,
    polls: usize,
}

struct FixturePort(Mutex<PortState>);

#[async_trait]
impl ServerPort for FixturePort {
    async fn open(&self, config: ServerSessionConfig) -> Result<String, ModuleCallError> {
        let mut state = self.0.lock().unwrap();
        if state.unavailable {
            return Err(ModuleCallError::Unavailable);
        }
        state.opened = Some(config.clone());
        Ok(config.session_id)
    }

    async fn open_http(&self, config: ServerHttpSessionConfig) -> Result<String, ModuleCallError> {
        self.open(config.server).await
    }

    async fn submit(
        &self,
        _session: &str,
        _headers: Map<String, Value>,
        input: ServerInput,
    ) -> Result<(), ModuleCallError> {
        let mut state = self.0.lock().unwrap();
        state.submit_count += 1;
        if let Some(previous) = &state.submitted {
            assert_eq!(
                previous, &input,
                "retries must reuse the operation identity"
            );
        } else {
            state.submitted = Some(input);
        }
        Ok(())
    }

    async fn poll(&self, _session: &str) -> Result<ServerOperationSnapshot, ModuleCallError> {
        let mut state = self.0.lock().unwrap();
        state.polls += 1;
        let input = state.submitted.clone().unwrap();
        if state.completed.is_some() {
            Ok(ServerOperationSnapshot {
                operation_id: input.operation_id,
                state: ServerOperationState::Complete {
                    response: Some("{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}".to_owned()),
                },
                http_response: None,
            })
        } else {
            Ok(ServerOperationSnapshot {
                operation_id: input.operation_id,
                state: ServerOperationState::Callback {
                    callback: tinymcp_bus::ServerCallback {
                        id: "callback-1".to_owned(),
                        call: ServerHostCall::ReadResource {
                            source_type: "mcp:test".to_owned(),
                            headers: Map::new(),
                            uri: "openhuman://prompts/identity".to_owned(),
                        },
                    },
                },
                http_response: None,
            })
        }
    }

    async fn complete(
        &self,
        _session: &str,
        _callback: &str,
        reply: ServerHostReply,
    ) -> Result<(), ModuleCallError> {
        let mut state = self.0.lock().unwrap();
        state.complete_count += 1;
        if state.lose_first_complete_reply && state.complete_count == 1 {
            return Err(ModuleCallError::TransportFailed);
        }
        if let Some(previous) = &state.completed {
            assert_eq!(previous, &reply, "callback replay must reuse its reply");
        } else {
            state.completed = Some(reply);
        }
        Ok(())
    }

    async fn cancel(&self, _operation: ServerOperationRef) -> Result<(), ModuleCallError> {
        self.0.lock().unwrap().cancelled = true;
        Ok(())
    }

    async fn close(&self, _session: &str) -> Result<(), ModuleCallError> {
        Ok(())
    }

    async fn http_session(
        &self,
        _request: ServerHttpSessionRequest,
    ) -> Result<ServerHttpResponseMetadata, ModuleCallError> {
        unreachable!()
    }
}

#[tokio::test]
async fn server_session_completes_host_resource_callback() {
    let port = Arc::new(FixturePort(Mutex::new(PortState::default())));
    let session = ServerSession::open_with_port(port.clone()).await.unwrap();
    assert!(!Uuid::parse_str(session.id()).unwrap().is_nil());

    let snapshot = session
        .submit(
            Map::new(),
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/read"}"#.to_owned(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(matches!(
        snapshot.state,
        ServerOperationState::Complete { .. }
    ));
    let state = port.0.lock().unwrap();
    assert!(matches!(
        state.completed,
        Some(ServerHostReply::Success { ref value }) if value["contents"][0]["uri"] == "openhuman://prompts/identity"
    ));
    let opened = state.opened.as_ref().unwrap();
    assert_eq!(opened.source_type_prefix, "mcp");
    assert_eq!(opened.info["name"], "openhuman-core");
}

#[tokio::test]
async fn retry_after_lost_callback_reply_reuses_operation_and_host_reply() {
    let port = Arc::new(FixturePort(Mutex::new(PortState {
        lose_first_complete_reply: true,
        ..PortState::default()
    })));
    let session = ServerSession::open_with_port(port.clone()).await.unwrap();
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"openhuman://prompts/identity"}}"#;

    let first = session
        .submit(Map::new(), line.to_owned(), &CancellationToken::new())
        .await;
    assert!(matches!(
        first,
        Err(ServerError::Module(ModuleCallError::TransportFailed))
    ));

    let retried = session
        .submit(Map::new(), line.to_owned(), &CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(
        retried.state,
        ServerOperationState::Complete { .. }
    ));

    let state = port.0.lock().unwrap();
    assert_eq!(state.submit_count, 2);
    assert_eq!(state.complete_count, 2);
    assert_eq!(state.submitted.as_ref().unwrap().line, line);
    assert!(matches!(
        state.completed,
        Some(ServerHostReply::Success { ref value })
            if value["contents"][0]["uri"] == "openhuman://prompts/identity"
    ));
}

#[tokio::test]
async fn server_session_cancellation_reaches_module_owner() {
    let port = Arc::new(FixturePort(Mutex::new(PortState::default())));
    let session = ServerSession::open_with_port(port.clone()).await.unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = session
        .submit(Map::new(), "{}".to_owned(), &cancellation)
        .await
        .unwrap_err();

    assert!(matches!(error, ServerError::Cancelled));
    assert!(port.0.lock().unwrap().cancelled);
}

#[tokio::test]
async fn closing_session_aborts_and_joins_a_running_host_callback() {
    let port = Arc::new(FixturePort(Mutex::new(PortState::default())));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let (dropped_tx, mut dropped_rx) = tokio::sync::oneshot::channel();
    let dropped_tx = Arc::new(Mutex::new(Some(dropped_tx)));
    let release = CancellationToken::new();
    let dispatch = {
        let started_tx = Arc::clone(&started_tx);
        let dropped_tx = Arc::clone(&dropped_tx);
        let release = release.clone();
        move |_call| {
            let started = started_tx.lock().unwrap().take();
            let dropped = dropped_tx.lock().unwrap().take();
            let release = release.clone();
            async move {
                let _drop_notice = CallbackDrop(dropped);
                if let Some(started) = started {
                    let _ = started.send(());
                }
                release.cancelled().await;
                ServerHostReply::Success { value: Value::Null }
            }
        }
    };
    let session = Arc::new(
        ServerSession::open_with_port_and_dispatch(port, dispatch)
            .await
            .unwrap(),
    );
    let submit_session = Arc::clone(&session);
    let submit = tokio::spawn(async move {
        submit_session
            .submit(
                Map::new(),
                r#"{"jsonrpc":"2.0","id":1,"method":"resources/read"}"#.to_owned(),
                &CancellationToken::new(),
            )
            .await
    });
    started_rx.await.expect("host callback started");

    session.close().await.unwrap();
    let callback_was_joined = dropped_rx.try_recv().is_ok();
    // Let the pre-fix implementation finish its detached task so a RED run
    // does not leave work running after the test returns.
    release.cancel();
    let _ = submit.await.unwrap();

    assert!(
        callback_was_joined,
        "terminal session close must abort and join an in-flight host callback"
    );
}

#[tokio::test]
async fn server_session_fails_closed_when_module_is_unavailable() {
    let port = Arc::new(FixturePort(Mutex::new(PortState {
        unavailable: true,
        ..PortState::default()
    })));

    let error = match ServerSession::open_with_port(port).await {
        Ok(_) => panic!("unavailable module unexpectedly opened"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        ServerError::Module(ModuleCallError::Unavailable)
    ));
}
