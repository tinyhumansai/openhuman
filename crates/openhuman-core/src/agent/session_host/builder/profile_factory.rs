//! Profile session construction carries the caller's exact conversation key.

use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use crate::core::runtime::DispatchCtx;

impl OpenHumanSessionHost {
    /// Build a config-routed agent with its caller's exact conversation key.
    /// Profile capabilities are fixed by the operator and narrowed before
    /// host-only session registration. Ordinary config agents keep their belt.
    ///
    /// # Errors
    /// As [`Self::from_config_for_agent`], or when a profile has no definition.
    pub fn from_config_for_agent_session(
        ctx: &DispatchCtx,
        config: &Config,
        agent_id: &str,
        session_id: Option<&str>,
    ) -> anyhow::Result<Self> {
        if let Some(capabilities) = crate::profiles::capabilities::ProfileCapabilities::in_ctx(ctx)
        {
            let mut definition = super::factory::resolve_target_definition(config, agent_id)?
                .ok_or_else(|| anyhow::anyhow!("profile agent definition is unavailable"))?;
            capabilities.apply_system_prompt(&mut definition);
            let factory = capabilities.factory(config);
            return Self::from_config_host_only(
                ctx,
                config,
                &definition,
                Some(&factory),
                session_id,
            );
        }
        Self::from_config_for_agent(ctx, config, agent_id)
    }
}
