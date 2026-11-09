//! Test-only constructors for [`CoreContext`]: a context with an explicit domain
//! set, workspace or embedder config, without the full [`CoreContext::init`] boot.

use super::*;

#[cfg(feature = "security-module")]
static NATIVE_FIXTURE_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("native-fixture")
            .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
            .enable_all()
            .build()
            .expect("native fixture runtime")
    });

impl CoreContext {
    /// Admit the real module on a retained runtime so synchronous host adapters
    /// can finish while a current-thread test waits. Negative availability
    /// fixtures use `scope_test_config` and perform their own admission attempt.
    pub(crate) async fn scope_test_native_config<F: std::future::Future>(
        config: &crate::config::Config,
        operation: F,
    ) -> F::Output {
        let context =
            Self::for_test_with_config(crate::core::runtime::DomainSet::full(), config.clone());
        #[cfg(feature = "security-module")]
        if config.modules.enabled {
            let boot_context = Arc::clone(&context);
            let boot_config = config.clone();
            NATIVE_FIXTURE_RUNTIME
                .spawn(async move {
                    Self::scope(
                        boot_context,
                        crate::modules::security::ensure_loaded(&boot_config),
                    )
                    .await
                })
                .await
                .expect("native fixture bootstrap task")
                .expect("admit the real native security fixture");
        }
        Self::scope(context, operation).await
    }

    /// Run a fixture under its explicit host configuration, preserving the real
    /// module loader and policy checks instead of borrowing process state.
    pub(crate) async fn scope_test_config<F: std::future::Future>(
        config: &crate::config::Config,
        operation: F,
    ) -> F::Output {
        Self::scope(
            Self::for_test_with_config(crate::core::runtime::DomainSet::full(), config.clone()),
            operation,
        )
        .await
    }

    /// Test-only constructor: build a context with an explicit
    /// [`DomainSet`](crate::core::runtime::DomainSet) and optional workspace, so
    /// cross-module tests (e.g. `core::all`'s registry filter) can exercise the
    /// ambient DomainSet gate without going through the full [`CoreContext::init`]
    /// boot sequence.
    pub(crate) fn for_test(
        domains: crate::core::runtime::DomainSet,
        workspace_dir: Option<std::path::PathBuf>,
    ) -> Arc<CoreContext> {
        Self::for_test_with_host_kind(HostKind::Cli, domains, workspace_dir)
    }

    /// Test-only constructor with an explicit host policy identity.
    pub(crate) fn for_test_with_host_kind(
        host_kind: HostKind,
        domains: crate::core::runtime::DomainSet,
        workspace_dir: Option<std::path::PathBuf>,
    ) -> Arc<CoreContext> {
        Arc::new(CoreContext {
            host_kind,
            workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                workspace_dir,
                config_path: None,
            }))),
            domains,
            tool_groups: Default::default(),
            autonomy_tier_source: AutonomyTierSource::LiveConfig,
            autonomy_tier_ceiling: None,
            components: Default::default(),
            embedder_config: None,
            latest_embedder_config: None,
            user_skill_roots: true,
            extensions: Default::default(),
            turn_origin: None,
            channel_thread: None,
            session_agent: None,
            profile: None,
            profile_admission: Arc::new(()),
            agent: Default::default(),
            bare: false,
        })
    }

    /// Test-only constructor with context-local hook registrations.
    pub(crate) fn for_test_with_extensions(
        domains: crate::core::runtime::DomainSet,
        workspace_dir: Option<std::path::PathBuf>,
        extensions: crate::core::slots::Extensions,
    ) -> Arc<CoreContext> {
        Self::for_test_with_host_kind_and_extensions(
            HostKind::Cli,
            domains,
            workspace_dir,
            extensions,
        )
    }

    /// Test-only constructor with explicit host policy and scoped registrations.
    pub(crate) fn for_test_with_host_kind_and_extensions(
        host_kind: HostKind,
        domains: crate::core::runtime::DomainSet,
        workspace_dir: Option<std::path::PathBuf>,
        extensions: crate::core::slots::Extensions,
    ) -> Arc<CoreContext> {
        let mut ctx = Arc::try_unwrap(Self::for_test_with_host_kind(
            host_kind,
            domains,
            workspace_dir,
        ))
        .ok()
        .unwrap();
        ctx.extensions = extensions;
        Arc::new(ctx)
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
        Self::for_test_with_host_kind_and_config(HostKind::Cli, domains, config)
    }

    /// Test-only constructor with both an explicit host identity and config.
    pub(crate) fn for_test_with_host_kind_and_config(
        host_kind: HostKind,
        domains: crate::core::runtime::DomainSet,
        config: crate::config::Config,
    ) -> Arc<CoreContext> {
        let latest_embedder_config = Arc::new(std::sync::Mutex::new(config.clone()));
        crate::config::Config::bind_context_snapshot(&latest_embedder_config);
        Arc::new(CoreContext {
            host_kind,
            workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                workspace_dir: Some(config.workspace_dir.clone()),
                config_path: Some(config.config_path.clone()),
            }))),
            domains,
            tool_groups: Default::default(),
            autonomy_tier_source: AutonomyTierSource::Captured,
            autonomy_tier_ceiling: None,
            components: Default::default(),
            embedder_config: Some(config),
            latest_embedder_config: Some(latest_embedder_config),
            user_skill_roots: true,
            extensions: Default::default(),
            turn_origin: None,
            channel_thread: None,
            session_agent: None,
            profile: None,
            profile_admission: Arc::new(()),
            agent: Default::default(),
            bare: false,
        })
    }

    /// Test-only constructor for a normal-boot binding with its resolved config
    /// path, without exposing an embedder config snapshot.
    pub(crate) fn for_test_with_boot_config_path(
        domains: crate::core::runtime::DomainSet,
        workspace_dir: std::path::PathBuf,
        config_path: std::path::PathBuf,
    ) -> Arc<CoreContext> {
        let mut ctx = Arc::try_unwrap(Self::for_test(domains, None)).ok().unwrap();
        ctx.workspace_binding = RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
            workspace_dir: Some(workspace_dir),
            config_path: Some(config_path),
        })));
        Arc::new(ctx)
    }
}
