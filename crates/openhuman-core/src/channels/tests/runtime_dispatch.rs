use super::super::context::{ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS};
use super::super::runtime::test_support::{
    linked_telegram_runtime, run_dispatch_harness, DispatchHarnessOptions,
};
use super::super::runtime::{
    process_channel_message, process_channel_runtime_message, run_message_dispatch_loop,
    RuntimeChannelMessage,
};
use super::common::{use_real_agent_handler, RecordingChannel, SlowModel};
use crate::agent::bus::{mock_agent_run_turn, AgentTurnRequest, AgentTurnResponse};
use crate::channels::runtime::ChannelHandle;
use crate::core::events::DomainEvent;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tinychannels_bus as traits;
use tokio::sync::Barrier;

#[tokio::test]
async fn message_dispatch_processes_messages_in_parallel() {
    // Install a deterministic stub that takes 250ms per turn and records
    // the peak number of in-flight turns. This proves concurrency directly
    // without relying on wall-clock thresholds that can wobble in CI.
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak_in_flight = Arc::new(AtomicUsize::new(0));
    let handler_barrier = Arc::new(Barrier::new(2));
    let _bus_guard = mock_agent_run_turn({
        let in_flight = in_flight.clone();
        let peak_in_flight = peak_in_flight.clone();
        let handler_barrier = handler_barrier.clone();
        move |_req: AgentTurnRequest| {
            let in_flight = in_flight.clone();
            let peak_in_flight = peak_in_flight.clone();
            let handler_barrier = handler_barrier.clone();
            async move {
                let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak_in_flight.fetch_max(current, Ordering::SeqCst);
                tokio::time::timeout(Duration::from_secs(2), handler_barrier.wait())
                    .await
                    .map_err(|_| "parallel dispatch handler barrier timed out".to_string())?;
                tokio::time::sleep(Duration::from_millis(250)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                Ok(AgentTurnResponse::new("echo: stub"))
            }
        }
    })
    .await;

    let build_runtime = || {
        let channel_impl = Arc::new(RecordingChannel::default());
        let channel = Arc::new(ChannelHandle::Test(channel_impl.clone()));

        let mut channels_by_name = HashMap::new();
        channels_by_name.insert(channel.name().to_string(), channel);

        let runtime_ctx = Arc::new(ChannelRuntimeContext {
            relay_thread: None,
            channels_by_name: Arc::new(channels_by_name),
            module_objects: Arc::new(HashMap::new()),
            turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
                Arc::new(SlowModel {
                    delay: Duration::from_millis(5),
                }),
            )),
            default_provider: Arc::new("test-provider".to_string()),
            tools_registry: Arc::new(vec![]),
            system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
            model: Arc::new("test-model".to_string()),
            temperature: 0.0,
            max_tool_iterations: 10,
            turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_url: None,
            inference_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            multimodal: crate::config::MultimodalConfig::default(),
            multimodal_files: crate::config::MultimodalFileConfig::default(),
            config: None,
            core: None,
        });

        (channel_impl, runtime_ctx)
    };

    let (parallel_channel, parallel_ctx) = build_runtime();
    let (tx, rx) = tokio::sync::mpsc::channel::<RuntimeChannelMessage>(4);
    tx.send(RuntimeChannelMessage::from(traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "1".to_string(),
        sender: "alice".to_string(),
        reply_target: "alice".to_string(),
        content: "hello".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 1,
        thread_ts: None,
        sender_name: None,
    }))
    .await
    .unwrap();
    tx.send(RuntimeChannelMessage::from(traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "2".to_string(),
        sender: "bob".to_string(),
        reply_target: "bob".to_string(),
        content: "world".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 2,
        thread_ts: None,
        sender_name: None,
    }))
    .await
    .unwrap();
    drop(tx);

    run_message_dispatch_loop(
        rx,
        parallel_ctx,
        2,
        crate::config::InboundDebounceConfig::disabled(),
    )
    .await;
    assert_eq!(peak_in_flight.load(Ordering::SeqCst), 2);

    let sent_messages = parallel_channel.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 2);
}

