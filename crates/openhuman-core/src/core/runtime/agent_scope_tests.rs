use super::*;
use crate::core::runtime::{ContextOverlay, DispatchCtx, DomainSet};
use crate::security::{AutonomyLevel, SecurityPolicy};
use crate::tools::toolpacks::ToolGroups;

#[derive(Default)]
struct Counter(Mutex<u32>);

fn root(dir: &str) -> Arc<CoreContext> {
    CoreContext::for_test(DomainSet::full(), Some(PathBuf::from(dir)))
}

fn overlay(workspace: &str) -> ContextOverlay {
    let mut config = crate::config::Config::default();
    config.workspace_dir = PathBuf::from(workspace);
    ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none())
}

fn agent(parent: &Arc<CoreContext>, id: &str) -> Arc<CoreContext> {
    parent.derive_with(overlay("/tmp/agent-scope-ws").session_agent(id))
}

fn bump(ctx: &CoreContext) -> u32 {
    let counter = ctx.agent_state().slot::<Counter>();
    let mut value = counter.0.lock().unwrap();
    *value += 1;
    *value
}

#[test]
fn every_agent_context_owns_its_slots() {
    let parent = root("/tmp/agent-scope-root");
    let alpha = agent(&parent, "alpha");
    let beta = agent(&parent, "beta");

    assert_eq!(bump(&alpha), 1);
    assert_eq!(bump(&alpha), 2);
    assert_eq!(bump(&beta), 1, "beta must not see alpha's slot");
    assert_eq!(bump(&parent), 1, "the root context keeps its own slot");
    assert_eq!(alpha.agent_state().len(), 1);
}

#[test]
fn autonomy_tier_source_and_ceiling_survive_derived_contexts() {
    let parent = root("/tmp/autonomy-source-parent");
    let live_child = parent.derive_with(overlay("/tmp/autonomy-source-live-child"));
    assert_eq!(
        live_child.autonomy_tier_source(),
        crate::core::runtime::AutonomyTierSource::LiveConfig,
        "an overlay without a source override inherits the live parent binding"
    );

    let fixed = parent.derive_with(
        overlay("/tmp/autonomy-source-fixed")
            .captured_autonomy_tier()
            .read_only_autonomy_ceiling(),
    );

    let child = fixed.derive_with(overlay("/tmp/autonomy-source-child"));
    assert_eq!(
        child.autonomy_tier_source(),
        crate::core::runtime::AutonomyTierSource::Captured
    );
    assert_eq!(child.autonomy_tier_ceiling(), Some(AutonomyLevel::ReadOnly));

    let mut source_override = overlay("/tmp/autonomy-source-override");
    source_override.autonomy_tier_source =
        Some(crate::core::runtime::AutonomyTierSource::LiveConfig);
    let overridden = fixed.derive_with(source_override);
    assert_eq!(
        overridden.autonomy_tier_source(),
        crate::core::runtime::AutonomyTierSource::LiveConfig
    );
    assert_eq!(
        overridden.autonomy_tier_ceiling(),
        Some(AutonomyLevel::ReadOnly),
        "changing the tier source cannot clear a fixed host ceiling"
    );
}

#[test]
fn contexts_derived_without_an_agent_share_the_parent_slots() {
    let parent = root("/tmp/agent-scope-shared");
    let alpha = agent(&parent, "alpha");
    let turn = alpha.derive_with(overlay("/tmp/agent-scope-ws"));

    assert_eq!(bump(&alpha), 1);
    assert_eq!(bump(&turn), 2, "a turn context keeps its agent's slots");
    assert_eq!(turn.session_agent(), Some("alpha"));
}

#[test]
fn clear_drops_every_slot() {
    let state = AgentScopedState::default();
    assert!(state.is_empty());
    *state.slot::<Counter>().0.lock().unwrap() = 7;
    state.clear();
    assert!(state.is_empty());
    assert_eq!(*state.slot::<Counter>().0.lock().unwrap(), 0);
}

#[test]
fn derive_with_carries_policy_catalogue_and_approval_flag() {
    let parent = root("/tmp/agent-scope-policy");
    let policy = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let definitions =
        Arc::new(crate::agent_registry::host_definition::AgentDefinitionRegistry::default());
    let mut spec = overlay("/tmp/agent-scope-ws")
        .session_agent("alpha")
        .agent_policy(Arc::clone(&policy))
        .definitions(Arc::clone(&definitions));
    spec.approvals_disabled = true;
    let alpha = parent.derive_with(spec);
    let turn = alpha.derive_with(overlay("/tmp/agent-scope-ws"));

    for ctx in [&alpha, &turn] {
        assert!(Arc::ptr_eq(&ctx.agent_policy().unwrap(), &policy));
        assert!(Arc::ptr_eq(&ctx.definitions().unwrap(), &definitions));
        assert!(ctx.approvals_disabled());
    }
    assert!(parent.agent_policy().is_none());
    assert!(parent.definitions().is_none());
    assert!(!parent.approvals_disabled());
}

#[test]
fn registry_resolves_live_agents_only() {
    let parent = root("/tmp/agent-scope-registry");
    let alpha = agent(&parent, "registry-alpha");
    let beta = agent(&parent, "registry-beta");
    AgentContextRegistry::register("registry-alpha", &alpha);
    AgentContextRegistry::register("registry-beta", &beta);

    assert!(Arc::ptr_eq(
        &AgentContextRegistry::get("registry-alpha").unwrap(),
        &alpha
    ));
    let live: Vec<String> = AgentContextRegistry::live()
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| id.starts_with("registry-"))
        .collect();
    assert_eq!(live, ["registry-alpha", "registry-beta"]);

    drop(beta);
    assert!(AgentContextRegistry::get("registry-beta").is_none());

    let impostor = agent(&parent, "registry-alpha");
    assert!(!AgentContextRegistry::deregister(
        "registry-alpha",
        &impostor
    ));
    assert!(AgentContextRegistry::get("registry-alpha").is_some());
    assert!(AgentContextRegistry::deregister("registry-alpha", &alpha));
    assert!(AgentContextRegistry::get("registry-alpha").is_none());
}

