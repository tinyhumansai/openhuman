use super::*;
use crate::openhuman::agent::harness::definition::{ModelSpec, ToolScope};

#[test]
fn lazy_resolver_tolerates_near_miss_slugs() {
    use crate::openhuman::context::prompt::ConnectedIntegrationTool;
    let mk = |name: &str| ConnectedIntegrationTool {
        name: name.into(),
        description: "d".into(),
        parameters: None,
    };
    let resolver = LazyToolkitResolver {
        config: std::sync::Arc::new(crate::openhuman::config::Config::default()),
        actions: vec![mk("GOOGLESLIDES_BATCH_UPDATE"), mk("GMAIL_LIST_MESSAGES")],
    };
    // Exact, case-insensitive, and separator/prefix drift all resolve
    // (bug-report-2026-05-26 A2).
    assert!(resolver.resolve("GMAIL_LIST_MESSAGES").is_some());
    assert!(resolver.resolve("gmail_list_messages").is_some());
    assert!(resolver.resolve("googleslides_batch_update").is_some());
    // A fabricated slug stays unresolved → routed to the "available tools"
    // error so the model self-corrects, not silently mis-dispatched.
    assert!(resolver.resolve("GMAIL_GET_LAST_3_MESSAGES").is_none());
}

#[test]
fn normalize_slug_collapses_separators_and_case() {
    assert_eq!(
        normalize_slug("GOOGLESLIDES_BATCH_UPDATE"),
        "googleslidesbatchupdate"
    );
    assert_eq!(
        normalize_slug("googleslides_batch_update"),
        "googleslidesbatchupdate"
    );
    assert_ne!(
        normalize_slug("GMAIL_GET_LAST_3_MESSAGES"),
        normalize_slug("GMAIL_LIST_MESSAGES")
    );
}

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
        iteration_policy: Default::default(),
        max_result_chars: None,
        timeout_secs: None,
        sandbox_mode: crate::openhuman::agent::harness::definition::SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        subagents: vec![],
        delegate_name: None,
        agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
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
}

#[test]
fn append_subagent_role_contract_adds_role_and_brevity_rules() {
    let rendered = append_subagent_role_contract("base prompt".to_string(), "researcher");
    assert!(rendered.contains("## Sub-agent Role Contract"));
    assert!(rendered.contains("You are a sub-agent working for a parent OpenHuman agent"));
    assert!(rendered.contains("Keep your final response concise and synthesis-ready"));
    assert!(rendered.contains("## Sub-agent Result Contract"));
    assert!(rendered.contains("Evidence used"));
    assert!(rendered.contains("Do not include facts in Answer that are not supported"));
    assert!(rendered.contains("truncated, partial, or too large"));
}

#[test]
fn append_subagent_role_contract_is_idempotent() {
    let once = append_subagent_role_contract("base prompt".to_string(), "researcher");
    let twice = append_subagent_role_contract(once.clone(), "researcher");
    assert_eq!(once, twice, "contract suffix should only appear once");
}

// ── End-to-end runner tests with mock provider ────────────────────────

use crate::openhuman::agent::harness::fork_context::with_parent_context;
use crate::openhuman::inference::provider::{
    ChatRequest as PChatRequest, ChatResponse, Provider, ProviderDelta, ToolCall,
};
use parking_lot::Mutex;
use std::sync::Arc;

/// Mock provider whose response queue can be inspected by the test
/// to verify the bytes that arrive at the model.
#[derive(Clone)]
struct CapturedRequest {
    messages: Vec<crate::openhuman::inference::provider::ChatMessage>,
    tool_count: usize,
    model: String,
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
        model: &str,
        _temperature: f64,
    ) -> anyhow::Result<ChatResponse> {
        self.captured.lock().push(CapturedRequest {
            messages: request.messages.to_vec(),
            tool_count: request.tools.map_or(0, |tools| tools.len()),
            model: model.to_string(),
        });
        let response = {
            let mut q = self.responses.lock();
            if q.is_empty() {
                ChatResponse {
                    text: Some(String::new()),
                    tool_calls: vec![],
                    usage: None,
                    reasoning_content: None,
                }
            } else {
                q.remove(0)
            }
        };
        // Mirror a real streaming provider: when the caller attached a
        // `stream` sink, forward this response's reasoning then visible
        // text as `ProviderDelta`s before returning the aggregate. Lets
        // the subagent runner's per-iteration sink exercise the
        // `SubagentThinkingDelta` / `SubagentTextDelta` forwarding path.
        if let Some(sink) = request.stream {
            if let Some(reasoning) = response.reasoning_content.as_deref() {
                if !reasoning.is_empty() {
                    let _ = sink
                        .send(ProviderDelta::ThinkingDelta {
                            delta: reasoning.to_string(),
                        })
                        .await;
                }
            }
            if let Some(text) = response.text.as_deref() {
                if !text.is_empty() {
                    let _ = sink
                        .send(ProviderDelta::TextDelta {
                            delta: text.to_string(),
                        })
                        .await;
                }
            }
        }
        Ok(response)
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
        reasoning_content: None,
    }
}

