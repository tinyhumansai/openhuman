use super::*;
use crate::agent::harness::definition::AgentDefinitionRegistry;
use crate::agent::harness::fork_context::{with_parent_context, ParentExecutionContext};
use crate::agent::prompts::ToolCallFormat;
use crate::config::AgentConfig;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tinytools::{ToolCallOptions, ToolRunContext};

struct ThreadContext(&'static str);

impl ToolRunContext for ThreadContext {
    fn thread_id(&self) -> Option<&str> {
        Some(self.0)
    }
}

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
    for key in ["context", "model", "task_title"] {
        assert!(props.contains_key(key), "missing {key}");
    }
    assert!(
        !props.contains_key("toolkit"),
        "the retired toolkit spawn argument must not be advertised"
    );
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
    let message = format_async_subagent_accepted("archivist", payload, &FleetToolSet::all());
    let prose = message
        .split("[async_subagent_ref]")
        .next()
        .expect("prose before structured reference");

    assert!(prose.contains("Accepted async sub-agent `archivist`"));
    assert!(!prose.contains("sub-internal-123"));
    assert!(message.contains("[async_subagent_ref]"));
    assert!(message.contains("sub-internal-123"));
}

/// A parent with `wait_subagent` but not `steer_subagent` (a wait-only
/// fleet) must be told it can wait/poll but never invited to "send more
/// input" — that guidance, and the `send_message` instruction, require
/// `steer_subagent` specifically. Regression for CodeRabbit finding on
/// `format_async_subagent_accepted`/`async_subagent_ref_payload` gating both
/// messages on `can_wait()` alone.
#[test]
fn wait_only_fleet_omits_steering_guidance_and_instruction() {
    let fleet = FleetToolSet::from_scope(
        &crate::agent::harness::definition::ToolScope::Named(vec!["wait_subagent".to_string()]),
        &[],
    );
    assert!(fleet.can_wait());
    assert!(!fleet.has("steer_subagent"));

    let payload = async_subagent_ref_payload(
        "sub-123",
        "subsess-456",
        "task_manager_agent",
        Some("thread-worker"),
        false,
        "created",
        "running",
        &fleet,
    );
    assert_eq!(payload["instructions"]["wait"]["tool"], "wait_subagent");
    assert!(
        !payload["instructions"]
            .as_object()
            .expect("instructions map")
            .contains_key("send_message"),
        "send_message offered to a parent without steer_subagent"
    );
    let serialized = serde_json::to_string(&payload).unwrap();
    assert!(
        !serialized.contains("steer_subagent"),
        "steer_subagent leaked into the envelope"
    );

    let message = format_async_subagent_accepted("task_manager_agent", &serialized, &fleet);
    let prose = message.split("[async_subagent_ref]").next().unwrap();
    assert!(prose.contains("wait for completion"));
    assert!(
        !prose.contains("send more input"),
        "wait-only parent told to send more input it cannot send"
    );
}

