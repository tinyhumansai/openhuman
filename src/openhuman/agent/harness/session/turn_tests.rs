use super::*;
use crate::openhuman::agent::dispatcher::{
    PFormatToolDispatcher, ToolDispatcher, XmlToolDispatcher,
};
use crate::openhuman::agent::experience::{
    AgentExperience, AgentExperienceStore, ExperienceOutcome, ExperienceSource,
};
use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage};
use crate::openhuman::agent::tool_policy::{
    GeneratedToolRuntimeContext, GeneratedToolRuntimeRisk, ToolPolicy, ToolPolicyDecision,
    ToolPolicyRequest,
};
use crate::openhuman::inference::provider::{ChatResponse, UsageInfo};
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::ToolResult;
use crate::openhuman::tools::{PermissionLevel, Tool};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tinyinference::message::Message;
use tinyinference::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::Notify;
use tokio::time::{timeout, Duration};

struct DummyProvider;

#[async_trait]
impl ChatModel<()> for DummyProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(ModelProfile::default);
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Ok(ModelResponse::assistant("unused"))
    }
}

struct SequenceProvider {
    responses: AsyncMutex<Vec<anyhow::Result<ChatResponse>>>,
    requests: AsyncMutex<Vec<Vec<ChatMessage>>>,
    /// Number of tool declarations the provider was offered on each call, in
    /// call order. Lets a test assert per-turn tool suppression (#1725): a
    /// chat/small-talk turn must send an empty tool schema.
    tool_counts: AsyncMutex<Vec<usize>>,
}

#[async_trait]
impl ChatModel<()> for SequenceProvider {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::LazyLock<ModelProfile> =
            std::sync::LazyLock::new(ModelProfile::default);
        Some(&PROFILE)
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.tool_counts.lock().await.push(request.tools.len());
        self.requests.lock().await.push(
            request
                .messages
                .iter()
                .map(|message| ChatMessage {
                    id: None,
                    role: match message {
                        Message::System(_) => "system",
                        Message::User(_) => "user",
                        Message::Assistant(_) => "assistant",
                        // SequenceProvider replaces the old prompt-guided
                        // Provider fixture. Its wire adapter flattened tool
                        // results into a user turn rather than sending the
                        // native `tool` role.
                        Message::Tool(_) => "user",
                    }
                    .to_string(),
                    content: match message {
                        Message::Tool(_) => format!("[Tool results]\n{}", message.text()),
                        _ => message.text(),
                    },
                    extra_metadata: None,
                })
                .collect(),
        );
        match self.responses.lock().await.remove(0) {
            Ok(response) => Ok(
                crate::openhuman::agent::tinyagents::model::native_model_response_for_request(
                    &response, &request,
                ),
            ),
            Err(error) => Err(tinyinference::Error::Model(error.to_string())),
        }
    }

    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        // The legacy fixture implemented `chat` but did not write provider
        // deltas. Preserve that non-streaming wire behavior: the harness still
        // receives the authoritative completed response, while turn-owned
        // continuation deltas remain independently observable.
        let response = self.invoke(state, request).await?;
        Ok(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::Completed(response),
        ])))
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "echo"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("echo-output"))
    }
}

struct CronAddProbeTool;

#[async_trait]
impl Tool for CronAddProbeTool {
    fn name(&self) -> &str {
        "cron_add"
    }

    fn description(&self) -> &str {
        "cron add probe"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success(format!("cron_add_args={args}")))
    }
}

struct CountingTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "counting"
    }

    fn description(&self) -> &str {
        "counting"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("counting-output"))
    }
}

struct DenyCountingPolicy;

#[async_trait]
impl ToolPolicy for DenyCountingPolicy {
    fn name(&self) -> &str {
        "deny_counting"
    }

    async fn check(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
        assert_eq!(request.tool_name, "counting");
        assert_eq!(request.context.session_id, "turn-test-session");
        assert_eq!(request.context.channel, "turn-test-channel");
        assert_eq!(request.context.agent_definition_id, "main");
        assert_eq!(request.context.call_id, "policy-1");
        assert_eq!(request.context.iteration, 1);
        ToolPolicyDecision::deny("locked by test policy")
    }
}

struct LongTool;

#[async_trait]
impl Tool for LongTool {
    fn name(&self) -> &str {
        "long"
    }