#[tokio::test]
async fn scope_dir_and_agent_id_follow_the_ambient_context() {
    let mut config = crate::config::Config::default();
    config.workspace_dir = PathBuf::from("/tmp/agent-scope-dir");
    let parent = root("/tmp/agent-scope-dir");
    let alpha = agent(&parent, "alpha");

    let (dir, id) = CoreContext::scope(alpha, async {
        (agent_scope_dir(&config), current_agent_id())
    })
    .await;
    assert_eq!(dir, PathBuf::from("/tmp/agent-scope-dir/agents/alpha"));
    assert_eq!(id.as_deref(), Some("alpha"));

    let (dir, id) = CoreContext::scope(parent, async {
        (agent_scope_dir(&config), current_agent_id())
    })
    .await;
    assert_eq!(dir, PathBuf::from("/tmp/agent-scope-dir"));
    assert!(id.is_none());
}

#[tokio::test]
async fn a_context_spawn_keeps_the_agent_context_and_tokio_spawn_does_not() {
    let parent = root("/tmp/agent-scope-spawn");
    let policy = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let alpha = parent.derive_with(
        overlay("/tmp/agent-scope-ws")
            .session_agent("alpha")
            .agent_policy(policy),
    );

    let (scoped, bare) = CoreContext::scope(alpha, async {
        let scoped = DispatchCtx::ambient()
            .spawn(async {
                (
                    current_agent_id(),
                    DispatchCtx::ambient().agent_policy().map(|p| p.autonomy),
                )
            })
            .await
            .unwrap();
        let bare = tokio::spawn(async { CoreContext::scoped().is_some() })
            .await
            .unwrap();
        (scoped, bare)
    })
    .await;

    assert_eq!(scoped.0.as_deref(), Some("alpha"));
    assert_eq!(scoped.1, Some(AutonomyLevel::ReadOnly));
    assert!(!bare, "a bare tokio::spawn loses the task-local context");
}

#[test]
fn slot_without_a_context_uses_the_unscoped_state() {
    let first = slot_for::<Counter>(&crate::core::runtime::DispatchCtx::ambient());
    let second = slot_for::<Counter>(&crate::core::runtime::DispatchCtx::ambient());
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn an_unscoped_saas_task_never_meets_shared_slots() {
    let first = slot_in::<Counter>(true, None);
    *first.0.lock().unwrap() = 5;
    let second = slot_in::<Counter>(true, None);
    assert_eq!(*second.0.lock().unwrap(), 0, "a throwaway slot each time");
    assert!(!Arc::ptr_eq(&first, &second));
    // Single-user processes keep the one shared unscoped slot.
    assert!(Arc::ptr_eq(
        &slot_in::<Counter>(false, None),
        &slot_in::<Counter>(false, None)
    ));
    let ctx = root("/tmp/x");
    assert!(Arc::ptr_eq(
        &slot_in::<Counter>(true, Some(&ctx)),
        &ctx.agent_state().slot::<Counter>()
    ));
}

#[tokio::test]
async fn an_explicit_dispatch_context_wins_over_the_ambient_one() {
    let parent = root("/tmp/agent-scope-explicit");
    let alpha = agent(&parent, "alpha");
    let beta = agent(&parent, "beta");

    // The ambient scope is beta's; the explicit context is alpha's, and the
    // slot must come from alpha's.
    let (explicit, ambient) = CoreContext::scope(Arc::clone(&beta), async {
        (
            slot_for::<Counter>(&DispatchCtx::new(Arc::clone(&alpha))),
            slot_for::<Counter>(&DispatchCtx::ambient()),
        )
    })
    .await;

    assert!(Arc::ptr_eq(
        &explicit,
        &alpha.agent_state().slot::<Counter>()
    ));
    assert!(Arc::ptr_eq(&ambient, &beta.agent_state().slot::<Counter>()));
    assert!(!Arc::ptr_eq(&explicit, &ambient));
}

#[tokio::test]
async fn explicit_contexts_keep_their_slots_across_awaits_and_spawned_tasks() {
    let parent = root("/tmp/agent-scope-concurrent");
    let alpha = DispatchCtx::new(agent(&parent, "alpha"));
    let beta = DispatchCtx::new(agent(&parent, "beta"));

    let work = |ctx: DispatchCtx, times: u32| async move {
        for _ in 0..times {
            tokio::task::yield_now().await;
            *slot_for::<Counter>(&ctx).0.lock().unwrap() += 1;
        }
        // A spawned task with no ambient scope still reaches the same slot
        // because the context travels as an argument.
        tokio::spawn(async move { *slot_for::<Counter>(&ctx).0.lock().unwrap() })
            .await
            .unwrap()
    };
    let (a, b) = tokio::join!(
        tokio::spawn(work(alpha.clone(), 3)),
        tokio::spawn(work(beta.clone(), 5))
    );

    assert_eq!(a.unwrap(), 3, "alpha counts only its own work");
    assert_eq!(b.unwrap(), 5, "beta counts only its own work");
}