fn text_response_with_reasoning(text: &str, reasoning: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: vec![],
        usage: None,
        reasoning_content: Some(reasoning.into()),
    }
}

fn tool_response(name: &str, args: &str) -> ChatResponse {
    ChatResponse {
        text: Some(String::new()),
        tool_calls: vec![ToolCall {
            id: "call-1".into(),
            name: name.into(),
            arguments: args.into(),
            extra_content: None,
        }],
        usage: None,
        reasoning_content: None,
    }
}

/// Build a minimal `ParentExecutionContext` suitable for runner tests.
/// Uses a no-op memory backend so we don't have to spin up a real one.
fn make_parent(provider: Arc<dyn Provider>, tools: Vec<Box<dyn Tool>>) -> ParentExecutionContext {
    let tool_specs: Vec<crate::openhuman::tools::ToolSpec> =
        tools.iter().map(|t| t.spec()).collect();
    ParentExecutionContext {
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: ["test".to_string(), "child".to_string(), "inner".to_string()]
            .into_iter()
            .collect(),
        provider,
        all_tools: Arc::new(tools),
        all_tool_specs: Arc::new(tool_specs),
        model_name: "test-model".into(),
        temperature: 0.5,
        workspace_dir: std::env::temp_dir(),
        memory: noop_memory(),
        agent_config: crate::openhuman::config::AgentConfig::default(),
        workflows: Arc::new(vec![]),
        memory_context: Arc::new(None),
        session_id: "test-session".into(),
        channel: "test".into(),
        connected_integrations: vec![],
        tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::PFormat,
        session_key: "0_test".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
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
async fn typed_mode_system_prompt_includes_subagent_role_contract() {
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
    let system_msg = captured[0]
        .messages
        .iter()
        .find(|m| m.role == "system")
        .expect("system message should be present");
    assert!(system_msg.content.contains("## Sub-agent Role Contract"));
    assert!(system_msg
        .content
        .contains("You are a sub-agent working for a parent OpenHuman agent"));
    assert!(system_msg
        .content
        .contains("Keep your final response concise and synthesis-ready"));
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
                model_override: None,
                task_id: Some("t1".into()),
                worker_thread_id: None,
                initial_history: None,
                checkpoint_dir: None,
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
    parent.memory_context = Arc::new(Some(
        "[Memory context]\n- prior fact: branch X failed\n".into(),
    ));
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
                model_override: None,
                task_id: None,
                worker_thread_id: None,
                initial_history: None,
                checkpoint_dir: None,
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
async fn runner_errors_outside_parent_context() {
    let def = make_def_named_tools(&[]);
    let result = run_subagent(&def, "x", SubagentRunOptions::default()).await;
    assert!(matches!(result, Err(SubagentRunError::NoParentContext)));
}

#[tokio::test]
async fn subagent_emits_checkpoint_at_iteration_cap_instead_of_erroring() {
    // A sub-agent that keeps calling tools and never finishes must hit its
    // cap and return a graceful partial-progress checkpoint (Ok), not a bare
    // MaxIterationsExceeded that discards its work — so the delegating agent
    // can continue from what it got (bug-report-2026-05-26 A1, mirrors the
    // main agent). Two tool rounds (max_iterations=2), then the summarize
    // call returns prose which becomes the checkpoint.
    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{}"),
        tool_response("file_read", "{}"),
        text_response("Progress so far: read the file. Remaining: keep going."),
    ]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let mut def = make_def_named_tools(&["file_read"]);
    def.max_iterations = 2;

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "keep reading forever", SubagentRunOptions::default()).await
    })
    .await
    .expect("hitting the iteration cap should return a checkpoint, not error");

    assert!(
        outcome.output.contains("Progress so far"),
        "expected the model-written checkpoint, got: {}",
        outcome.output
    );
}