    fn description(&self) -> &str {
        "long"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success("x".repeat(800)))
    }
}

struct CountingWriteTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingWriteTool {
    fn name(&self) -> &str {
        "write_notes"
    }

    fn description(&self) -> &str {
        "write notes"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("write-output"))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
}

struct GeneratedContextTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for GeneratedContextTool {
    fn name(&self) -> &str {
        "generated_send"
    }

    fn description(&self) -> &str {
        "generated send"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::success("generated-output"))
    }

    fn host_call_extension(
        &self,
        _args: &serde_json::Value,
    ) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        Some(Box::new(GeneratedToolRuntimeContext {
            provider_id: "mail.runtime".to_string(),
            capability_id: "email.send".to_string(),
            risk: GeneratedToolRuntimeRisk::ExternalWrite,
            source_digest: Some("sha256:abc".to_string()),
            approval_id: Some("approval-1".to_string()),
        }))
    }
}

struct RequireGeneratedContextPolicy;

#[async_trait]
impl ToolPolicy for RequireGeneratedContextPolicy {
    fn name(&self) -> &str {
        "require_generated_context"
    }

    async fn check(&self, request: &ToolPolicyRequest) -> ToolPolicyDecision {
        let context = request
            .generated_tool
            .as_ref()
            .expect("generated tool context should be threaded");
        assert_eq!(context.provider_id, "mail.runtime");
        assert_eq!(context.capability_id, "email.send");
        assert_eq!(context.risk, GeneratedToolRuntimeRisk::ExternalWrite);
        assert_eq!(context.approval_id.as_deref(), Some("approval-1"));
        ToolPolicyDecision::require_approval("generated context requires approval")
    }
}

struct RecordingHook {
    calls: Arc<AsyncMutex<Vec<TurnContext>>>,
    notify: Arc<Notify>,
}

#[async_trait]
impl PostTurnHook for RecordingHook {
    fn name(&self) -> &str {
        "recording"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        self.calls.lock().await.push(ctx.clone());
        self.notify.notify_waiters();
        Ok(())
    }
}

/// Point `OPENHUMAN_WORKSPACE` at a scratch directory for the lifetime of a
/// test, restoring the previous value on drop.
///
/// Needed by any test that lets the harness reach `Config::load_or_init()` —
/// notably the triggered `agent_memory` path, whose deterministic fast path
/// (`subagent_runner::ops::runner::try_deterministic_memory_retrieval`, #4677)
/// loads the **host** config and queries the real memory tree behind it, not
/// the `Memory` handed to the `Agent` under test. Without this the test reads
/// the developer's own `~/.openhuman`: on a populated machine `fast_retrieve`
/// returns hits, the fast path short-circuits with zero provider calls, and the
/// mock provider's queued responses land on the wrong turns. CI has an empty
/// home, so the failure only ever reproduces locally.
///
/// Same shape as the guards in `memory::ops::files` / `memory::query::
/// test_workspace`; `TEST_ENV_LOCK` serializes it against them.
struct WorkspaceEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        unsafe {
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
        }
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(previous) = self.previous.take() {
                std::env::set_var("OPENHUMAN_WORKSPACE", previous);
            } else {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            }
        }
    }
}

fn make_agent(visible_tool_names: Option<HashSet<String>>) -> Agent {
    make_agent_with_tool_sets(vec![Box::new(EchoTool)], Vec::new(), visible_tool_names)
}

/// [`make_agent`] with caller-chosen durable and synthesised tool sets.
fn make_agent_with_tool_sets(
    tools: Vec<Box<dyn Tool>>,
    synthesized_tools: Vec<Box<dyn Tool>>,
    visible_tool_names: Option<HashSet<String>>,
) -> Agent {
    // The embedding seam fails loudly when unwired; before the memory
    // extraction this was a direct call and needed no setup.
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    // `create_memory` reaches the engine's embedding seam, which fails loudly
    // when unwired rather than degrading — a deliberate choice, so an unwired
    // host cannot corrupt an embedding space quietly. Nothing in a unit test
    // runs the startup wiring that installs it, so the helper installs it
    // itself. `install_for_tests` is idempotent (a `Once`), so every helper in
    // this file calling it costs one install for the whole binary.
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    let mut builder = Agent::builder()
        .chat_model(Arc::new(DummyProvider))
        .tools(tools)
        .synthesized_tools(synthesized_tools)
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("turn-test-session", "turn-test-channel")
        .config(crate::openhuman::config::AgentConfig {
            max_history_messages: 3,
            ..crate::openhuman::config::AgentConfig::default()
        });

    if let Some(names) = visible_tool_names {
        builder = builder.visible_tool_names(names);
    }

    builder.build().unwrap()
}

