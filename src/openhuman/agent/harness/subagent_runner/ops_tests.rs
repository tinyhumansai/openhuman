use super::*;
use crate::openhuman::agent::harness::definition::{ModelSpec, ToolScope};

fn make_def_named_tools(names: &[&str]) -> AgentDefinition {
    AgentDefinition {
        id: "test".into(),
        when_to_use: "t".into(),
        display_name: None,
        system_prompt: PromptSource::Inline("system".into()),
        omit_identity: true,
        omit_memory_context: true,
        omit_safety_preamble: true,
        omit_skills_catalog: true,
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.4,
        tools: ToolScope::Named(names.iter().map(|s| s.to_string()).collect()),
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 5,
        timeout_secs: None,
        sandbox_mode: crate::openhuman::agent::harness::definition::SandboxMode::None,
        background: false,
        uses_fork_context: false,
        subagents: vec![],
        delegate_name: None,
        source: crate::openhuman::agent::harness::definition::DefinitionSource::Builtin,
    }
}

/// Local tool used to populate `parent_tools` in tests.
struct StubTool {
    name: &'static str,
}

use crate::openhuman::tools::{PermissionLevel, ToolResult};
use async_trait::async_trait;

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "stub"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }
}

fn stub(name: &'static str) -> Box<dyn Tool> {
    Box::new(StubTool { name })
}

#[test]
fn filter_named_scope_keeps_only_named() {
    let parent: Vec<Box<dyn Tool>> = vec![stub("alpha"), stub("beta"), stub("gamma")];
    let def = make_def_named_tools(&["alpha", "gamma"]);
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, None);
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["alpha", "gamma"]);
}

#[test]
fn filter_wildcard_includes_all_minus_disallowed() {
    let parent: Vec<Box<dyn Tool>> = vec![stub("alpha"), stub("beta"), stub("gamma")];
    let mut def = make_def_named_tools(&[]);
    def.tools = ToolScope::Wildcard;
    def.disallowed_tools = vec!["beta".into()];
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, None);
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["alpha", "gamma"]);
}

#[test]
fn filter_skill_filter_restricts_to_prefix() {
    let parent: Vec<Box<dyn Tool>> = vec![
        stub("notion__search"),
        stub("notion__read"),
        stub("gmail__send"),
        stub("file_read"),
    ];
    let mut def = make_def_named_tools(&[]);
    def.tools = ToolScope::Wildcard;
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, Some("notion"));
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["notion__search", "notion__read"]);
}

#[test]
fn filter_skill_filter_combined_with_named_scope() {
    // Named scope intersects with skill_filter — only tools that
    // appear in the named list AND match the prefix survive.
    let parent: Vec<Box<dyn Tool>> = vec![
        stub("notion__search"),
        stub("notion__read"),
        stub("gmail__send"),
    ];
    let def = make_def_named_tools(&["notion__search", "gmail__send"]);
    let idx = filter_tool_indices(&parent, &def.tools, &def.disallowed_tools, Some("notion"));
    let names: Vec<&str> = idx.iter().map(|&i| parent[i].name()).collect();
    assert_eq!(names, vec!["notion__search"]);
}

#[test]
fn subagent_mode_as_str_roundtrip() {
    assert_eq!(SubagentMode::Typed.as_str(), "typed");
    assert_eq!(SubagentMode::Fork.as_str(), "fork");
}

// ── End-to-end runner tests with mock provider ────────────────────────

use crate::openhuman::agent::harness::fork_context::{with_fork_context, with_parent_context};
use crate::openhuman::providers::{ChatRequest as PChatRequest, ChatResponse, Provider, ToolCall};
use parking_lot::Mutex;
use std::sync::Arc;

/// Mock provider whose response queue can be inspected by the test
/// to verify the bytes that arrive at the model.
#[derive(Clone)]
struct CapturedRequest {
    messages: Vec<crate::openhuman::providers::ChatMessage>,
    tool_count: usize,
}

struct ScriptedProvider {
    responses: Mutex<Vec<ChatResponse>>,
    captured: Mutex<Vec<CapturedRequest>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            captured: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<String> {
        Ok("noop".into())
    }

    async fn chat(
        &self,
        request: PChatRequest<'_>,
        _model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.captured.lock().push(CapturedRequest {
            messages: request.messages.to_vec(),
            tool_count: request.tools.map_or(0, |tools| tools.len()),
        });
        let mut q = self.responses.lock();
        if q.is_empty() {
            return Ok(ChatResponse {
                text: Some(String::new()),
                tool_calls: vec![],
                usage: None,
            });
        }
        Ok(q.remove(0))
    }

    fn supports_native_tools(&self) -> bool {
        true
    }
}

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: vec![],
        usage: None,
    }
}