#[tokio::test]
async fn subagent_checkpoint_falls_back_to_deterministic_when_summary_empty() {
    // Same cap, but the summarize call yields nothing (response queue
    // exhausted → empty). The runner must fall back to a deterministic
    // partial-progress digest so the parent still gets a usable result
    // (bug-report-2026-05-26 A1).
    let provider = ScriptedProvider::new(vec![
        tool_response("file_read", "{}"),
        tool_response("file_read", "{}"),
    ]);
    let parent = make_parent(provider.clone(), vec![stub("file_read")]);
    let mut def = make_def_named_tools(&["file_read"]);
    def.max_iterations = 2;

    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "keep reading forever", SubagentRunOptions::default()).await
    })
    .await
    .expect("empty summary should fall back, not error");

    assert!(
        outcome.output.contains("tool-call limit"),
        "expected the deterministic fallback checkpoint, got: {}",
        outcome.output
    );
    assert!(
        outcome.output.contains("file_read"),
        "deterministic checkpoint should list the tool work done, got: {}",
        outcome.output
    );
}

#[tokio::test]
async fn runner_allows_spawn_at_max_depth() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![]);
    let def = make_def_named_tools(&[]);

    let outcome = with_parent_context(parent, async {
        with_spawn_depth(MAX_SPAWN_DEPTH - 1, async {
            run_subagent(&def, "x", SubagentRunOptions::default()).await
        })
        .await
    })
    .await
    .expect("runner should allow the configured maximum depth");

    assert_eq!(outcome.output, "ok");
    assert_eq!(provider.captured.lock().len(), 1);
    assert_eq!(
        current_spawn_depth(),
        0,
        "depth task-local must not leak after the run"
    );
}

#[tokio::test]
async fn runner_rejects_spawn_beyond_max_depth() {
    let provider = ScriptedProvider::new(vec![text_response("should not be called")]);
    let parent = make_parent(provider.clone(), vec![]);
    let def = make_def_named_tools(&[]);

    let result = with_parent_context(parent, async {
        with_spawn_depth(MAX_SPAWN_DEPTH, async {
            run_subagent(&def, "x", SubagentRunOptions::default()).await
        })
        .await
    })
    .await;

    assert!(matches!(
        result,
        Err(SubagentRunError::SpawnDepthExceeded {
            attempted_depth,
            max_depth
        }) if attempted_depth == MAX_SPAWN_DEPTH + 1 && max_depth == MAX_SPAWN_DEPTH
    ));
    assert!(
        provider.captured.lock().is_empty(),
        "depth rejection must happen before provider dispatch"
    );
    assert_eq!(
        current_spawn_depth(),
        0,
        "depth task-local must not leak after rejection"
    );
}

#[tokio::test]
async fn typed_mode_model_override_pins_exact_model_for_spawn() {
    let provider = ScriptedProvider::new(vec![text_response("ok")]);
    let parent = make_parent(provider.clone(), vec![]);
    let mut def = make_def_named_tools(&[]);
    def.model = ModelSpec::Inherit;

    let _ = with_parent_context(parent, async {
        run_subagent(
            &def,
            "use the pinned model",
            SubagentRunOptions {
                model_override: Some("deepseek/deepseek-r2".into()),
                ..Default::default()
            },
        )
        .await
    })
    .await
    .expect("runner should succeed");

    let captured = provider.captured.lock();
    assert_eq!(captured[0].model, "deepseek/deepseek-r2");
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

/// A sub-agent's streamed visible text and reasoning are forwarded to the
/// parent's progress sink as `SubagentTextDelta` / `SubagentThinkingDelta`
/// events tagged with the child's `agent_id` / `task_id`, in order, and
/// the concatenated text deltas reconstruct the final assistant text. The
/// web-channel bridge turns these into `subagent_text_delta` /
/// `subagent_thinking_delta` socket events the parent thread renders live.
#[tokio::test]
async fn typed_mode_forwards_child_text_and_thinking_deltas() {
    use crate::openhuman::agent::progress::AgentProgress;

    let provider = ScriptedProvider::new(vec![text_response_with_reasoning(
        "the final answer",
        "let me reason about this",
    )]);
    let mut parent = make_parent(provider, vec![]);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
    parent.on_progress = Some(tx);

    let def = make_def_named_tools(&[]);
    let outcome = with_parent_context(parent, async {
        run_subagent(&def, "answer me", SubagentRunOptions::default()).await
    })
    .await
    .expect("runner should succeed");
    assert_eq!(outcome.output, "the final answer");

    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }

    let thinking: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentProgress::SubagentThinkingDelta {
                agent_id,
                task_id,
                delta,
                iteration,
            } => Some((agent_id.clone(), task_id.clone(), delta.clone(), *iteration)),
            _ => None,
        })
        .collect();
    assert_eq!(thinking.len(), 1, "one thinking delta forwarded");
    assert_eq!(thinking[0].2, "let me reason about this");
    assert_eq!(thinking[0].3, 1, "tagged with the child iteration");
    assert!(!thinking[0].1.is_empty(), "carries the child task id");

    let text: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentProgress::SubagentTextDelta {
                agent_id,
                task_id,
                delta,
                iteration,
            } => Some((agent_id.clone(), task_id.clone(), delta.clone(), *iteration)),
            _ => None,
        })
        .collect();
    assert_eq!(text.len(), 1, "one text delta forwarded");
    assert_eq!(text[0].2, "the final answer");
    assert_eq!(text[0].3, 1);
    // Same child identity on both delta kinds so the UI attributes them to
    // one subagent row.
    assert_eq!(text[0].0, thinking[0].0, "same agent_id");
    assert_eq!(text[0].1, thinking[0].1, "same task_id");

    // Ordering: the thinking delta precedes the text delta within the
    // iteration, matching the provider's emission order.
    let thinking_pos = events
        .iter()
        .position(|e| matches!(e, AgentProgress::SubagentThinkingDelta { .. }))
        .unwrap();
    let text_pos = events
        .iter()
        .position(|e| matches!(e, AgentProgress::SubagentTextDelta { .. }))
        .unwrap();
    assert!(
        thinking_pos < text_pos,
        "thinking streams before visible text"
    );
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

