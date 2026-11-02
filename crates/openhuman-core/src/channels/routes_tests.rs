use super::*;
use crate::channels::context::{
    conversation_history_key, ChannelRuntimeContext, RouteSelectionMap, TurnModelSourceCacheMap,
};
use crate::channels::host::ChannelTurnStateSubscriber;
use crate::core::events::DomainEvent;
use async_trait::async_trait;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use tinybus::EventHandler;
use tinychannels_bus::ChannelMessage;
use tinytools::{Tool, ToolResult};

struct DummyTool;

#[async_trait]
impl Tool for DummyTool {
    fn name(&self) -> &str {
        "dummy"
    }

    fn description(&self) -> &str {
        "dummy"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

use crate::channels::runtime::{ChannelHandle, TestChannel};

pub(super) fn runtime_context(workspace_dir: PathBuf) -> ChannelRuntimeContext {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "ok",
        ]));
    ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(HashMap::new()),
        module_objects: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(model)),
        default_provider: Arc::new("openai".into()),
        tools_registry: Arc::new(vec![Box::new(DummyTool) as Box<dyn Tool>]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("prompt"),
        model: Arc::new("reasoning-v1".into()),
        temperature: 0.0,
        max_tool_iterations: 1,
        turn_model_source_cache: TurnModelSourceCacheMap::default(),
        route_overrides: RouteSelectionMap::default(),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        workspace_dir: Arc::new(workspace_dir),
        message_timeout_secs: 60,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    }
}

#[test]
fn configured_control_channel_requires_the_prepared_account_identity() {
    use tinychannels_bus::module_remote::RemoteSessionScope;

    let mut ctx = runtime_context(PathBuf::from("/tmp"));
    let mut scope = RemoteSessionScope {
        channel: tinychannels_bus::ChannelRef {
            id: "telegram".into(),
            account_id: None,
        },
        conversation: tinychannels_bus::ConversationRef {
            id: "chat-1".into(),
            ..Default::default()
        },
        session_key: "session-1".into(),
    };
    let handle = |name: &str| Arc::new(ChannelHandle::Test(Arc::new(TestChannel::new(name))));

    ctx.channels_by_name = Arc::new(HashMap::from([(
        "telegram:bot-a".into(),
        handle("telegram:bot-a"),
    )]));
    assert!(configured_control_channel(&ctx, Some(&scope)).is_none());

    ctx.channels_by_name = Arc::new(HashMap::from([
        ("telegram:bot-a".into(), handle("telegram:bot-a")),
        ("telegram:bot-b".into(), handle("telegram:bot-b")),
    ]));
    assert!(configured_control_channel(&ctx, Some(&scope)).is_none());

    scope.channel.account_id = Some("telegram:bot-b".into());
    let configured = configured_control_channel(&ctx, Some(&scope));
    assert_eq!(
        configured.as_deref().map(|channel| channel.name.as_str()),
        Some("telegram:bot-b")
    );
    scope.channel.account_id = Some("telegram:bot-a".into());
    scope.channel.id = "discord".into();
    assert!(configured_control_channel(&ctx, Some(&scope)).is_none());
}

