use anyhow::Result;
use async_trait::async_trait;
use openhuman_core::openhuman::agent::debug::{
    dump_agent_prompt, write_prompt_dumps, DumpPromptOptions, DumpedPrompt,
};
use openhuman_core::openhuman::agent::harness::archivist::ArchivistHook;
use openhuman_core::openhuman::agent::harness::{
    run_subagent, with_parent_context, AgentDefinition, DefinitionSource, ModelSpec,
    ParentExecutionContext, PromptSource, SandboxMode, SubagentRunError, SubagentRunOptions,
    ToolScope,
};
use openhuman_core::openhuman::agent::hooks::{PostTurnHook, ToolCallRecord, TurnContext};
use openhuman_core::openhuman::config::AgentConfig;
use openhuman_core::openhuman::context::prompt::ToolCallFormat;
use openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities;
use openhuman_core::openhuman::inference::provider::{
    ChatRequest, ChatResponse, Provider, ToolCall, UsageInfo,
};
use openhuman_core::openhuman::memory::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use openhuman_core::openhuman::memory_store::{events, fts5, profile, segments};
use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use parking_lot::Mutex;
use rusqlite::Connection;
use serde_json::json;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

struct ScriptedProvider {
    responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
    requests: Mutex<Vec<String>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<anyhow::Result<ChatResponse>>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(VecDeque::from(responses)),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().clone()
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            vision: false,
        }
    }

    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        message: &str,
        _model: &str,
        _temperature: f64,
    ) -> Result<String> {
        Ok(format!("summary:{message}"))
    }

    async fn chat(
        &self,
        request: ChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> Result<ChatResponse> {
        self.requests.lock().push(
            request
                .messages
                .iter()
                .map(|message| format!("{}:{}", message.role, message.content))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        self.responses
            .lock()
            .pop_front()
            .unwrap_or_else(|| Ok(text_response("fallback final")))
    }
}

struct StubMemory;

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "round21-memory"
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
        Ok(Vec::new())
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
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Deterministic round21 echo"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        Ok(ToolResult::success(format!("echo:{args}")))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }
}

fn setup_conn() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    conn.execute_batch(fts5::EPISODIC_INIT_SQL)
        .expect("episodic schema");
    conn.execute_batch(segments::SEGMENTS_INIT_SQL)
        .expect("segments schema");
    conn.execute_batch(events::EVENTS_INIT_SQL)
        .expect("events schema");
    conn.execute_batch(profile::PROFILE_INIT_SQL)
        .expect("profile schema");
    Arc::new(Mutex::new(conn))
}

fn turn(session_id: &str, user_message: &str, assistant_response: &str) -> TurnContext {
    TurnContext {
        user_message: user_message.to_string(),
        assistant_response: assistant_response.to_string(),
        tool_calls: Vec::new(),
        turn_duration_ms: 25,
        session_id: Some(session_id.to_string()),
        agent_id: Some("round21_agent".to_string()),
        entrypoint: Some("raw-e2e".to_string()),
        iteration_count: 1,
    }
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.to_string()),
        tool_calls: Vec::new(),
        usage: Some(UsageInfo {
            input_tokens: 13,
            output_tokens: 5,
            context_window: 8192,
            cached_input_tokens: 2,
            charged_amount_usd: 0.001,
        }),
        reasoning_content: None,
    }
}

fn tool_response(name: &str, arguments: serde_json::Value) -> ChatResponse {
    ChatResponse {
        text: Some("calling echo".to_string()),
        tool_calls: vec![ToolCall {
            id: "round21-call".to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
            extra_content: None,
        }],
        usage: None,
        reasoning_content: Some("scripted tool use".to_string()),
    }
}

fn definition(max_iterations: usize) -> AgentDefinition {
    AgentDefinition {
        id: "round21_worker".to_string(),
        when_to_use: "round21 raw coverage".to_string(),
        display_name: Some("Round 21 Worker".to_string()),
        system_prompt: PromptSource::Inline("Round21 worker prompt".to_string()),
        omit_identity: true,
        omit_memory_context: false,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        trigger_memory_agent: Default::default(),
        model: ModelSpec::Inherit,
        temperature: 0.0,
        tools: ToolScope::Named(vec!["echo".to_string()]),
        disallowed_tools: Vec::new(),
        skill_filter: None,
        extra_tools: Vec::new(),
        max_iterations,
        iteration_policy: Default::default(),
        max_result_chars: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        subagents: Vec::new(),
        delegate_name: None,
        agent_tier: Default::default(),
        source: DefinitionSource::Builtin,
    }
}