// Truncation tests live in ops_truncation_tests.rs to keep this file
// under the ~500-line guideline.

// ── resolve_subagent_provider ─────────────────────────────────────────

/// `Arc<dyn Provider>` identity helper — every test below uses a fresh
/// `ScriptedProvider` and we want to assert "is this the *same* Arc as
/// the parent's" without leaning on `PartialEq` on dyn trait objects.
fn arc_ptr_eq<P: ?Sized>(a: &std::sync::Arc<P>, b: &std::sync::Arc<P>) -> bool {
    std::sync::Arc::ptr_eq(a, b)
}

#[test]
fn resolve_subagent_provider_inherit_uses_parent_provider_and_model() {
    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (resolved_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Inherit,
        "test_agent",
        None,
        parent.clone(),
        "parent-model-x".to_string(),
        false,
        None,
    );
    assert!(
        arc_ptr_eq(&parent, &resolved_provider),
        "Inherit must return the parent's Arc unchanged"
    );
    assert_eq!(resolved_model, "parent-model-x");
}

#[test]
fn resolve_subagent_provider_exact_overrides_only_model() {
    // Exact keeps the parent's provider but replaces the model name.
    // This is the explicit "I want a cheaper tier on the same backend"
    // escape hatch.
    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (resolved_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Exact("haiku-mini".to_string()),
        "test_agent",
        None,
        parent.clone(),
        "parent-model-x".to_string(),
        false,
        None,
    );
    assert!(
        arc_ptr_eq(&parent, &resolved_provider),
        "Exact must keep the parent's provider — only the model name changes"
    );
    assert_eq!(resolved_model, "haiku-mini");
}

#[test]
fn resolve_subagent_provider_spawn_override_wins_over_definition_model() {
    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (resolved_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Exact("definition-model".to_string()),
        "test_agent",
        None,
        parent.clone(),
        "parent-model-x".to_string(),
        false,
        Some("spawn-model-y"),
    );
    assert!(
        arc_ptr_eq(&parent, &resolved_provider),
        "inline spawn override should not change the provider"
    );
    assert_eq!(resolved_model, "spawn-model-y");
}

#[test]
fn resolve_subagent_provider_config_model_wins_over_definition_model() {
    use crate::openhuman::config::{Config, TeamModelConfig};

    let mut config = Config::default();
    config.teams.insert(
        "test_agent".to_string(),
        TeamModelConfig {
            lead_model: None,
            agent_model: Some("configured-agent-model".to_string()),
        },
    );

    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (resolved_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Exact("definition-model".to_string()),
        "test_agent",
        Some(&config),
        parent.clone(),
        "parent-model-x".to_string(),
        false,
        None,
    );
    assert!(
        arc_ptr_eq(&parent, &resolved_provider),
        "config model pin should not change the provider"
    );
    assert_eq!(resolved_model, "configured-agent-model");
}

