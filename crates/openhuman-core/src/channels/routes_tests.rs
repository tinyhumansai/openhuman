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
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use tinybus::EventHandler;
use tinychannels::routes::resolve_provider_alias;
use tinychannels_bus::context::ChatMessage;
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

#[derive(Default)]
struct RecordingChannel {
    sent: Mutex<Vec<SendMessage>>,
}

#[async_trait]
impl Channel for RecordingChannel {
    fn name(&self) -> &str {
        "recording"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(message.clone());
        Ok(())
    }

    async fn listen(&self, _tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        Ok(())
    }
}

fn runtime_context(workspace_dir: PathBuf) -> ChannelRuntimeContext {
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
            "ok",
        ]));
    ChannelRuntimeContext {
        relay_thread: None,
        channels_by_name: Arc::new(HashMap::new()),
        turn_model_source: Some(crate::agent::tinyagents::TurnModelSource::from_model(model)),
        default_provider: Arc::new("openai".into()),
        tools_registry: Arc::new(vec![Box::new(DummyTool) as Box<dyn Tool>]),
        system_prompt: crate::channels::ChannelSystemPrompt::fixed("prompt"),
        model: Arc::new("reasoning-v1".into()),
        temperature: 0.0,
        max_tool_iterations: 1,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        turn_model_source_cache: TurnModelSourceCacheMap::default(),
        route_overrides: RouteSelectionMap::default(),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::config::ReliabilityConfig::default()),
        provider_runtime_options: crate::inference::provider::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(workspace_dir),
        message_timeout_secs: 60,
        multimodal: crate::config::MultimodalConfig::default(),
        multimodal_files: crate::config::MultimodalFileConfig::default(),
        config: None,
        core: None,
    }
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
        message: Box::new(ChannelMessage {
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
fn provider_alias_and_route_selection_round_trip() {
    let first_provider = provider::list_providers()
        .into_iter()
        .next()
        .expect("provider registry should not be empty");
    assert_eq!(
        resolve_provider_alias(first_provider.name, &provider_descriptors()).as_deref(),
        Some(first_provider.name)
    );
    assert!(resolve_provider_alias("   ", &provider_descriptors()).is_none());

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
async fn cached_models_and_help_responses_render_expected_text() {
    let tempdir = tempfile::tempdir().unwrap();
    let state_dir = tempdir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("models_cache.json"),
        serde_json::json!({
            "entries": [
                {
                    "provider": "openai",
                    "models": ["gpt-5", "gpt-5-mini", "gpt-4.1"]
                }
            ]
        })
        .to_string(),
    )
    .unwrap();

    let models = module_route_help_response(tempdir.path(), "/model", Vec::new()).await;
    assert!(models.contains("Current provider: `openai`"));
    assert!(models.contains("Cached model IDs"));
    assert!(models.contains("- `gpt-5-mini`"));

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
    assert!(response.contains("No cached model list found for `openai`"));
}

#[tokio::test]
async fn handle_runtime_command_unknown_provider_sends_helpful_error() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = ChannelMessage {
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

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .content
        .contains("Unknown provider `definitely-not-a-provider`"));
    assert_eq!(sent[0].thread_ts.as_deref(), Some("thread-1"));
}

#[tokio::test]
async fn handle_runtime_command_set_model_clears_sender_history_and_persists_route_override() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let key = "telegram_alice_room";
    ctx.conversation_histories
        .lock()
        .unwrap()
        .insert(key.to_string(), vec![ChatMessage::user("old history")]);
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "room".into(),
        content: "/model gpt-5-mini".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
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

    assert!(ctx
        .conversation_histories
        .lock()
        .unwrap()
        .get(key)
        .is_none());
    assert_eq!(
        get_route_selection(&ctx, key),
        ChannelRouteSelection {
            provider: "openai".into(),
            model: "gpt-5-mini".into()
        }
    );

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].content.contains("Model switched to `gpt-5-mini`"));
}

#[tokio::test]
async fn handle_runtime_command_telegram_status_replies_without_agent() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/status".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
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

    let sent = channel_impl.sent.lock().unwrap();
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
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let msg = ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/help".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: None,
        sender_name: None,
    };

    let handled =
        handle_runtime_command_if_needed(&ctx, &msg, None, &conversation_history_key(&msg)).await;
    assert!(handled);
}

