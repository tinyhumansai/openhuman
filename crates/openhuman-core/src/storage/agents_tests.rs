use super::*;
use crate::core::runtime::{ContextOverlay, DomainSet};

/// A context derived for `agent` and registered as that agent's live one,
/// the way an embed agent is. Each test uses its own agent ids, so tests
/// sharing the process-wide registry never see each other's agents.
fn live_agent(agent: &str) -> Arc<CoreContext> {
    let context = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        )
        .session_agent(agent),
    );
    AgentContextRegistry::register(agent, &context);
    context
}

/// Serializes the tests that depend on, or clear, the process-wide record
/// cache.
static CACHE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn memory_backend() -> Arc<dyn StorageBackend> {
    Arc::new(crate::storage::MemoryStorage::new())
}

fn visited(
    backend: Option<Arc<dyn StorageBackend>>,
    fallback: Option<&Arc<CoreContext>>,
) -> Vec<String> {
    contexts_in(backend, fallback)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

#[test]
fn a_registered_agent_is_visited_through_its_live_context() {
    let context = live_agent("agents-test-live");
    let (_, found) = contexts_in(None, None)
        .into_iter()
        .find(|(id, _)| id == "agents-test-live")
        .expect("a live agent is visited");
    assert!(Arc::ptr_eq(&found, &context), "the live context is used");
    assert!(Arc::ptr_eq(
        &context_for("agents-test-live").unwrap(),
        &context
    ));
    assert!(AgentContextRegistry::deregister(
        "agents-test-live",
        &context
    ));
    drop((found, context));
    assert!(!visited(None, None).contains(&"agents-test-live".to_string()));
}

#[test]
fn for_agent_swaps_only_the_agent() {
    let parent = live_agent("agents-test-parent");
    let child = parent.for_agent("agents-test-other");
    assert_eq!(child.session_agent(), Some("agents-test-other"));
    assert_eq!(parent.session_agent(), Some("agents-test-parent"));
    AgentContextRegistry::deregister("agents-test-parent", &parent);
}

#[test]
fn a_recorded_agent_is_visited_through_the_fallback_context() {
    let backend = memory_backend();
    record_in(Arc::clone(&backend), "agents-test-recorded-only");
    assert!(recorded(Arc::clone(&backend)).contains(&"agents-test-recorded-only".to_string()));

    let fallback = CoreContext::for_test(DomainSet::full(), None);
    let found = contexts_in(Some(backend), Some(&fallback));
    let (_, context) = found
        .iter()
        .find(|(id, _)| id == "agents-test-recorded-only")
        .expect("the recorded agent is visited");
    assert_eq!(context.session_agent(), Some("agents-test-recorded-only"));
}

#[test]
fn without_a_fallback_recorded_agents_are_not_visited() {
    let backend = memory_backend();
    record_in(Arc::clone(&backend), "agents-test-no-fallback");
    assert!(!visited(Some(backend), None).contains(&"agents-test-no-fallback".to_string()));
}

#[test]
fn an_agent_is_recorded_once_per_backend() {
    let _serial = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let first = memory_backend();
    record_in(Arc::clone(&first), "agents-test-once");
    // Removing the record behind the cache's back shows the second call is
    // skipped: the cache says this backend already has it.
    let docs = Arc::clone(first.for_scope(&Scope::local()).unwrap().documents());
    block_on(async move {
        docs.delete(AGENTS, "agents-test-once", Precondition::None)
            .await
            .map(|_| ())
    })
    .unwrap();
    record_in(Arc::clone(&first), "agents-test-once");
    assert!(recorded(Arc::clone(&first)).is_empty());
    // Another backend is a separate record.
    // `first` stays alive so `second` cannot reuse its address (the cache key).
    let second = memory_backend();
    record_in(Arc::clone(&second), "agents-test-once");
    assert_eq!(recorded(second), vec!["agents-test-once".to_string()]);
}

#[test]
fn reset_recorded_empties_the_cache() {
    let _serial = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let backend = memory_backend();
    record_in(Arc::clone(&backend), "agents-test-reset");
    reset_recorded();
    assert!(!RECORDED
        .lock()
        .unwrap()
        .contains(&(backend_key(&backend), "agents-test-reset".to_string())));
}

#[tokio::test]
async fn without_a_backend_only_the_local_scope_runs() {
    // The lib test binary never installs a backend into the process slot.
    if installed().is_some() {
        return;
    }
    let agent = live_agent("agents-test-unvisited");
    let runs = for_each_scope(
        &crate::core::runtime::DispatchCtx::host_default(),
        "test",
        |_| async { CoreContext::current().and_then(|c| c.session_agent().map(str::to_string)) },
    )
    .await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, None);
    assert!(for_each_agent("test", |_| async {}).await.is_empty());
    assert_eq!(
        find_owner(
            &crate::core::runtime::DispatchCtx::host_default(),
            "test",
            |_| async { Ok(false) }
        )
        .await,
        Ok(None),
        "nothing anywhere"
    );
    assert_eq!(
        find_owner(
            &crate::core::runtime::DispatchCtx::host_default(),
            "test",
            |_| async { Ok(true) }
        )
        .await,
        Ok(Some(None))
    );
    assert!(
        find_owner(
            &crate::core::runtime::DispatchCtx::host_default(),
            "test",
            |_| async { Err("down".to_string()) }
        )
        .await
        .is_err(),
        "a failed lookup with no match fails closed"
    );
    let inside = within_agent(
        &crate::core::runtime::DispatchCtx::host_default(),
        Some("agents-test-unvisited"),
        async { CoreContext::current().and_then(|c| c.session_agent().map(str::to_string)) },
    )
    .await;
    assert_eq!(inside.flatten().as_deref(), Some("agents-test-unvisited"));
    assert_eq!(
        within_agent(
            &crate::core::runtime::DispatchCtx::host_default(),
            None,
            async { 7 }
        )
        .await,
        Some(7)
    );
    AgentContextRegistry::deregister("agents-test-unvisited", &agent);
}

