use super::*;
use crate::openhuman::agent::context::prompt::ToolCallFormat;
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::openhuman::config::AgentConfig;
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

#[test]
fn parameters_schema_advertises_fire_and_forget_fields() {
    let tool = SpawnAsyncSubagentTool::new();
    let schema = tool.parameters_schema();
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("required list");
    assert!(required.iter().any(|v| v.as_str() == Some("agent_id")));
    assert!(required.iter().any(|v| v.as_str() == Some("prompt")));

    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("properties");
    for key in ["context", "model", "toolkit", "task_title"] {
        assert!(props.contains_key(key), "missing {key}");
    }
}

#[test]
fn background_contract_forbids_user_attention() {
    let wrapped = add_background_contract("archive this fact");
    assert!(wrapped.contains("[Background Contract]"));
    assert!(wrapped.contains("Do not call ask_user_clarification"));
    assert!(wrapped.contains("[Task]\narchive this fact"));
}

#[test]
fn accepted_message_hides_task_id_from_prose() {
    let payload = r#"{"task_id":"sub-internal-123","agent_id":"archivist","mode":"async"}"#;
    let message = format_async_subagent_accepted("archivist", payload);
    let prose = message
        .split("[async_subagent_ref]")
        .next()
        .expect("prose before structured reference");

    assert!(prose.contains("Accepted async sub-agent `archivist`"));
    assert!(!prose.contains("sub-internal-123"));
    assert!(message.contains("[async_subagent_ref]"));
    assert!(message.contains("sub-internal-123"));
}

#[test]
fn async_reference_payload_includes_agent_id_and_control_instructions() {
    let payload = async_subagent_ref_payload(
        "sub-123",
        "subsess-456",
        "researcher",
        Some("thread-worker"),
        false,
        "created",
        "running",
    );

    assert_eq!(payload["agent_id"], "researcher");
    assert_eq!(payload["agentId"], "researcher");
    assert_eq!(payload["instructions"]["wait"]["tool"], "wait_subagent");
    assert_eq!(
        payload["instructions"]["timeout_tick"]["arguments"]["timeout_secs"],
        1
    );
    assert_eq!(payload["instructions"]["delayed_tick"]["tool"], "wait");
    assert_eq!(payload["instructions"]["delayed_loop"]["tool"], "wait_loop");
    assert_eq!(
        payload["instructions"]["send_message"]["tool"],
        "steer_subagent"
    );
}

#[test]
fn durable_task_key_defaults_to_prompt_not_display_title() {
    let args = json!({
        "task_title": "Research",
        "prompt": "Research the async subagent cache behavior for example.com"
    });
    assert_eq!(
        durable_task_key_source(&args, args["prompt"].as_str().unwrap(), None),
        "Research the async subagent cache behavior for example.com"
    );
}

#[test]
fn durable_task_key_includes_context_when_no_explicit_key() {
    let args = json!({
        "prompt": "Analyze this issue"
    });
    let source = durable_task_key_source(
        &args,
        args["prompt"].as_str().unwrap(),
        Some("issue body A"),
    );
    assert!(source.contains("Analyze this issue"));
    assert!(source.contains("[Context]\nissue body A"));
    assert_ne!(
        subagent_sessions::normalize_task_key(&source),
        subagent_sessions::normalize_task_key(&durable_task_key_source(
            &args,
            args["prompt"].as_str().unwrap(),
            Some("issue body B")
        ))
    );
}

#[test]
fn durable_task_key_uses_explicit_task_key_when_present() {
    let args = json!({
        "task_key": "audit:example.com",
        "task_title": "Research",
        "prompt": "Research the async subagent cache behavior for example.com"
    });
    assert_eq!(
        durable_task_key_source(&args, args["prompt"].as_str().unwrap(), Some("ignored")),
        "audit:example.com"
    );
}

#[test]
fn reusable_follow_up_message_preserves_context() {
    let rendered = reusable_follow_up_message("Continue the audit", Some("prior result: 42"));
    assert!(rendered.contains("[Context]\nprior result: 42"));
    assert!(rendered.contains("[Task]\nContinue the audit"));
}

