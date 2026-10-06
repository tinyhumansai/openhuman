//! Tests for the turn prelude's tool surface across a resumed thread.

use std::sync::Arc;

use tinyagents_runtime::ToolSnapshot;
use tinytools::ToolSpec;

#[tokio::test]
async fn committed_progress_uses_each_turns_receipt_sender() {
    use crate::agent::progress::AgentProgress;
    use tinyagents_runtime::{
        CommitReceipt, ResumeMode, SessionTurnOutcome, TranscriptTurnOptions,
    };

    let receipt = |tx| {
        let mut context = crate::agent::tinyagents::host::OpenHumanRunContext::new();
        context.progress = Some(tx);
        CommitReceipt {
            outcome: SessionTurnOutcome {
                history: Vec::new(),
                output: Some("answer".into()),
                interrupted: false,
            },
            options: TranscriptTurnOptions {
                request_id: None,
                thread_id: None,
                stream: true,
                resume: ResumeMode::Never,
                context,
            },
            transcript: None,
        }
    };
    let (first_tx, mut first_rx) = tokio::sync::mpsc::channel(2);
    let (second_tx, mut second_rx) = tokio::sync::mpsc::channel(2);
    let first = receipt(first_tx);
    let second = receipt(second_tx);

    assert!(super::progress::send_receipt_progress(&second, "question", "answer", 2).await);
    assert!(matches!(
        second_rx.recv().await,
        Some(AgentProgress::TurnContent { .. })
    ));
    assert!(matches!(
        second_rx.recv().await,
        Some(AgentProgress::TurnCompleted { iterations: 2 })
    ));
    assert!(matches!(
        first_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    drop(first);
}

#[test]
fn clearing_progress_releases_warm_prelude_sender() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    let mut host =
        crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
            .expect("orchestrator");
    host.ensure_runtime_session().expect("warm runtime");

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    host.set_on_progress(Some(tx));
    host.set_on_progress(None);
    assert!(matches!(
        rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
    ));
}

/// A full progress channel must not discard the only terminal signal. A busy
/// bridge can catch up after the turn commits; it cannot infer completion from
/// an event that was dropped.
#[tokio::test]
async fn committed_turn_completion_waits_for_a_full_progress_channel() {
    use crate::agent::progress::AgentProgress;

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.send(AgentProgress::TurnStarted).await.unwrap();
    let send = super::progress::send_committed_turn_progress(&tx, "question", "answer", 2);
    tokio::pin!(send);
    assert!(matches!(
        futures::poll!(send.as_mut()),
        std::task::Poll::Pending
    ));

    assert!(matches!(rx.recv().await, Some(AgentProgress::TurnStarted)));
    assert!(send.await);
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("terminal progress event"),
        Some(AgentProgress::TurnCompleted { iterations: 2 })
    ));
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("content after terminal progress event"),
        Some(AgentProgress::TurnContent {
            input: Some(input),
            output: Some(output),
        }) if input == "question" && output == "answer"
    ));
}

/// A receiver can remain alive while its bridge is stalled. Once the send
/// deadline passes, the committed chat response must still be able to return.
#[tokio::test(start_paused = true)]
async fn committed_turn_completion_is_bounded_when_progress_stalls() {
    use crate::agent::progress::AgentProgress;

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.send(AgentProgress::TurnStarted).await.unwrap();

    assert!(!super::progress::send_committed_turn_progress(&tx, "question", "answer", 2).await);
    assert!(matches!(rx.recv().await, Some(AgentProgress::TurnStarted)));
    assert!(rx.try_recv().is_err(), "timed-out send must be cancelled");
}