fn make_agent_with_builder(
    provider: Arc<dyn ChatModel<()>>,
    tools: Vec<Box<dyn Tool>>,
    post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    config: crate::openhuman::config::AgentConfig,
    context_config: crate::openhuman::config::ContextConfig,
) -> Agent {
    make_agent_with_builder_and_dispatcher(
        provider,
        tools,
        post_turn_hooks,
        config,
        context_config,
        Box::new(XmlToolDispatcher),
    )
}

fn make_agent_with_builder_and_dispatcher(
    provider: Arc<dyn ChatModel<()>>,
    tools: Vec<Box<dyn Tool>>,
    post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    config: crate::openhuman::config::AgentConfig,
    context_config: crate::openhuman::config::ContextConfig,
    tool_dispatcher: Box<dyn ToolDispatcher>,
) -> Agent {
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let _memory_cfg = crate::openhuman::config::MemoryConfig {
        backend: "none".into(),
        ..crate::openhuman::config::MemoryConfig::default()
    };
    // The embedding seam, as above.
    let mem: Arc<dyn Memory> = crate::openhuman::memory::test_support::noop_memory();

    Agent::builder()
        .chat_model(provider)
        .tools(tools)
        .memory(mem)
        .tool_dispatcher(tool_dispatcher)
        .post_turn_hooks(post_turn_hooks)
        .config(config)
        .context_config(context_config)
        .workspace_dir(workspace_path)
        .auto_save(true)
        .event_context("turn-test-session", "turn-test-channel")
        .build()
        .unwrap()
}

// Removed: execute_tool_call_applies_inline_result_budget — see the note above;
// it exercised the deleted direct tool executor.

// ── Explicit-preferences narrow path ──────────────────────────────────────────
//
// These tests verify that `fetch_learned_context` correctly handles the three
// flag combinations:
//  1. both flags off   → empty context
//  2. explicit_preferences_enabled=true, learning_enabled=false
//     → only general user_pref entries returned, no inference data
//  3. learning_enabled=true  → full path (existing tests cover this; we only
//     verify that explicit entries are included as well)
//
// We use the real `UnifiedMemory` backend (sqlite) so the list/store round-trip
// is exercised end-to-end without mocking the memory layer.

fn make_agent_with_memory(
    memory: Arc<dyn Memory>,
    workspace_dir: std::path::PathBuf,
    learning_enabled: bool,
    explicit_preferences_enabled: bool,
) -> Agent {
    Agent::builder()
        .chat_model(Arc::new(DummyProvider))
        .tools(vec![])
        .memory(memory)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_dir)
        .event_context("pref-test-session", "pref-test-channel")
        .learning_enabled(learning_enabled)
        .explicit_preferences_enabled(explicit_preferences_enabled)
        .build()
        .unwrap()
}

fn make_real_memory(_workspace: &std::path::Path) -> Arc<dyn Memory> {
    Arc::new(crate::openhuman::memory::tool_memory::test_helpers::MockMemory::default())
}

// ── bound_cached_transcript_messages — TAURI-RUST-7 trailing-strip ─────
//
// `bound_cached_transcript_messages` operates on a `Vec<ChatMessage>` (the
// dispatcher-serialised wire format), so its detection runs through
// `assistant_message_has_tool_calls`. Verify the symmetric trailing-strip
// pops unpaired tool_calls envelopes while leaving plain assistant replies
// untouched.

fn tool_calls_envelope(id: &str) -> String {
    serde_json::json!({
        "content": "calling tool",
        "tool_calls": [{
            "id": id,
            "name": "shell",
            "arguments": "{}"
        }]
    })
    .to_string()
}

#[path = "turn_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "turn_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "turn_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "turn_tests_part_04_tests.rs"]
mod part_04_tests;
#[path = "turn_tests_part_05_tests.rs"]
mod part_05_tests;
