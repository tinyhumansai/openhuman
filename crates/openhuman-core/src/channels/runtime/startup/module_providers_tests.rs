use super::*;
use crate::config::test_env::EnvVarGuard;
use std::sync::Arc;
use std::time::Duration;
use tinychannels_bus::module_progressive::{
    ChannelOutputEvent, ProgressiveCommand, ProgressiveResult, ProgressiveTransportRequest,
    ProgressiveTransportResult,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn prepared_module_handle_keeps_progressive_send_alive() {
    let env_lock = EnvVarGuard::locked_async().await;
    let server = MockServer::start().await;
    let _env = env_lock.with("OPENHUMAN_TELEGRAM_BOT_API_BASE", server.uri());
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;

    Mock::given(method("POST"))
        .and(path("/bottoken/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": []
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/deleteWebhook"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": true
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottoken/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 991,
                "date": 1,
                "chat": { "id": 42, "type": "private" },
                "text": "done"
            }
        })))
        .expect(1..)
        .mount(&server)
        .await;

    let workspace = tempfile::tempdir().expect("temporary module workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.modules.allow_download = false;
    config.modules.source = crate::config::schema::ModuleSourceMode::Linked;
    config.channels_config.telegram = Some(tinychannels_bus::config::TelegramConfig {
        bot_token: "token".into(),
        chat_id: None,
        allowed_users: vec!["42".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1,
        silent_streaming: true,
        mention_only: false,
    });
    let context = Arc::new(OnceLock::new());
    let (dispatch, _receiver) = tokio::sync::mpsc::channel(4);
    let mut runtime = ModuleChannelRuntime::prepare(
        &crate::core::runtime::DispatchCtx::host_default(),
        Arc::new(config.clone()),
        context,
        dispatch,
    )
    .await
    .expect("compiled TinyBus module should prepare configured Telegram");
    assert!(runtime.channels().contains_key("telegram"));
    runtime
        .start(&config)
        .await
        .expect("mock Telegram provider starts without a network dependency");

    let scope = tinychannels_bus::ConversationRef {
        kind: tinychannels_bus::ConversationKind::Dm,
        id: "42".into(),
        ..Default::default()
    };
    let opened: ProgressiveResult = tokio::time::timeout(
        Duration::from_secs(10),
        runtime.lease.object().call_confidential(
            tinychannels_bus::methods::EXECUTE_PROGRESSIVE,
            (ProgressiveCommand::OpenOutput {
                channel: tinychannels_bus::ChannelRef {
                    id: "telegram".into(),
                    account_id: Some("telegram".into()),
                },
                conversation: scope,
                turn_id: "compiled-module-turn".into(),
                lifetime_ms: 30_000,
            },),
        ),
    )
    .await
    .expect("opening progressive output is bounded")
    .expect("module should open a private progressive handle");
    let ProgressiveResult::Opened { handle } = opened else {
        panic!("expected compiled module to return an output handle");
    };

    let completed: ProgressiveResult = tokio::time::timeout(
        Duration::from_secs(10),
        runtime.lease.object().call_confidential(
            tinychannels_bus::methods::EXECUTE_PROGRESSIVE,
            (ProgressiveCommand::FeedOutput {
                handle: handle.clone(),
                turn_id: None,
                channel_ref: None,
                conversation: None,
                sequence: None,
                events: vec![
                    ChannelOutputEvent::TextDelta {
                        text: "working".into(),
                        iteration: Some(1),
                    },
                    ChannelOutputEvent::FinalMessage {
                        text: "nested callback delivery".into(),
                        parts: Vec::new(),
                        receipt: None,
                    },
                ],
            },),
        ),
    )
    .await
    .expect("module-to-host-to-module send must not deadlock")
    .expect("the configured module adapter should deliver the final message");
    // Close the finished handle so the module does not report the turn as
    // cancelled when the runtime shuts down.
    let closed: ProgressiveResult = runtime
        .lease
        .object()
        .call_confidential(
            tinychannels_bus::methods::EXECUTE_PROGRESSIVE,
            (ProgressiveCommand::Close { handle },),
        )
        .await
        .expect("the finished handle closes");
    assert_eq!(closed, ProgressiveResult::Closed);
    let receipt = match completed {
        ProgressiveResult::DeliveryOutcome { outcome } => {
            assert!(outcome.error.is_none());
            assert_eq!(
                outcome.receipt.primary_platform_message_id.as_deref(),
                Some("991")
            );
            outcome.receipt
        }
        other => panic!("expected typed delivery outcome, got {other:?}"),
    };

    // The module owns provider edits and deletes now (the `EditMessage` and
    // `DeleteMessage` interactions are gone), so only the host's fail-closed
    // receipt scope check is exercised here against the compiled module object.
    let adapter = ModuleChannel {
        name: "telegram".into(),
        object: runtime.lease.object().clone(),
        interaction: tinychannels_bus::ChannelInteractionInfo {
            capabilities: Default::default(),
            proactive_target: None,
            supports_reactions: false,
            supports_draft_updates: false,
        },
        relay: None,
        relay_fronted: false,
    };
    let channel_ref = tinychannels_bus::ChannelRef {
        id: "telegram".into(),
        account_id: Some("telegram".into()),
    };
    let conversation = tinychannels_bus::ConversationRef {
        kind: tinychannels_bus::ConversationKind::Dm,
        id: "42".into(),
        ..Default::default()
    };
    let mut wrong_scope = conversation.clone();
    wrong_scope.kind = tinychannels_bus::ConversationKind::Group;
    let rejected = adapter
        .dispatch_progressive(ProgressiveTransportRequest::Delete {
            channel: "telegram".into(),
            turn_id: Some("compiled-module-turn".into()),
            channel_ref: Some(channel_ref),
            conversation: Some(wrong_scope),
            message_id: "991".into(),
            receipt: Some(receipt),
        })
        .await
        .expect("a mismatched scoped receipt should fail closed");
    assert!(matches!(rejected, ProgressiveTransportResult::Unavailable));

    let requests = server.received_requests().await.expect("mock requests");
    let send = requests
        .iter()
        .find(|request| request.url.path() == "/bottoken/sendMessage")
        .expect("one mock Telegram sendMessage request");
    let body: serde_json::Value = serde_json::from_slice(&send.body).expect("JSON request body");
    assert_eq!(body["chat_id"], "42");
    assert_eq!(body["text"], "nested callback delivery");
    server.verify().await;
    runtime
        .close()
        .await
        .expect("module runtime should shut down");
}

#[tokio::test]
async fn compiled_module_hosted_progressive_callback_is_scoped_and_typed() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    let _env_lock = EnvVarGuard::locked_async().await;
    let _storage = crate::channels::module_fixture::install_memory_storage();
    let client = crate::channels::module_fixture::client().await;
    let relay = Arc::new(crate::channels::providers::relay::RelayChannel::new(
        "telegram",
        "relay-fixture-client",
        "channel:telegram/sender-7/chat-42",
        "relay-fixture-request",
        "provider-message-9",
    ));
    let channel = tinychannels_bus::ChannelRef {
        id: "telegram".into(),
        account_id: Some("tenant-profile:7:telegram".into()),
    };
    let conversation = tinychannels_bus::ConversationRef {
        kind: tinychannels_bus::ConversationKind::Group,
        id: "chat-42".into(),
        parent_id: Some("chat-parent".into()),
        thread_id: Some("topic-3".into()),
        topic_id: Some("topic-3".into()),
        ..Default::default()
    };
    let mut web_events = crate::web_chat::subscribe_web_channel_events();
    let config = Config::default();
    let mut overlay = ContextOverlay::new(config.clone(), DomainSet::kernel(), ToolGroups::none())
        .session_agent("relay-agent");
    overlay.profile = Some("relay-profile".into());
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None).derive_with(overlay));
    let projection = Arc::new(
        crate::web_chat::Surface::new(
            &ctx,
            crate::web_chat::TurnIds {
                client_id: "relay-fixture-client".into(),
                thread_id: "channel:telegram/sender-7/chat-42".into(),
                request_id: "relay-fixture-request".into(),
            },
            config,
        )
        .await,
    );
    let callback = crate::channels::providers::relay::progressive::RelayProgressiveTransport::new(
        ctx,
        Arc::clone(&relay),
        channel.clone(),
        conversation.clone(),
        "relay-turn-1".into(),
        projection,
    );
    let runtime = ChannelRuntimeLease::open(client, callback)
        .await
        .expect("host callback runtime should attach to the compiled module");

    // The module accepts the hosted callback route without a locally
    // configured Telegram account, but a mismatched address is rejected by
    // the callback captured for this trusted tenant/conversation.
    let mut wrong_account = channel.clone();
    wrong_account.account_id = Some("tenant-profile:8:telegram".into());
    let rejected = run_hosted_compiled_turn(
        runtime.object(),
        wrong_account,
        conversation.clone(),
        "relay-turn-1",
        "must not escape the captured account",
    )
    .await;
    assert!(!crate::channels::runtime::dispatch::delivery_succeeded(
        &rejected
    ));
    assert!(relay.sent().is_empty());

    let mut wrong_conversation = conversation.clone();
    wrong_conversation.id = "chat-43".into();
    let rejected = run_hosted_compiled_turn(
        runtime.object(),
        channel.clone(),
        wrong_conversation,
        "relay-turn-1",
        "must not escape the captured conversation",
    )
    .await;
    assert!(!crate::channels::runtime::dispatch::delivery_succeeded(
        &rejected
    ));
    assert!(relay.sent().is_empty());

    let delivered = run_hosted_compiled_turn(
        runtime.object(),
        channel.clone(),
        conversation.clone(),
        "relay-turn-1",
        "typed hosted relay reply",
    )
    .await;
    // A hosted callback accepts delivery without a provider receipt: the
    // gateway reports receipts separately, so the module answers with the
    // distinct `hosted_accepted` result rather than an empty outcome.
    let ProgressiveResult::DeliveryBatch {
        outcomes,
        unsupported,
        failure: None,
        hosted_accepted: true,
    } = &delivered
    else {
        panic!("compiled hosted send should be accepted by the callback: {delivered:?}");
    };
    assert!(outcomes.is_empty() && unsupported.is_empty());
    assert!(crate::channels::runtime::dispatch::delivery_succeeded(
        &delivered
    ));
    assert_eq!(relay.sent(), vec!["typed hosted relay reply".to_owned()]);
    let mut projected = None;
    let mut terminal = None;
    tokio::time::timeout(Duration::from_secs(5), async {
        while projected.is_none() || terminal.is_none() {
            let event = web_events.recv().await.expect("web event bus");
            if event.context().request_id != "relay-fixture-request" {
                continue;
            }
            match event.event_name() {
                "chat_done" => projected = Some(event),
                crate::channels::providers::relay::CHANNEL_OUTBOUND_EVENT => terminal = Some(event),
                _ => panic!("unexpected relayed output event: {}", event.event_name()),
            }
        }
    })
    .await
    .expect("web projection and SaaS terminal both arrive without dropping either");
    let projected = projected.expect("compiled relay output is projected through the web adapter");
    let projected_wire = serde_json::to_value(&projected).expect("projected wire event");
    assert_eq!(projected.context().client_id, "relay-fixture-client");
    assert_eq!(
        projected.context().thread_id,
        "channel:telegram/sender-7/chat-42"
    );
    assert_eq!(projected.context().seq, Some(1));
    assert!(projected.belongs_to("relay-agent"));
    assert!(projected.belongs_to_profile("relay-profile"));
    assert_eq!(projected_wire["full_response"], "typed hosted relay reply");
    let terminal = terminal.expect("SaaS terminal delivery remains available");
    assert_eq!(terminal.context().client_id, "relay-fixture-client");
    assert!(terminal.belongs_to("relay-agent"));
    assert!(terminal.belongs_to_profile("relay-profile"));
    assert_eq!(relay.sent().len(), 1, "terminal output is not duplicated");
    runtime
        .close()
        .await
        .expect("hosted module runtime should close after the turn");
}

