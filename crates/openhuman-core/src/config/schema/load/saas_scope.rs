//! Where `Config::load_or_init` gets its config in SaaS mode.
//!
//! A single-user process resolves its config from the process: the
//! `OPENHUMAN_WORKSPACE` override, `active_user.toml`, the workspace marker.
//! A SaaS process serves many users, so any of those would hand one user's
//! work another's state — or the host's. There, the config is always the
//! one the current context carries: a user agent's forced config inside that
//! agent's scope, the operator's under the operator plane. With no context at
//! all, loading fails instead of guessing.

use super::super::Config;

/// The config a SaaS process loads, or `None` in single-user mode.
pub(super) fn saas_scoped_config() -> Option<anyhow::Result<Config>> {
    resolve(
        crate::core::runtime::is_saas(),
        // The task's own scope only: falling back to the process default
        // would hand a task that lost its user scope the operator's config.
        crate::core::runtime::CoreContext::scoped().and_then(|ctx| ctx.embedder_config().cloned()),
    )
}

/// [`saas_scoped_config`] as a pure function of the mode and the context's
/// config, so the rule is testable without locking the test process to SaaS.
pub(super) fn resolve(saas: bool, scoped: Option<Config>) -> Option<anyhow::Result<Config>> {
    if !saas {
        return None;
    }
    Some(scoped.ok_or_else(|| {
        log::error!("[saas][config] Config::load_or_init called outside any agent or operator scope");
        anyhow::anyhow!(
            "[saas] no config in scope: SaaS work must run under a user agent's or the operator's context"
        )
    }))
}

#[cfg(test)]
#[path = "saas_scope_tests.rs"]
mod tests;