#[tokio::test]
async fn process_channel_message_sends_typing_before_the_provider_reply() {
    let _bus_guard = use_real_agent_handler().await;
    crate::core::bus::init().await.expect("bus init");
    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("built-in host agent definitions should load");
    let linked = linked_telegram_runtime().await;
    let channels_by_name = linked.channels();

    let mut runtime_ctx = ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name,
        module_objects: linked.module_objects(),
        turn_model_source: None,
        default_provider: Arc::new("openai".to_string()),
        tools_registry: Arc::new(vec![]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
        model: Arc::new("channel-test-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 10,
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(linked.config().workspace_dir.clone()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    };
    let _core =
        crate::channels::runtime::test_support::bind_test_core_context_with_provider_endpoint(
            &mut runtime_ctx,
            linked.config(),
            &linked.provider_endpoint(),
        );
    let runtime_ctx = Arc::new(runtime_ctx);
    let configured_account = runtime_ctx
        .channels_by_name
        .keys()
        .find(|name| tinychannels_bus::provider_id(name) == "telegram")
        .cloned()
        .expect("linked Telegram should have a configured account identity");
    let message = traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "typing-msg".to_string(),
        sender: "alice".to_string(),
        reply_target: "42".to_string(),
        content: "hello".to_string(),
        channel: "telegram".to_string(),
        timestamp: 1,
        thread_ts: None,
        sender_name: None,
    };
    let envelope = tinychannels_bus::ChannelInboundEnvelope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: Some(configured_account),
        },
        message_id: message.id.clone(),
        conversation: tinychannels_bus::ConversationRef {
            kind: tinychannels_bus::ConversationKind::Dm,
            id: message.reply_target.clone(),
            ..Default::default()
        },
        sender: tinychannels_bus::SenderRef {
            id: message.sender.clone(),
            name: message.sender_name.clone(),
            ..Default::default()
        },
        timestamp_unix_ms: Some(message.timestamp * 1000),
        text: message.content.clone(),
        parts: vec![tinychannels_bus::ContentPart::Text {
            text: message.content.clone(),
            markdown_dialect: None,
        }],
        ..Default::default()
    };
    let prepared = crate::modules::channel_controller::prepare_inbound(
        &crate::channels::context::module_client(&runtime_ctx).await,
        tinychannels_bus::module_inbound::PrepareChannelInboundRequest {
            source: tinychannels_bus::module_inbound::ChannelInboundSource::Envelope {
                envelope,
                timestamp: message.timestamp,
            },
            namespace: "main".into(),
            session_policy: Default::default(),
            preparation_mode: Default::default(),
        },
    )
    .await
    .expect("linked module should prepare the trusted configured channel input");
    process_channel_runtime_message(
        runtime_ctx,
        RuntimeChannelMessage {
            message: prepared.message.clone(),
            inbound_envelope: Some(prepared.envelope.clone()),
            authenticated_relay: false,
            prepared_inbound: Some(prepared),
            imported_history: Vec::new(),
            legacy_history: None,
            progressive_target: None,
        },
    )
    .await;

    let requests = linked.provider_request_paths().await;
    // The host drives typing through the module's `StartTyping` interaction
    // for the factory-built adapter, and it must precede the reply.
    assert_eq!(
        requests
            .iter()
            .filter(|path| path.ends_with("/sendMessage"))
            .count(),
        1,
        "the module-backed provider should receive the completed reply; got {requests:?}"
    );
    let typing_indices = requests
        .iter()
        .enumerate()
        .filter(|(_, path)| path.ends_with("/sendChatAction"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let reply_index = requests
        .iter()
        .position(|path| path.ends_with("/sendMessage"))
        .expect("provider should receive the model reply");
    assert!(
        !typing_indices.is_empty(),
        "the host should start typing on the module-backed provider: {requests:?}"
    );
    assert!(
        typing_indices.iter().all(|index| *index < reply_index),
        "typing should precede reply: {requests:?}"
    );
    let sent = linked
        .provider_request_bodies("/bottoken/sendMessage")
        .await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["text"], "typing fixture reply");
}

