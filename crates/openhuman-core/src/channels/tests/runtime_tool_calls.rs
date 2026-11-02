use super::super::context::{
    conversation_history_key, ChannelRouteSelection, ChannelRuntimeContext,
    CHANNEL_MESSAGE_TIMEOUT_SECS,
};
use super::super::runtime::process_channel_message;
use super::super::runtime::test_support::linked_telegram_runtime;
use super::common::{
    IterativeToolModel, MockPriceTool, ModelCaptureModel, RecordingChannel, ToolCallingModel,
};
use crate::channels::runtime::ChannelHandle;
use crate::channels::runtime::TestChannel;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tinychannels_bus as traits;
use tinyinference_llm::model::ChatModel;

#[tokio::test]
async fn process_channel_message_executes_native_tool_calls() {
    let _bus_guard = super::common::use_real_agent_handler().await;
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel = Arc::new(ChannelHandle::Test(channel_impl.clone()));

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels_by_name),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
            Arc::new(ToolCallingModel),
        )),
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
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
            id: "msg-2".to_string(),
            sender: "bob".to_string(),
            reply_target: "chat-84".to_string(),
            content: "What is the BTC price now?".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
            sender_name: None,
        },
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 1);
    assert!(sent_messages[0].starts_with("chat-84:"));
    assert!(sent_messages[0].contains("BTC is currently around"));
    assert!(!sent_messages[0].contains("mock_price"));
}

#[tokio::test]
async fn process_channel_message_handles_models_command_without_llm_call() {
    let _bus_guard = super::common::use_real_agent_handler().await;
    let linked = linked_telegram_runtime().await;
    crate::core::bus::init().await.expect("bus init");
    let _builtins =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins();
    let channel_impl = Arc::new(TestChannel::new("telegram"));
    let channel = Arc::new(ChannelHandle::Test(channel_impl.clone()));

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let default_provider_impl = Arc::new(ModelCaptureModel::new("default-model"));
    let default_provider: Arc<dyn ChatModel<()>> = default_provider_impl.clone();
    let fallback_provider_impl = Arc::new(ModelCaptureModel::new("fallback-model"));
    let fallback_provider: Arc<dyn ChatModel<()>> = fallback_provider_impl.clone();

    let mut provider_cache_seed = HashMap::new();
    provider_cache_seed.insert(
        "test-provider".to_string(),
        crate::agent::tinyagents::TurnModelSource::from_model(Arc::clone(&default_provider)),
    );
    provider_cache_seed.insert(
        "openrouter".to_string(),
        crate::agent::tinyagents::TurnModelSource::from_model(fallback_provider),
    );

    let mut runtime_ctx = ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels_by_name),
        module_objects: linked.module_objects(),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
            Arc::clone(&default_provider),
        )),
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 5,
        turn_model_source_cache: Arc::new(Mutex::new(provider_cache_seed)),
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
    let _core = crate::channels::runtime::test_support::bind_test_core_context(
        &mut runtime_ctx,
        linked.config(),
    );
    // `/models` offers the configured cloud providers; register the managed
    // backend so the command can switch to it.
    crate::channels::register_hosted_route_test_policy();
    if let Some(config) = runtime_ctx.config.as_mut() {
        Arc::make_mut(config)
            .cloud_providers
            .push(crate::config::schema::CloudProviderCreds {
                id: "p_managed_models_test".to_string(),
                slug: "openhuman".to_string(),
                label: "Managed Backend".to_string(),
                endpoint: "https://api.openhuman.ai/v1".to_string(),
                auth_style: crate::config::schema::AuthStyle::OpenhumanJwt,
                default_model: Some("default-model".to_string()),
            });
    }
    let runtime_ctx = Arc::new(runtime_ctx);

    let cmd_msg = traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "msg-cmd-1".to_string(),
        sender: "alice".to_string(),
        reply_target: "chat-1".to_string(),
        content: "/models openhuman".to_string(),
        channel: "telegram".to_string(),
        timestamp: 1,
        thread_ts: None,
        sender_name: None,
    };
    let route_key = conversation_history_key(&cmd_msg);
    process_channel_message(runtime_ctx.clone(), cmd_msg).await;

    let sent = channel_impl.sent_messages.lock().await;
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].contains("Provider switched to `openhuman`"),
        "the module-handled command should confirm its route change: {sent:?}"
    );

    let route = runtime_ctx
        .route_overrides
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&route_key)
        .cloned()
        .expect("route should be stored for sender");
    assert_eq!(route.provider, "openhuman");
    assert_eq!(route.model, "default-model");

    assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 0);
    assert_eq!(fallback_provider_impl.call_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn process_channel_message_uses_route_override_provider_and_model() {
    let _bus_guard = super::common::use_real_agent_handler().await;
    let channel_impl = Arc::new(TestChannel::new("telegram"));
    let channel = Arc::new(ChannelHandle::Test(channel_impl.clone()));

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let default_provider_impl = Arc::new(ModelCaptureModel::new("default-model"));
    let default_provider: Arc<dyn ChatModel<()>> = default_provider_impl.clone();
    let routed_provider_impl = Arc::new(ModelCaptureModel::new("route-model"));
    let routed_provider: Arc<dyn ChatModel<()>> = routed_provider_impl.clone();

    let mut provider_cache_seed = HashMap::new();
    provider_cache_seed.insert(
        "test-provider".to_string(),
        crate::agent::tinyagents::TurnModelSource::from_model(Arc::clone(&default_provider)),
    );
    provider_cache_seed.insert(
        "openrouter".to_string(),
        crate::agent::tinyagents::TurnModelSource::from_model(routed_provider),
    );

    let routed_msg = traits::ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "msg-routed-1".to_string(),
        sender: "alice".to_string(),
        reply_target: "chat-1".to_string(),
        content: "hello routed provider".to_string(),
        channel: "telegram".to_string(),
        timestamp: 2,
        thread_ts: None,
        sender_name: None,
    };
    let route_key = conversation_history_key(&routed_msg);
    let mut route_overrides = HashMap::new();
    route_overrides.insert(
        route_key,
        ChannelRouteSelection {
            provider: "openrouter".to_string(),
            model: "route-model".to_string(),
        },
    );

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels_by_name),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
            Arc::clone(&default_provider),
        )),
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
        model: Arc::new("default-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 5,
        turn_model_source_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(route_overrides)),
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

    process_channel_message(runtime_ctx, routed_msg).await;

    assert_eq!(default_provider_impl.call_count.load(Ordering::SeqCst), 0);
    assert_eq!(routed_provider_impl.call_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        routed_provider_impl
            .models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_slice(),
        &["route-model".to_string()]
    );
}