/// The registry override must survive the real session-host parent snapshot,
/// not only the builder's schema assertions. This drives the same tool
/// execution path used by a running turn and verifies that an enabled custom
/// child passes both definition lookup and the parent's effective allowlist.
#[tokio::test]
async fn running_session_allows_a_registry_override_custom_spawn() {
    use crate::agent::harness::{with_parent_context, AgentDefinitionRegistry};
    use crate::agent::orchestration::tools::SpawnAsyncSubagentTool;
    use crate::agent::registry::types::AgentSubagentPolicy;
    use crate::agent::registry::{AgentRegistryEntry, AgentRegistrySource};
    use crate::config::Config;
    use tinytools::Tool;

    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tmp = tempfile::tempdir().expect("workspace");
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace directory");
    config.agent_registry.entries = vec![
        AgentRegistryEntry {
            id: "researcher".into(),
            name: "Researcher".into(),
            description: "Researches a topic".into(),
            source: AgentRegistrySource::Custom,
            enabled: true,
            model: None,
            system_prompt: Some("Research the topic.".into()),
            tool_allowlist: Vec::new(),
            tool_denylist: Vec::new(),
            subagents: AgentSubagentPolicy::default(),
            tags: Vec::new(),
            metadata: serde_json::Value::Null,
        },
        AgentRegistryEntry {
            id: "orchestrator".into(),
            name: "Orchestrator".into(),
            description: "Coordinates work".into(),
            source: AgentRegistrySource::Default,
            enabled: true,
            model: None,
            system_prompt: None,
            tool_allowlist: Vec::new(),
            tool_denylist: Vec::new(),
            subagents: AgentSubagentPolicy::from_allowlist(vec!["researcher".into()]),
            tags: Vec::new(),
            metadata: serde_json::Value::Null,
        },
    ];

    let mut host =
        crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
            .expect("orchestrator session");
    host.ensure_runtime_session().expect("warm runtime");
    let prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("turn prelude");
    let parent = prelude.parent_context();
    assert!(parent.allowed_subagent_ids.contains("researcher"));
    assert!(parent
        .all_tools
        .iter()
        .any(|tool| tool.name() == "spawn_async_subagent"));

    let result = with_parent_context(parent, async move {
        SpawnAsyncSubagentTool::new()
            .execute(serde_json::json!({
                "agent_id": "researcher",
                "prompt": "survey the literature",
            }))
            .await
    })
    .await
    .expect("tool execution");
    let output = result.output();
    assert!(!output.contains("unknown agent_id"), "{output}");
    assert!(!output.contains("subagents.allowlist"), "{output}");
    assert!(output.contains("no parent chat thread"), "{output}");
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: format!("{name} description"),
        parameters: serde_json::json!({ "type": "object", "properties": {} }),
    }
}

