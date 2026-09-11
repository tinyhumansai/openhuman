//! Debug-build harnesses for raw integration coverage of the channel runtime.

pub use super::dispatch::test_support::{
    build_channel_context_block_for_test, select_acknowledgment_reaction_for_test,
};
use super::dispatch::{
    process_channel_message, process_channel_runtime_message, RuntimeChannelMessage,
};
pub use super::startup::test_support::resolve_yuanbao_app_secret_for_test;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::channels::context::{ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS};
use crate::openhuman::channels::traits::{ChannelMessage, SendMessage};
use crate::openhuman::channels::Channel;
use crate::openhuman::channels::ChannelSystemPrompt;
use crate::openhuman::config::{MultimodalConfig, MultimodalFileConfig, ReliabilityConfig};
use crate::openhuman::inference::provider::ProviderRuntimeOptions;
use crate::openhuman::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tinyinference::model::{ChatModel, ModelRequest, ModelResponse};
use tinymemory_api::types::{MemoryCategory, MemoryEntry};

#[derive(Debug, Clone)]
pub struct DispatchHarnessOptions {
    pub channel_name: String,
    pub content: String,
    pub thread_ts: Option<String>,
    pub inbound_envelope: Option<tinychannels::ChannelInboundEnvelope>,
    pub streaming: bool,
    pub supports_reactions: bool,
    pub response_text: Option<String>,
    pub handler_error: Option<String>,
    pub handler_delay_ms: u64,
    pub timeout_secs: u64,
    pub seed_history_len: usize,
    pub memory_entries: Vec<TestMemoryEntry>,
    /// Workspace the runtime context reports; `None` falls back to the OS temp dir.
    ///
    /// `pub`, like every other field here, and it has to be: integration tests
    /// under `tests/` are a separate crate, and a struct-update expression
    /// (`..DispatchHarnessOptions::default()`) requires **every** field to be
    /// visible — including the ones the caller never names. Added as
    /// `pub(crate)` in `a45b1c1af`, which broke three of them with E0451.
    pub workspace_dir: Option<PathBuf>,
    /// The prompt every dispatch is seeded with; the default pins a fixed literal.
    /// Pass one `ChannelSystemPrompt::refreshing` clone to several dispatches
    /// to observe a re-render across them.
    ///
    /// Wrapped in [`HarnessSystemPrompt`] rather than exposing
    /// `Option<ChannelSystemPrompt>` — see that type for why.
    pub system_prompt: HarnessSystemPrompt,
}

/// Opaque carrier for the harness's system prompt.
///
/// [`DispatchHarnessOptions`] is `pub` and integration tests build it with
/// `..DispatchHarnessOptions::default()`, so every field must be visible from
/// another crate. The prompt itself must **not** become visible with it:
/// `ChannelSystemPrompt` is `pub(crate)`, its `Refreshing` variant wraps a
/// `Mutex`-backed render cache (`RefreshingInner`), and widening it would
/// cascade to `RefreshingInner` and `ChannelPromptInputs` — production types
/// with real invariants — purely to satisfy a test harness. A `pub` field of a
/// `pub(crate)` type is also exactly what the `private_interfaces` lint exists
/// to catch, and `-D warnings` makes that a hard error.
///
/// A `pub` newtype with a **private** field settles both: the field is visible,
/// the type inside it is not. An out-of-crate caller can therefore only ever
/// get the default (no prompt), which is all those tests want; in-crate tests
/// set one with [`HarnessSystemPrompt::new`].
#[derive(Clone, Default)]
pub struct HarnessSystemPrompt(Option<ChannelSystemPrompt>);

impl HarnessSystemPrompt {
    /// Seed the harness with `prompt`. Crate-internal, because the prompt is.
    pub(crate) fn new(prompt: ChannelSystemPrompt) -> Self {
        Self(Some(prompt))
    }

    /// The prompt, if one was set.
    pub(crate) fn into_inner(self) -> Option<ChannelSystemPrompt> {
        self.0
    }
}