async fn handle_linked_runtime_command(
    ctx: &mut ChannelRuntimeContext,
    msg: &ChannelMessage,
    target_channel: Option<&Arc<ChannelHandle>>,
    linked: &crate::channels::runtime::test_support::LinkedTelegramRuntime,
    canonical_conversation_id: Option<&str>,
) -> bool {
    use tinychannels_bus::module_inbound::{ChannelInboundSource, PrepareChannelInboundRequest};
    use tinychannels_bus::module_remote::RemoteSessionScope;

    ctx.channels_by_name = linked.channels();
    ctx.module_objects = linked.module_objects();
    let core = ctx
        .core
        .as_ref()
        .and_then(std::sync::Weak::upgrade)
        .unwrap_or_else(|| {
            crate::channels::runtime::test_support::bind_test_core_context(ctx, linked.config())
        });
    crate::core::runtime::CoreContext::scope(core, async {
        let account_id = linked
            .channels()
            .keys()
            .find(|name| tinychannels_bus::provider_id(name) == msg.channel)
            .cloned()
            .expect("linked runtime should expose the configured provider instance");
        let prepared = crate::modules::channel_controller::prepare_inbound(
            &crate::channels::context::module_client(ctx).await,
            PrepareChannelInboundRequest {
                source: ChannelInboundSource::Envelope {
                    envelope: tinychannels_bus::ChannelInboundEnvelope {
                        channel: tinychannels_bus::ChannelRef {
                            id: msg.channel.clone(),
                            account_id: Some(account_id),
                        },
                        message_id: msg.id.clone(),
                        reply_to_id: msg.reply_to_id.clone(),
                        conversation: tinychannels_bus::ConversationRef {
                            id: canonical_conversation_id
                                .unwrap_or(&msg.reply_target)
                                .to_owned(),
                            thread_id: msg.thread_ts.clone(),
                            topic_id: msg.topic_id.clone(),
                            ..Default::default()
                        },
                        sender: tinychannels_bus::SenderRef {
                            id: msg.sender.clone(),
                            name: msg.sender_name.clone(),
                            ..Default::default()
                        },
                        timestamp_unix_ms: Some(msg.timestamp.saturating_mul(1000)),
                        text: msg.content.clone(),
                        parts: vec![tinychannels_bus::ContentPart::Text {
                            text: msg.content.clone(),
                            markdown_dialect: None,
                        }],
                        ..Default::default()
                    },
                    timestamp: msg.timestamp,
                },
                namespace: "main".into(),
                session_policy: Default::default(),
                preparation_mode: Default::default(),
            },
        )
        .await
        .expect("linked TinyChannels should prepare the command's trusted scope");
        let session_key = prepared.session_key.clone();
        let scope = RemoteSessionScope {
            channel: prepared.envelope.channel,
            conversation: prepared.envelope.conversation,
            session_key: session_key.clone(),
        };
        handle_runtime_command_with_reset(
            ctx,
            &prepared.message,
            target_channel,
            &prepared.history_key,
            Some(scope),
            Some(session_key),
            &prepared.message,
            || async { Ok(()) },
        )
        .await
    })
    .await
}

#[cfg(feature = "modules")]
async fn module_route_help_response(
    workspace_dir: &Path,
    content: &str,
    providers: Vec<tinychannels_bus::module_control::ProviderDescriptor>,
) -> String {
    use tinychannels_bus::module_control::{ChannelControlRequest, ChannelControlResult};
    use tinychannels_bus::module_remote::RemoteRoute;

    let request = ChannelControlRequest::Route {
        workspace_dir: workspace_dir.to_string_lossy().into_owned(),
        capabilities: Some(Box::new({
            let mut capabilities = tinychannels_bus::ChannelCapabilities::unknown();
            capabilities.remote_control = true;
            capabilities
        })),
        message: Box::new(ChannelMessage {
            reply_to_id: None,
            topic_id: None,
            id: "help-fixture".into(),
            sender: "operator".into(),
            reply_target: "chat".into(),
            content: content.into(),
            channel: "telegram".into(),
            timestamp: 0,
            thread_ts: None,
            sender_name: None,
        }),
        route: RemoteRoute {
            provider: "openai".into(),
            model: "gpt-5".into(),
        },
        history_len: 0,
        session_scope: None,
        configured_channel: None,
        providers,
    };
    let response: ChannelControlResult = crate::channels::module_fixture::client()
        .await
        .call_confidential(
            "tinychannels",
            tinychannels_bus::methods::PREPARE_CHANNEL_CONTROL,
            (request,),
        )
        .await
        .expect("the owning module should prepare command help");
    match response {
        ChannelControlResult::Route {
            reply: Some(reply), ..
        } => reply,
        other => panic!("expected module-generated help text, got {other:?}"),
    }
}

