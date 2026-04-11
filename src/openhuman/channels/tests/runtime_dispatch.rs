use super::super::context::{ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS};
use super::super::runtime::{process_channel_message, run_message_dispatch_loop};
use super::super::{traits, Channel};
use super::common::{use_real_agent_handler, NoopMemory, RecordingChannel, SlowProvider};
use crate::openhuman::agent::bus::{mock_agent_run_turn, AgentTurnResponse};
use crate::openhuman::providers;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[tokio::test]
async fn message_dispatch_processes_messages_in_parallel() {
    // Install the real `agent.run_turn` handler and hold the shared bus
    // lock for the whole test so no parallel stub-installing test can
    // clobber the handler mid-dispatch.
    let _bus_guard = use_real_agent_handler().await;

    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(SlowProvider {
            delay: Duration::from_millis(250),
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
        provider_runtime_options: providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::openhuman::config::MultimodalConfig::default(),
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(4);
    tx.send(traits::ChannelMessage {
        id: "1".to_string(),
        sender: "alice".to_string(),
        reply_target: "alice".to_string(),
        content: "hello".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 1,
        thread_ts: None,
    })
    .await
    .unwrap();
    tx.send(traits::ChannelMessage {
        id: "2".to_string(),
        sender: "bob".to_string(),
        reply_target: "bob".to_string(),
        content: "world".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 2,
        thread_ts: None,
    })
    .await
    .unwrap();
    drop(tx);

    let started = Instant::now();
    run_message_dispatch_loop(rx, runtime_ctx, 2).await;
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(430),
        "expected parallel dispatch (<430ms), got {:?}",
        elapsed
    );

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 2);
}

#[tokio::test]
async fn process_channel_message_cancels_scoped_typing_task() {
    let _bus_guard = use_real_agent_handler().await;
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider: Arc::new(SlowProvider {
            delay: Duration::from_millis(20),
        }),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
        provider_runtime_options: providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::openhuman::config::MultimodalConfig::default(),
    });

    process_channel_message(
        runtime_ctx,
        traits::ChannelMessage {
            id: "typing-msg".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-typing".to_string(),
            content: "hello".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
        },
    )
    .await;

    let starts = channel_impl.start_typing_calls.load(Ordering::SeqCst);
    let stops = channel_impl.stop_typing_calls.load(Ordering::SeqCst);
    assert_eq!(starts, 1, "start_typing should be called once");
    assert_eq!(stops, 1, "stop_typing should be called once");
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
            Ok(AgentTurnResponse {
                text: "CANNED_RESPONSE_FROM_BUS_STUB".to_string(),
            })
        }
    })
    .await;

    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        // Still need a Provider for the Arc field, but the stubbed bus
        // handler never invokes it — so a minimal no-op is fine.
        provider: Arc::new(super::common::DummyProvider),
        default_provider: Arc::new("test-provider".to_string()),
        memory: Arc::new(NoopMemory),
        tools_registry: Arc::new(vec![]),
        system_prompt: Arc::new("test-system-prompt".to_string()),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 10,
        min_relevance_score: 0.0,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: None,
        api_url: None,
        reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
        provider_runtime_options: providers::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(std::env::temp_dir()),
        message_timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
        multimodal: crate::openhuman::config::MultimodalConfig::default(),
    });

    process_channel_message(
        runtime_ctx,
        traits::ChannelMessage {
            id: "bus-stub-msg".to_string(),
            sender: "alice".to_string(),
            reply_target: "alice".to_string(),
            content: "hello from bus test".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
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
