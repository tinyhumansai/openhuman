use super::*;

impl CoreContext {
    /// Derive a no-store context for read-only inspection under a selected
    /// config. It inherits host registrations and policies while owning fresh
    /// agent state slots; the parent's tenant and agent identities are retained.
    pub fn derive_for_inspection(&self, mut overlay: ContextOverlay) -> Arc<CoreContext> {
        let components =
            crate::core::runtime::ComponentSet::from_config(&overlay.config.components);
        overlay.domains = components.narrow_domains(overlay.domains);
        overlay.tool_groups = components.narrow_tool_groups(overlay.tool_groups);
        let parent_config = self.latest_embedder_config();
        self.derive_with_parent_config_inner(parent_config.as_ref(), overlay, true)
    }

    /// Construct a no-store inspection context before runtime boot. It carries
    /// the supplied config only for the scoped inspection and does not publish
    /// a process-default context or initialize stores.
    pub fn for_inspection(config: crate::config::Config) -> Arc<CoreContext> {
        Self::for_runtime_inspection(
            config,
            HostKind::Cli,
            crate::core::runtime::DomainSet::full(),
            Default::default(),
        )
    }

    /// Build an inspection runtime context while retaining the host's selected
    /// domains and tool groups. The builder merges its scoped extensions after
    /// this constructor returns.
    pub(super) fn for_runtime_inspection(
        config: crate::config::Config,
        host_kind: HostKind,
        domains: crate::core::runtime::DomainSet,
        tool_groups: crate::tools::toolpacks::ToolGroups,
    ) -> Arc<CoreContext> {
        let components = crate::core::runtime::ComponentSet::from_config(&config.components);
        let domains = components.narrow_domains(domains);
        let tool_groups = components.narrow_tool_groups(tool_groups);
        let latest_embedder_config = Some(Arc::new(std::sync::Mutex::new(config.clone())));
        Arc::new(CoreContext {
            host_kind,
            workspace_binding: RwLock::new(Arc::new(RwLock::new(WorkspaceBinding {
                workspace_dir: Some(config.workspace_dir.clone()),
                config_path: Some(config.config_path.clone()),
            }))),
            domains,
            tool_groups,
            autonomy_tier_source: super::AutonomyTierSource::Captured,
            autonomy_tier_ceiling: None,
            components,
            embedder_config: Some(config),
            latest_embedder_config,
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
}