#[test]
fn route_selection_round_trip() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let sender_key = "telegram_alice_reply";
    assert_eq!(
        get_route_selection(&ctx, sender_key),
        ChannelRouteSelection {
            provider: "openai".into(),
            model: "reasoning-v1".into()
        }
    );

    set_route_selection(
        &ctx,
        sender_key,
        ChannelRouteSelection {
            provider: "anthropic".into(),
            model: "claude".into(),
        },
    );
    assert_eq!(
        get_route_selection(&ctx, sender_key),
        ChannelRouteSelection {
            provider: "anthropic".into(),
            model: "claude".into()
        }
    );

    set_route_selection(&ctx, sender_key, default_route_selection(&ctx));
    assert!(ctx.route_overrides.lock().unwrap().is_empty());
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn module_route_help_ignores_stale_model_cache_and_projects_providers() {
    let tempdir = tempfile::tempdir().unwrap();
    let state_dir = tempdir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("models_cache.json"),
        serde_json::json!({
            "entries": [{
                "provider": "openai",
                "models": ["stale-model"]
            }]
        })
        .to_string(),
    )
    .unwrap();

    let models = module_route_help_response(tempdir.path(), "/model", Vec::new()).await;
    assert!(models.contains("Current provider: `openai`"));
    // The module no longer reads `models_cache.json` (tinychannels 0.1.38):
    // a catalog-less channel points the user at the web UI instead of listing
    // a possibly stale workspace file.
    assert!(models.contains("Model catalog is unavailable in this channel"));
    assert!(!models.contains("stale-model"));
    assert!(!models.contains("gpt-5-mini"));
    assert!(!models.contains("Cached model IDs"));

    let providers = module_route_help_response(
        tempdir.path(),
        "/models",
        vec![
            tinychannels_bus::module_control::ProviderDescriptor {
                name: "openai".into(),
                aliases: vec!["oa".into()],
            },
            tinychannels_bus::module_control::ProviderDescriptor {
                name: "anthropic".into(),
                aliases: Vec::new(),
            },
        ],
    )
    .await;
    assert!(providers.contains("Switch provider with `/models <provider>`"));
    assert!(providers.contains("Available providers:"));
    assert!(providers.contains("openai (aliases: oa)"));
}

#[test]
fn model_command_messages_use_thread_aware_history_keys() {
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "room".into(),
        content: "/model gpt-5".into(),
        channel: "discord".into(),
        timestamp: 0,
        thread_ts: Some("thread-1".into()),
        sender_name: None,
    };
    assert_eq!(
        super::super::context::conversation_history_key(&msg),
        "discord_alice_room_thread:thread-1"
    );
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn sign_out_cancels_remote_invalidation_while_it_waits_for_the_session_table() {
    let core_context =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let expected_core_context = Arc::clone(&core_context);
    crate::core::runtime::CoreContext::scope(core_context, async {
        let guard = crate::web_chat::lock_thread_sessions_for_test(
            &crate::core::runtime::DispatchCtx::ambient(),
        )
        .await;
        let cancellation = tokio_util::sync::CancellationToken::new();
        let callback = RouteCallbacks::new(
            runtime_context(PathBuf::from("/tmp/channel-callback")),
            cancellation.clone(),
            None,
        );
        assert!(Arc::ptr_eq(
            callback.core_context.as_ref().unwrap(),
            &expected_core_context
        ));
        assert_eq!(callback.tenant, crate::core::runtime::Tenant::default());

        let request = tinychannels_bus::ChannelBackendRequest::RemoteControl(
            tinychannels_bus::module_remote::RemoteHostRequest::InvalidateThread {
                thread_id: "sign-out-in-flight".into(),
            },
        );
        let mut operation = Box::pin(callback.dispatch_controller_backend(request));
        let waker = Waker::noop().clone();
        let mut task_context = Context::from_waker(&waker);

        assert!(
            matches!(operation.as_mut().poll(&mut task_context), Poll::Pending),
            "the callback should be waiting on the locked session table"
        );
        cancellation.cancel();
        assert!(
            matches!(
                operation.as_mut().poll(&mut task_context),
                Poll::Ready(Err(_))
            ),
            "sign-out must cancel the callback before it can mutate session state"
        );

        drop(guard);
    })
    .await;
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn load_cached_model_preview_returns_empty_when_cache_json_is_invalid() {
    let tempdir = tempfile::tempdir().unwrap();
    let state_dir = tempdir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("models_cache.json"),
        "{ definitely invalid json",
    )
    .unwrap();

    let response = module_route_help_response(tempdir.path(), "/model", Vec::new()).await;
    assert!(response.contains("Model catalog is unavailable in this channel for `openai`"));
}

#[tokio::test]
async fn handle_runtime_command_unknown_provider_sends_helpful_error() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_state = Arc::new(TestChannel::new("recording"));
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_state)));
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "room".into(),
        content: "/models definitely-not-a-provider".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("thread-1".into()),
        sender_name: None,
    };

    let handled = handle_runtime_command_if_needed(
        &ctx,
        &msg,
        Some(&channel),
        &conversation_history_key(&msg),
    )
    .await;
    assert!(handled);

    let sent = channel_state.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .content
        .contains("Unknown provider `definitely-not-a-provider`"));
    assert_eq!(sent[0].thread_ts.as_deref(), Some("thread-1"));
}

