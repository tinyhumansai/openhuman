//! Wall-clock timeout for tool execution (node/tool runtime + agent loop).
//!
//! Resolution order, highest precedence first:
//! 1. `OPENHUMAN_TOOL_TIMEOUT_SECS` environment variable (operator override).
//! 2. The value pushed in from the persisted config via [`set_tool_timeout_secs`]
//!    (driven by the UI / `config.update_agent_settings` RPC).
//! 3. The built-in [`DEFAULT_TIMEOUT_SECS`] (120) default.
//!
//! The effective value lives in a process-global [`AtomicU64`] and is read
//! fresh on every tool call, so a UI change takes effect on the **next** tool
//! call without a restart. The operator env var, when set to a valid value,
//! always wins — config pushes are ignored while it is present (logged).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Default tool-execution timeout in seconds when nothing else is configured.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Smallest accepted timeout. `0` would disable the timeout entirely, so it is
/// rejected and falls back to the default.
pub const MIN_TIMEOUT_SECS: u64 = 1;
/// Largest accepted timeout (1 hour) — guards against typos that would make a
/// hung tool wedge a session indefinitely.
pub const MAX_TIMEOUT_SECS: u64 = 3600;
/// Operator override env var. Takes precedence over the persisted config value.
pub const ENV_VAR: &str = "OPENHUMAN_TOOL_TIMEOUT_SECS";
/// Effective-unbounded cap (24h) for sandbox backends, which require a finite
/// deadline. Scripting tools run truly unbounded on the native path, but the
/// sandbox path substitutes this generous cap when no explicit `timeout_secs`
/// was requested — long enough not to kill a legitimate long job, finite
/// enough to eventually reclaim a wedged sandbox process.
pub const SANDBOX_UNBOUNDED_CAP_SECS: u64 = 86_400;

/// Effective timeout in seconds. `0` is the "not yet seeded" sentinel: the
/// first read resolves env/default and stores it. Config pushes overwrite it
/// (unless the env override is active).
static RUNTIME_SECS: AtomicU64 = AtomicU64::new(0);

/// Parse a raw env-var value into a bounded timeout.
///
/// Testable split from the global resolution: this function is pure and never
/// touches global state, so unit tests can exercise every path without racing
/// on the atomic or mutating the process environment.
///
/// - `None` or a non-numeric string returns [`DEFAULT_TIMEOUT_SECS`].
/// - Values outside `MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS` are rejected (returns
///   [`DEFAULT_TIMEOUT_SECS`]).
/// - Valid values pass through unchanged.
pub fn parse_tool_timeout_secs(raw: Option<&str>) -> u64 {
    raw.and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| (MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&n))
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// The operator env override, if `ENV_VAR` is set to a value inside the valid
/// range. A present-but-invalid env value (non-numeric, `0`, out of range) is
/// treated as "no override" so the config value still applies.
fn env_override_from(raw: Option<&str>) -> Option<u64> {
    raw.and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| (MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&n))
}

/// Pure resolver used by both seeding and config pushes. Env override wins;
/// otherwise the (bounded) config value applies.
fn resolve_effective(config_secs: u64, env_raw: Option<&str>) -> u64 {
    match env_override_from(env_raw) {
        Some(env) => env,
        None => parse_tool_timeout_secs(Some(&config_secs.to_string())),
    }
}

fn read_env() -> Option<String> {
    std::env::var(ENV_VAR).ok()
}

/// `true` when the operator env var is set to a valid override, meaning UI /
/// config changes to the timeout are ignored in favour of it. Surfaced to the
/// frontend so the settings panel can explain why its control has no effect.
pub fn env_override_active() -> bool {
    env_override_from(read_env().as_deref()).is_some()
}

/// Resolve the effective timeout, seeding the atomic from env/default on first
/// read. Concurrent first reads converge on the same seed value.
fn current_secs() -> u64 {
    let v = RUNTIME_SECS.load(Ordering::Relaxed);
    if v == 0 {
        let seeded = resolve_effective(DEFAULT_TIMEOUT_SECS, read_env().as_deref());
        RUNTIME_SECS.store(seeded, Ordering::Relaxed);
        seeded
    } else {
        v
    }
}

