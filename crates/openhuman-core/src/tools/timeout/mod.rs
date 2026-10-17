//! Host adapter for tool-execution timeouts.
//!
//! The bounds, resolution, process-group capture and cleanup live in
//! `tinytools_std::timeout`. This module keeps only what OpenHuman owns: the
//! `OPENHUMAN_TOOL_TIMEOUT_SECS` operator override, pushing the persisted
//! config value into the module's settings, installing the harness-side
//! settings, and carrying `CoreContext` into the module's waiter tasks.
//!
//! Resolution order, highest precedence first:
//! 1. `OPENHUMAN_TOOL_TIMEOUT_SECS` (operator override, when valid).
//! 2. The value pushed from config via [`set_tool_timeout_secs`].
//! 3. `tinytools_std::timeout::DEFAULT_TIMEOUT_SECS`.

use std::sync::{Once, OnceLock};

use tinyagents_harness::tool::ToolTimeoutSettings;
use tinytools_std::timeout::{
    self as module, MAX_TIMEOUT_SECS, MIN_TIMEOUT_SECS, TOOL_TIMEOUT_GRACE_SECS,
};

/// Operator override env var. Takes precedence over the persisted config value.
pub const ENV_VAR: &str = "OPENHUMAN_TOOL_TIMEOUT_SECS";

static HARNESS_SETTINGS: OnceLock<ToolTimeoutSettings> = OnceLock::new();
static CONFIGURE: Once = Once::new();

fn read_env() -> Option<String> {
    std::env::var(ENV_VAR).ok()
}

/// Seed the module's process-global settings from env/default and register the
/// task wrapper that keeps the caller's `CoreContext` in the module's waiter
/// tasks. Idempotent; every entry point of this adapter calls it.
fn ensure_configured() {
    CONFIGURE.call_once(|| {
        let secs = module::resolve_effective(module::DEFAULT_TIMEOUT_SECS, read_env().as_deref());
        module::configure(module::TimeoutSettings::with_inherited_secs(secs));
        module::set_task_wrapper(|fut| {
            Box::pin(crate::core::runtime::DispatchCtx::ambient().enter(fut))
        });
        log::debug!("[tool_timeout] configured module settings inherited={secs}s");
    });
}

fn harness_settings() -> &'static ToolTimeoutSettings {
    HARNESS_SETTINGS.get_or_init(|| {
        ensure_configured();
        ToolTimeoutSettings::new(
            module::settings().inherited_timeout().as_millis() as u64,
            MIN_TIMEOUT_SECS * 1000,
            MAX_TIMEOUT_SECS * 1000,
            TOOL_TIMEOUT_GRACE_SECS * 1000,
        )
    })
}

/// Install the process-global per-tool timeout settings on `harness`.
///
/// The harness enforces a tool's `ToolTimeout` policy only when settings are
/// installed; without them every `Inherit` tool (MCP calls, `use_skill`,
/// integration actions) ran until the run's wall-clock budget. The installed
/// value shares its atomic with later [`set_tool_timeout_secs`] pushes.
pub fn install_harness_tool_timeouts<State: Send + Sync, Ctx: Send + Sync>(
    harness: &mut tinyagents_harness::runtime::AgentHarness<State, Ctx>,
) {
    let settings = harness_settings().clone();
    tracing::debug!(
        inherited_secs = settings.inherited_timeout().map_or(0, |d| d.as_secs()),
        "[tool_timeout] installing per-tool timeout settings on the harness"
    );
    harness.with_tool_timeout_settings(settings);
}

/// `true` when the operator env var is set to a valid override, meaning UI /
/// config changes to the timeout are ignored in favour of it.
pub fn env_override_active() -> bool {
    module::override_from(read_env().as_deref()).is_some()
}

/// Push a config-sourced timeout into the runtime. The operator env override,
/// when active, always wins. Returns the effective value stored.
pub fn set_tool_timeout_secs(config_secs: u64) -> u64 {
    ensure_configured();
    let env_raw = read_env();
    let effective = module::resolve_effective(config_secs, env_raw.as_deref());
    let ms = effective.saturating_mul(1000);
    module::settings().set_inherited_timeout_ms(ms);
    harness_settings().set_inherited_timeout_ms(ms);
    if module::override_from(env_raw.as_deref()).is_some() {
        log::debug!(
            "[tool_timeout] config update ignored: env {ENV_VAR}={effective}s overrides requested {config_secs}s"
        );
    } else {
        log::debug!(
            "[tool_timeout] runtime timeout set to {effective}s (requested {config_secs}s)"
        );
    }
    effective
}

/// Effective timeout in seconds, read fresh on every call.
pub fn tool_execution_timeout_secs() -> u64 {
    ensure_configured();
    module::settings().inherited_timeout().as_secs()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
