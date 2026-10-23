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
use tinychannels_bus::context::ChatMessage;
use tinychannels_bus::{
    ChannelInboundEnvelope, ChannelOutboundMessage, ChannelRef, ConversationRef,
    OutboundContentPart, SenderRef,
};
use tinytools::{Tool, ToolResult};

fn inbound(
    message_id: String,
    sender_id: String,
    conversation_id: String,
    text: String,
    channel_id: String,
    thread_id: Option<String>,
) -> ChannelInboundEnvelope {
    ChannelInboundEnvelope {
        message_id,
        sender: SenderRef {
            id: sender_id,
            ..Default::default()
        },
        conversation: ConversationRef {
            id: conversation_id,
            thread_id,
            ..Default::default()
        },
        channel: ChannelRef {
            id: channel_id,
            ..Default::default()
        },
        text,
        ..Default::default()
    }
}

#[test]
#[allow(
    deprecated,
    reason = "The route contract still requires the v21 ChannelMessage projection"
)]
fn route_compat_message_preserves_legacy_addressing_and_sender_metadata() {
    let mut message = inbound(
        "message-1".to_string(),
        "sender-1".to_string(),
        "conversation-1".to_string(),
        "/status".to_string(),
        "telegram".to_string(),
        Some("thread-1".into()),
    );
    message.timestamp_unix_ms = Some(123);
    message.reply_to_id = Some("reply-1".into());
    message.conversation.topic_id = Some("topic-1".into());
    message.sender.name = Some("Alice".into());

    let legacy = super::route_compat_message(&message);
    assert_eq!(legacy.id, "message-1");
    assert_eq!(legacy.sender, "sender-1");
    assert_eq!(legacy.reply_target, "conversation-1");
    assert_eq!(legacy.content, "/status");
    assert_eq!(legacy.channel, "telegram");
    assert_eq!(legacy.timestamp, 123);
    assert_eq!(legacy.thread_ts.as_deref(), Some("thread-1"));
    assert_eq!(legacy.reply_to_id.as_deref(), Some("reply-1"));
    assert_eq!(legacy.topic_id.as_deref(), Some("topic-1"));
    assert_eq!(legacy.sender_name.as_deref(), Some("Alice"));
}

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
pub(super) struct RecordingChannel {
    pub(super) sent: Mutex<Vec<ChannelOutboundMessage>>,
}

#[async_trait]
impl Channel for RecordingChannel {
    fn name(&self) -> &str {
        "recording"
    }

    #[allow(
        deprecated,
        reason = "TinyChannels v22 still requires the legacy send trait method"
    )]
    async fn send(&self, _message: &tinychannels_bus::SendMessage) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send_typed(&self, message: &ChannelOutboundMessage) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(message.clone());
        Ok(())
    }

    #[allow(
        deprecated,
        reason = "TinyChannels v22 still requires the legacy listener trait method"
    )]
    async fn listen(
        &self,
        _tx: tokio::sync::mpsc::Sender<tinychannels_bus::ChannelMessage>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

fn sent_text(message: &ChannelOutboundMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            OutboundContentPart::Content {
                part: tinychannels_bus::channel::ContentPart::Text { text, .. },
            } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn runtime_context(workspace_dir: PathBuf) -> ChannelRuntimeContext {
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
        provider_runtime_options: crate::tools::delegate::ProviderRuntimeOptions::default(),
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
        message: Box::new(super::route_compat_message(&inbound(
            "help-fixture".into(),
            "operator".into(),
            "chat".into(),
            content.into(),
            "telegram".into(),
            None,
        ))),
        route: RemoteRoute {
            provider: "openai".into(),
            model: "gpt-5".into(),
        },
        history_len: 0,
        capabilities: None,
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

#[test]
fn provider_descriptors_use_configured_slugs_and_keep_managed_aliases() {
    use crate::config::schema::{AuthStyle, CloudProviderCreds};

    super::register_hosted_route_test_policy();
    let config = crate::config::Config {
        cloud_providers: vec![
            CloudProviderCreds {
                id: "p_custom_a8c3f".into(),
                slug: "custom-provider".into(),
                label: "Custom Provider Label".into(),
                endpoint: "https://inference.example.test/v1".into(),
                auth_style: AuthStyle::Bearer,
                default_model: None,
            },
            CloudProviderCreds {
                id: "p_openhuman_managed".into(),
                slug: "openhuman".into(),
                label: "Managed Route".into(),
                endpoint: "https://api.openhuman.ai/v1".into(),
                auth_style: AuthStyle::OpenhumanJwt,
                default_model: None,
            },
        ],
        ..crate::config::Config::default()
    };

    let providers = super::provider_descriptors(&config);
    assert_eq!(
        providers
            .iter()
            .map(|provider| provider.name.as_str())
            .collect::<Vec<_>>(),
        ["custom-provider", "openhuman"]
    );
    assert_eq!(providers[0].aliases, Vec::<String>::new());
    assert_eq!(providers[1].aliases, ["backend", "openhuman-backend"]);
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
    let msg = inbound(
        "1".into(),
        "alice".into(),
        "room".into(),
        "/model gpt-5".into(),
        "discord".into(),
        Some("thread-1".into()),
    );
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
    assert!(response.contains("Model catalog is unavailable in this channel for `openai`"));
}

#[tokio::test]
async fn handle_runtime_command_unknown_provider_sends_helpful_error() {
    let ctx = runtime_context(PathBuf::from("/tmp"));
    let channel_impl = Arc::new(RecordingChannel::default());
    let channel: Arc<dyn Channel> = channel_impl.clone();
    let msg = inbound(
        "1".into(),
        "alice".into(),
        "room".into(),
        "/models definitely-not-a-provider".into(),
        "telegram".into(),
        Some("thread-1".into()),
    );

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
    assert!(sent_text(&sent[0]).contains("Unknown provider `definitely-not-a-provider`"));
    assert_eq!(sent[0].thread_id.as_deref(), Some("thread-1"));
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
    let msg = inbound(
        "1".into(),
        "alice".into(),
        "room".into(),
        "/model gpt-5-mini".into(),
        "telegram".into(),
        None,
    );

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
    assert!(sent_text(&sent[0]).contains("Model switched to `gpt-5-mini`"));
}

#[path = "routes_runtime_command_tests.rs"]
mod runtime_command;