/// Integration test that proves channel dispatch actually routes through
/// the native bus: registers a stub `agent.run_turn` handler that returns
/// a canned response, drives a real `ChannelRuntimeContext` through
/// `process_channel_message`, and asserts that the stubbed response was
/// the one delivered to the channel.
///
/// This is the end-to-end coverage that closes the decoupling loop — if
/// `dispatch.rs` ever reverts to calling `run_tool_call_loop` directly,
/// this test will start failing because the stub handler won't be invoked.
#[tokio::test]
async fn dispatch_routes_through_agent_run_turn_bus_handler() {
    // Install a typed stub for `agent.run_turn` via the shared
    // `mock_agent_run_turn` helper. The returned guard holds the
    // workspace-wide bus handler lock and re-registers the production
    // handler on drop — no manual lock juggling or restoration.
    let stub_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stub_calls_for_handler = Arc::clone(&stub_calls);
    let _bus_guard = mock_agent_run_turn(move |req| {
        let stub_calls = Arc::clone(&stub_calls_for_handler);
        async move {
            stub_calls.fetch_add(1, Ordering::SeqCst);
            // Basic sanity on the payload the dispatch built for us.
            assert_eq!(req.channel_name, "test-channel");
            assert_eq!(req.provider_name, "test-provider");
            assert_eq!(req.model, "test-model");
            assert!(
                req.history.len() >= 2,
                "history should include at least the system prompt and user message"
            );
            Ok(AgentTurnResponse::new("CANNED_RESPONSE_FROM_BUS_STUB"))
        }
    })
    .await;

    let channel_impl = Arc::new(RecordingChannel::default());
    let channel = Arc::new(ChannelHandle::Test(channel_impl.clone()));

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels_by_name),
        // Still need a model for the context field, but the stubbed bus
        // handler never invokes it — so a minimal no-op is fine.
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
            Arc::new(super::common::DummyModel),
        )),
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 10,
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    });

    process_channel_message(
        runtime_ctx,
        traits::ChannelMessage {
            reply_to_id: None,
            topic_id: None,
            id: "bus-stub-msg".to_string(),
            sender: "alice".to_string(),
            reply_target: "alice".to_string(),
            content: "hello from bus test".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            sender_name: None,
        },
    )
    .await;

    // The stub must have been called exactly once.
    assert_eq!(
        stub_calls.load(Ordering::SeqCst),
        1,
        "channel dispatch must route through `agent.run_turn` native bus handler"
    );

    // And the canned response must have reached the channel.
    let sent = channel_impl.sent_messages.lock().await;
    assert_eq!(sent.len(), 1, "expected one message delivered");
    assert!(
        sent[0].contains("CANNED_RESPONSE_FROM_BUS_STUB"),
        "delivered message should contain the stubbed text, got {:?}",
        sent[0]
    );

    // No manual restore — dropping `_bus_guard` re-registers the
    // production `agent.run_turn` handler automatically so the next test
    // that expects the real path sees a consistent registry.
}

/// Start draining the bus for the `ChannelMessageProcessed` event of
/// `message_id` and hand it back through the returned handle.
///
/// The drain must run *while* the message is processed, not after: the
/// process-wide bus is a bounded broadcast ring (`SIGNAL_BUFFER`), the full
/// lib suite publishes from hundreds of parallel tests, and a receiver that
/// is only polled once `process_channel_message` returns has its target event
/// overwritten by then (the receiver reports `Lagged` and skips it). A fixed
/// event-count budget on the scan has the same failure. Only wall-clock time
/// bounds the wait.
fn watch_for_channel_processed(message_id: &str) -> tokio::task::JoinHandle<DomainEvent> {
    let mut events = crate::core::bus::BUS
        .get()
        .expect("event bus should be initialized")
        .receiver();
    let message_id = message_id.to_string();
    crate::core::runtime::DispatchCtx::ambient().spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let event = tokio::time::timeout_at(deadline, events.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!("did not observe ChannelMessageProcessed for {message_id}")
                })
                .expect("the bus closed before the expected event arrived");
            if matches!(
                &event,
                DomainEvent::ChannelMessageProcessed { message_id: id, .. } if *id == message_id
            ) {
                return event;
            }
        }
    })
}

fn unrelated_processed_event(n: usize) -> DomainEvent {
    DomainEvent::ChannelMessageProcessed {
        channel: "noise".into(),
        message_id: format!("noise-{n}"),
        sender: "noise".into(),
        reply_target: "noise".into(),
        content: String::new(),
        thread_ts: None,
        response: String::new(),
        provider: "noise".into(),
        model: "noise".into(),
        elapsed_ms: 0,
        success: true,
        workspace_dir: std::env::temp_dir(),
    }
}

