//! Debug-build harnesses for raw integration coverage of the channel runtime.

use super::dispatch::process_channel_message;
pub use super::dispatch::test_support::{
    build_channel_context_block_for_test, select_acknowledgment_reaction_for_test,
};
pub use super::startup::test_support::resolve_yuanbao_app_secret_for_test;
use crate::core::event_bus::{init_global, register_native_global, DEFAULT_CAPACITY};
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::channels::context::{ChannelRuntimeContext, CHANNEL_MESSAGE_TIMEOUT_SECS};
use crate::openhuman::channels::traits::{ChannelMessage, SendMessage};
use crate::openhuman::channels::Channel;
use crate::openhuman::config::{MultimodalConfig, MultimodalFileConfig, ReliabilityConfig};
use crate::openhuman::inference::provider::{ChatMessage, Provider, ProviderRuntimeOptions};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use crate::openhuman::tools::{Tool, ToolResult};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DispatchHarnessOptions {
    pub channel_name: String,
    pub content: String,
    pub thread_ts: Option<String>,
    pub streaming: bool,
    pub supports_reactions: bool,
    pub response_text: Option<String>,
    pub handler_error: Option<String>,
    pub handler_delay_ms: u64,
    pub timeout_secs: u64,
    pub seed_history_len: usize,
    pub memory_entries: Vec<TestMemoryEntry>,
}

impl Default for DispatchHarnessOptions {
    fn default() -> Self {
        Self {
            channel_name: "test-channel".to_string(),
            content: "hello".to_string(),
            thread_ts: None,
            streaming: false,
            supports_reactions: false,
            response_text: Some("dispatch ok".to_string()),
            handler_error: None,
            handler_delay_ms: 0,
            timeout_secs: CHANNEL_MESSAGE_TIMEOUT_SECS,
            seed_history_len: 0,
            memory_entries: Vec::new(),
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

struct HarnessProvider;

#[async_trait]
impl Provider for HarnessProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok(format!("provider echo: {message}"))
    }
}

struct HarnessMemory {
    entries: Vec<MemoryEntry>,
}

#[async_trait]
impl Memory for HarnessMemory {
    fn name(&self) -> &str {
        "harness-memory"
    }

    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
    ) -> Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(self.entries.clone())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(self.entries.len())
    }

    async fn health_check(&self) -> bool {
        true
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
        taint: crate::openhuman::memory::MemoryTaint::Internal,
    }
}

/// Shared serialization guard for any test code that mutates the
/// process-global native agent-turn handler (`AGENT_RUN_TURN_METHOD`).
///
/// The dispatch harness registers a *mock* `AGENT_RUN_TURN_METHOD` handler,
/// while `start_channels` registers the *real* one (latest-wins on the global
/// registry). Both can run concurrently inside the same test binary, so the
/// real handler can clobber the harness's mock mid-run — producing flaky
/// assertions (e.g. `handler_had_progress` going false because the real
/// handler never feeds the harness progress channel). Every test path that
/// touches that global slot must hold this guard for the whole run.
fn agent_handler_lock() -> &'static tokio::sync::Mutex<()> {
    static HARNESS_GUARD: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    HARNESS_GUARD.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Acquire the shared agent-handler guard. Hold the returned guard across any
/// call that re-registers `AGENT_RUN_TURN_METHOD` (the harness, or
/// `start_channels`) so concurrent registrations cannot race in the same
/// process.
pub async fn lock_agent_handler() -> tokio::sync::MutexGuard<'static, ()> {
    agent_handler_lock().lock().await
}

pub async fn run_dispatch_harness(options: DispatchHarnessOptions) -> DispatchHarnessObservation {
    // `init_global` + `register_native_global` mutate process-global state, so
    // concurrent harness runs (and concurrent `start_channels` calls) in the
    // same process can overwrite each other's handlers mid-run and produce
    // flaky assertions. Serialize the whole run (handler registration through
    // observation capture) behind the shared agent-handler lock.
    let _harness_guard = lock_agent_handler().await;

    init_global(DEFAULT_CAPACITY);
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

    register_native_global::<AgentTurnRequest, AgentTurnResponse, _, _>(AGENT_RUN_TURN_METHOD, {
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
                        })
                        .await;
                }

                if !handler_delay.is_zero() {
                    tokio::time::sleep(handler_delay).await;
                }

                match handler_error {
                    Some(message) => Err(message),
                    None => Ok(AgentTurnResponse {
                        text: response_text,
                    }),
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

    let provider: Arc<dyn Provider> = Arc::new(HarnessProvider);
    let mut provider_cache = HashMap::new();
    provider_cache.insert("harness-provider".to_string(), Arc::clone(&provider));
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

    let ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name: Arc::new(channels_by_name),
        provider,
        default_provider: Arc::new("harness-provider".to_string()),
        memory: Arc::new(HarnessMemory {
            entries: options
                .memory_entries
                .into_iter()
                .map(memory_entry)
                .collect(),
        }),
        tools_registry: Arc::new(vec![Box::new(HarnessTool) as Box<dyn Tool>]),
        system_prompt: Arc::new("system prompt".to_string()),
        model: Arc::new("harness-model".to_string()),
        temperature: 0.0,
        auto_save_memory: true,
        max_tool_iterations: 3,
        min_relevance_score: 0.2,
        conversation_histories: Arc::clone(&conversation_histories),
        provider_cache: Arc::new(Mutex::new(provider_cache)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: None,
        inference_url: None,
        reliability: Arc::new(ReliabilityConfig::default()),
        provider_runtime_options: ProviderRuntimeOptions::default(),
        workspace_dir: Arc::new(PathBuf::from(std::env::temp_dir())),
        message_timeout_secs: options.timeout_secs,
        multimodal: MultimodalConfig::default(),
        multimodal_files: MultimodalFileConfig::default(),
    });

    process_channel_message(
        Arc::clone(&ctx),
        ChannelMessage {
            id: "m1".to_string(),
            sender: "alice".to_string(),
            reply_target: "reply".to_string(),
            content: options.content,
            channel: options.channel_name,
            timestamp: 1,
            thread_ts: options.thread_ts,
        },
    )
    .await;

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
        handler_history_roles,
        handler_history_text,
        handler_provider_name,
        handler_channel_name,
        handler_had_progress: handler_progress.load(Ordering::SeqCst) > 0,
        retained_history_len,
    }
}
