use super::*;
use std::path::PathBuf;

fn ctx(dir: &str) -> Arc<CoreContext> {
    Arc::new(CoreContext {
        host_kind: HostKind::Cli,
        workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
            workspace_dir: Some(PathBuf::from(dir)),
        }))),
        domains: crate::core::runtime::DomainSet::full(),
        tool_groups: Default::default(),
        embedder_config: None,
        user_skill_roots: true,
        backend_transport: None,
    })
}

// The ambient-scope primitive is the mechanism Phase 3 multi-tenant
// isolation is built on: a dispatch scoped to context A must see A's state,
// not the process default or another tenant's. These assert the primitive
// directly (independent of the process DEFAULT_CONTEXT global, since
// `current()` inside a scope resolves the scoped value).

// ---- embedder-supplied config (the library-embedding seam) ---------------
//
// `CoreBuilder::config(..)` is only half of the story, and the half that is
// easy to get wrong. Setting the config at boot does NOT reach RPC handlers:
// they call `load_config_with_timeout()` per dispatch, which re-runs
// `Config::load_or_init()` and re-resolves the process-global workspace. The
// context has to carry it, and the loader has to prefer it, or an embedder
// configures boot and watches its turns run somewhere else entirely.

fn ctx_with_config(config: crate::config::Config) -> Arc<CoreContext> {
    Arc::new(CoreContext {
        host_kind: HostKind::Cli,
        workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
            workspace_dir: Some(config.workspace_dir.clone()),
        }))),
        domains: crate::core::runtime::DomainSet::full(),
        tool_groups: Default::default(),
        embedder_config: Some(config),
        user_skill_roots: true,
        backend_transport: None,
    })
}

#[test]
fn a_context_without_an_embedder_config_reports_none() {
    // The default for every host that lets the core discover its own
    // config, which is all of them but a library embedder.
    assert!(ctx("/tmp/ws").embedder_config().is_none());
}

#[test]
fn an_embedder_config_is_readable_from_the_context() {
    let mut config = crate::config::Config::default();
    config.workspace_dir = PathBuf::from("/tmp/embedder-ws");
    config.default_model = Some("embedder-model".into());

    let ctx = ctx_with_config(config);
    let read = ctx.embedder_config().expect("supplied config is readable");
    assert_eq!(read.workspace_dir, PathBuf::from("/tmp/embedder-ws"));
    assert_eq!(read.default_model.as_deref(), Some("embedder-model"));
}

#[tokio::test]
async fn the_current_dispatch_sees_the_scoped_embedder_config() {
    // This is the read path `load_config_with_timeout` uses. If it resolved
    // to the process default instead of the scoped context, a second
    // embedder in the same process would silently serve the first's config.
    let mut config = crate::config::Config::default();
    config.workspace_dir = PathBuf::from("/tmp/scoped-ws");
    config.default_model = Some("scoped-model".into());

    let scoped = CoreContext::scope(ctx_with_config(config), async {
        CoreContext::current_embedder_config()
    })
    .await;

    let scoped = scoped.expect("a scoped embedder config is visible to the dispatch");
    assert_eq!(scoped.default_model.as_deref(), Some("scoped-model"));
    assert_eq!(scoped.workspace_dir, PathBuf::from("/tmp/scoped-ws"));
}

// ---- derived per-agent contexts (the multi-agent library seam) -----------
//
// `derive_with` is how one booted runtime hosts many independently configured
// agents: a child context carrying that agent's config, domain set, tool groups
// and skill-root policy, with no boot of its own. Everything a handler reads
// through `CoreContext::current()` must follow the child inside its scope.

#[test]
fn derive_with_keeps_the_host_and_overrides_the_per_agent_fields() {
    let parent = ctx("/tmp/parent-ws");
    let mut config = crate::config::Config::default();
    config.workspace_dir = PathBuf::from("/tmp/agent-ws");
    config.default_model = Some("agent-model".into());
    let overlay = ContextOverlay::new(
        config,
        crate::core::runtime::DomainSet::kernel(),
        crate::tools::toolpacks::ToolGroups::none(),
    )
    .without_user_skill_roots();

    let child = parent.derive_with(overlay);

    assert_eq!(child.host_kind(), parent.host_kind());
    assert_eq!(child.domains(), crate::core::runtime::DomainSet::kernel());
    assert_eq!(
        child.tool_groups(),
        crate::tools::toolpacks::ToolGroups::none()
    );
    assert!(!child.user_skill_roots());
    assert_eq!(
        child.workspace_dir().expect("child workspace"),
        PathBuf::from("/tmp/agent-ws")
    );
    assert_eq!(
        child
            .embedder_config()
            .and_then(|c| c.default_model.clone())
            .as_deref(),
        Some("agent-model")
    );
    // The parent is untouched: no boot ran, nothing was rebound.
    assert!(parent.embedder_config().is_none());
    assert!(parent.user_skill_roots());
}

