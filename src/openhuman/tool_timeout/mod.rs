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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_when_env_missing() {
        assert_eq!(parse_tool_timeout_secs(None), DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn default_when_value_not_numeric() {
        assert_eq!(
            parse_tool_timeout_secs(Some("not-a-number")),
            DEFAULT_TIMEOUT_SECS
        );
        assert_eq!(parse_tool_timeout_secs(Some("")), DEFAULT_TIMEOUT_SECS);
        assert_eq!(parse_tool_timeout_secs(Some("12x")), DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn default_when_value_zero() {
        // 0 seconds would disable the timeout — reject and fall back.
        assert_eq!(parse_tool_timeout_secs(Some("0")), DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn default_when_value_above_max() {
        assert_eq!(parse_tool_timeout_secs(Some("3601")), DEFAULT_TIMEOUT_SECS);
        assert_eq!(
            parse_tool_timeout_secs(Some("99999999999")),
            DEFAULT_TIMEOUT_SECS
        );
    }

    #[test]
    fn default_when_value_negative_or_signed() {
        // Negative values fail u64 parse and fall back to default.
        assert_eq!(parse_tool_timeout_secs(Some("-5")), DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn accepts_valid_values_at_boundaries() {
        assert_eq!(parse_tool_timeout_secs(Some("1")), MIN_TIMEOUT_SECS);
        assert_eq!(parse_tool_timeout_secs(Some("3600")), MAX_TIMEOUT_SECS);
    }

    #[test]
    fn accepts_valid_midrange_value() {
        assert_eq!(parse_tool_timeout_secs(Some("300")), 300);
    }

    #[test]
    fn env_override_takes_precedence_over_config() {
        // When the env var holds a valid value it wins over the config value.
        assert_eq!(resolve_effective(300, Some("600")), 600);
    }

    #[test]
    fn config_value_used_when_env_absent_or_invalid() {
        // No env → config drives the effective value (bounded).
        assert_eq!(resolve_effective(300, None), 300);
        // Present-but-invalid env (non-numeric / 0 / out of range) is ignored,
        // so the config value still applies.
        assert_eq!(resolve_effective(300, Some("nonsense")), 300);
        assert_eq!(resolve_effective(300, Some("0")), 300);
        assert_eq!(resolve_effective(300, Some("4000")), 300);
    }

    #[test]
    fn config_value_is_bounded() {
        // An out-of-range config value falls back to the default rather than
        // being applied verbatim.
        assert_eq!(resolve_effective(0, None), DEFAULT_TIMEOUT_SECS);
        assert_eq!(resolve_effective(99_999, None), DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn explicit_call_timeout_unbounded_when_absent_or_disabled() {
        // No request, or an explicit 0, means "run unbounded" (None).
        assert_eq!(explicit_call_timeout_secs(None, MAX_TIMEOUT_SECS), None);
        assert_eq!(explicit_call_timeout_secs(Some(0), MAX_TIMEOUT_SECS), None);
    }

    #[test]
    fn explicit_call_timeout_enforces_and_clamps_request() {
        // An in-range request is enforced verbatim.
        assert_eq!(
            explicit_call_timeout_secs(Some(900), MAX_TIMEOUT_SECS),
            Some(900)
        );
        // Below the floor clamps up to MIN.
        assert_eq!(
            explicit_call_timeout_secs(Some(0), MAX_TIMEOUT_SECS),
            None,
            "0 disables rather than clamping to MIN"
        );
        // Above the cap clamps down to the cap.
        assert_eq!(
            explicit_call_timeout_secs(Some(99_999), MAX_TIMEOUT_SECS),
            Some(MAX_TIMEOUT_SECS)
        );
        // A tool with a tighter own ceiling (node/npm at 1800) clamps to it.
        assert_eq!(explicit_call_timeout_secs(Some(99_999), 1800), Some(1800));
        assert_eq!(explicit_call_timeout_secs(Some(600), 1800), Some(600));
    }

    #[test]
    fn explicit_call_timeout_duration_matches_secs() {
        assert_eq!(
            explicit_call_timeout_duration(Some(900), MAX_TIMEOUT_SECS),
            Some(Duration::from_secs(900))
        );
        assert_eq!(explicit_call_timeout_duration(None, MAX_TIMEOUT_SECS), None);
    }

    #[test]
    fn env_override_from_rejects_invalid() {
        assert_eq!(env_override_from(None), None);
        assert_eq!(env_override_from(Some("")), None);
        assert_eq!(env_override_from(Some("0")), None);
        assert_eq!(env_override_from(Some("abc")), None);
        assert_eq!(env_override_from(Some("3601")), None);
        assert_eq!(env_override_from(Some("120")), Some(120));
    }
}