fn tool_response(name: &str, args: &str) -> ChatResponse {
    ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![ToolCall {
            id: "call-1".into(),
            name: name.into(),
            arguments: args.into(),
        }],
        usage: None,
    }
}

/// Build a minimal `ParentExecutionContext` suitable for runner tests.
/// Uses a no-op memory backend so we don't have to spin up a real one.
fn make_parent(provider: Arc<dyn Provider>, tools: Vec<Box<dyn Tool>>) -> ParentExecutionContext {
    let tool_specs: Vec<crate::openhuman::tools::ToolSpec> =
        tools.iter().map(|t| t.spec()).collect();
    ParentExecutionContext {
        provider,
        all_tools: Arc::new(tools),
        all_tool_specs: Arc::new(tool_specs),
        model_name: "test-model".into(),
        temperature: 0.5,
        workspace_dir: std::env::temp_dir(),
        memory: noop_memory(),
        agent_config: crate::openhuman::config::AgentConfig::default(),
        skills: Arc::new(vec![]),
        memory_context: None,
        session_id: "test-session".into(),
        channel: "test".into(),
        connected_integrations: vec![],
        composio_client: None,
        tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::PFormat,
        session_key: "0_test".into(),
        session_parent_prefix: None,
        on_progress: None,
    }
}

fn noop_memory() -> Arc<dyn crate::openhuman::memory::Memory> {
    struct NoopMemory;
    #[async_trait]
    impl crate::openhuman::memory::Memory for NoopMemory {
        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: crate::openhuman::memory::MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
            Ok(vec![])
        }
        async fn get(
            &self,
            _namespace: &str,
            _key: &str,
        ) -> anyhow::Result<Option<crate::openhuman::memory::MemoryEntry>> {
            Ok(None)
        }
        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&crate::openhuman::memory::MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::MemoryEntry>> {
            Ok(vec![])
        }
        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(true)
        }
        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
            Ok(vec![])
        }
        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }
        async fn health_check(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "noop"
        }
    }
    Arc::new(NoopMemory)
}

#[tokio::test]
async fn typed_mode_injects_current_date_and_time_into_user_message() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let def = make_def_named_tools(&[]);

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "the actual task prompt",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await
    .unwrap();

    let captured = provider.captured.lock();
    let user_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message should be present");
    assert!(
        user_msg.content.contains("Current Date & Time:"),
        "subagent user message must include current date/time context, got: {}",
        user_msg.content
    );
}

#[tokio::test]
async fn typed_mode_returns_text_through_runner() {
    let provider = ScriptedProvider::new(vec![text_response("X is Y")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let def = make_def_named_tools(&[]);

    let outcome = with_parent_context(parent, async {
        run_subagent(
            &def,
            "summarise X",
            SubagentRunOptions {
                skill_filter_override: None,
                toolkit_override: None,
                context: None,
                task_id: Some("t1".into()),
                worker_thread_id: None,
            },
        )
        .await
    })
    .await
    .expect("runner should succeed");

    assert_eq!(outcome.output, "X is Y");
    assert_eq!(outcome.iterations, 1);
    assert_eq!(outcome.mode, SubagentMode::Typed);
    assert_eq!(outcome.task_id, "t1");
}

#[tokio::test]
async fn typed_mode_no_memory_context_in_user_message() {
    // Verifies that sub-agents skip memory loading entirely: the
    // user message sent to the provider does NOT contain
    // `[Memory context]`.
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let def = make_def_named_tools(&[]);

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "the actual task prompt",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await
    .unwrap();

    let captured = provider.captured.lock();
    assert_eq!(captured.len(), 1);
    let user_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message should be present");
    assert!(
        !user_msg.content.contains("[Memory context]"),
        "subagent user message must not include memory recall section, got: {}",
        user_msg.content
    );
    assert!(user_msg.content.contains("the actual task prompt"));
}

#[tokio::test]
async fn typed_mode_includes_memory_context_when_definition_allows_it() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let mut parent = make_parent(provider.clone(), vec![stub("file_read")]);
    parent.memory_context = Some("[Memory context]\n- prior fact: branch X failed\n".into());
    let mut def = make_def_named_tools(&[]);
    def.omit_memory_context = false;

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "the actual task prompt",
            SubagentRunOptions::default(),
        )
        .await
    })
    .await
    .unwrap();

    let captured = provider.captured.lock();
    let user_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "user")
        .expect("user message should be present");
    assert!(user_msg.content.contains("[Memory context]"));
    assert!(user_msg.content.contains("branch X failed"));
}