/// Regression: `derive_with` must clamp the overlay's requested `DomainSet`
/// to what the parent context actually has, not adopt it verbatim. Without
/// the intersection, a runtime booted with a restricted `DomainSet` (say,
/// `kernel()`, which has `agent`/`memory`/`mcp` off) could still derive a
/// child scoped with `DomainSet::full()`, and every reader that dispatches
/// through that child (the config loader, the DomainSet gate, skill
/// discovery) would observe the wider set the runtime never registered.
#[test]
fn derive_with_clamps_overlay_domains_to_the_parent_registered_set() {
    let mut parent_ctx = ctx("/tmp/parent-ws");
    Arc::get_mut(&mut parent_ctx).expect("sole owner").domains =
        crate::core::runtime::DomainSet::kernel();
    assert!(
        !parent_ctx.domains().agent,
        "sanity: kernel() has agent off"
    );
    assert!(!parent_ctx.domains().mcp, "sanity: kernel() has mcp off");

    let overlay = ContextOverlay::new(
        crate::config::Config::default(),
        crate::core::runtime::DomainSet::full(),
        Default::default(),
    );
    let child = parent_ctx.derive_with(overlay);

    assert!(
        !child.domains().agent,
        "a disabled parent family must not reappear via an overlay: {:?}",
        child.domains()
    );
    assert!(!child.domains().mcp);
    // Families the parent *did* register, and the overlay also asked for,
    // still come through.
    assert!(child.domains().threads);
    assert!(child.domains().config);
}

#[test]
fn derive_with_defaults_to_visible_user_skill_roots() {
    let overlay = ContextOverlay::new(
        crate::config::Config::default(),
        crate::core::runtime::DomainSet::full(),
        Default::default(),
    );
    assert!(overlay.user_skill_roots);
    assert!(ctx("/tmp/ws").derive_with(overlay).user_skill_roots());
}

#[tokio::test]
async fn two_derived_contexts_serve_their_own_config_to_the_dispatch() {
    // The read path `load_config_with_timeout` uses. Two agents scoped one
    // after the other must each see their own overlay, never the sibling's.
    let parent = ctx("/tmp/parent-ws");
    let mut a = crate::config::Config::default();
    a.workspace_dir = PathBuf::from("/tmp/agent-a");
    a.default_model = Some("model-a".into());
    let mut b = crate::config::Config::default();
    b.workspace_dir = PathBuf::from("/tmp/agent-b");
    b.default_model = Some("model-b".into());
    let ctx_a = parent.derive_with(ContextOverlay::new(
        a,
        crate::core::runtime::DomainSet::embedded(),
        Default::default(),
    ));
    let ctx_b = parent.derive_with(
        ContextOverlay::new(
            b,
            crate::core::runtime::DomainSet::kernel(),
            Default::default(),
        )
        .without_user_skill_roots(),
    );

    let seen_a = CoreContext::scope(ctx_a, async {
        (
            CoreContext::current_embedder_config().and_then(|c| c.default_model),
            CoreContext::current().map(|c| c.domains()),
            CoreContext::current_user_skill_roots(),
        )
    })
    .await;
    let seen_b = CoreContext::scope(ctx_b, async {
        (
            CoreContext::current_embedder_config().and_then(|c| c.default_model),
            CoreContext::current().map(|c| c.domains()),
            CoreContext::current_user_skill_roots(),
        )
    })
    .await;

    assert_eq!(seen_a.0.as_deref(), Some("model-a"));
    assert_eq!(seen_a.1, Some(crate::core::runtime::DomainSet::embedded()));
    assert!(seen_a.2);
    assert_eq!(seen_b.0.as_deref(), Some("model-b"));
    assert_eq!(seen_b.1, Some(crate::core::runtime::DomainSet::kernel()));
    assert!(!seen_b.2);
}

// ---- store-init gating (#4796 DoD item 3) --------------------------------
// `init_stores` side-effects on process globals with no init-state probe, so
// the gating is proven via the pure `StoreInitPlan` the registrar consumes.