async fn resolved_route_ctx(
    send_error: Option<&str>,
) -> (Arc<ChannelRuntimeContext>, Arc<RecordingChannel>) {
    let mut channel_state = RecordingChannel::default();
    channel_state.send_error = send_error.map(str::to_owned);
    let channel_impl = Arc::new(channel_state);
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_impl)));
    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    (
        Arc::new(ChannelRuntimeContext {
            relay_thread: None,
            channels_by_name: Arc::new(channels_by_name),
            module_objects: Arc::new(HashMap::new()),
            turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
                Arc::new(super::common::DummyModel),
            )),
            default_provider: Arc::new("requested-provider".to_string()),
            tools_registry: Arc::new(vec![]),
            system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
            model: Arc::new("requested-model".to_string()),
            temperature: 0.0,
            max_tool_iterations: 10,
            turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
            route_overrides: Arc::new(Mutex::new(HashMap::new())),
            api_url: None,
            inference_url: None,
            reliability: Arc::new(crate::config::ReliabilityConfig::default()),
            workspace_dir: Arc::new(std::env::temp_dir()),
            message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            multimodal: crate::config::MultimodalConfig::default(),
            multimodal_files: crate::config::MultimodalFileConfig::default(),
            config: None,
            core: None,
        }),
        channel_impl,
    )
}

async fn run_resolved_route_message(
    message_id: &str,
    noise_after: usize,
    send_error: Option<&str>,
) {
    crate::core::bus::init().await.expect("bus init");
    let watcher = watch_for_channel_processed(message_id);

    let _bus_guard = mock_agent_run_turn(move |_req| async move {
        Ok(AgentTurnResponse::with_resolved_route(
            "CANNED_RESPONSE_FROM_RESOLVED_ROUTE",
            "actual-provider",
            "actual-model",
        ))
    })
    .await;

    let (ctx, channel) = resolved_route_ctx(send_error).await;
    process_channel_message(
        ctx,
        traits::ChannelMessage {
            reply_to_id: None,
            topic_id: None,
            id: message_id.to_string(),
            sender: "alice".to_string(),
            reply_target: "alice".to_string(),
            content: "hello from resolved route test".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            sender_name: None,
        },
    )
    .await;

    // Other tests share this bus: model a burst of their traffic landing
    // right after our event, more than the ring holds.
    tokio::task::yield_now().await;
    // Paced in small bursts so other tests' receivers on the shared bus keep
    // draining while this one floods it.
    for n in 0..noise_after {
        crate::core::bus::BUS.publish(unrelated_processed_event(n));
        if n % 16 == 15 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    let event = watcher.await.expect("watcher task");
    let DomainEvent::ChannelMessageProcessed {
        provider,
        model,
        response,
        success,
        ..
    } = event
    else {
        unreachable!("wait_for_channel_processed returns only ChannelMessageProcessed");
    };
    assert_eq!(success, send_error.is_none());
    assert_eq!(response, "CANNED_RESPONSE_FROM_RESOLVED_ROUTE");
    assert_eq!(provider, "actual-provider");
    assert_eq!(model, "actual-model");
    let sends = channel.sent.lock().expect("test sends lock");
    if send_error.is_some() {
        assert!(
            sends.is_empty(),
            "a failed send must not be recorded as delivered"
        );
    } else {
        assert_eq!(sends.len(), 1, "successful reply delivery must be recorded");
    }
}

#[tokio::test]
async fn channel_processed_event_records_resolved_agent_route() {
    run_resolved_route_message("resolved-route-msg", 0, None).await;
}

#[tokio::test]
async fn channel_processed_event_reports_failed_reply_delivery() {
    run_resolved_route_message("failed-reply-delivery-msg", 0, Some("send failed")).await;
}

/// Regression: the event used to be read only after the message finished, by
/// which time a burst of other tests' events had overwritten it in the bounded
/// bus ring, so the test failed only in the full parallel suite.
#[tokio::test]
async fn channel_processed_event_is_found_behind_a_flood_of_unrelated_events() {
    run_resolved_route_message("resolved-route-flood-msg", 600, None).await;
}

/// Security regression for the `[FILE:…]` smuggling vector: a remote
/// channel user (Slack/Discord/Telegram/WhatsApp/etc) putting
/// `[FILE:/etc/passwd]` (or any other local-path marker) into a normal
/// message must NOT result in a file read. `process_channel_message`
/// MUST override the operator-supplied `ctx.multimodal_files` with the
/// hardened `MultimodalFileConfig::for_untrusted_channel_input()` so
/// `prepare_messages_for_provider` rejects the marker with
/// `TooManyFiles` before any disk access.
#[tokio::test]
async fn dispatch_replies_with_timeout_notice_when_handler_exceeds_deadline() {
    let timed_out = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "test-channel".to_string(),
        content: "force timeout".to_string(),
        handler_delay_ms: 31_000,
        timeout_secs: tinychannels_bus::module_runtime::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS,
        ..Default::default()
    })
    .await;
    assert!(timed_out.sends.iter().any(|send| {
        send.content.contains("time budget")
            && send.content.contains("retry your question in this thread")
    }));
}
