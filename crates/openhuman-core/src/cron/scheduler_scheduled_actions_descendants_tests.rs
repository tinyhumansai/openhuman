//! End to end: which descendants of a scheduled-actions cron run may act.
//!
//! The job names the orchestrator, whose definition here opts into
//! `scheduled_actions`, so the root runs as `TrustedAutomation { CronActions }`.
//! Its children call `delete_artifact` on an artifact that does not exist: a
//! child the gate lets through reaches the tool and gets its "not found" error,
//! a child held read-only gets the cron denial instead.

use super::headless_descendants_tests::{
    dispatch_scheduled_agent, list_runs_off_runtime, scheduled_agent_fixture_with,
};
use super::*;
use async_trait::async_trait;
use tinyagents_definition::{SandboxMode, SubagentEntry, ToolScope};
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};

const CHILD_PROMPT: &str = "scheduled-actions-child-marker";
const GRANDCHILD_PROMPT: &str = "scheduled-actions-grandchild-marker";
const ACTING_TOOL: &str = "delete_artifact";
const CRON_DENIAL: &str = "Scheduled agent turns may use read-only tools only";

/// Scripts a three-level tree: the root spawns `child_agent`; the child either
/// calls the acting tool or spawns `grandchild_agent`, which calls it.
struct ActionTreeModel {
    child_agent: &'static str,
    grandchild_agent: Option<&'static str>,
    root_calls: std::sync::atomic::AtomicUsize,
    child_calls: std::sync::atomic::AtomicUsize,
    grandchild_calls: std::sync::atomic::AtomicUsize,
}

impl ActionTreeModel {
    fn new(child_agent: &'static str, grandchild_agent: Option<&'static str>) -> Self {
        Self {
            child_agent,
            grandchild_agent,
            root_calls: Default::default(),
            child_calls: Default::default(),
            grandchild_calls: Default::default(),
        }
    }
}

fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ModelResponse {
    let mut response = ModelResponse::assistant("");
    response.message.tool_calls = vec![tinyinference_llm::tool::ToolCall::new(id, name, arguments)];
    response.finish_reason = Some("tool_calls".into());
    response
}

fn spawn(agent_id: &str, prompt: &str, task_key: &str) -> ModelResponse {
    tool_call(
        &format!("spawn-{task_key}"),
        "spawn_subagent",
        serde_json::json!({
            "agent_id": agent_id,
            "prompt": prompt,
            "task_key": task_key,
            "context": "scheduled actions regression",
            "model": "test-model"
        }),
    )
}

fn acting_call(id: &str) -> ModelResponse {
    tool_call(
        id,
        ACTING_TOOL,
        serde_json::json!({"artifact_id": "missing-scheduled-actions-artifact"}),
    )
}

fn user_text_contains(request: &ModelRequest, marker: &str) -> bool {
    request.messages.iter().any(|message| {
        matches!(message, tinyinference_llm::message::Message::User(_))
            && message.text().contains(marker)
    })
}

