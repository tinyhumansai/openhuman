#![cfg(not(windows))]

use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
use openhuman_core::openhuman::agent::harness::run_queue::RunQueue;
use openhuman_core::openhuman::agent::harness::session::Agent;
use openhuman_core::openhuman::agent::host_runtime::NativeRuntime;
use openhuman_core::openhuman::agent::tinyagents::thread_context::with_thread_id;
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::security::{AuditLogger, AutonomyLevel, SecurityPolicy};
use openhuman_core::openhuman::subconscious::monitors::tools::{
    MonitorListTool, MonitorReadTool, MonitorStopTool, MonitorTool,
};
use openhuman_core::openhuman::tools::Tool;
use parking_lot::Mutex;
use serde_json::json;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tempfile::TempDir;
use tinyagents::harness::message::{AssistantMessage, Message};
use tinyagents::harness::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyagents::harness::tool::ToolCall;
use tokio::time::{sleep, Duration};

type ResponseFactory = Box<dyn Fn(&[Message]) -> ModelResponse + Send + Sync>;

enum ModelStep {
    Static(ModelResponse),
    Delayed(Duration, ModelResponse),
    FromHistory(ResponseFactory),
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    messages: Vec<Message>,
    tool_names: Vec<String>,
}

struct ScriptedModel {
    steps: Mutex<VecDeque<ModelStep>>,
    requests: Mutex<Vec<CapturedRequest>>,
}

fn monitor_e2e_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

