use super::*;
use crate::agent::subagent_host::ParentExecutionContext;
use crate::agent_registry::host_definition::AgentDefinitionRegistry;
use crate::config::AgentConfig;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use tinytools_agent::dialect::ToolCallFormat;

#[test]
fn parameters_schema_advertises_fire_and_forget_fields() {
    let tool = SpawnSubagentTool::new();
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
    for key in ["context", "model", "task_title", "task_key", "image_paths"] {
        assert!(props.contains_key(key), "missing {key}");
    }
    for retired in ["blocking", "dedicated_thread", "archetype"] {
        assert!(
            !props.contains_key(retired),
            "legacy {retired} was advertised"
        );
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
fn async_reference_payload_routes_follow_up_through_the_same_task_key() {
    let payload = async_subagent_ref_payload(
        "sub-123",
        "subsess-456",
        "research-work",
        "task_manager_agent",
        Some("thread-worker"),
        false,
        "created",
        "running",
    );

    assert_eq!(payload["agent_id"], "task_manager_agent");
    assert_eq!(payload["task_key"], "research-work");
    assert_eq!(
        payload["instructions"]["answer_or_resume"]["tool"],
        "spawn_subagent"
    );
    assert_eq!(
        payload["instructions"]["answer_or_resume"]["arguments"]["task_key"],
        "research-work"
    );
    let serialized = serde_json::to_string(&payload).unwrap();
    for retired in [
        "continue_subagent",
        "wait_subagent",
        "steer_subagent",
        "close_subagent",
    ] {
        assert!(
            !serialized.contains(retired),
            "{retired} leaked into the envelope"
        );
    }
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
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        CreateConversationThread {
            id: "thread-parent".into(),
            title: "Main chat".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .expect("thread created");

    let history = vec![TranscriptMessage::tool(
        r#"{"type":"workflow_proposal","persisted":false,"name":"Daily X Trending Email","graph":{"nodes":[],"edges":[]}}"#,
    )];
    let summary = attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
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
    let messages = conversations::get_messages(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        "thread-parent",
    )
    .expect("messages");
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

/// A thread holding one delivered proposal (task `sub-propose`), named `name`.
fn thread_with_delivered_proposal(name: &str) -> tempfile::TempDir {
    use crate::threads::store::CreateConversationThread;
    let temp = tempfile::tempdir().expect("tempdir");
    conversations::ensure_thread(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        CreateConversationThread {
            id: "thread-parent".into(),
            title: "Main chat".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .expect("thread created");
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-propose",
        "workflow_builder",
        &[TranscriptMessage::tool(format!(
            r#"{{"type":"workflow_proposal","persisted":false,"name":"{name}","graph":{{"nodes":[],"edges":[]}}}}"#
        ))],
        "proposal".to_string(),
    );
    temp
}

fn delivered_proposal_metadata(temp: &tempfile::TempDir) -> serde_json::Value {
    conversations::get_messages(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        "thread-parent",
    )
    .expect("messages")
    .into_iter()
    .find(|m| m.id == "workflow-proposal:sub-propose")
    .expect("proposal message")
    .extra_metadata
}

#[test]
fn an_agent_save_of_the_delivered_proposal_is_recorded_on_its_card() {
    // The user said "save it" in chat: a later builder run created the flow.
    // The proposal card must resume at that flow instead of creating another.
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-save",
        "workflow_builder",
        &[TranscriptMessage::tool(
            r#"{"type":"workflow_created","flow_id":"flow-1","name":"Friday Inbox Wrap-up","enabled":false}"#,
        )],
        "saved".to_string(),
    );
    assert_eq!(
        delivered_proposal_metadata(&temp)["saved_flow_id"],
        "flow-1"
    );
}

#[test]
fn an_agent_save_of_another_workflow_leaves_the_proposal_alone() {
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-save",
        "workflow_builder",
        &[TranscriptMessage::tool(
            r#"{"type":"workflow_saved","flow_id":"flow-9","name":"Something Else","enabled":true}"#,
        )],
        "saved".to_string(),
    );
    assert!(delivered_proposal_metadata(&temp)
        .get("saved_flow_id")
        .is_none());
}

#[test]
fn a_save_followed_by_a_revision_does_not_settle_the_revision() {
    // The run saved "Digest", then proposed a revised "Digest": the earlier
    // save must not be recorded on the revision, or the card would enable the
    // old flow and drop the revised graph.
    let history = vec![
        TranscriptMessage::tool(
            r#"{"type":"workflow_saved","flow_id":"flow-old","name":"Digest","enabled":true}"#,
        ),
        TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Digest","graph":{"nodes":[],"edges":[]}}"#,
        ),
    ];
    assert!(extract_saved_workflow_from_history(&history).is_none());
}

#[test]
fn a_reused_session_settles_the_delivered_proposal_without_duplicating_it() {
    // Turn 1 delivered the proposal; the continuation (same task_key) only
    // saved it. The runner returns the whole session history, turn 1 included.
    let temp = thread_with_delivered_proposal("Friday Inbox Wrap-up");
    let seed = vec![
        TranscriptMessage::user("build it"),
        TranscriptMessage::tool(
            r#"{"type":"workflow_proposal","persisted":false,"name":"Friday Inbox Wrap-up","graph":{"nodes":[],"edges":[]}}"#,
        ),
        TranscriptMessage::user("save it"),
    ];
    let mut full = seed.clone();
    full.push(TranscriptMessage::tool(
        r#"{"type":"workflow_created","flow_id":"flow-1","name":"Friday Inbox Wrap-up","enabled":false}"#,
    ));

    let summary = attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path(),
        Some("thread-parent"),
        "sub-save",
        "workflow_builder",
        this_run_history(&full, seed.len()),
        "saved".to_string(),
    );

    // Turn 1's proposal is not re-delivered or re-persisted...
    assert!(!summary.contains("[workflow_proposal]"));
    let messages = conversations::get_messages(
        &crate::core::runtime::DispatchCtx::ambient(),
        temp.path().to_path_buf(),
        "thread-parent",
    )
    .expect("messages");
    assert!(!messages
        .iter()
        .any(|m| m.id == "workflow-proposal:sub-save"));
    // ...and the save settles the proposal the card shows.
    assert_eq!(
        delivered_proposal_metadata(&temp)["saved_flow_id"],
        "flow-1"
    );
}

#[test]
fn a_history_shorter_than_its_seed_is_used_whole() {
    let history = vec![TranscriptMessage::user("compacted")];
    assert_eq!(this_run_history(&history, 3).len(), 1);
    assert!(this_run_history(&history, 1).is_empty());
}

#[test]
fn attach_workflow_proposal_without_proposal_returns_summary_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let summary = attach_workflow_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
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
    let tool = SpawnSubagentTool::new();
    let result = tool.execute(json!({ "prompt": "do work" })).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("agent_id"));
}