#[test]
fn resolve_subagent_provider_inline_override_wins_over_config_model() {
    use crate::openhuman::config::{Config, TeamModelConfig};

    let mut config = Config::default();
    config.teams.insert(
        "test_agent".to_string(),
        TeamModelConfig {
            lead_model: None,
            agent_model: Some("configured-agent-model".to_string()),
        },
    );

    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (_resolved_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Exact("definition-model".to_string()),
        "test_agent",
        Some(&config),
        parent.clone(),
        "parent-model-x".to_string(),
        false,
        Some("inline-model"),
    );
    assert_eq!(resolved_model, "inline-model");
}

#[test]
fn resolve_subagent_provider_config_alias_matches_issue_team_examples() {
    use crate::openhuman::config::{Config, TeamModelConfig};

    let mut config = Config::default();
    config.teams.insert(
        "research".to_string(),
        TeamModelConfig {
            lead_model: Some("research-lead-model".to_string()),
            agent_model: Some("research-agent-model".to_string()),
        },
    );

    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Hint("agentic".to_string()),
        "researcher",
        Some(&config),
        parent,
        "parent-model-x".to_string(),
        false,
        None,
    );
    assert_eq!(resolved_model, "research-agent-model");
}

#[test]
fn resolve_subagent_provider_hint_with_no_config_falls_back() {
    // The async config load failed (transient I/O, missing file, etc.).
    // The Hint arm must NOT silently swallow the failure and synthesise
    // `{workload}-v1` — that's the OpenHuman-only naming that breaks
    // Anthropic/OpenAI. Fall back to the parent's known-good
    // (provider, model) instead.
    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (resolved_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Hint("agentic".to_string()),
        "test_agent",
        None, // no config loaded
        parent.clone(),
        "real-claude-id".to_string(),
        false,
        None,
    );
    assert!(
        arc_ptr_eq(&parent, &resolved_provider),
        "config-load failure must fall back to parent provider, not synthesize a new one"
    );
    assert_eq!(
        resolved_model, "real-claude-id",
        "model must be parent's current model — NOT '{{workload}}-v1'"
    );
}

#[test]
fn resolve_subagent_provider_hint_with_config_routes_via_factory() {
    // The Hint arm with a real config takes the workload-factory path.
    // We don't assert the *resulting* provider identity here (the
    // factory may return a fresh OpenHuman backend or whatever
    // primary_cloud resolves to), but we DO assert the resolved model
    // matches the workload's configured exact id — not the legacy
    // `{workload}-v1` synthesis.
    use crate::openhuman::config::Config;
    let mut config = Config::default();
    // Route `agentic` to OpenHuman backend explicitly. The backend returns
    // the configured default_model. Use `coding-v1` — a recognized tier
    // that the factory validation accepts and that differs from the old
    // `agentic-v1` synthesis, making the assertion meaningful.
    config.agentic_provider = Some("openhuman".to_string());
    config.default_model = Some("coding-v1".to_string());

    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (_resolved_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Hint("agentic".to_string()),
        "test_agent",
        Some(&config),
        parent.clone(),
        "parent-model-ignored-on-hint".to_string(),
        false,
        None,
    );
    assert_eq!(
        resolved_model, "coding-v1",
        "Hint must use the factory-resolved exact model, not synthesise `agentic-v1` \
         and not fall back to parent's model"
    );
}

#[test]
fn resolve_subagent_provider_hint_falls_back_on_factory_error() {
    // An invalid provider string in the workload config (e.g. a typo
    // like "groq:something") makes the factory return Err. The Hint
    // arm must fall back to the parent provider rather than
    // propagating — sub-agent execution should degrade to "use what
    // the parent uses" not crash entirely.
    use crate::openhuman::config::Config;
    let mut config = Config::default();
    config.agentic_provider = Some("groq:not-a-real-prefix".to_string());

    let parent: Arc<dyn Provider> = ScriptedProvider::new(vec![]);
    let (resolved_provider, resolved_model) = super::resolve_subagent_provider(
        &ModelSpec::Hint("agentic".to_string()),
        "test_agent",
        Some(&config),
        parent.clone(),
        "fallback-model".to_string(),
        false,
        None,
    );
    assert!(
        arc_ptr_eq(&parent, &resolved_provider),
        "factory error must fall back to parent provider"
    );
    assert_eq!(resolved_model, "fallback-model");
}