#[test]
fn async_reference_payload_includes_agent_id_and_control_instructions() {
    let payload = async_subagent_ref_payload(
        "sub-123",
        "subsess-456",
        "task_manager_agent",
        Some("thread-worker"),
        false,
        "created",
        "running",
        &FleetToolSet::all(),
    );

    assert_eq!(payload["agent_id"], "task_manager_agent");
    assert_eq!(payload["agentId"], "task_manager_agent");
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

/// The shipped orchestrator (#5701) has no wait/steer/close tools; the envelope
/// must not name them, and must say the result arrives on its own.
#[test]
fn async_reference_matches_the_orchestrator_fleet_vocabulary() {
    let registry = AgentDefinitionRegistry::builtins_only();
    let def = registry.get("orchestrator").expect("built-in orchestrator");
    let fleet = FleetToolSet::from_scope(&def.tools, &def.disallowed_tools);

    let payload = async_subagent_ref_payload(
        "sub-123",
        "subsess-456",
        "task_manager_agent",
        None,
        false,
        "created",
        "running",
        &fleet,
    );
    let instructions = payload["instructions"]
        .as_object()
        .expect("instructions map");
    for absent in [
        "wait",
        "timeout_tick",
        "delayed_tick",
        "delayed_loop",
        "send_message",
    ] {
        assert!(
            !instructions.contains_key(absent),
            "{absent} offered to a parent without it"
        );
    }
    assert_eq!(
        payload["instructions"]["answer_or_resume"]["tool"],
        "continue_subagent"
    );
    assert_eq!(payload["result_delivery"], "automatic");
    let serialized = serde_json::to_string(&payload).unwrap();
    for name in [
        "wait_subagent",
        "steer_subagent",
        "wait_loop",
        "close_subagent",
    ] {
        assert!(
            !serialized.contains(name),
            "{name} leaked into the envelope"
        );
    }
    let next: Vec<&str> = payload["next_actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(next
        .iter()
        .any(|a| a.contains("delivered to you automatically")));

    let message = format_async_subagent_accepted("task_manager_agent", &serialized, &fleet);
    let prose = message.split("[async_subagent_ref]").next().unwrap();
    assert!(prose.contains("delivered to you automatically"));
    assert!(!prose.contains("wait for completion"));
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
        TranscriptMessage::user("build me a workflow"),
        TranscriptMessage::tool(r#"{"type":"something_else","x":1}"#),
        TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Old Draft"}"#,
        ),
        TranscriptMessage::assistant("revising…"),
        TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Daily X Trending Email"}"#,
        ),
        TranscriptMessage::assistant("Here's the proposed workflow."),
    ];
    let proposal = extract_workflow_proposal_from_history(&history).expect("proposal extracted");
    // The LAST proposal wins — later revisions supersede earlier drafts.
    assert_eq!(proposal["name"], "Daily X Trending Email");
}

#[test]
fn extract_workflow_proposal_ignores_non_proposal_history() {
    let history = vec![
        TranscriptMessage::user("hello"),
        TranscriptMessage::tool("plain text tool output, not json"),
        TranscriptMessage::assistant("done"),
    ];
    assert!(extract_workflow_proposal_from_history(&history).is_none());
}