#[tokio::test]
async fn handle_runtime_command_telegram_help_replies_with_remote_command_list() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/help".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
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

    let sent = channel_impl.sent.lock().unwrap();
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
    let tempdir = tempfile::tempdir().unwrap();
    let ctx = runtime_context(tempdir.path().to_path_buf());
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/sessions".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
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

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .content
        .contains("No conversation threads yet. Send `/new` to create one."));
    assert_eq!(sent[0].thread_ts.as_deref(), Some("42"));
}

#[tokio::test]
#[ignore = "the linked tinychannels-module is given no storage backend here, so /new fails to persist the chat binding (host storage is unavailable); needs a storage fixture"]
async fn handle_runtime_command_telegram_new_status_and_sessions_round_trip() {
    let tempdir = tempfile::tempdir().unwrap();
    let ctx = runtime_context(tempdir.path().to_path_buf());
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let sender_key = "telegram_alice_chat-remote";

    ctx.conversation_histories.lock().unwrap().insert(
        sender_key.to_string(),
        vec![ChatMessage::user("old history")],
    );

    let new_msg = ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/new".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
        sender_name: None,
    };
    assert!(
        handle_runtime_command_if_needed(
            &ctx,
            &new_msg,
            Some(&channel),
            &conversation_history_key(&new_msg)
        )
        .await
    );
    assert!(ctx
        .conversation_histories
        .lock()
        .unwrap()
        .get(sender_key)
        .is_none());

    ctx.conversation_histories
        .lock()
        .unwrap()
        .insert(sender_key.to_string(), vec![ChatMessage::user("after new")]);
    set_route_selection(
        &ctx,
        sender_key,
        ChannelRouteSelection {
            provider: "anthropic".into(),
            model: "claude-3".into(),
        },
    );

    let subscriber = ChannelTurnStateSubscriber::new(tempdir.path().to_path_buf());
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "2".into(),
            sender: "alice".into(),
            reply_target: "chat-remote".into(),
            content: "work".into(),
            thread_ts: Some("42".into()),
            inbound_envelope: None,
            workspace_dir: tempdir.path().to_path_buf(),
        })
        .await;

    let status_msg = ChannelMessage {
        id: "3".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/status".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
        sender_name: None,
    };
    assert!(
        handle_runtime_command_if_needed(
            &ctx,
            &status_msg,
            Some(&channel),
            &conversation_history_key(&status_msg)
        )
        .await
    );

    let sessions_msg = ChannelMessage {
        id: "4".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/sessions".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
        sender_name: None,
    };
    assert!(
        handle_runtime_command_if_needed(
            &ctx,
            &sessions_msg,
            Some(&channel),
            &conversation_history_key(&sessions_msg)
        )
        .await
    );

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 3);
    assert!(sent[0].content.contains("Started new session"));
    assert!(sent[0]
        .content
        .contains("In-memory channel history cleared for this chat."));

    assert!(sent[1].content.contains("**Status**"));
    assert!(sent[1].content.contains("Thread: `Telegram"));
    assert!(sent[1].content.contains("Provider: `anthropic`"));
    assert!(sent[1].content.contains("Model: `claude-3`"));
    assert!(sent[1].content.contains("In-memory turns: 1"));
    assert!(sent[1].content.contains("Turn: in progress"));

    assert!(sent[2].content.contains("**Recent sessions**"));
    assert!(sent[2].content.contains("→ `Telegram"));
    assert!(sent
        .iter()
        .all(|message| message.thread_ts.as_deref() == Some("42")));
}

#[tokio::test]
async fn handle_runtime_command_telegram_new_reports_unavailable_storage() {
    let tempdir = tempfile::tempdir().unwrap();
    let ctx = runtime_context(tempdir.path().to_path_buf());
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let sender_key = "telegram_alice_chat-remote";

    ctx.conversation_histories.lock().unwrap().insert(
        sender_key.to_string(),
        vec![ChatMessage::user("old history")],
    );

    let msg = ChannelMessage {
        id: "1".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/new".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
        sender_name: None,
    };
    assert!(
        handle_runtime_command_if_needed(
            &ctx,
            &msg,
            Some(&channel),
            &conversation_history_key(&msg)
        )
        .await
    );
    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].content.contains("Created thread"),
        "{}",
        sent[0].content
    );
    assert!(sent[0]
        .content
        .contains("failed to persist the chat binding"));
    assert!(sent[0].content.contains("host storage is unavailable"));
    assert_eq!(sent[0].thread_ts.as_deref(), Some("42"));
}