#[async_trait]
impl ChatModel<()> for ActionTreeModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
        Some(PROFILE.get_or_init(|| ModelProfile {
            tool_calling: true,
            parallel_tool_calls: true,
            ..ModelProfile::default()
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        use std::sync::atomic::Ordering::SeqCst;
        if user_text_contains(&request, GRANDCHILD_PROMPT) {
            return Ok(match self.grandchild_calls.fetch_add(1, SeqCst) {
                0 => acting_call("scheduled-actions-grandchild-act"),
                _ => ModelResponse::assistant("scheduled-actions-grandchild-done"),
            });
        }
        if user_text_contains(&request, CHILD_PROMPT) {
            return Ok(match (self.child_calls.fetch_add(1, SeqCst), self.grandchild_agent) {
                (0, Some(grandchild)) => spawn(grandchild, GRANDCHILD_PROMPT, "grandchild"),
                (0, None) => acting_call("scheduled-actions-child-act"),
                _ => ModelResponse::assistant("scheduled-actions-child-done"),
            });
        }
        Ok(match self.root_calls.fetch_add(1, SeqCst) {
            0 => spawn(self.child_agent, CHILD_PROMPT, "child"),
            _ => ModelResponse::assistant("scheduled-actions-root-done"),
        })
    }
}

/// The builtin registry with the orchestrator opted into scheduled actions
/// (it already names its tools and is sandboxed) and `customize` applied.
fn scheduled_actions_registry(
    customize: impl FnOnce(&mut crate::agent_registry::host_definition::AgentDefinitionRegistry),
) -> crate::agent_registry::host_definition::AgentDefinitionRegistry {
    let mut registry =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only();
    let mut root = registry.get("orchestrator").expect("orchestrator").clone();
    root.extensions.scheduled_actions = true;
    assert!(matches!(root.tools, ToolScope::Named(_)));
    registry.insert(root);
    customize(&mut registry);
    registry
}

/// `task_manager_agent` (declares `delete_artifact`) opted in and sandboxed.
fn opt_in_task_manager(
    registry: &mut crate::agent_registry::host_definition::AgentDefinitionRegistry,
) {
    let mut child = registry
        .get("task_manager_agent")
        .expect("task_manager_agent")
        .clone();
    child.extensions.scheduled_actions = true;
    child.sandbox_mode = SandboxMode::Sandboxed;
    registry.insert(child);
}

/// Runs the job to completion and returns the `ToolCompleted` error each
/// descendant agent recorded for the acting tool, keyed by agent id.
async fn run_and_collect_acting_tool_errors(
    model: std::sync::Arc<ActionTreeModel>,
    registry: crate::agent_registry::host_definition::AgentDefinitionRegistry,
) -> std::collections::BTreeMap<String, String> {
    let _model =
        crate::inference::provider::factory::test_provider_override::install_model(model.clone());
    let (_temp, _storage, _storage_guard, config, core, dispatch, job) =
        scheduled_agent_fixture_with(900, Some("orchestrator"), registry);
    let inspect_dispatch = dispatch.clone();
    let scheduler_config = config.clone();
    let run =
        tokio::spawn(
            async move { dispatch_scheduled_agent(core, dispatch, scheduler_config).await },
        );
    match tokio::time::timeout(std::time::Duration::from_secs(90), run).await {
        Ok(joined) => joined.expect("cron scheduler task"),
        Err(_) => panic!(
            "scheduled run did not drain (root calls {}, child calls {}, grandchild calls {})",
            model.root_calls.load(std::sync::atomic::Ordering::SeqCst),
            model.child_calls.load(std::sync::atomic::Ordering::SeqCst),
            model.grandchild_calls.load(std::sync::atomic::Ordering::SeqCst),
        ),
    }
    let runs = list_runs_off_runtime(&config, &job.id)
        .await
        .expect("read persisted run history");
    assert_eq!(runs.len(), 1, "scheduler persisted one run");

    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &config.workspace_dir,
        &format!("cron:{}", job.id),
        None,
    );
    let jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&inspect_dispatch)
    .try_list_owned(owner)
    .expect("inspect the Cron session's descendants");
    let mut errors = std::collections::BTreeMap::new();
    for job in jobs {
        let Some(run_id) = job.subagent_run_id.as_deref() else {
            continue;
        };
        let events = crate::agent::tinyagents::journal::read_run_events(&inspect_dispatch, run_id, 0)
            .await
            .expect("read a descendant's durable tool events");
        if let Some(error) = events.iter().find_map(|observation| match &observation.event {
            tinyagents_harness::events::AgentEvent::ToolCompleted {
                tool_name,
                error: Some(error),
                ..
            } if tool_name == ACTING_TOOL => Some(error.clone()),
            _ => None,
        }) {
            errors.insert(job.agent.clone(), error);
        }
    }
    errors
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_actions_child_that_opts_in_and_is_sandboxed_runs_its_acting_tool() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let model = std::sync::Arc::new(ActionTreeModel::new("task_manager_agent", None));
    let errors = run_and_collect_acting_tool_errors(
        model.clone(),
        scheduled_actions_registry(opt_in_task_manager),
    )
    .await;
    assert_eq!(
        model.child_calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the child acted, then answered"
    );
    let error = errors
        .get("task_manager_agent")
        .expect("the child's journal records its delete_artifact call");
    assert!(
        !error.contains(CRON_DENIAL),
        "the granted child passed the cron stage: {error}"
    );
    assert!(
        error.contains("not found"),
        "the tool itself ran and found no such artifact: {error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_actions_child_that_does_not_opt_in_stays_read_only() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let model = std::sync::Arc::new(ActionTreeModel::new("task_manager_agent", None));
    // Sandboxed, but its definition does not set `scheduled_actions`.
    let errors = run_and_collect_acting_tool_errors(
        model,
        scheduled_actions_registry(|registry| {
            opt_in_task_manager(registry);
            let mut child = registry.get("task_manager_agent").unwrap().clone();
            child.extensions.scheduled_actions = false;
            registry.insert(child);
        }),
    )
    .await;
    let error = errors
        .get("task_manager_agent")
        .expect("the child's journal records its delete_artifact call");
    assert!(error.contains(CRON_DENIAL), "{error}");
}

const RELAY_AGENT: &str = "scheduled_actions_relay";
const LEAF_AGENT: &str = "scheduled_actions_leaf";

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_actions_grandchild_stays_read_only_even_when_it_opts_in() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let model = std::sync::Arc::new(ActionTreeModel::new(RELAY_AGENT, Some(LEAF_AGENT)));
    let errors = run_and_collect_acting_tool_errors(
        model.clone(),
        scheduled_actions_registry(|registry| {
            let mut root = registry.get("orchestrator").unwrap().clone();
            root.subagents
                .push(SubagentEntry::AgentId(RELAY_AGENT.into()));
            registry.insert(root);
            // An opted-in, sandboxed direct child that delegates to the leaf.
            let mut relay = registry.get("task_manager_agent").unwrap().clone();
            relay.id = RELAY_AGENT.into();
            relay.agent_tier = tinyagents_definition::AgentTier::Reasoning;
            relay.tools = ToolScope::Named(vec!["spawn_subagent".into(), ACTING_TOOL.into()]);
            relay.subagents = vec![SubagentEntry::AgentId(LEAF_AGENT.into())];
            relay.extensions.scheduled_actions = true;
            relay.sandbox_mode = SandboxMode::Sandboxed;
            registry.insert(relay);
            // The leaf opts in and is sandboxed exactly like a granted child.
            let mut leaf = registry.get("task_manager_agent").unwrap().clone();
            leaf.id = LEAF_AGENT.into();
            leaf.extensions.scheduled_actions = true;
            leaf.sandbox_mode = SandboxMode::Sandboxed;
            registry.insert(leaf);
        }),
    )
    .await;
    assert_eq!(
        model
            .grandchild_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the grandchild ran and tried to act"
    );
    let error = errors
        .get(LEAF_AGENT)
        .expect("the grandchild's journal records its delete_artifact call");
    assert!(error.contains(CRON_DENIAL), "{error}");
}