#[tokio::test]
async fn process_channel_message_respects_configured_max_tool_iterations_above_default() {
    let _bus_guard = super::common::use_real_agent_handler().await;
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel = Arc::new(ChannelHandle::Test(channel_impl.clone()));

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels_by_name),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
            Arc::new(IterativeToolModel {
                required_tool_iterations: 11,
            }),
        )),
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 12,
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
            id: "msg-iter-success".to_string(),
            sender: "alice".to_string(),
            reply_target: "chat-iter-success".to_string(),
            content: "Loop until done".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 1,
            thread_ts: None,
            sender_name: None,
        },
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 1);
    assert!(sent_messages[0].starts_with("chat-iter-success:"));
    assert!(sent_messages[0].contains("Completed after 11 tool iterations."));
    assert!(!sent_messages[0].contains("⚠️ Error:"));
}

#[tokio::test]
async fn process_channel_message_reports_configured_max_tool_iterations_limit() {
    let _bus_guard = super::common::use_real_agent_handler().await;
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel = Arc::new(ChannelHandle::Test(channel_impl.clone()));

    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(channel.name().to_string(), channel);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(channels_by_name),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(
            Arc::new(IterativeToolModel {
                required_tool_iterations: 20,
            }),
        )),
        default_provider: Arc::new("test-provider".to_string()),
        tools_registry: Arc::new(vec![Box::new(MockPriceTool)]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("test-system-prompt"),
        model: Arc::new("test-model".to_string()),
        temperature: 0.0,
        max_tool_iterations: 3,
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
            id: "msg-iter-fail".to_string(),
            sender: "bob".to_string(),
            reply_target: "chat-iter-fail".to_string(),
            content: "Loop forever".to_string(),
            channel: "test-channel".to_string(),
            timestamp: 2,
            thread_ts: None,
            sender_name: None,
        },
    )
    .await;

    let sent_messages = channel_impl.sent_messages.lock().await;
    assert_eq!(sent_messages.len(), 1);
    assert!(sent_messages[0].starts_with("chat-iter-fail:"));
    assert!(
        sent_messages[0].contains("maximum number of tool steps")
            && sent_messages[0].contains("retry the same question in this thread"),
        "the user-facing max-iteration copy should explain the cap and allow retrying: {sent_messages:?}"
    );
}