#[tokio::test]
async fn clear_history_command_uses_reset_callback_and_persists_route_override() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let key = "telegram_alice_room";
    let channel_state = Arc::new(TestChannel::new("recording"));
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_state)));
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "room".into(),
        content: "/model gpt-5-mini".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        sender_name: None,
    };

    let reset_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback_flag = Arc::clone(&reset_called);
    let handled = handle_runtime_command_with_reset(
        &ctx,
        &msg,
        Some(&channel),
        &conversation_history_key(&msg),
        None,
        None,
        &msg,
        move || async move {
            callback_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    assert!(handled);
    assert!(reset_called.load(std::sync::atomic::Ordering::SeqCst));

    assert_eq!(
        get_route_selection(&ctx, key),
        ChannelRouteSelection {
            provider: "openai".into(),
            model: "gpt-5-mini".into()
        }
    );

    let sent = channel_state.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].content.contains("Model switched to `gpt-5-mini`"));
}

#[tokio::test]
async fn failed_history_reset_does_not_apply_requested_route() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let key = "telegram_alice_room";
    let before = ChannelRouteSelection {
        provider: "openai".into(),
        model: "reasoning-v1".into(),
    };
    set_route_selection(&ctx, key, before.clone());
    let channel_state = Arc::new(TestChannel::new("recording"));
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_state)));
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "reset-fails".into(),
        sender: "alice".into(),
        reply_target: "room".into(),
        content: "/model gpt-5-mini".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        sender_name: None,
    };

    let handled = handle_runtime_command_with_reset(
        &ctx,
        &msg,
        Some(&channel),
        key,
        None,
        None,
        &msg,
        || async { Err("durable reset failed".to_owned()) },
    )
    .await;

    assert!(handled);
    assert_eq!(get_route_selection(&ctx, key), before);
    let sent = channel_state.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].content, "Something went wrong. Please try again.");
    assert!(!sent[0].content.contains("durable reset failed"));
}

#[tokio::test]
async fn handle_runtime_command_telegram_status_replies_without_agent() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    let mut ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_state = Arc::new(TestChannel::new("recording"));
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_state)));
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/status".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
        sender_name: None,
    };

    let handled =
        handle_linked_runtime_command(&mut ctx, &msg, Some(&channel), &linked, None).await;
    assert!(handled);

    let sent = channel_state.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].content.contains("**Status**"),
        "status command returned unexpected reply: {:?}",
        sent[0].content
    );
    assert!(sent[0].content.contains("Provider:"));
}

#[tokio::test]
async fn handle_runtime_command_without_target_channel_still_consumes_command() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    let mut ctx = runtime_context(PathBuf::from("/tmp"));
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/help".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        sender_name: None,
    };

    let handled = handle_linked_runtime_command(&mut ctx, &msg, None, &linked, None).await;
    assert!(handled);
}

#[tokio::test]
async fn handle_runtime_command_telegram_help_replies_with_remote_command_list() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    let mut ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_state = Arc::new(TestChannel::new("recording"));
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_state)));
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: None,
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/help".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
        sender_name: None,
    };

    let handled =
        handle_linked_runtime_command(&mut ctx, &msg, Some(&channel), &linked, None).await;
    assert!(handled);

    let sent = channel_state.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].content.contains("Remote control:"));
    assert!(sent[0].content.contains("`/status`"));
    assert!(sent[0].content.contains("`/sessions`"));
    assert!(sent[0].content.contains("`/new`"));
    assert!(sent[0]
        .content
        .contains("Model routing: `/model`, `/models`"));
}

#[tokio::test]
async fn handle_runtime_command_telegram_sessions_reports_empty_store() {
    let linked = crate::channels::runtime::test_support::linked_telegram_runtime().await;
    let tempdir = tempfile::tempdir().unwrap();
    let mut ctx = runtime_context(tempdir.path().to_path_buf());
    let channel_state = Arc::new(TestChannel::new("recording"));
    let channel = Arc::new(ChannelHandle::Test(Arc::clone(&channel_state)));
    let msg = ChannelMessage {
        reply_to_id: None,
        topic_id: Some("42".into()),
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/sessions".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        sender_name: None,
    };

    let handled =
        handle_linked_runtime_command(&mut ctx, &msg, Some(&channel), &linked, None).await;
    assert!(handled);

    let sent = channel_state.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .content
        .contains("No conversation threads yet. Send `/new` to create one."));
    assert_eq!(sent[0].topic_id.as_deref(), Some("42"));
}

#[path = "routes_tests_telegram.rs"]
mod telegram;
