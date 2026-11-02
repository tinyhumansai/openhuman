use super::*;

use tinyagents_harness::store::InMemoryStore;

#[test]
fn only_root_orchestrator_turns_get_the_block() {
    assert!(applies(false, Some("orchestrator")));
    assert!(!applies(true, Some("orchestrator")), "a sub-agent run");
    assert!(!applies(false, Some("planner")));
    assert!(!applies(false, None));
}

#[test]
fn install_needs_the_scope_and_a_store() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
    let installed = |store: Option<Arc<dyn Store>>, is_subagent: bool, agent: Option<&str>| {
        let mut harness = AgentHarness::<(), ()>::new();
        let before = harness.middleware().len();
        install(
            &mut harness,
            store.map(|store| (store, "standalone".to_string())),
            is_subagent,
            agent,
            false,
        );
        harness.middleware().len() - before
    };
    assert_eq!(
        installed(Some(store.clone()), false, Some("orchestrator")),
        1
    );
    assert_eq!(installed(None, false, Some("orchestrator")), 0);
    assert_eq!(
        installed(Some(store.clone()), true, Some("orchestrator")),
        0
    );
    assert_eq!(installed(Some(store), false, Some("planner")), 0);
}

/// A turn that suppresses the thread goal (`suppress_active_goal`) still gets
/// the block, without the goal: the plan shows and the goal does not.
#[tokio::test]
async fn a_turn_that_suppresses_the_goal_sees_the_plan_but_not_the_goal() {
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_harness::middleware::Middleware;
    use tinyagents_harness_tools::goals::store as goal_store;
    use tinyagents_harness_tools::todos::{store as todo_store, TodoItem};
    use tinyinference_llm::message::Message;
    use tinyinference_llm::model::ModelRequest;

    let store: Arc<dyn Store> = Arc::new(InMemoryStore::default());
    goal_store::set(&store, "t", "Migrate the billing tables", None)
        .await
        .unwrap();
    todo_store::replace(&store, "t", vec![TodoItem::new("Dump the schema")])
        .await
        .unwrap();
    let block = |goal_suppressed: bool| {
        let block_mw = middleware(store.clone(), "standalone".to_string(), goal_suppressed);
        async move {
            let mut ctx = RunContext::new(RunConfig::new("state-block").with_thread("t"), ());
            let mut request = ModelRequest {
                messages: vec![Message::system("persona"), Message::user("hey")],
                ..Default::default()
            };
            Middleware::<(), ()>::before_model_dispatch(&block_mw, &mut ctx, &(), &mut request)
                .await
                .unwrap();
            request
                .messages
                .iter()
                .map(Message::text)
                .collect::<Vec<_>>()
                .join("\n")
        }
    };

    let shown = block(false).await;
    assert!(shown.contains("Migrate the billing tables"), "{shown}");
    assert!(shown.contains("Dump the schema"), "{shown}");

    let suppressed = block(true).await;
    assert!(
        !suppressed.contains("Migrate the billing tables"),
        "{suppressed}"
    );
    assert!(suppressed.contains("Dump the schema"), "{suppressed}");

    // And the turn still installs it.
    let mut harness = AgentHarness::<(), ()>::new();
    let before = harness.middleware().len();
    install(
        &mut harness,
        Some((store, "standalone".to_string())),
        false,
        Some("orchestrator"),
        true,
    );
    assert_eq!(harness.middleware().len() - before, 1);
}

#[tokio::test]
async fn a_finished_plan_hands_its_notes_over_at_the_end_of_the_turn() {
    use tinyagents_harness_tools::scratchpad::ScratchpadUpdate;
    use tinyagents_harness_tools::todos::{store as todo_store, TodoItem};

    let workspace = tempfile::tempdir().unwrap();
    let store = crate::storage::sessions::stores_or_ephemeral(workspace.path()).kv;
    todo_store::replace(&store, "t", vec![TodoItem::new("Write the migration")])
        .await
        .unwrap();
    scratchpad_store::apply(
        &store,
        "t",
        &ScratchpadUpdate {
            facts: vec!["the table is auth_users".to_string()],
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let dispatch = crate::core::runtime::DispatchCtx::ambient();
    assert!(close_finished_plan(workspace.path(), None, &dispatch)
        .await
        .is_none());
    assert!(
        close_finished_plan(workspace.path(), Some("t"), &dispatch)
            .await
            .is_none(),
        "the plan is still open"
    );
    todo_store::complete(&store, "t", 1).await.unwrap();
    let finished = close_finished_plan(workspace.path(), Some("t"), &dispatch)
        .await
        .expect("the plan finished with notes");
    assert_eq!(finished.plan.notes.facts, ["the table is auth_users"]);
    assert_eq!(finished.goal, None, "the thread has no goal");
    assert!(
        close_finished_plan(workspace.path(), Some("t"), &dispatch)
            .await
            .is_none(),
        "handed over once"
    );
}