#[tokio::test]
async fn dropped_scoped_media_read_lease_aborts_the_compiled_module_handle() {
    let env_lock = EnvVarGuard::locked_async().await;
    let server = MockServer::start().await;
    let _env = env_lock.with("OPENHUMAN_TELEGRAM_BOT_API_BASE", server.uri());
    let _storage = crate::channels::module_fixture::install_memory_storage();
    crate::channels::module_fixture::client().await;

    for endpoint in ["getUpdates", "deleteWebhook"] {
        Mock::given(method("POST"))
            .and(path(format!("/bottoken/{endpoint}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": if endpoint == "getUpdates" { serde_json::json!([]) } else { serde_json::json!(true) }
            })))
            .mount(&server)
            .await;
    }

    let workspace = tempfile::tempdir().expect("temporary module workspace");
    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.modules.allow_download = false;
    config.modules.source = crate::config::schema::ModuleSourceMode::Linked;
    config.channels_config.telegram = Some(tinychannels_bus::config::TelegramConfig {
        bot_token: "token".into(),
        chat_id: None,
        allowed_users: vec!["42".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1,
        silent_streaming: true,
        mention_only: false,
    });
    let dispatch_ctx = crate::core::runtime::DispatchCtx::host_default();
    let (dispatch, _receiver) = tokio::sync::mpsc::channel(4);
    let mut runtime = ModuleChannelRuntime::prepare(
        &dispatch_ctx,
        Arc::new(config.clone()),
        Arc::new(OnceLock::new()),
        dispatch,
    )
    .await
    .expect("compiled TinyBus module should prepare configured Telegram");
    runtime.start(&config).await.expect("mock Telegram starts");

    let scope = tinychannels_bus::HostVoiceScope {
        configured_instance: "telegram".into(),
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: Some("telegram".into()),
        },
        conversation: tinychannels_bus::ConversationRef {
            kind: tinychannels_bus::ConversationKind::Dm,
            id: "42".into(),
            ..Default::default()
        },
        turn_id: "media-read-lease-test".into(),
    };
    let object = runtime.lease.object();
    let bytes = vec![b'x'; tinychannels_bus::module_media::MEDIA_READ_CHUNK_BYTES + 1];
    let begin: tinychannels_bus::MediaUploadHandle = object
        .call_confidential(
            tinychannels_bus::methods::BEGIN_MEDIA_UPLOAD,
            (tinychannels_bus::MediaUploadBegin {
                instance: "telegram".into(),
                scope: scope.clone(),
                kind: tinychannels_bus::MediaKind::Document,
                content_type: "text/plain".into(),
                file_name: Some("note.txt".into()),
                duration_ms: None,
                codec: None,
                byte_count: bytes.len(),
            },),
        )
        .await
        .expect("module accepts a scoped upload");
    let split = tinychannels_bus::module_media::MEDIA_UPLOAD_CHUNK_BYTES;
    object
        .call_confidential::<u32>(
            tinychannels_bus::methods::APPEND_MEDIA_UPLOAD_CHUNK,
            (tinychannels_bus::MediaUploadChunk {
                handle: begin.clone(),
                sequence: 0,
                bytes: bytes[..split].to_vec(),
            },),
        )
        .await
        .expect("module accepts the upload bytes");
    object
        .call_confidential::<u32>(
            tinychannels_bus::methods::APPEND_MEDIA_UPLOAD_CHUNK,
            (tinychannels_bus::MediaUploadChunk {
                handle: begin.clone(),
                sequence: 1,
                bytes: bytes[split..].to_vec(),
            },),
        )
        .await
        .expect("module accepts the final upload chunk");
    let media: tinychannels_bus::MediaReference = object
        .call_confidential(
            tinychannels_bus::methods::FINISH_MEDIA_UPLOAD,
            (tinychannels_bus::MediaUploadFinish(begin),),
        )
        .await
        .expect("module persists an opaque scoped media reference");

    let read: tinychannels_bus::MediaReadInfo = object
        .call_confidential(
            tinychannels_bus::methods::BEGIN_MEDIA_READ,
            (tinychannels_bus::MediaReadBegin {
                instance: "telegram".into(),
                scope,
                media,
                max_bytes: bytes.len() as u64,
            },),
        )
        .await
        .expect("module opens a scoped read handle");
    let first_chunk: tinychannels_bus::MediaReadChunkResult = object
        .call_confidential(
            tinychannels_bus::methods::READ_MEDIA_CHUNK,
            (tinychannels_bus::MediaReadChunk {
                handle: read.handle.clone(),
                sequence: 0,
            },),
        )
        .await
        .expect("read handle is usable before lease drop");
    assert!(!first_chunk.done);
    let replay: tinychannels_bus::MediaReadChunkResult = object
        .call_confidential(
            tinychannels_bus::methods::READ_MEDIA_CHUNK,
            (tinychannels_bus::MediaReadChunk {
                handle: read.handle.clone(),
                sequence: 0,
            },),
        )
        .await
        .expect("the module retains the last chunk for retry before lease drop");
    assert_eq!(replay, first_chunk);
    let handle = read.handle.clone();
    crate::channels::runtime::dispatch::drop_media_read_lease_for_test(
        runtime.lease.object().clone(),
        dispatch_ctx,
        read.handle,
    );

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let chunk = object
                .call_confidential::<tinychannels_bus::MediaReadChunkResult>(
                    tinychannels_bus::methods::READ_MEDIA_CHUNK,
                    (tinychannels_bus::MediaReadChunk {
                        handle: handle.clone(),
                        sequence: 0,
                    },),
                )
                .await;
            if let Err(error) = chunk {
                assert_eq!(error, tinybus::module::ModuleCallError::ModuleFault);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping the lease eventually aborts the module read handle");

    runtime
        .close()
        .await
        .expect("module runtime closes cleanly");
}