/// Push a config-sourced timeout into the runtime. The operator env override,
/// when active, always wins and `config_secs` is ignored (logged at debug).
/// Returns the effective value stored after the call. Idempotent and safe to
/// call repeatedly (e.g. at startup and on every config update).
pub fn set_tool_timeout_secs(config_secs: u64) -> u64 {
    let env_raw = read_env();
    let effective = resolve_effective(config_secs, env_raw.as_deref());
    RUNTIME_SECS.store(effective, Ordering::Relaxed);
    if env_override_from(env_raw.as_deref()).is_some() {
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

/// Effective timeout in seconds — used for logging and matching frontend
/// timeouts. Read fresh on every call.
pub fn tool_execution_timeout_secs() -> u64 {
    current_secs()
}

/// Effective timeout as a [`Duration`] for `tokio::time::timeout`-style callers.
pub fn tool_execution_timeout_duration() -> Duration {
    Duration::from_secs(current_secs())
}

/// Resolve an **explicit** per-call timeout request for a tool that is
/// otherwise unbounded (the scripting tools: `shell`, `node_exec`, `npm_exec`).
///
/// Unlike most tools — which inherit the global config-driven timeout so a hung
/// network/MCP call can't wedge a session — scripting tools run with **no**
/// default deadline: a build / solver / test run legitimately takes minutes and
/// must not be hard-killed by a default cap (issue #4023). A deadline applies
/// only when the caller explicitly asks for one via `timeout_secs`.
///
/// Returns:
/// - `None` → run unbounded. Used when no `timeout_secs` was supplied
///   (`None`) or it was explicitly disabled (`Some(0)`).
/// - `Some(secs)` → enforce this budget, clamped to `MIN_TIMEOUT_SECS..=cap`.
///   `cap` lets callers with a tighter own-ceiling (e.g. node/npm at 1800s)
///   pass it; most callers pass [`MAX_TIMEOUT_SECS`].
pub fn explicit_call_timeout_secs(requested: Option<u64>, cap: u64) -> Option<u64> {
    let cap = cap.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
    match requested {
        None | Some(0) => None,
        Some(n) => Some(n.clamp(MIN_TIMEOUT_SECS, cap)),
    }
}

/// [`explicit_call_timeout_secs`] as a [`Duration`], or `None` for unbounded.
pub fn explicit_call_timeout_duration(requested: Option<u64>, cap: u64) -> Option<Duration> {
    explicit_call_timeout_secs(requested, cap).map(Duration::from_secs)
}

/// Extra slack added on top of an explicit per-call budget before the hard
/// `tokio::time::timeout` fires, so a tool that finishes right at its requested
/// deadline isn't killed by scheduler jitter. The user-facing `timeout_secs`
/// reported on a timeout is the un-padded request.
const TOOL_TIMEOUT_GRACE_SECS: u64 = 5;

/// Resolve a tool's [`ToolTimeout`] policy into the `(deadline, timeout_secs)`
/// pair the agent tool-execution loop enforces:
/// - `Inherit` → the global config-driven timeout (a finite deadline).
/// - `Secs(req)` → the clamped request, padded by [`TOOL_TIMEOUT_GRACE_SECS`]
///   for the actual deadline while `timeout_secs` reports the un-padded budget.
/// - `Unbounded` → `(None, 0)`: no deadline; the tool runs to completion.
///
/// Moved out of the retired legacy `engine::tools` module during the tinyagents
/// migration (issue #4249); it lives here next to the timeout constants it uses.
pub fn resolve_tool_deadline(
    policy: crate::openhuman::tools::traits::ToolTimeout,
) -> (Option<Duration>, u64) {
    use crate::openhuman::tools::traits::ToolTimeout;
    match policy {
        ToolTimeout::Inherit => {
            let s = tool_execution_timeout_secs();
            (Some(Duration::from_secs(s)), s)
        }
        ToolTimeout::Secs(req) => {
            let s = req.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
            (
                Some(Duration::from_secs(
                    s.saturating_add(TOOL_TIMEOUT_GRACE_SECS),
                )),
                s,
            )
        }
        ToolTimeout::Unbounded => (None, 0),
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
