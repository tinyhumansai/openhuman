use super::*;
use crate::core::event_bus::{DomainEvent, EventHandler};
use crate::openhuman::channels::context::{
    ChannelRuntimeContext, ProviderCacheMap, RouteSelectionMap,
};
use crate::openhuman::channels::telegram::{TelegramRemoteCommand, TelegramRemoteSubscriber};
use crate::openhuman::channels::traits::ChannelMessage;
use crate::openhuman::inference::provider::{ChatMessage, Provider};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
use crate::openhuman::tools::{Tool, ToolResult};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct DummyProvider;

#[async_trait]
impl Provider for DummyProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("ok".into())
    }
}

struct DummyMemory;

#[async_trait]
impl Memory for DummyMemory {
    fn name(&self) -> &str {
        "dummy"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: crate::openhuman::memory::RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(
        &self,
    ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
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
    ChannelRuntimeContext {
        channels_by_name: Arc::new(HashMap::new()),
        provider: Arc::new(DummyProvider),
        default_provider: Arc::new("openai".into()),
        memory: Arc::new(DummyMemory),
        tools_registry: Arc::new(vec![Box::new(DummyTool) as Box<dyn Tool>]),
        system_prompt: Arc::new("prompt".into()),
        model: Arc::new("reasoning-v1".into()),
        temperature: 0.0,
        auto_save_memory: false,
        max_tool_iterations: 1,
        min_relevance_score: 0.4,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        provider_cache: ProviderCacheMap::default(),
        route_overrides: RouteSelectionMap::default(),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(crate::openhuman::config::ReliabilityConfig::default()),
        provider_runtime_options:
            crate::openhuman::inference::provider::ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(workspace_dir),
        message_timeout_secs: 60,
        multimodal: crate::openhuman::config::MultimodalConfig::default(),
    }
}

#[test]
fn runtime_command_parsing_and_provider_support_are_channel_scoped() {
    assert!(supports_runtime_model_switch("telegram"));
    assert!(supports_runtime_model_switch("discord"));
    assert!(!supports_runtime_model_switch("slack"));

    assert_eq!(
        parse_runtime_command("telegram", "/models"),
        Some(ChannelRuntimeCommand::ShowProviders)
    );
    assert_eq!(
        parse_runtime_command("discord", "/models openai"),
        Some(ChannelRuntimeCommand::SetProvider("openai".into()))
    );
    assert_eq!(
        parse_runtime_command("telegram", "/model gpt-5"),
        Some(ChannelRuntimeCommand::SetModel("gpt-5".into()))
    );
    assert_eq!(
        parse_runtime_command("telegram", "/model"),
        Some(ChannelRuntimeCommand::ShowModel)
    );
    assert_eq!(
        parse_runtime_command("telegram", "/status@OpenHumanBot"),
        Some(ChannelRuntimeCommand::TelegramRemote(
            TelegramRemoteCommand::Status
        ))
    );
    assert_eq!(
        parse_runtime_command("telegram", "/help"),
        Some(ChannelRuntimeCommand::TelegramRemote(
            TelegramRemoteCommand::Help
        ))
    );
    assert_eq!(parse_runtime_command("slack", "/models"), None);
    assert_eq!(parse_runtime_command("discord", "/status"), None);
    assert_eq!(parse_runtime_command("telegram", "hello"), None);
}

#[test]
fn provider_alias_and_route_selection_round_trip() {
    let first_provider = provider::list_providers()
        .into_iter()
        .next()
        .expect("provider registry should not be empty");
    assert_eq!(
        resolve_provider_alias(first_provider.name).as_deref(),
        Some(first_provider.name)
    );
    assert!(resolve_provider_alias("   ").is_none());

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
fn cached_models_and_help_responses_render_expected_text() {
    let tempdir = tempfile::tempdir().unwrap();
    let state_dir = tempdir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join(MODEL_CACHE_FILE),
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

    let preview = load_cached_model_preview(tempdir.path(), "openai");
    assert_eq!(preview, vec!["gpt-5", "gpt-5-mini", "gpt-4.1"]);
    assert!(load_cached_model_preview(tempdir.path(), "missing").is_empty());

    let current = ChannelRouteSelection {
        provider: "openai".into(),
        model: "gpt-5".into(),
    };
    let models = build_models_help_response(&current, tempdir.path());
    assert!(models.contains("Current provider: `openai`"));
    assert!(models.contains("Cached model IDs"));
    assert!(models.contains("- `gpt-5-mini`"));

    let providers = build_providers_help_response(&current);
    assert!(providers.contains("Switch provider with `/models <provider>`"));
    assert!(providers.contains("Available providers:"));
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
    };
    assert_eq!(
        super::super::context::conversation_history_key(&msg),
        "discord_alice_room_thread:thread-1"
    );
}

#[test]
fn load_cached_model_preview_returns_empty_when_cache_json_is_invalid() {
    let tempdir = tempfile::tempdir().unwrap();
    let state_dir = tempdir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join(MODEL_CACHE_FILE),
        "{ definitely invalid json",
    )
    .unwrap();

    assert!(load_cached_model_preview(tempdir.path(), "openai").is_empty());
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
    };

    let handled = handle_runtime_command_if_needed(&ctx, &msg, Some(&channel)).await;
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
    };

    let handled = handle_runtime_command_if_needed(&ctx, &msg, Some(&channel)).await;
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
    };

    let handled = handle_runtime_command_if_needed(&ctx, &msg, Some(&channel)).await;
    assert!(handled);

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].content.contains("**Status**"));
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
    };

    let handled = handle_runtime_command_if_needed(&ctx, &msg, None).await;
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
    };

    let handled = handle_runtime_command_if_needed(&ctx, &msg, Some(&channel)).await;
    assert!(handled);

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .content
        .contains("OpenHuman Telegram remote control (phase 1):"));
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
    };

    let handled = handle_runtime_command_if_needed(&ctx, &msg, Some(&channel)).await;
    assert!(handled);

    let sent = channel_impl.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0]
        .content
        .contains("No conversation threads yet. Send `/new` to create one."));
    assert_eq!(sent[0].thread_ts.as_deref(), Some("42"));
}

#[tokio::test]
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
    };
    assert!(handle_runtime_command_if_needed(&ctx, &new_msg, Some(&channel)).await);
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

    let subscriber = TelegramRemoteSubscriber::new(tempdir.path().to_path_buf());
    subscriber
        .handle(&DomainEvent::ChannelMessageReceived {
            channel: "telegram".into(),
            message_id: "2".into(),
            sender: "alice".into(),
            reply_target: "chat-remote".into(),
            content: "work".into(),
            thread_ts: Some("42".into()),
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
    };
    assert!(handle_runtime_command_if_needed(&ctx, &status_msg, Some(&channel)).await);

    let sessions_msg = ChannelMessage {
        id: "4".into(),
        sender: "alice".into(),
        reply_target: "chat-remote".into(),
        content: "/sessions".into(),
        channel: "telegram".into(),
        timestamp: 0,
        thread_ts: Some("42".into()),
    };
    assert!(handle_runtime_command_if_needed(&ctx, &sessions_msg, Some(&channel)).await);

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