#[tokio::test]
async fn typed_mode_filters_tools_by_skill_filter() {
    // Parent has tools spanning notion__*, gmail__*, and a generic
    // file_read; spawn the runner with skill_filter override "notion"
    // and assert that only the notion tools end up in the request.
    let provider = ScriptedProvider::new(vec![text_response("done")]);
    let parent = make_parent(
        provider.clone(),
        vec![
            stub("notion__search"),
            stub("notion__read"),
            stub("gmail__send"),
            stub("file_read"),
        ],
    );
    // Wildcard scope so skill_filter is the only restrictor.
    let mut def = make_def_named_tools(&[]);
    def.tools = ToolScope::Wildcard;

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "lookup",
            SubagentRunOptions {
                skill_filter_override: Some("notion".into()),
                toolkit_override: None,
                context: None,
                task_id: None,
                worker_thread_id: None,
            },
        )
        .await
    })
    .await
    .unwrap();

    // The narrow system prompt should mention the notion tools by
    // name and NOT mention gmail/file_read.
    let captured = provider.captured.lock();
    let system_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "system")
        .expect("system message present");
    assert!(system_msg.content.contains("notion__search"));
    assert!(system_msg.content.contains("notion__read"));
    assert!(
        !system_msg.content.contains("gmail__send"),
        "skill_filter should have excluded gmail__send"
    );
    assert!(
        !system_msg.content.contains("file_read"),
        "skill_filter should have excluded file_read"
    );
}

#[tokio::test]
async fn typed_mode_executes_one_tool_then_returns() {
    // Two-round script: round 1 returns a tool call, round 2 returns
    // the final text. Verifies the inner tool-call loop wires up the
    // tool result into history correctly.
    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{\"path\":\"x\"}"),
        text_response("the file contents say hello"),
    ]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    // Allow the runner to call file_read.
    let def = make_def_named_tools(&["file_read"]);

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "read x", SubagentRunOptions::default()).await
    })
    .await
    .expect("runner should succeed");

    assert!(outcome.output.contains("hello"));
    assert_eq!(outcome.iterations, 2);
    // Second request should include the role=tool message produced
    // by the runner from StubTool's "ok" output.
    let captured = provider.captured.lock();
    assert_eq!(captured.len(), 2);
    let second_call_messages = &captured[1].messages;
    let has_tool_msg = second_call_messages.iter().any(|m| m.role == "tool");
    assert!(
        has_tool_msg,
        "second provider call should include role=tool message"
    );
}

#[tokio::test]
async fn typed_mode_blocks_unallowed_tool_calls() {
    // Provider tries to call a tool that's not in the allowlist.
    // Runner should surface an error tool result and the next
    // iteration should be able to recover.
    let provider = ScriptedProvider::new(vec![
        tool_response("forbidden_tool", "{}"),
        text_response("oops, I'll try something else"),
    ]);
    let parent = make_parent(
        provider.clone(),
        vec![stub("file_read"), stub("forbidden_tool")],
    );
    // Definition only allows file_read.
    let def = make_def_named_tools(&["file_read"]);

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "do thing", SubagentRunOptions::default()).await
    })
    .await
    .expect("runner should succeed");

    assert!(outcome.output.contains("oops"));
    let captured = provider.captured.lock();
    let second_call_messages = &captured[1].messages;
    let tool_msg = second_call_messages
        .iter()
        .find(|m| m.role == "tool")
        .expect("tool result message should be present");
    assert!(
        tool_msg.content.contains("not available"),
        "blocked tool should produce a 'not available' error message"
    );
}

#[tokio::test]
async fn fork_mode_replays_parent_prefix_bytes() {
    // Construct a fake fork context with a known message prefix.
    // The runner should replay it byte-for-byte plus a single
    // appended user message carrying the fork directive.
    let provider = ScriptedProvider::new(vec![text_response("fork done")]);
    let parent = make_parent(provider.clone(), vec![stub("file_read"), stub("shell")]);

    let prefix = vec![
        crate::openhuman::providers::ChatMessage::system("PARENT_SYSTEM_PROMPT_BYTES"),
        crate::openhuman::providers::ChatMessage::user("first user msg"),
        crate::openhuman::providers::ChatMessage::assistant("parent assistant"),
    ];

    let fork = ForkContext {
        system_prompt: Arc::new("PARENT_SYSTEM_PROMPT_BYTES".into()),
        tool_specs: Arc::new(vec![parent.all_tool_specs[0].clone()]),
        message_prefix: Arc::new(prefix.clone()),
        fork_task_prompt: "ANALYSE THIS BRANCH".into(),
    };

    let def = crate::openhuman::agent::harness::builtin_definitions::fork_definition();

    let outcome = with_parent_context(parent, async move {
        with_fork_context(fork, async {
            run_subagent(
                &def,
                "ignored — fork uses fork_task_prompt",
                SubagentRunOptions::default(),
            )
            .await
        })
        .await
    })
    .await
    .expect("fork runner should succeed");

    assert_eq!(outcome.mode, SubagentMode::Fork);
    assert_eq!(outcome.output, "fork done");

    // Verify the request that hit the provider replays the parent
    // prefix exactly and appends only the fork directive.
    let captured = provider.captured.lock();
    let first_call = &captured[0];
    assert_eq!(first_call.messages.len(), prefix.len() + 1);
    for (i, msg) in prefix.iter().enumerate() {
        assert_eq!(first_call.messages[i].role, msg.role);
        assert_eq!(first_call.messages[i].content, msg.content);
    }
    // The appended user message carries the fork directive.
    let appended = first_call.messages.last().unwrap();
    assert_eq!(appended.role, "user");
    assert_eq!(appended.content, "ANALYSE THIS BRANCH");
    assert_eq!(first_call.tool_count, 1);
}