#[test]
fn attach_workflow_proposal_persists_thread_message_and_extends_summary() {
    use crate::threads::store::CreateConversationThread;
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

    let history = vec![TranscriptMessage::tool(
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
        "task_manager_agent",
        &[TranscriptMessage::tool("no proposal here")],
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

/// B40 / Gap 4: a delegating agent (e.g. the orchestrator) calling
/// `spawn_async_subagent` directly from a thread-less context (flow
/// `agent` node, CLI, cron) must get a clear, actionable error instead of
/// silently accepting the spawn and later dropping its result in
/// `background_delivery`'s "headless batch" path. Sets up a real parent
/// turn context (so the call gets past the `current_parent()` /
/// allowlist / registry checks) but deliberately does NOT wrap the call
/// without an explicit parent thread, so the child has no thread — the exact
/// condition that used to sail through to `tokio::spawn` and lose the
/// result.
#[tokio::test]
async fn errors_clearly_when_no_parent_thread_for_delivery() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");

    let result = with_parent_context(parent_context(workspace.path()), async {
        SpawnAsyncSubagentTool::new()
            .execute(json!({
                "agent_id": "task_manager_agent",
                "prompt": "investigate x",
            }))
            .await
    })
    .await
    .unwrap();

    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains("no parent chat thread"), "{out}");
    // The recommended escape hatch must name a route that does NOT loop back
    // into this guard: a synchronous one, with the argument that makes it
    // synchronous.
    //
    // The literal `spawn_subagent` is deliberately no longer asserted. The
    // guidance used to name it; it now recommends `delegate_*` instead, which
    // is the better advice for the reason this comment already gave — plain
    // `spawn_subagent` defaults to async and would steer the model straight
    // back here. Verified callable under the product feature profile:
    // `delegate_<agent_id>` tools are generated in
    // `tools/orchestrator_tools.rs:116`, and they accept `blocking`
    // (`archetype_delegation.rs:115` declares it, `:245` reads it) — so the
    // guidance names a real tool family with a real argument, not a route the
    // model cannot take.
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

    let thread = ThreadContext("t-parent");
    let result = with_parent_context(parent_context(workspace.path()), async {
        SpawnAsyncSubagentTool::new()
            .execute_with_context(
                json!({
                    "agent_id": "task_manager_agent",
                    "prompt": "investigate x",
                }),
                ToolCallOptions::default(),
                Some(&thread),
            )
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
        runtime_config: None,
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: HashSet::from(["task_manager_agent".to_string()]),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(vec!["done"]),
        )),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: workspace_dir.to_path_buf(),
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

/// The session builder swaps in a scoped instance so the WIRE schema (read
/// from the registered tool on native tool calling) carries only the ids the
/// parent may dispatch, sorted and deduplicated, and says so.
#[test]
fn scoped_instance_advertises_exactly_the_allowlist() {
    let tool = SpawnAsyncSubagentTool::scoped(vec![
        "workflow_builder".to_string(),
        "agent_memory".to_string(),
        "workflow_builder".to_string(),
    ]);
    let schema = tool.parameters_schema();
    assert_eq!(
        schema["properties"]["agent_id"]["enum"],
        serde_json::json!(["agent_memory", "workflow_builder"])
    );
    assert!(schema["properties"]["agent_id"]["description"]
        .as_str()
        .unwrap_or_default()
        .contains("only these are dispatchable"));
}

#[test]
fn scoped_instance_advertises_an_empty_enum_for_deny_all() {
    let schema = SpawnAsyncSubagentTool::scoped(Vec::new()).parameters_schema();
    assert_eq!(
        schema["properties"]["agent_id"]["enum"],
        serde_json::json!([])
    );
}

/// #6934: a custom agent lives only in `config.agent_registry`, so a
/// registry-only lookup refused every allowlisted custom id as "unknown
/// agent_id" even once the parent's allowlist named it. With the parent's
/// config snapshot on the context the id resolves; the call then proceeds to
/// the same delivery guard the test above pins, which proves it got past
/// both the lookup and the allowlist gate.
#[tokio::test]
async fn an_allowlisted_custom_agent_resolves_through_the_parent_config() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");

    let mut config = crate::config::Config::default();
    config.agent_registry.entries = vec![crate::agent::registry::AgentRegistryEntry {
        id: "researcher".to_string(),
        name: "Researcher".to_string(),
        description: "Deep research on one topic.".to_string(),
        source: crate::agent::registry::AgentRegistrySource::Custom,
        enabled: true,
        model: None,
        system_prompt: Some("Research the topic.".to_string()),
        tool_allowlist: vec!["web_search_tool".to_string()],
        tool_denylist: Vec::new(),
        subagents: crate::agent::registry::types::AgentSubagentPolicy::default(),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
    }];
    let mut parent = parent_context(workspace.path());
    parent.allowed_subagent_ids = HashSet::from(["researcher".to_string()]);
    parent.runtime_config = Some(Arc::new(config));

    let result = with_parent_context(parent, async {
        SpawnAsyncSubagentTool::new()
            .execute(json!({
                "agent_id": "researcher",
                "prompt": "survey the literature",
            }))
            .await
    })
    .await
    .unwrap();

    let out = result.output();
    assert!(
        !out.contains("unknown agent_id"),
        "custom id must resolve: {out}"
    );
    assert!(
        !out.contains("subagents.allowlist"),
        "allowlist gate must pass: {out}"
    );
    assert!(
        out.contains("no parent chat thread"),
        "reached the delivery guard: {out}"
    );
}

/// Without a config snapshot the lookup is registry-only, and the error lists
/// what *is* spawnable rather than failing silently.
#[tokio::test]
async fn a_custom_agent_is_unknown_to_a_parent_without_a_config() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let mut parent = parent_context(workspace.path());
    parent.allowed_subagent_ids = HashSet::from(["researcher".to_string()]);

    let result = with_parent_context(parent, async {
        SpawnAsyncSubagentTool::new()
            .execute(json!({ "agent_id": "researcher", "prompt": "survey" }))
            .await
    })
    .await
    .unwrap();

    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains("unknown agent_id 'researcher'"), "{out}");
    assert!(
        out.contains("task_manager_agent"),
        "lists the spawnable ids: {out}"
    );
}