#[cfg(feature = "mcp")]
#[test]
fn connected_mcp_actions_enter_search_and_leave_on_disconnect() {
    use crate::mcp::registry::action_tool::searchable_name;
    use crate::mcp::registry::types::{ConnectedServerOverview, McpTool};

    crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins()
        .expect("builtin definitions");
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    let mut host =
        crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
            .expect("orchestrator");
    host.ensure_runtime_session().expect("runtime session");
    let prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("prelude");
    prelude
        .mutable
        .lock()
        .expect("prelude state")
        .connected_mcp_tools = vec![ConnectedServerOverview {
        server_id: "server-1".into(),
        qualified_name: "example/weather".into(),
        display_name: "Weather".into(),
        description: None,
        instructions: None,
        tools: vec![McpTool {
            name: "forecast".into(),
            description: Some("Get weather forecast".into()),
            input_schema: serde_json::json!({"type":"object","properties":{}}),
        }],
    }];

    let action = searchable_name("server-1", "example/weather", "forecast");
    prelude.refresh_delegation_tool_surface().unwrap();
    assert!(prelude.synthesized_tool_names_for_test().contains(&action));
    {
        let surface = prelude.tool_surface.lock().expect("tool surface");
        assert!(surface.deferred_tool_names.contains(&action));
        assert!(!surface.visible_tool_names.contains(&action));
    }

    prelude
        .mutable
        .lock()
        .expect("prelude state")
        .connected_mcp_tools
        .clear();
    prelude.refresh_delegation_tool_surface().unwrap();
    assert!(!prelude.synthesized_tool_names_for_test().contains(&action));
    assert!(!prelude
        .tool_surface
        .lock()
        .expect("tool surface")
        .deferred_tool_names
        .contains(&action));
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn desktop_browser_setting_keeps_deferred_tools_in_fresh_and_resumed_surfaces() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(workspace.join("state")).expect("state dir");
    // A desktop onboarding snapshot predating TinyBrowser has other enabled
    // tools but cannot name browser/browser_open. The explicit Browser setting
    // must still register both deferred tools for the user-facing session.
    std::fs::write(
        workspace.join("state/app-state.json"),
        r#"{"onboardingTasks":{"enabledTools":["shell","file_read"]}}"#,
    )
    .expect("app state");
    let mut config = crate::config::Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    config.browser.enabled = true;
    config.http_request.allowed_domains = vec!["example.com".into()];
    let mut host =
        crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
            .expect("desktop orchestrator");
    for name in ["browser", "browser_open"] {
        assert!(host.deferred_tool_names_for_test().contains(name), "{name}");
        assert!(!host
            .visible_tool_specs_arc()
            .iter()
            .any(|spec| spec.name == name));
    }
    host.ensure_runtime_session().expect("runtime session");
    let prelude = host
        .runtime_state
        .lock()
        .expect("runtime state")
        .prelude
        .clone()
        .expect("prelude");
    let fresh = prelude.prepare(true).await.expect("fresh tool surface");
    let fresh = fresh.tools.expect("fresh tools");
    for name in ["browser", "browser_open"] {
        assert!(fresh.specs().iter().any(|spec| spec.name == name), "{name}");
    }
    prelude.adopt_recorded_tools(Some(&fresh));
    prelude.refresh_delegation_tool_surface().unwrap();
    let resumed = prelude.prepare(false).await.expect("resumed tool surface");
    for name in ["browser", "browser_open"] {
        assert!(
            resumed
                .tools
                .as_ref()
                .unwrap()
                .specs()
                .iter()
                .any(|spec| spec.name == name),
            "{name}"
        );
    }

    config.browser.enabled = false;
    let disabled =
        crate::agent::OpenHumanSessionHost::from_config_for_agent(&config, "orchestrator")
            .expect("browser-disabled orchestrator");
    for name in ["browser", "browser_open"] {
        assert!(
            !disabled.deferred_tool_names_for_test().contains(name),
            "{name}"
        );
    }
}

/// The incident this guards: a thread resumed in a fresh process (empty
/// integrations cache) lost every Composio action, so the orchestrator's
/// `tool_search` had nothing to find although its restored prompt told it to
/// search for the Gmail action. Rehydration is permitted only after the
/// current integration authorization snapshot confirms Gmail is connected.
#[tokio::test]
async fn a_resumed_orchestrator_keeps_the_integration_actions_it_was_sent() {
    let _ = crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins();
    let action_dir = tempfile::tempdir().expect("tempdir");
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(Vec::new())
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .agent_definition_name("orchestrator")
        .build()
        .expect("session build");
    host.ensure_runtime_session().expect("runtime session");

    let state = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let prelude = state.prelude.as_ref().expect("prelude");

    // Fresh process: no integrations known, so no actions are synthesised.
    prelude.refresh_delegation_tool_surface().unwrap();
    assert!(!prelude
        .synthesized_tool_names_for_test()
        .contains("GMAIL_SEND_EMAIL"));

    // Resume hands back what the thread was sent.
    let recorded = ToolSnapshot::new(vec![
        spec("GMAIL_SEND_EMAIL"),
        spec("GMAIL_FETCH_EMAILS"),
        spec("web_fetch"),
    ])
    .expect("snapshot");
    prelude.adopt_recorded_tools(Some(&recorded));
    {
        let mut mutable = prelude.mutable.lock().expect("prelude state");
        mutable.connected_integrations = vec![crate::agent::prompts::ConnectedIntegration {
            toolkit: "gmail".into(),
            description: String::new(),
            tools: Vec::new(),
            gated_tools: Vec::new(),
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        }];
        mutable.connected_integrations_authoritative = true;
    }
    prelude.refresh_delegation_tool_surface().unwrap();

    let names = prelude.synthesized_tool_names_for_test();
    assert!(names.contains("GMAIL_SEND_EMAIL"), "{names:?}");
    assert!(names.contains("GMAIL_FETCH_EMAILS"), "{names:?}");
    assert!(
        !names.contains("web_fetch"),
        "only integration actions are rebuilt from the record"
    );
}