impl ScriptedModel {
    fn new(steps: Vec<ModelStep>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into()),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().clone()
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: OnceLock<ModelProfile> = OnceLock::new();
        Some(PROFILE.get_or_init(|| {
            let mut profile = ModelProfile::default();
            profile.tool_calling = true;
            profile.parallel_tool_calls = true;
            profile
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        let messages = request.messages;
        self.requests.lock().push(CapturedRequest {
            messages: messages.clone(),
            tool_names: request.tools.iter().map(|tool| tool.name.clone()).collect(),
        });

        let step = self.steps.lock().pop_front();
        match step {
            Some(ModelStep::Static(response)) => Ok(response),
            Some(ModelStep::Delayed(delay, response)) => {
                sleep(delay).await;
                Ok(response)
            }
            Some(ModelStep::FromHistory(factory)) => Ok(factory(&messages)),
            None => Ok(text_response("default monitor final")),
        }
    }
}

#[derive(Default)]
struct StubMemory {
    entries: Mutex<Vec<MemoryEntry>>,
}

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "monitor-agent-e2e-memory"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()> {
        let mut entries = self.entries.lock();
        let id = format!("{namespace}:{key}:{}", entries.len());
        entries.push(MemoryEntry {
            id,
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            category,
            timestamp: "2026-06-04T00:00:00Z".to_string(),
            session_id: session_id.map(str::to_string),
            score: Some(0.9),
            taint: Default::default(),
        });
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        limit: usize,
        _opts: RecallOpts<'_>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(self.entries.lock().iter().take(limit).cloned().collect())
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .iter()
            .find(|entry| entry.namespace.as_deref() == Some(namespace) && entry.key == key)
            .cloned())
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        Ok(self
            .entries
            .lock()
            .iter()
            .filter(|entry| namespace.is_none_or(|ns| entry.namespace.as_deref() == Some(ns)))
            .filter(|entry| category.is_none_or(|cat| &entry.category == cat))
            .filter(|entry| session_id.is_none_or(|sid| entry.session_id.as_deref() == Some(sid)))
            .cloned()
            .collect())
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool> {
        let mut entries = self.entries.lock();
        let before = entries.len();
        entries.retain(|entry| entry.namespace.as_deref() != Some(namespace) || entry.key != key);
        Ok(entries.len() != before)
    }

    async fn namespace_summaries(&self) -> Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> Result<usize> {
        Ok(self.entries.lock().len())
    }

    async fn health_check(&self) -> bool {
        true
    }
}

fn text_response(text: &str) -> ModelResponse {
    ModelResponse::assistant(text)
}

fn tool_response(id: &str, name: &str, arguments: serde_json::Value) -> ModelResponse {
    ModelResponse {
        message: AssistantMessage {
            id: None,
            content: Vec::new(),
            tool_calls: vec![ToolCall::new(id, name, arguments)],
            usage: None,
        },
        usage: None,
        finish_reason: Some("tool_calls".to_string()),
        raw: None,
        resolved_model: None,
        continue_turn: None,
    }
}

fn all_messages_text(messages: &[Message]) -> String {
    messages
        .iter()
        .map(Message::text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_monitor_id(messages: &[Message]) -> String {
    let text = all_messages_text(messages);
    let start = text
        .find("mon_")
        .unwrap_or_else(|| panic!("expected monitor id in messages:\n{text}"));
    text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect()
}

fn contains_tool_json_pair(text: &str, key: &str, value: &str) -> bool {
    text.contains(&format!("\"{key}\":\"{value}\""))
        || text.contains(&format!("\\\"{key}\\\":\\\"{value}\\\""))
}

fn monitor_tools(workspace: &Path, autonomy: AutonomyLevel) -> Vec<Box<dyn Tool>> {
    let security = Arc::new(SecurityPolicy {
        autonomy,
        workspace_dir: workspace.to_path_buf(),
        action_dir: workspace.to_path_buf(),
        ..SecurityPolicy::default()
    });
    vec![
        Box::new(MonitorTool::new(
            security,
            Arc::new(NativeRuntime::new()),
            AuditLogger::disabled(),
        )),
        Box::new(MonitorListTool),
        Box::new(MonitorStopTool),
        Box::new(MonitorReadTool),
    ]
}

fn build_agent(
    workspace: &Path,
    provider: Arc<ScriptedModel>,
    tools: Vec<Box<dyn Tool>>,
    max_tool_iterations: usize,
) -> Agent {
    let mut agent = Agent::builder()
        .chat_model(provider)
        .tools(tools)
        .memory(Arc::new(StubMemory::default()))
        .tool_dispatcher(Box::new(NativeToolDispatcher))
        .config(AgentConfig {
            max_tool_iterations,
            max_history_messages: 16,
            ..AgentConfig::default()
        })
        .model_name("monitor-e2e-model".to_string())
        .temperature(0.0)
        .workspace_dir(workspace.to_path_buf())
        .workflows(Vec::new())
        .auto_save(false)
        .event_context("monitor-e2e-session", "monitor-e2e-channel")
        .agent_definition_name("orchestrator")
        .omit_profile(true)
        .omit_memory_md(true)
        .explicit_preferences_enabled(false)
        .build()
        .unwrap();
    agent.set_connected_integrations(Vec::new());
    agent
}

async fn run_monitor_turn(
    tmp: &TempDir,
    provider: Arc<ScriptedModel>,
    autonomy: AutonomyLevel,
    max_tool_iterations: usize,
) -> String {
    let mut agent = build_agent(
        tmp.path(),
        provider,
        monitor_tools(tmp.path(), autonomy),
        max_tool_iterations,
    );
    agent.set_run_queue(Some(RunQueue::new()));
    with_thread_id("monitor-agent-e2e-thread", async move {
        agent
            .turn("Start a background monitor and react when it reports.")
            .await
            .unwrap()
    })
    .await
}

#[tokio::test]
async fn orchestrator_monitor_line_reaches_next_llm_call_as_collect_context() {
    let _guard = monitor_e2e_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let provider = ScriptedModel::new(vec![
        ModelStep::Static(tool_response(
            "call-monitor",
            "monitor",
            json!({
                "command": "printf 'MONITOR_READY\\n'; sleep 0.1",
                "description": "e2e line injection monitor",
                "timeout_ms": 2_000,
                "persistent": false
            }),
        )),
        ModelStep::Delayed(
            Duration::from_millis(150),
            tool_response("call-list", "monitor_list", json!({})),
        ),
        ModelStep::FromHistory(Box::new(|messages| {
            let text = all_messages_text(messages);
            assert!(
                text.contains("[Additional context from user]: [Monitor mon_"),
                "expected monitor collect injection in messages:\n{text}"
            );
            assert!(
                text.contains("MONITOR_READY"),
                "expected monitor line in messages:\n{text}"
            );
            text_response("orchestrator observed monitor event")
        })),
    ]);

    let answer = run_monitor_turn(&tmp, provider.clone(), AutonomyLevel::Supervised, 5).await;

    assert_eq!(answer, "orchestrator observed monitor event");
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].tool_names.contains(&"monitor".to_string()));
    assert!(requests[0].tool_names.contains(&"monitor_list".to_string()));
    assert!(requests[2]
        .messages
        .iter()
        .any(|message| message.text().contains("MONITOR_READY")));
}

#[tokio::test]
async fn orchestrator_reads_monitor_output_after_registration() {
    let _guard = monitor_e2e_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let provider = ScriptedModel::new(vec![
        ModelStep::Static(tool_response(
            "call-monitor",
            "monitor",
            json!({
                "command": "printf 'READBACK_LINE\\n'",
                "description": "e2e monitor read",
                "timeout_ms": 2_000,
                "persistent": false
            }),
        )),
        ModelStep::Delayed(
            Duration::from_millis(120),
            tool_response("call-list", "monitor_list", json!({})),
        ),
        ModelStep::FromHistory(Box::new(|messages| {
            let monitor_id = first_monitor_id(messages);
            tool_response(
                "call-read",
                "monitor_read",
                json!({ "monitor_id": monitor_id, "max_bytes": 4096 }),
            )
        })),
        ModelStep::FromHistory(Box::new(|messages| {
            let text = all_messages_text(messages);
            assert!(text.contains("READBACK_LINE"));
            text_response("orchestrator read monitor output")
        })),
    ]);

    let answer = run_monitor_turn(&tmp, provider, AutonomyLevel::Supervised, 6).await;

    assert_eq!(answer, "orchestrator read monitor output");
}

#[tokio::test]
async fn orchestrator_stops_a_running_monitor_by_id_from_tool_result() {
    let _guard = monitor_e2e_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let provider = ScriptedModel::new(vec![
        ModelStep::Static(tool_response(
            "call-monitor",
            "monitor",
            json!({
                "command": "sleep 5",
                "description": "e2e stoppable monitor",
                "timeout_ms": 5_000,
                "persistent": false
            }),
        )),
        ModelStep::FromHistory(Box::new(|messages| {
            let monitor_id = first_monitor_id(messages);
            tool_response(
                "call-stop",
                "monitor_stop",
                json!({ "monitor_id": monitor_id }),
            )
        })),
        ModelStep::FromHistory(Box::new(|messages| {
            let text = all_messages_text(messages);
            assert!(
                contains_tool_json_pair(&text, "status", "stopped"),
                "expected stopped status in messages:\n{text}"
            );
            text_response("orchestrator stopped monitor")
        })),
    ]);

    let answer = run_monitor_turn(&tmp, provider, AutonomyLevel::Supervised, 5).await;

    assert_eq!(answer, "orchestrator stopped monitor");
}

#[tokio::test]
async fn orchestrator_sees_monitor_timeout_status_through_list() {
    let _guard = monitor_e2e_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let provider = ScriptedModel::new(vec![
        ModelStep::Static(tool_response(
            "call-monitor",
            "monitor",
            json!({
                "command": "sleep 1",
                "description": "e2e timed monitor",
                "timeout_ms": 40,
                "persistent": false
            }),
        )),
        ModelStep::Delayed(
            Duration::from_millis(120),
            tool_response("call-list", "monitor_list", json!({})),
        ),
        ModelStep::FromHistory(Box::new(|messages| {
            let text = all_messages_text(messages);
            assert!(
                text.contains("e2e timed monitor")
                    && contains_tool_json_pair(&text, "status", "timed_out"),
                "expected timed_out status in messages:\n{text}"
            );
            text_response("orchestrator saw monitor timeout")
        })),
    ]);

    let answer = run_monitor_turn(&tmp, provider, AutonomyLevel::Supervised, 5).await;

    assert_eq!(answer, "orchestrator saw monitor timeout");
}

#[tokio::test]
async fn orchestrator_gets_denial_when_monitor_command_violates_policy() {
    let _guard = monitor_e2e_lock().lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let provider = ScriptedModel::new(vec![
        ModelStep::Static(tool_response(
            "call-monitor",
            "monitor",
            json!({
                "command": "touch denied-monitor-file",
                "description": "e2e denied monitor",
                "timeout_ms": 2_000,
                "persistent": false
            }),
        )),
        ModelStep::FromHistory(Box::new(|messages| {
            let text = all_messages_text(messages);
            assert!(
                text.contains("[policy-blocked] Tool 'monitor' was blocked by the security policy"),
                "expected policy denial in messages:\n{text}"
            );
            assert!(
                !text.contains("\"monitorId\":\"mon_"),
                "denied monitor must not return a monitor id:\n{text}"
            );
            text_response("orchestrator received monitor denial")
        })),
    ]);

    let answer = run_monitor_turn(&tmp, provider, AutonomyLevel::ReadOnly, 4).await;

    assert_eq!(answer, "orchestrator received monitor denial");
    assert!(!tmp.path().join("denied-monitor-file").exists());
}
