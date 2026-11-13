//! Test-only constructors for [`CoreContext`]: a context with an explicit domain
//! set, workspace or embedder config, without the full [`CoreContext::init`] boot.

use super::*;

impl CoreContext {
    /// Test-only constructor: build a context with an explicit
    /// [`DomainSet`](crate::core::runtime::DomainSet) and optional workspace, so
    /// cross-module tests (e.g. `core::all`'s registry filter) can exercise the
    /// ambient DomainSet gate without going through the full [`CoreContext::init`]
    /// boot sequence.
    pub(crate) fn for_test(
        domains: crate::core::runtime::DomainSet,
        workspace_dir: Option<std::path::PathBuf>,
    ) -> Arc<CoreContext> {
        Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                workspace_dir,
            }))),
            domains,
            tool_groups: Default::default(),
            components: Default::default(),
            embedder_config: None,
            user_skill_roots: true,
            backend_transport: None,
            turn_origin: None,
            channel_thread: None,
            session_agent: None,
            profile: None,
            agent: Default::default(),
            bare: false,
        })
    }

    /// Test-only: given `[components]` switches.
    pub(crate) fn for_test_with_components(c: crate::core::runtime::ComponentSet) -> Arc<Self> {
        let mut ctx = Arc::try_unwrap(Self::for_test(
            crate::core::runtime::DomainSet::full(),
            None,
        ))
        .ok()
        .unwrap();
        ctx.components = c;
        Arc::new(ctx)
    }

    /// Test-only constructor that carries an embedder-supplied config, so a
    /// cross-module test can exercise the `load_config_with_timeout()` read
    /// path (which prefers [`CoreContext::current_embedder_config`]) without a
    /// full boot or a racy on-disk `config.toml`.
    ///
    /// Distinct from [`CoreContext::for_test`] — which always sets
    /// `embedder_config: None` — so the ~30 existing `for_test` call sites are
    /// unaffected. The workspace binding is anchored to the config's
    /// `workspace_dir`, matching how [`CoreContext::init`] wires an
    /// embedder-supplied config.
    pub(crate) fn for_test_with_config(
        domains: crate::core::runtime::DomainSet,
        config: crate::config::Config,
    ) -> Arc<CoreContext> {
        Arc::new(CoreContext {
            host_kind: HostKind::Cli,
            workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                workspace_dir: Some(config.workspace_dir.clone()),
            }))),
            domains,
            tool_groups: Default::default(),
            components: Default::default(),
            embedder_config: Some(config),
            user_skill_roots: true,
            backend_transport: None,
            turn_origin: None,
            channel_thread: None,
            session_agent: None,
            profile: None,
            agent: Default::default(),
            bare: false,
        })
    }
}
