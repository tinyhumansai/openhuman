//! The turn start and the thread goal: a paused goal stays paused, and a turn
//! that gets the run-state block does not also carry the goal in its message.

use super::*;

use tinyagents_harness_tools::goals::store as goal_store;
use tinyagents_harness_tools::goals::ThreadGoalStatus;

const THREAD: &str = "goal-thread";

/// A session for `agent_id` bound to [`THREAD`], with its runtime session up.
async fn host_for(agent_id: &str) -> (crate::agent::OpenHumanSessionHost, tempfile::TempDir) {
    let _ = crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins();
    crate::agent::session_host::test_support::install_model_fixture();
    let tmp = tempfile::TempDir::new().unwrap();
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        default_model: Some("test/session-fixture".to_string()),
        ..crate::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    // Building the orchestrator's model passes the backend-session gate; a
    // standing API key in this session's own state dir satisfies it, and the
    // model fixture above answers instead of a real provider.
    crate::credentials::api_key::store_api_key(&config, "test-goal-key")
        .await
        .expect("test api key");
    let definition = crate::agent_registry::agents::load_builtins()
        .unwrap()
        .into_iter()
        .find(|def| def.id == agent_id)
        .expect("builtin definition");
    let mut host = OpenHumanSessionHost::from_config_with_definition(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &definition,
    )
    .await
    .expect("session builds");
    host.set_thread_id(Some(THREAD));
    host.ensure_runtime_session().expect("runtime session");
    (host, tmp)
}

fn store(tmp: &tempfile::TempDir) -> Arc<dyn tinyagents_harness::store::Store> {
    crate::storage::sessions::stores_or_ephemeral(&tmp.path().join("workspace")).kv
}

/// The user message the turn start builds for `host`.
async fn enriched(host: &crate::agent::OpenHumanSessionHost) -> String {
    let prelude = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .prelude
        .clone()
        .expect("prelude");
    let mut run_context = OpenHumanRunContext::new();
    prelude
        .enrich_request(
            "what's the weather?",
            &crate::agent::session_host::types::TurnInputs::default(),
            &mut run_context,
        )
        .await
}

#[tokio::test]
async fn a_paused_goal_is_not_resumed_by_a_new_message() {
    let (host, tmp) = host_for("orchestrator").await;
    let store = store(&tmp);
    goal_store::set(&store, THREAD, "Migrate the auth schema", None)
        .await
        .unwrap();
    goal_store::pause(&store, THREAD).await.unwrap();

    let message = enriched(&host).await;

    assert!(!message.contains("Migrate the auth schema"), "{message}");
    let goal = goal_store::get(&store, THREAD).await.unwrap().unwrap();
    assert_eq!(goal.status, ThreadGoalStatus::Paused);
}

#[tokio::test]
async fn an_orchestrator_turn_leaves_the_goal_to_the_state_block() {
    let (host, tmp) = host_for("orchestrator").await;
    goal_store::set(&store(&tmp), THREAD, "Migrate the auth schema", None)
        .await
        .unwrap();
    let message = enriched(&host).await;
    assert!(
        !message.contains("Migrate the auth schema"),
        "the run-state block shows it: {message}"
    );
}

#[tokio::test]
async fn a_turn_without_the_state_block_still_carries_the_active_goal() {
    let (host, tmp) = host_for("summarizer").await;
    goal_store::set(&store(&tmp), THREAD, "Migrate the auth schema", None)
        .await
        .unwrap();
    let message = enriched(&host).await;
    assert!(
        message.contains("[thread goal]") && message.contains("Migrate the auth schema"),
        "{message}"
    );
}

/// A turn that suppresses the thread goal tells the run context, so the
/// run-state block (which reads the goal from the store) leaves the goal out.
#[tokio::test]
async fn a_suppressed_goal_reaches_the_run_context() {
    let (host, tmp) = host_for("orchestrator").await;
    goal_store::set(&store(&tmp), THREAD, "Migrate the auth schema", None)
        .await
        .unwrap();
    let prelude = host
        .runtime_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .prelude
        .clone()
        .expect("prelude");
    for suppress in [false, true] {
        let inputs = crate::agent::session_host::types::TurnInputs {
            overrides: crate::agent::session_host::types::TurnOverrides {
                suppress_active_goal: suppress,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut run_context = OpenHumanRunContext::new();
        let message = prelude
            .enrich_request("hey", &inputs, &mut run_context)
            .await;
        assert_eq!(run_context.goal_suppressed, suppress);
        assert!(!message.contains("Migrate the auth schema"), "{message}");
    }
}