#[tokio::test]
async fn without_a_backend_the_live_pass_runs_only_local() {
    if installed().is_some() {
        return;
    }
    let agent = live_agent("agents-test-live-pass");
    let runs = for_each_live_scope(
        &crate::core::runtime::DispatchCtx::host_default(),
        "test",
        |_| async { 1 },
    )
    .await;
    assert_eq!(runs, vec![(None, 1)]);
    AgentContextRegistry::deregister("agents-test-live-pass", &agent);
}

#[test]
fn the_first_scope_reporting_the_record_owns_it() {
    let found = decide(vec![
        (None, Ok(false)),
        (Some("a".into()), Err("down".into())),
        (Some("b".into()), Ok(true)),
    ]);
    assert_eq!(found, Ok(Some(Some("b".to_string()))));
    let failed = decide(vec![
        (None, Ok(false)),
        (Some("a".into()), Err("down".into())),
    ]);
    let failed = failed.unwrap_err();
    assert_eq!(failed.agent.as_deref(), Some("a"));
    assert_eq!(failed.to_string(), "lookup failed in scope a: down");
    assert_eq!(decide(Vec::new()), Ok(None));
}

#[tokio::test]
async fn the_local_pass_runs_outside_the_callers_agent() {
    if installed().is_some() {
        return;
    }
    let agent = live_agent("agents-test-caller");
    let seen = CoreContext::scope(Arc::clone(&agent), async {
        for_each_scope(
            &crate::core::runtime::DispatchCtx::host_default(),
            "test",
            |_| async {
                CoreContext::current().and_then(|c| c.session_agent().map(str::to_string))
            },
        )
        .await
    })
    .await;
    // Without a default context the caller's own context stays; with one,
    // the local pass leaves the agent.
    if CoreContext::default_context().is_some() {
        assert_eq!(seen, vec![(None, None)]);
    }
    AgentContextRegistry::deregister("agents-test-caller", &agent);
}