async fn run_hosted_compiled_turn(
    object: &tinybus::module::ModuleObjectClient,
    channel: tinychannels_bus::ChannelRef,
    conversation: tinychannels_bus::ConversationRef,
    turn_id: &str,
    text: &str,
) -> ProgressiveResult {
    let opened: ProgressiveResult = object
        .call_confidential(
            tinychannels_bus::methods::EXECUTE_PROGRESSIVE,
            (ProgressiveCommand::OpenHostedOutput {
                channel,
                conversation,
                turn_id: turn_id.into(),
                lifetime_ms: 30_000,
            },),
        )
        .await
        .expect("compiled module should open a hosted output handle");
    let ProgressiveResult::Opened { handle } = opened else {
        panic!("expected compiled module hosted output handle: {opened:?}");
    };
    let result: ProgressiveResult = object
        .call_confidential(
            tinychannels_bus::methods::EXECUTE_PROGRESSIVE,
            (ProgressiveCommand::FeedOutput {
                handle: handle.clone(),
                turn_id: None,
                channel_ref: None,
                conversation: None,
                sequence: None,
                events: vec![ChannelOutputEvent::FinalMessage {
                    text: text.into(),
                    parts: vec![],
                    receipt: None,
                }],
            },),
        )
        .await
        .expect("compiled module should return the typed callback outcome");
    object
        .call_confidential::<ProgressiveResult>(
            tinychannels_bus::methods::EXECUTE_PROGRESSIVE,
            (ProgressiveCommand::Close { handle },),
        )
        .await
        .expect("compiled module hosted output should close");
    result
}

#[path = "module_providers_hosted_tests.rs"]
mod hosted_tests;