#[tokio::test]
async fn missing_prompt_returns_error() {
    let tool = SpawnSubagentTool::new();
    let result = tool
        .execute(json!({ "agent_id": "archivist" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("prompt"));
}

/// Headless callers have no thread for follow-up chat delivery. They still
/// start an asynchronous child and retain its result in the run log.
#[tokio::test]
async fn headless_spawn_is_async_and_reports_its_result_to_the_run_log() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(64);
    let mut parent = parent_context(workspace.path());
    parent.session_id = "headless-spawn-session".into();
    parent.on_progress = Some(progress_tx);
    let (live, _core) = headless_core_backed_parent(workspace.path(), parent);
    assert!(
        live.data.dispatch_ctx().resolved().is_some(),
        "the headless parent carrier retains its core context"
    );

    let result = SpawnSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "investigate x",
            }),
            None,
            live.data.child(),
            crate::agent::subagent_host::detached_subagent_parent(&live, "headless-child")
                .expect("detached parent"),
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().contains("Accepted async sub-agent"));
    let reference = result
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .expect("async response contains task reference");
    let task_id = reference["task_id"].as_str().expect("task id");
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match progress_rx.recv().await {
                Some(crate::progress::AgentProgress::SubagentCompleted {
                    task_id: event_task_id,
                    output,
                    ..
                }) if event_task_id == task_id => break output,
                Some(crate::progress::AgentProgress::SubagentFailed {
                    task_id: event_task_id,
                    error,
                    ..
                }) if event_task_id == task_id => {
                    panic!("headless child failed before completion: {error}")
                }
                Some(_) => {}
                None => panic!("headless completion progress stream closed"),
            }
        }
    })
    .await
    .expect("headless result is projected to the run log");
    assert_eq!(output, "done");
    let jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&live.data.dispatch_ctx());
    let job = jobs
        .list()
        .into_iter()
        .find(|job| {
            job.extensions
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                == Some(task_id)
        })
        .expect("headless child is registered");
    let finished = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        jobs.wait_owned(
            job.id.as_str(),
            crate::agent::tinyagents::tool_registration::subagent_owner_key(
                workspace.path(),
                "headless-spawn-session",
                None,
            ),
        ),
    )
    .await
    .expect("headless child completes")
    .expect("headless child remains registered");
    assert_eq!(
        finished.status,
        tinyagents_orchestration::subagent::SubAgentJobStatus::Completed
    );
    assert_eq!(finished.output.as_deref(), Some("done"));
}

pub(super) fn headless_core_backed_parent(
    workspace: &Path,
    mut parent: ParentExecutionContext,
) -> (
    tinyagents_harness::context::RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    Arc<crate::core::runtime::CoreContext>,
) {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let mut definition = AgentDefinitionRegistry::builtins_only()
        .get("task_manager_agent")
        .expect("task manager definition")
        .clone();
    definition.model = tinyagents_definition::ModelSpec::Inherit;
    let definitions =
        Arc::new(AgentDefinitionRegistry::builtins_only().with_definitions([definition]));
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.components.juice = false;
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).definitions(definitions),
    );
    parent.workspace_dir = workspace.to_path_buf();
    let host = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent.clone());
    (
        tinyagents_harness::context::RunContext::new(
            tinyagents_harness::context::RunConfig::new(parent.session_id),
            host,
        ),
        core,
    )
}

pub(super) fn parent_context(workspace_dir: &Path) -> ParentExecutionContext {
    ParentExecutionContext {
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
        host_env: Default::default(),
    }
}

/// The session builder swaps in a scoped instance so the WIRE schema (read
/// from the registered tool on native tool calling) carries only the ids the
/// parent may dispatch, sorted and deduplicated, and says so.
#[test]
fn scoped_instance_advertises_exactly_the_allowlist() {
    let tool = SpawnSubagentTool::scoped(vec![
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