// ── Probe regression tests (#1710 Wave 2) ──────────────────────────
//
// `user_is_signed_in_to_composio` replaces the legacy
// `parent.composio_client.is_none()` gate. The legacy probe was
// backend-only by construction: a direct-mode user with a stored API
// key but no backend session token was falsely reported as "not signed
// in" and the spawn-time integration refresh path was silently
// skipped. These tests pin the new behaviour so that regression
// can't sneak back in.

#[test]
fn direct_mode_user_with_stored_key_passes_signed_in_check() {
    use super::user_is_signed_in_to_composio;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Direct mode + inline API key (the `config.composio.api_key`
    // fallback path inside `create_composio_client` — equivalent to a
    // stored direct key as far as the probe is concerned).
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".into());
    assert!(
        user_is_signed_in_to_composio(&config),
        "direct-mode user with stored api key must be reported as signed in"
    );
}

#[test]
fn unsigned_in_user_fails_probe() {
    use super::user_is_signed_in_to_composio;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Default mode = backend, no session token → factory errors with
    // "no backend session". Direct fallback is unreachable because
    // mode is not "direct".
    assert!(
        !user_is_signed_in_to_composio(&config),
        "user with neither backend session nor direct key must NOT be reported as signed in"
    );
}

/// Sanity-check: a parent agent delegating to a sub-agent must complete
/// without panicking, even on a worker thread with a tight stack — this
/// is the same recursion shape that crashed the
/// `chat-harness-subagent` Playwright lane in production with
/// `thread 'tokio-rt-worker' has overflowed its stack, fatal runtime
/// error: stack overflow`.
///
/// The deep ground-truth regression catcher for this is the
/// `chat-harness-subagent.spec.ts` Playwright spec, which exercises the
/// real orchestrator → researcher dispatch end-to-end (real provider
/// stream, real config load, real tool registry). The scripted unit
/// path here has much smaller per-frame state than production, so a
/// single stack size doesn't cleanly bracket boxed-vs-unboxed — we use
/// the loose 1 MiB worker stack as a smoke check that the dispatch
/// path remains poll-bounded after refactors. See `subagent_runner/
/// ops.rs` `Box::pin` callsites for the structural fix.
#[test]
fn nested_subagent_dispatch_runs_on_a_constrained_worker_stack() {
    use async_trait::async_trait;
    use std::sync::Arc;

    struct RecursiveDelegateTool {
        inner_def: AgentDefinition,
    }

    #[async_trait]
    impl Tool for RecursiveDelegateTool {
        fn name(&self) -> &str {
            "delegate_inner"
        }
        fn description(&self) -> &str {
            "Dispatches a nested sub-agent — reproduces the recursive engine poll."
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object","properties":{}})
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::Execute
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let outcome = run_subagent(&self.inner_def, "inner go", SubagentRunOptions::default())
                .await
                .map_err(|e| anyhow::anyhow!("nested run_subagent failed: {e}"))?;
            Ok(ToolResult::success(outcome.output))
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(1024 * 1024)
        .enable_all()
        .build()
        .expect("build constrained-stack tokio runtime");

    let outcome = runtime.block_on(async {
        // Three scripted responses, shared by outer + inner runs
        // (providers are Arc-cloned, so both pull from the same queue):
        //   [0] outer round 1: call `delegate_inner`
        //   [1] inner round 1: return final text
        //   [2] outer round 2: return final text using the tool result
        let provider = ScriptedProvider::new(vec![
            tool_response("delegate_inner", "{}"),
            text_response("inner-final"),
            text_response("outer-final: inner-final"),
        ]);

        let inner_def = make_def_named_tools(&[]);
        let delegate_tool: Box<dyn Tool> = Box::new(RecursiveDelegateTool { inner_def });
        let parent = make_parent(
            Arc::clone(&(provider.clone() as Arc<dyn Provider>)),
            vec![delegate_tool],
        );
        let outer_def = make_def_named_tools(&["delegate_inner"]);

        with_parent_context(parent, async {
            run_subagent(&outer_def, "outer go", SubagentRunOptions::default()).await
        })
        .await
    });

    let outcome = outcome.expect(
        "nested run_subagent must complete on a 1 MiB worker stack — \
         a stack overflow here means the recursion boundary in \
         `run_typed_mode` regressed (see `Box::pin` callsites around \
         `run_inner_loop` and `run_turn_engine`).",
    );
    assert!(
        outcome.output.contains("inner-final"),
        "outer should fold the inner sub-agent's result into its final \
         answer, got: {}",
        outcome.output
    );
}