/// The incident this guards: `session_locator()` is called from more than one
/// place while assembling a session's runtime turn machinery (the
/// `before_resume` resume target and the eager construction-time
/// `builder.session(...)` bind), and tinyagents only accepts a later
/// transcript-target change when it is the exact same locator object
/// (`Arc::ptr_eq`), not merely an equivalent one over the same file. Before
/// this was memoized, each call minted a fresh `FileTranscriptLocator`, so a
/// thread's second turn was rejected with "cannot change a transcript target
/// after it is bound or committed" even though both binds agreed on the
/// destination. Pin the fix directly: every call must return the identical
/// `Arc`.
#[tokio::test]
async fn session_locator_is_memoized_across_calls() {
    let action_dir = tempfile::tempdir().expect("tempdir");
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let host = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(Vec::new())
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .agent_definition_name("orchestrator")
        .build()
        .expect("session build");

    let first = host.session_locator();
    let second = host.session_locator();
    assert!(
        Arc::ptr_eq(&first, &second),
        "session_locator() must return the same Arc on every call, or tinyagents' \
         same-binding check rejects the second transcript bind"
    );
}

const THREAD_GOAL_TOOLS: [&str; 3] = ["goal_get", "goal_set", "goal_complete"];
/// The goal tool this fixture's default (non-orchestrator) session surfaces on a
/// thread; the others are admitted only for agents that list them.
const SURFACED_GOAL_TOOL: &str = "goal_complete";

/// A text-dialect (XML) session over the real per-thread goal tools, optionally
/// bound to a chat thread, with its runtime session already initialised.
fn text_dialect_host(
    thread_id: Option<&str>,
) -> (crate::agent::OpenHumanSessionHost, tempfile::TempDir) {
    let _ = crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins();
    let action_dir = tempfile::tempdir().expect("tempdir");
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let tools = crate::agent::goals::goal_tools(action_dir.path());
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(tools)
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .build()
        .expect("session build");
    host.set_thread_id(thread_id);
    host.ensure_runtime_session().expect("runtime session");
    (host, action_dir)
}

/// Render the first-turn system prompt and declared tool list of `host`.
async fn prompt_and_declared_tools(
    host: &crate::agent::OpenHumanSessionHost,
) -> (String, ToolSnapshot) {
    let prelude = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .prelude
        .clone()
        .expect("prelude");
    let tiered = prelude.build_system_prompt_tiered().expect("system prompt");
    let snapshot = prelude
        .prepare(true)
        .await
        .expect("tool surface")
        .tools
        .expect("declared tools");
    (tiered.text, snapshot)
}

fn assert_no_goal_tools(prompt: &str, snapshot: &ToolSnapshot) {
    for name in THREAD_GOAL_TOOLS {
        assert!(!prompt.contains(name), "{name} leaked into the prompt");
        assert!(
            !snapshot.specs().iter().any(|spec| spec.name == name),
            "{name} leaked into the declared tools"
        );
    }
}

/// Issue #6956 follow-up: the harness drops `goal_*` for a thread-less turn, so
/// a text-dialect prompt that still catalogued them taught the model tools that
/// answer "unregistered tool" when called.
#[tokio::test]
async fn text_dialect_prompt_omits_thread_goal_tools_without_a_thread() {
    let (host, _dir) = text_dialect_host(None);
    let (prompt, snapshot) = prompt_and_declared_tools(&host).await;
    assert_no_goal_tools(&prompt, &snapshot);
}