#[tokio::test]
async fn fork_mode_errors_when_no_fork_context() {
    let provider = ScriptedProvider::new(vec![text_response("unused")]);
    let parent = make_parent(provider, vec![stub("file_read")]);
    let def = crate::openhuman::agent::harness::builtin_definitions::fork_definition();

    let result = with_parent_context(parent, async {
        run_subagent(&def, "x", SubagentRunOptions::default()).await
    })
    .await;

    assert!(matches!(result, Err(SubagentRunError::NoForkContext)));
}

#[tokio::test]
async fn runner_errors_outside_parent_context() {
    let def = make_def_named_tools(&[]);
    let result = run_subagent(&def, "x", SubagentRunOptions::default()).await;
    assert!(matches!(result, Err(SubagentRunError::NoParentContext)));
}

/// #1122 — when the parent attaches a progress sink, the inner loop
/// emits `SubagentIterationStarted` for each round and a paired
/// `SubagentToolCallStarted` / `SubagentToolCallCompleted` for each
/// child tool call. The web-channel bridge translates these into the
/// `subagent_iteration_start` / `subagent_tool_call` /
/// `subagent_tool_result` socket events the parent thread renders.
#[tokio::test]
async fn typed_mode_emits_child_progress_events_when_sink_attached() {
    use crate::openhuman::agent::progress::AgentProgress;

    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{\"path\":\"x\"}"),
        text_response("done"),
    ]);
    let mut parent = make_parent(provider, vec![stub("file_read")]);

    // Wire the parent's progress sink so the runner re-emits child
    // lifecycle events through the same channel a real session would
    // expose to the web bridge.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
    parent.on_progress = Some(tx);

    let def = make_def_named_tools(&["file_read"]);
    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "read x", SubagentRunOptions::default()).await
    })
    .await
    .expect("runner should succeed");
    assert_eq!(outcome.iterations, 2);

    // Drain everything the runner sent. The receiver's sender half is
    // dropped when `parent` falls out of scope above, so `recv` returns
    // None once the queue empties.
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }

    let iter_starts = events
        .iter()
        .filter(|e| matches!(e, AgentProgress::SubagentIterationStarted { .. }))
        .count();
    assert_eq!(iter_starts, 2, "one iteration_start per round");

    let tool_starts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentProgress::SubagentToolCallStarted {
                call_id,
                tool_name,
                iteration,
                ..
            } => Some((call_id.clone(), tool_name.clone(), *iteration)),
            _ => None,
        })
        .collect();
    assert_eq!(tool_starts.len(), 1);
    assert_eq!(tool_starts[0].1, "file_read");
    assert_eq!(tool_starts[0].2, 1);

    let tool_done: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentProgress::SubagentToolCallCompleted {
                call_id,
                success,
                iteration,
                ..
            } => Some((call_id.clone(), *success, *iteration)),
            _ => None,
        })
        .collect();
    assert_eq!(tool_done.len(), 1);
    assert_eq!(tool_done[0].0, tool_starts[0].0, "matching call_id pair");
    assert!(tool_done[0].1, "stub tool returns ok");
    assert_eq!(tool_done[0].2, 1);
}

/// Runs without an attached sink must remain backwards compatible — the
/// runner is a no-op for child progress and the outcome is unchanged.
#[tokio::test]
async fn typed_mode_progress_emission_is_a_noop_without_sink() {
    let provider = ScriptedProvider::new(vec![text_response("done")]);
    let parent = make_parent(provider, vec![]);
    assert!(parent.on_progress.is_none());
    let def = make_def_named_tools(&[]);
    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "x", SubagentRunOptions::default()).await
    })
    .await
    .expect("runner should succeed");
    assert_eq!(outcome.iterations, 1);
}