fn parent_context(workspace: &Path, provider: Arc<ScriptedProvider>) -> ParentExecutionContext {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(EchoTool)];
    let specs = tools.iter().map(|tool| tool.spec()).collect();
    ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: [
            "test".to_string(),
            "archivist".to_string(),
            "summarizer".to_string(),
        ]
        .into_iter()
        .collect(),
        provider,
        all_tools: Arc::new(tools),
        all_tool_specs: Arc::new(specs),
        model_name: "round21-parent-model".to_string(),
        temperature: 0.0,
        workspace_dir: workspace.to_path_buf(),
        memory: Arc::new(StubMemory),
        agent_config: AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(Some("parent memory".to_string())),
        session_id: "round21-parent-session".to_string(),
        channel: "round21-channel".to_string(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::Native,
        session_key: "1700000000_round21_parent".to_string(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

#[tokio::test]
async fn archivist_flush_finalizes_open_segment_and_extracts_profile_events() -> Result<()> {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);
    let session = "round21-archivist-session";

    hook.on_turn_complete(&turn(
        session,
        "I prefer concise updates. I am a maintainer based in Oakland.",
        "Noted for future replies.",
    ))
    .await?;

    let open_before = segments::open_segment_for_session(&conn, session)?;
    assert!(open_before.is_some());
    assert_eq!(hook.rolling_segment_recap(session).await, None);

    hook.flush_open_segment(session).await;

    assert!(segments::open_segment_for_session(&conn, session)?.is_none());
    let closed = segments::segments_by_namespace(&conn, "global", 10)?
        .into_iter()
        .find(|segment| segment.session_id == session)
        .expect("closed segment");
    assert_eq!(closed.status, segments::SegmentStatus::Summarised);
    assert!(closed.summary.as_deref().unwrap_or("").contains("prefer"));

    let preference_events = events::events_by_type(&conn, "global", "preference", 10)?;
    assert!(preference_events
        .iter()
        .any(|event| event.content.contains("prefer concise updates")));
    let profile_facets = profile::profile_select_all(&conn)?;
    assert!(profile_facets
        .iter()
        .any(|facet| facet.value.contains("prefer concise updates")));
    Ok(())
}

#[tokio::test]
async fn archivist_disabled_and_unknown_session_paths_are_noops() -> Result<()> {
    let conn = setup_conn();
    let disabled = ArchivistHook::disabled();
    assert_eq!(disabled.name(), "archivist");
    disabled
        .on_turn_complete(&TurnContext {
            user_message: "ignored".to_string(),
            assistant_response: "ignored".to_string(),
            tool_calls: vec![ToolCallRecord {
                name: "shell".to_string(),
                arguments: json!({"cmd": "false"}),
                success: false,
                output_summary: "shell: failed (error)".to_string(),
                duration_ms: 1,
            }],
            turn_duration_ms: 1,
            session_id: None,
            agent_id: None,
            entrypoint: None,
            iteration_count: 1,
        })
        .await?;

    assert!(fts5::episodic_session_entries(&conn, "unknown")?.is_empty());
    let enabled = ArchivistHook::new(conn, true);
    enabled.flush_open_segment("missing-session").await;
    assert_eq!(enabled.rolling_segment_recap("missing-session").await, None);
    Ok(())
}

#[tokio::test]
async fn subagent_no_parent_and_checkpoint_fallback_are_deterministic() -> Result<()> {
    let no_parent = run_subagent(
        &definition(1),
        "outside a parent context",
        SubagentRunOptions::default(),
    )
    .await
    .expect_err("run without parent should fail");
    assert!(matches!(no_parent, SubagentRunError::NoParentContext));

    let tmp = TempDir::new()?;
    let provider = ScriptedProvider::new(vec![
        Ok(tool_response("echo", json!({"message": "first"}))),
        Err(anyhow::anyhow!("checkpoint model unavailable")),
    ]);
    let parent = parent_context(tmp.path(), provider.clone());

    let outcome = with_parent_context(parent, async {
        run_subagent(
            &definition(1),
            "use the echo tool once",
            SubagentRunOptions {
                task_id: Some("round21-task".to_string()),
                ..SubagentRunOptions::default()
            },
        )
        .await
    })
    .await?;

    assert_eq!(outcome.iterations, 1);
    assert!(outcome.output.contains("tool-call limit (1 steps)"));
    assert!(outcome
        .output
        .contains("echo [ok]: echo:{\"message\":\"first\"}"));
    assert!(provider
        .requests()
        .iter()
        .any(|request| request.contains("reached your tool-call limit")));
    Ok(())
}

#[tokio::test]
async fn debug_prompt_dump_requires_toolkit_before_composio_network() -> Result<()> {
    let tmp = TempDir::new()?;
    let err = dump_agent_prompt(DumpPromptOptions {
        agent_id: "integrations_agent".to_string(),
        toolkit: None,
        workspace_dir_override: Some(tmp.path().to_path_buf()),
        model_override: Some("round21-debug-model".to_string()),
    })
    .await
    .expect_err("integrations_agent without toolkit should fail locally");

    assert!(err.to_string().contains("requires a `toolkit` argument"));
    let opts = DumpPromptOptions::new("orchestrator");
    assert_eq!(opts.agent_id, "orchestrator");
    assert!(opts.toolkit.is_none());
    Ok(())
}

#[test]
fn debug_dump_writer_sanitizes_names_and_writes_summary_sidecars() -> Result<()> {
    let tmp = TempDir::new()?;
    let dumps = vec![DumpedPrompt {
        agent_id: "agent/with spaces".to_string(),
        toolkit: Some("gmail:primary".to_string()),
        mode: "session",
        model: "round21-model".to_string(),
        workspace_dir: PathBuf::from("/tmp/round21-workspace"),
        text: "SYSTEM PROMPT\n".to_string(),
        tool_names: vec!["echo".to_string(), "search".to_string()],
        skill_tool_count: 1,
    }];

    let summary = write_prompt_dumps(tmp.path(), &dumps)?;
    assert_eq!(
        summary.prompt_paths[0],
        tmp.path().join("1_agent_with_spaces_gmail_primary.md")
    );
    assert_eq!(
        std::fs::read_to_string(&summary.prompt_paths[0])?,
        "SYSTEM PROMPT\n"
    );
    let meta = std::fs::read_to_string(
        tmp.path()
            .join("1_agent_with_spaces_gmail_primary.meta.txt"),
    )?;
    assert!(meta.contains("agent:          agent/with spaces"));
    assert!(meta.contains("toolkit:        gmail:primary"));
    let summary_text = std::fs::read_to_string(summary.summary_path)?;
    assert!(summary_text.contains("agent/with spaces@gmail:primary"));
    assert!(summary_text.contains("tools=2"));
    Ok(())
}