#[test]
fn store_init_plan_full_initializes_every_store() {
    let plan = StoreInitPlan::for_domains(crate::core::runtime::DomainSet::full());
    assert_eq!(
        plan,
        StoreInitPlan {
            memory: true,
            agent_attachments: true,
            skills_prune: true,
        },
        "full() must initialize every workspace-bound store"
    );
}

#[test]
fn store_init_plan_none_initializes_nothing() {
    let plan = StoreInitPlan::for_domains(crate::core::runtime::DomainSet::none());
    assert_eq!(
        plan,
        StoreInitPlan {
            memory: false,
            agent_attachments: false,
            skills_prune: false,
        },
        "none() must leave every workspace-bound store uninitialized"
    );
}

#[test]
fn store_init_plan_harness_gates_by_owning_group() {
    let plan = StoreInitPlan::for_domains(crate::core::runtime::DomainSet::harness());
    // harness() = agent + memory + threads + config + security.
    assert!(plan.memory, "harness keeps the memory binding (Memory)");
    assert!(
        plan.agent_attachments,
        "harness keeps agent attachments sidecar (Agent)"
    );
    // Skills is NOT in harness → its store work stays off.
    assert!(
        !plan.skills_prune,
        "harness must skip skills legacy-prune (Skills)"
    );
}

#[tokio::test]
async fn scope_sets_current_context() {
    let a = ctx("/tmp/ctx-a");
    let seen = CoreContext::scope(a, async {
        CoreContext::current().map(|c| c.workspace_dir().unwrap())
    })
    .await;
    assert_eq!(seen, Some(PathBuf::from("/tmp/ctx-a")));
}

#[tokio::test]
async fn propagate_carries_scoped_context_into_spawned_task() {
    let a = ctx("/tmp/ctx-propagated");
    let seen = CoreContext::scope(a, async {
        tokio::spawn(CoreContext::propagate(async {
            CoreContext::current().unwrap().workspace_dir().unwrap()
        }))
        .await
        .unwrap()
    })
    .await;
    assert_eq!(seen, PathBuf::from("/tmp/ctx-propagated"));
}

#[tokio::test]
async fn scoped_context_exposes_its_domain_set() {
    // The ambient `current().domains()` must reflect the scoped context's
    // DomainSet — this is the seam the registry filter reads (#4796).
    let harness = crate::core::runtime::DomainSet::harness();
    let ctx = CoreContext::for_test(harness, Some(PathBuf::from("/tmp/ctx-domains")));
    let seen = CoreContext::scope(ctx, async { CoreContext::current().map(|c| c.domains()) }).await;
    assert_eq!(seen, Some(harness));
    assert!(seen.unwrap().allows(crate::core::all::DomainGroup::Memory));
    assert!(!seen.unwrap().allows(crate::core::all::DomainGroup::Web3));
}

#[tokio::test]
async fn nested_scope_overrides_then_restores() {
    let a = ctx("/tmp/ctx-a");
    let b = ctx("/tmp/ctx-b");
    let (inner, outer) = CoreContext::scope(a, async {
        let inner = CoreContext::scope(b, async {
            CoreContext::current().unwrap().workspace_dir().unwrap()
        })
        .await;
        let outer = CoreContext::current().unwrap().workspace_dir().unwrap();
        (inner, outer)
    })
    .await;
    // Inner dispatch sees tenant B; the outer scope is restored to A after.
    assert_eq!(inner, PathBuf::from("/tmp/ctx-b"));
    assert_eq!(outer, PathBuf::from("/tmp/ctx-a"));
}

#[test]
fn degraded_context_rejects_workspace_bound_stores() {
    let ctx = CoreContext {
        host_kind: HostKind::Cli,
        workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
            workspace_dir: None,
        }))),
        domains: crate::core::runtime::DomainSet::full(),
        tool_groups: Default::default(),
        embedder_config: None,
        user_skill_roots: true,
        backend_transport: None,
    };

    // `workspace_dir()` is the gate every workspace-bound store goes
    // through, so it is asserted directly. This used to go through
    // `CoreContext::people()`, which was simply the first such store; it
    // resolves through the memory binding now and no longer exists.
    let err = match ctx.workspace_dir() {
        Ok(_) => panic!("degraded context unexpectedly resolved a workspace"),
        Err(err) => err,
    };
    assert!(
        err.contains("workspace unavailable"),
        "unexpected error: {err}"
    );
}