#[tokio::test]
async fn text_dialect_prompt_lists_thread_goal_tools_on_a_thread() {
    let (host, _dir) = text_dialect_host(Some("thread-goals"));
    let (prompt, snapshot) = prompt_and_declared_tools(&host).await;
    assert!(
        prompt.contains(SURFACED_GOAL_TOOL),
        "{SURFACED_GOAL_TOOL} missing from the prompt"
    );
    assert!(
        snapshot
            .specs()
            .iter()
            .any(|spec| spec.name == SURFACED_GOAL_TOOL),
        "{SURFACED_GOAL_TOOL} missing from the declared tools"
    );
}

/// The prompt and tool snapshot are cached from the first turn, so binding a
/// thread after the runtime session exists is refused: the thread-less
/// surface (no goal tools) must not be contradicted by a later thread id.
#[tokio::test]
async fn a_thread_bound_after_session_init_is_refused_and_keeps_goal_tools_out() {
    let (mut host, _dir) = text_dialect_host(None);
    host.set_thread_id(Some("late-thread"));
    assert_eq!(host.thread_id(), None, "late thread id must be refused");
    assert_eq!(host.session_id(), None, "no durable identity was minted");
    let (prompt, snapshot) = prompt_and_declared_tools(&host).await;
    assert_no_goal_tools(&prompt, &snapshot);
}

#[tokio::test]
async fn rebinding_the_same_thread_after_session_init_is_a_no_op() {
    let (mut host, _dir) = text_dialect_host(Some("same-thread"));
    host.set_thread_id(Some("same-thread"));
    assert_eq!(host.thread_id(), Some("same-thread"));
    let (prompt, _snapshot) = prompt_and_declared_tools(&host).await;
    assert!(prompt.contains(SURFACED_GOAL_TOOL));
}

#[tokio::test]
async fn newly_connected_action_cannot_shadow_a_permanent_source() {
    use super::*;
    struct AttachedAction;
    #[async_trait::async_trait]
    impl tinytools::Tool for AttachedAction {
        fn name(&self) -> &str {
            "GMAIL_SEND_EMAIL"
        }
        fn description(&self) -> &str {
            "host-owned action"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }
        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
            Ok(tinytools::ToolResult::success("host"))
        }
    }
    let _ = crate::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins();
    let action_dir = tempfile::tempdir().unwrap();
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let mut host = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(vec![Box::new(AttachedAction)])
        .permanent_tool_names(std::collections::HashSet::from(["GMAIL_SEND_EMAIL".into()]))
        .action_dir(action_dir.path().to_path_buf())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .agent_definition_name("orchestrator")
        .build()
        .unwrap();
    host.ensure_runtime_session().unwrap();
    let prelude = host.runtime_state.lock().unwrap().prelude.clone().unwrap();
    prelude.refresh_delegation_tool_surface().unwrap();
    let before = prelude
        .tool_surface
        .lock()
        .unwrap()
        .visible_tool_specs
        .clone();
    prelude.adopt_recorded_tools(Some(
        &ToolSnapshot::new(vec![spec("GMAIL_SEND_EMAIL")]).unwrap(),
    ));
    {
        let mut mutable = prelude.mutable.lock().unwrap();
        mutable.connected_integrations = vec![crate::agent::prompts::ConnectedIntegration {
            toolkit: "gmail".into(),
            description: String::new(),
            tools: Vec::new(),
            gated_tools: Vec::new(),
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        }];
        mutable.connected_integrations_authoritative = true;
    }
    let error = prelude.refresh_delegation_tool_surface().unwrap_err();
    assert!(error
        .to_string()
        .contains("collision with synthesized tool"));
    let surface = prelude.tool_surface.lock().unwrap();
    assert!(Arc::ptr_eq(&surface.visible_tool_specs, &before));
    assert_eq!(surface.tools[0].description(), "host-owned action");
    assert!(!surface
        .synthesized_tools
        .iter()
        .any(|tool| tool.name() == "GMAIL_SEND_EMAIL"));
}
