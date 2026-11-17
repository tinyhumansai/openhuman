//! The operator's own switches that turn the sandbox off, bound to the facts of a scope.
//!
//! This only reads settings. It decides nothing: the module is told the operator's setting
//! as the scope's `environment` fact and alone decides whether a spawn may skip the jail
//! (never under SaaS, never for a source that is not the trusted operator). A SaaS core
//! sends the same setting and the module ignores it.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;

/// Host switch that turns the agent sandbox off for the whole process, for hosts that
/// already isolate the core (a container, a CI or benchmark task image, a VM). `off`,
/// `none`, `0`, `false` or `disabled` (case-insensitive) mean off; anything else, or
/// unset, leaves it on.
pub(crate) const SANDBOX_OFF_ENV: &str = "OPENHUMAN_SANDBOX";

/// Whether `value` (the `OPENHUMAN_SANDBOX` setting) says the sandbox is off.
pub(crate) fn off_value(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("off" | "none" | "0" | "false" | "disabled")
    )
}

/// `[components] sandbox = false` in the context `ctx` runs under. With no context the
/// component is on, since the set only narrows.
pub(crate) fn component_off(ctx: &DispatchCtx) -> bool {
    let off = !ctx.components().sandbox;
    if off {
        tracing::debug!("[sandbox] component 'sandbox' disabled by the operator");
    }
    off
}

/// The saved config says no isolation: the legacy `enabled = false` flag or
/// `[components] sandbox = false`.
pub(crate) fn config_off(config: &Config) -> bool {
    config.sandbox.enabled == Some(false)
        || !crate::core::runtime::ComponentSet::from_config(&config.components).sandbox
}

/// Whether the operator turned the sandbox off by the environment, the component set or
/// the configuration. A setting, not a verdict: see the module docs.
pub(crate) fn sandbox_off(ctx: &DispatchCtx, config: &Config) -> bool {
    off_value(std::env::var(SANDBOX_OFF_ENV).ok().as_deref())
        || component_off(ctx)
        || config_off(config)
}

#[cfg(test)]
#[path = "operator_tests.rs"]
mod tests;