impl std::fmt::Debug for HarnessSystemPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(prompt) => f.debug_tuple("HarnessSystemPrompt").field(prompt).finish(),
            None => f.write_str("HarnessSystemPrompt(default)"),
        }
    }
}

impl Default for DispatchHarnessOptions {
    fn default() -> Self {
        Self {
            channel_name: "test-channel".to_string(),
            content: "hello".to_string(),
            thread_ts: None,
            inbound_envelope: None,
            streaming: false,
            supports_reactions: false,
            response_text: Some("dispatch ok".to_string()),
            handler_error: None,
            handler_delay_ms: 0,
            timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            seed_history_len: 0,
            memory_entries: Vec::new(),
            workspace_dir: None,
            system_prompt: HarnessSystemPrompt::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestMemoryEntry {
    pub key: String,
    pub content: String,
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedSend {
    pub kind: &'static str,
    pub recipient: String,
    pub content: String,
    pub thread_ts: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DispatchHarnessObservation {
    pub sends: Vec<ObservedSend>,
    pub start_typing_calls: usize,
    pub stop_typing_calls: usize,
    pub received_event_envelope: Option<tinychannels::ChannelInboundEnvelope>,
    pub handler_history_roles: Vec<String>,
    pub handler_history_text: String,
    pub handler_provider_name: String,
    pub handler_channel_name: String,
    pub handler_had_progress: bool,
    pub retained_history_len: usize,
}

#[derive(Default)]
struct HarnessState {
    sends: tokio::sync::Mutex<Vec<ObservedSend>>,
    start_typing_calls: AtomicUsize,
    stop_typing_calls: AtomicUsize,
}

struct HarnessChannel {
    name: String,
    streaming: bool,
    supports_reactions: bool,
    state: Arc<HarnessState>,
}

#[async_trait]
impl Channel for HarnessChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, message: &SendMessage) -> Result<()> {
        self.state.sends.lock().await.push(ObservedSend {
            kind: "send",
            recipient: message.recipient.clone(),
            content: message.content.clone(),
            thread_ts: message.thread_ts.clone(),
        });
        Ok(())
    }

    async fn listen(&self, _tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> Result<()> {
        Ok(())
    }

    async fn start_typing(&self, _recipient: &str) -> Result<()> {
        self.state.start_typing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn stop_typing(&self, _recipient: &str) -> Result<()> {
        self.state.stop_typing_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn supports_reactions(&self) -> bool {
        self.supports_reactions
    }

    fn supports_draft_updates(&self) -> bool {
        self.streaming
    }

    async fn send_draft(&self, message: &SendMessage) -> Result<Option<String>> {
        self.state.sends.lock().await.push(ObservedSend {
            kind: "draft",
            recipient: message.recipient.clone(),
            content: message.content.clone(),
            thread_ts: message.thread_ts.clone(),
        });
        Ok(Some("draft-1".to_string()))
    }

    async fn update_draft(&self, recipient: &str, message_id: &str, text: &str) -> Result<()> {
        self.state.sends.lock().await.push(ObservedSend {
            kind: "update_draft",
            recipient: format!("{recipient}:{message_id}"),
            content: text.to_string(),
            thread_ts: None,
        });
        Ok(())
    }

    async fn finalize_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<()> {
        self.state.sends.lock().await.push(ObservedSend {
            kind: "finalize_draft",
            recipient: format!("{recipient}:{message_id}"),
            content: text.to_string(),
            thread_ts: thread_ts.map(str::to_string),
        });
        Ok(())
    }
}

struct HarnessModel;

#[async_trait]
impl ChatModel<()> for HarnessModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        let message = request
            .messages
            .last()
            .map(|message| message.text())
            .unwrap_or_default();
        Ok(ModelResponse::assistant(format!("model echo: {message}")))
    }
}

/// A provider whose `recall` answers with a fixed entry list regardless of
/// query.
///
/// Deliberately not [`InMemoryProvider`](crate::openhuman::memory::guard::in_memory::InMemoryProvider):
/// that one substring-matches, and these harness entries are scripted to come
/// back for whatever the test sends. The point here is the channel pipeline
/// downstream of recall, not recall itself.
struct HarnessMemory {
    entries: Vec<MemoryEntry>,
}

#[async_trait]
impl tinymemory_api::provider::MemoryCore for HarnessMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
        _taint: tinymemory_api::types::MemoryTaint,
    ) -> std::result::Result<(), tinymemory_api::error::MemoryError> {
        Ok(())
    }

    async fn get(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> std::result::Result<Option<MemoryEntry>, tinymemory_api::error::MemoryError> {
        Ok(None)
    }

    async fn forget(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> std::result::Result<bool, tinymemory_api::error::MemoryError> {
        Ok(false)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> std::result::Result<Vec<MemoryEntry>, tinymemory_api::error::MemoryError> {
        Ok(Vec::new())
    }

    async fn namespaces(
        &self,
    ) -> std::result::Result<
        Vec<tinymemory_api::types::NamespaceSummary>,
        tinymemory_api::error::MemoryError,
    > {
        Ok(Vec::new())
    }
}

#[async_trait]
impl tinymemory_api::provider::MemoryRecall for HarnessMemory {
    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: &tinymemory_api::recall::OwnedRecallOpts,
        _scope: Option<&tinymemory_api::provider::types::SourceScope>,
    ) -> std::result::Result<Vec<MemoryEntry>, tinymemory_api::error::MemoryError> {
        Ok(self.entries.clone())
    }
}

#[async_trait]
impl tinymemory_api::provider::MemoryPortability for HarnessMemory {
    async fn export_page(
        &self,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> std::result::Result<
        tinymemory_api::provider::types::ExportPage,
        tinymemory_api::error::MemoryError,
    > {
        Err(tinymemory_api::error::MemoryError::Other(anyhow::anyhow!(
            "harness memory does not export"
        )))
    }

    async fn import_records(
        &self,
        _records: Vec<tinymemory_api::provider::types::ExportRecord>,
    ) -> std::result::Result<
        tinymemory_api::provider::types::ImportOutcome,
        tinymemory_api::error::MemoryError,
    > {
        Err(tinymemory_api::error::MemoryError::Other(anyhow::anyhow!(
            "harness memory does not import"
        )))
    }
}

#[async_trait]
impl tinymemory_api::provider::MemoryProvider for HarnessMemory {
    fn driver_id(&self) -> &str {
        "harness-memory"
    }

    fn capabilities(&self) -> tinymemory_api::capabilities::Capabilities {
        tinymemory_api::capabilities::Capabilities::mandatory()
    }

    async fn health(&self) -> tinymemory_api::health::MemoryHealth {
        tinymemory_api::health::MemoryHealth::Ready
    }
}

struct HarnessTool;

#[async_trait]
impl Tool for HarnessTool {
    fn name(&self) -> &str {
        "harness_tool"
    }

    fn description(&self) -> &str {
        "debug harness tool"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

fn memory_entry(input: TestMemoryEntry) -> MemoryEntry {
    MemoryEntry {
        id: input.key.clone(),
        key: input.key,
        content: input.content,
        namespace: None,
        category: MemoryCategory::Conversation,
        timestamp: "now".to_string(),
        session_id: None,
        score: input.score,
        taint: tinymemory_api::types::MemoryTaint::Internal,
    }
}

/// Acquire the shared agent-handler guard. Hold the returned guard across any
/// call that re-registers `AGENT_RUN_TURN_METHOD` (the harness, or
/// `start_channels`) so concurrent registrations cannot race in the same
/// process.
pub async fn lock_agent_handler() -> tokio::sync::MutexGuard<'static, ()> {
    crate::core::bus_testing::BUS_HANDLER_LOCK.lock().await
}

pub async fn run_dispatch_harness(options: DispatchHarnessOptions) -> DispatchHarnessObservation {
    // `init_global` + `register_native_global` mutate process-global state, so
    // concurrent harness runs (and concurrent `start_channels` calls) in the
    // same process can overwrite each other's handlers mid-run and produce
    // flaky assertions. Serialize the whole run (handler registration through
    // observation capture) behind the shared agent-handler lock.
    let _harness_guard = lock_agent_handler().await;

    crate::core::bus::init().await.expect("bus init");
    let mut event_rx = crate::core::bus::BUS
        .get()
        .expect("bus initialised")
        .receiver();
    let _ =
        crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins(
        );

    let handler_roles = Arc::new(Mutex::new(Vec::new()));
    let handler_text = Arc::new(Mutex::new(String::new()));
    let handler_provider = Arc::new(Mutex::new(String::new()));
    let handler_channel = Arc::new(Mutex::new(String::new()));
    let handler_progress = Arc::new(AtomicUsize::new(0));
    let response_text = options
        .response_text
        .clone()
        .unwrap_or_else(|| "dispatch ok".to_string());
    let handler_error = options.handler_error.clone();
    let handler_delay = Duration::from_millis(options.handler_delay_ms);

    BUS.native()
        .register::<AgentTurnRequest, AgentTurnResponse, _, _>(AGENT_RUN_TURN_METHOD, {
            let handler_roles = Arc::clone(&handler_roles);
            let handler_text = Arc::clone(&handler_text);
            let handler_provider = Arc::clone(&handler_provider);
            let handler_channel = Arc::clone(&handler_channel);
            let handler_progress = Arc::clone(&handler_progress);
            move |req| {
                let handler_roles = Arc::clone(&handler_roles);
                let handler_text = Arc::clone(&handler_text);
                let handler_provider = Arc::clone(&handler_provider);
                let handler_channel = Arc::clone(&handler_channel);
                let handler_progress = Arc::clone(&handler_progress);
                let response_text = response_text.clone();
                let handler_error = handler_error.clone();
                async move {
                    *handler_roles.lock().expect("roles lock") =
                        req.history.iter().map(|msg| msg.role.clone()).collect();
                    *handler_text.lock().expect("text lock") = req
                        .history
                        .iter()
                        .map(|msg| msg.content.as_str())
                        .collect::<Vec<_>>()
                        .join("\n---\n");
                    *handler_provider.lock().expect("provider lock") = req.provider_name;
                    *handler_channel.lock().expect("channel lock") = req.channel_name;

                    if let Some(tx) = req.on_progress {
                        handler_progress.fetch_add(1, Ordering::SeqCst);
                        let _ = tx.send(AgentProgress::TurnStarted).await;
                        let _ = tx
                            .send(AgentProgress::ThinkingDelta {
                                delta: "thinking".to_string(),
                                iteration: 1,
                            })
                            .await;
                        let _ = tx
                            .send(AgentProgress::TextDelta {
                                delta: "partial ".to_string(),
                                iteration: 1,
                            })
                            .await;
                        let _ = tx
                            .send(AgentProgress::ToolCallStarted {
                                call_id: "call-1".to_string(),
                                tool_name: "harness_tool".to_string(),
                                arguments: serde_json::json!({}),
                                iteration: 1,
                                display_label: None,
                                display_detail: None,
                            })
                            .await;
                    }

                    if !handler_delay.is_zero() {
                        tokio::time::sleep(handler_delay).await;
                    }

                    match handler_error {
                        Some(message) => Err(message),
                        None => Ok(AgentTurnResponse::new(response_text)),
                    }
                }
            }
        });

    let state = Arc::new(HarnessState::default());
    let channel_impl = Arc::new(HarnessChannel {
        name: options.channel_name.clone(),
        streaming: options.streaming,
        supports_reactions: options.supports_reactions,
        state: Arc::clone(&state),
    });
    let channel: Arc<dyn Channel> = channel_impl;
    let mut channels_by_name = HashMap::new();
    channels_by_name.insert(options.channel_name.clone(), channel);

    let model: Arc<dyn ChatModel<()>> = Arc::new(HarnessModel);
    let mut provider_cache = HashMap::new();
    provider_cache.insert(
        "harness-provider".to_string(),
        crate::openhuman::agent::tinyagents::TurnModelSource::from_model(Arc::clone(&model)),
    );
    let conversation_histories = Arc::new(Mutex::new(HashMap::new()));
    let history_key = if options.channel_name == "telegram" {
        format!("{}_alice_reply", options.channel_name)
    } else if let Some(thread_ts) = options.thread_ts.as_deref() {
        format!("{}_alice_reply_thread:{thread_ts}", options.channel_name)
    } else {
        format!("{}_alice_reply", options.channel_name)
    };
    if options.seed_history_len > 0 {
        conversation_histories.lock().expect("history lock").insert(
            history_key.clone(),
            (0..options.seed_history_len)
                .map(|idx| ChatMessage::assistant(format!("prior {idx} {}", "x".repeat(700))))
                .collect(),
        );
    }

    let harness_system_prompt = options
        .system_prompt
        .clone()
        .into_inner()
        .unwrap_or_else(|| ChannelSystemPrompt::fixed("system prompt"));
    let harness_workspace_dir = options
        .workspace_dir
        .clone()
        .unwrap_or_else(std::env::temp_dir);
    let ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        turn_model_source: Some(
            crate::openhuman::agent::tinyagents::TurnModelSource::from_model(model),
        ),
        default_provider: Arc::new("harness-provider".to_string()),
        memory: crate::openhuman::memory::guard::in_memory::guard_over(Arc::new(HarnessMemory {
            entries: options
                .memory_entries
                .into_iter()
                .map(memory_entry)
                .collect(),
        })),
        tools_registry: Arc::new(vec![Box::new(HarnessTool) as Box<dyn Tool>]),
        system_prompt: harness_system_prompt,
        model: Arc::new("harness-model".to_string()),
        temperature: 0.0,
        auto_save_memory: true,
        max_tool_iterations: 3,
        min_relevance_score: 0.2,
        conversation_histories: Arc::clone(&conversation_histories),
        turn_model_source_cache: Arc::new(Mutex::new(provider_cache)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(ReliabilityConfig::default()),
        provider_runtime_options: ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(harness_workspace_dir),
        message_timeout_secs: options.timeout_secs,
        multimodal: MultimodalConfig::default(),
        multimodal_files: MultimodalFileConfig::default(),
        config: None,
    });

    let expected_event_channel = options.channel_name.clone();

    let message = ChannelMessage {
        id: "m1".to_string(),
        sender: "alice".to_string(),
        reply_target: "reply".to_string(),
        content: options.content,
        channel: options.channel_name,
        timestamp: 1,
        thread_ts: options.thread_ts,
    };
    if let Some(inbound_envelope) = options.inbound_envelope {
        process_channel_runtime_message(
            Arc::clone(&ctx),
            RuntimeChannelMessage::with_inbound_envelope(message, inbound_envelope),
        )
        .await;
    } else {
        process_channel_message(Arc::clone(&ctx), message).await;
    }

    let received_event_envelope = loop {
        match event_rx.try_recv() {
            Ok(DomainEvent::ChannelMessageReceived {
                channel,
                message_id,
                inbound_envelope,
                ..
            }) if channel == expected_event_channel && message_id == "m1" => {
                break inbound_envelope;
            }
            Ok(_) => {}
            Err(_) => break None,
        }
    };

    let sends = state.sends.lock().await.clone();
    let handler_history_roles = handler_roles.lock().expect("roles lock").clone();
    let handler_history_text = handler_text.lock().expect("text lock").clone();
    let handler_provider_name = handler_provider.lock().expect("provider lock").clone();
    let handler_channel_name = handler_channel.lock().expect("channel lock").clone();
    let retained_history_len = conversation_histories
        .lock()
        .expect("history lock")
        .get(&history_key)
        .map(Vec::len)
        .unwrap_or_default();

    DispatchHarnessObservation {
        sends,
        start_typing_calls: state.start_typing_calls.load(Ordering::SeqCst),
        stop_typing_calls: state.stop_typing_calls.load(Ordering::SeqCst),
        received_event_envelope,
        handler_history_roles,
        handler_history_text,
        handler_provider_name,
        handler_channel_name,
        handler_had_progress: handler_progress.load(Ordering::SeqCst) > 0,
        retained_history_len,
    }
}