#[test]
fn extract_workflow_proposal_finds_last_proposal_tool_result() {
    let history = vec![
        ChatMessage::user("build me a workflow"),
        ChatMessage::tool(r#"{"type":"something_else","x":1}"#),
        ChatMessage::tool(r#"{"type":"workflow_proposal","persisted":false,"name":"Old Draft"}"#),
        ChatMessage::assistant("revising…"),
        ChatMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Daily X Trending Email"}"#,
        ),
        ChatMessage::assistant("Here's the proposed workflow."),
    ];
    let proposal = extract_workflow_proposal_from_history(&history).expect("proposal extracted");
    // The LAST proposal wins — later revisions supersede earlier drafts.
    assert_eq!(proposal["name"], "Daily X Trending Email");
}

#[test]
fn extract_workflow_proposal_ignores_non_proposal_history() {
    let history = vec![
        ChatMessage::user("hello"),
        ChatMessage::tool("plain text tool output, not json"),
        ChatMessage::assistant("done"),
    ];
    assert!(extract_workflow_proposal_from_history(&history).is_none());
}

#[test]
fn attach_workflow_proposal_persists_thread_message_and_extends_summary() {
    use crate::openhuman::memory::conversations::CreateConversationThread;
    let temp = tempfile::tempdir().expect("tempdir");
    conversations::ensure_thread(
        temp.path().to_path_buf(),
        CreateConversationThread {
            id: "thread-parent".into(),
            title: "Main chat".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .expect("thread created");

    let history = vec![ChatMessage::tool(
        r#"{"type":"workflow_proposal","persisted":false,"name":"Daily X Trending Email","graph":{"nodes":[],"edges":[]}}"#,
    )];
    let summary = attach_workflow_proposal(
        temp.path(),
        Some("thread-parent"),
        "sub-task-1",
        "workflow_builder",
        &history,
        "Here's the proposed workflow.".to_string(),
    );

    // Delivery notice carries the machine-readable envelope.
    assert!(summary.starts_with("Here's the proposed workflow."));
    assert!(summary.contains("[workflow_proposal]"));
    assert!(summary.contains("\"name\":\"Daily X Trending Email\""));
    assert!(summary.contains("[/workflow_proposal]"));

    // Proposal is durably persisted in the parent thread with rehydratable
    // metadata (this is what survives reload / a dropped socket event).
    let messages =
        conversations::get_messages(temp.path().to_path_buf(), "thread-parent").expect("messages");
    let proposal_msg = messages
        .iter()
        .find(|m| m.id == "workflow-proposal:sub-task-1")
        .expect("proposal message persisted");
    assert_eq!(proposal_msg.sender, "agent");
    assert!(proposal_msg.content.contains("Daily X Trending Email"));
    assert_eq!(proposal_msg.extra_metadata["scope"], "workflow_proposal");
    assert_eq!(
        proposal_msg.extra_metadata["proposal"]["name"],
        "Daily X Trending Email"
    );
    assert_eq!(proposal_msg.extra_metadata["task_id"], "sub-task-1");
}

#[test]
fn attach_workflow_proposal_without_proposal_returns_summary_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let summary = attach_workflow_proposal(
        temp.path(),
        Some("thread-x"),
        "sub-task-2",
        "researcher",
        &[ChatMessage::tool("no proposal here")],
        "research done".to_string(),
    );
    assert_eq!(summary, "research done");
}

#[tokio::test]
async fn missing_agent_id_returns_error() {
    let tool = SpawnAsyncSubagentTool::new();
    let result = tool.execute(json!({ "prompt": "do work" })).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("agent_id"));
}

#[tokio::test]
async fn missing_prompt_returns_error() {
    let tool = SpawnAsyncSubagentTool::new();
    let result = tool
        .execute(json!({ "agent_id": "archivist" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("prompt"));
}

/// B40 / Gap 4: a delegating agent (orchestrator/subconscious) calling
/// `spawn_async_subagent` directly from a thread-less context (flow
/// `agent` node, CLI, cron) must get a clear, actionable error instead of
/// silently accepting the spawn and later dropping its result in
/// `background_delivery`'s "headless batch" path. Sets up a real parent
/// turn context (so the call gets past the `current_parent()` /
/// allowlist / registry checks) but deliberately does NOT wrap the call
/// in `with_thread_id`, so `current_thread_id()` is None — the exact
/// condition that used to sail through to `tokio::spawn` and lose the
/// result.
#[tokio::test]
async fn errors_clearly_when_no_parent_thread_for_delivery() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");

    let result = with_parent_context(parent_context(workspace.path()), async {
        SpawnAsyncSubagentTool::new()
            .execute(json!({
                "agent_id": "researcher",
                "prompt": "investigate x",
            }))
            .await
    })
    .await
    .unwrap();

    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains("no parent chat thread"), "{out}");
    // The recommended escape hatch must name `blocking: true` — plain
    // `spawn_subagent` defaults to async and would otherwise be steered
    // straight back into this same guard.
    assert!(out.contains("spawn_subagent"), "{out}");
    assert!(out.contains("blocking: true"), "{out}");
    assert!(out.contains("delegate_"), "{out}");
}

/// The positive half of the branch above: with a chat thread bound, the
/// guard must NOT fire. This asserts only that the call gets *past* the
/// `parent_thread_id.is_none()` check — driving the full spawn/session
/// machinery to a successful "Accepted" is out of scope for a unit test,
/// so a later failure is acceptable; a "no parent chat thread" failure is
/// not. Pins that the guard keys on thread presence and nothing else.
#[tokio::test]
async fn guard_does_not_fire_when_parent_thread_is_bound() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");

    let result = with_parent_context(parent_context(workspace.path()), async {
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-parent", async {
            SpawnAsyncSubagentTool::new()
                .execute(json!({
                    "agent_id": "researcher",
                    "prompt": "investigate x",
                }))
                .await
        })
        .await
    })
    .await
    .unwrap();

    assert!(
        !result.output().contains("no parent chat thread"),
        "guard fired despite a bound parent thread: {}",
        result.output()
    );
}

fn parent_context(workspace_dir: &Path) -> ParentExecutionContext {
    ParentExecutionContext {
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: HashSet::from(["researcher".to_string()]),
        turn_model_source: crate::openhuman::agent::tinyagents::TurnModelSource::from_model(
            Arc::new(tinyagents_harness::testkit::ScriptedModel::replies(vec![
                "done",
            ])),
        ),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: workspace_dir.to_path_buf(),
        memory: Arc::new(NoopMemory),
        agent_config: AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "parent-session".into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: ToolCallFormat::Native,
        session_key: "parent-key".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
    }
}

struct NoopMemory;

#[async_trait::async_trait]
impl Memory for NoopMemory {
    fn name(&self) -> &str {
        "noop"
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
        _opts: RecallOpts<'_>,
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

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }
}
