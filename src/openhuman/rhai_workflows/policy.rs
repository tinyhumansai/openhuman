//! Maps openhuman's autonomy tier and tool-timeout clamps onto a
//! [`tinyagents::ReplPolicy`], fail-closed.
//!
//! Every Rhai session is bounded: the resolved policy is always based on the
//! conservative crate default, the wall-clock timeout is always set (never
//! unbounded), the `readonly` tier is refused outright, and caller-supplied
//! limit overrides are clamped — a `full`-tier caller may raise call-count
//! limits up to a hard 2× ceiling, everyone else may only tighten them.

use std::time::Duration;

use tinyagents::ReplPolicy;

use crate::openhuman::security::policy::AutonomyLevel;
use crate::openhuman::tool_timeout;

use super::types::RhaiLimitsOverride;

/// Default per-cell wall-clock timeout when the caller does not specify one.
/// Matches the `rhai_workflows` tool's default `ToolTimeout::Secs(300)` so the inner
/// deadline and the harness backstop agree.
pub const DEFAULT_RHAI_TIMEOUT_SECS: u64 = 300;

/// Hard upper bound on batched concurrency, regardless of tier or override.
pub const MAX_RHAI_CONCURRENCY: usize = 8;

/// Ceiling multiplier applied to the crate default call-count limits when a
/// `full`-tier caller raises them via the tool's `limits` argument.
pub const LIMIT_CEILING_MULTIPLIER: usize = 2;

/// Maximum sub-agent recursion depth an Rhai session may drive. Mirrors the
/// harness `MAX_SPAWN_DEPTH` (kept in lock-step by intent; a divergence only
/// tightens one side, never opens an unbounded path).
pub const RHAI_MAX_DEPTH: usize = 3;

/// Resolves a [`ReplPolicy`] for a session opened at autonomy `tier`, with an
/// optional caller `timeout_secs` and `overrides`.
///
/// Returns `Err(reason)` — a model-consumable string — when the tier forbids
/// Rhai entirely (`readonly`). The `reason` is surfaced verbatim to the model.
pub fn resolve_policy(
    tier: AutonomyLevel,
    timeout_secs: Option<u64>,
    overrides: Option<&RhaiLimitsOverride>,
) -> Result<ReplPolicy, String> {
    if tier == AutonomyLevel::ReadOnly {
        return Err(
            "the `rhai_workflows` tool is disabled at the read-only autonomy tier (it can drive tools, \
             models, and sub-agents); raise autonomy to `supervised` or `full` to use it"
                .to_string(),
        );
    }
    let allow_raise = tier == AutonomyLevel::Full;

    let base = ReplPolicy::default();

    // Wall-clock timeout: clamp the caller's request to [1, 3600] and cap it,
    // defaulting to DEFAULT_RHAI_TIMEOUT_SECS. Never unbounded.
    let secs =
        tool_timeout::explicit_call_timeout_secs(timeout_secs, tool_timeout::MAX_TIMEOUT_SECS)
            .unwrap_or(DEFAULT_RHAI_TIMEOUT_SECS);

    let ov = overrides.cloned().unwrap_or_default();
    let policy = ReplPolicy {
        timeout: Some(Duration::from_secs(secs)),
        max_depth: base.max_depth.min(RHAI_MAX_DEPTH),
        max_model_calls: clamp_limit(base.max_model_calls, ov.max_model_calls, allow_raise),
        max_agent_calls: clamp_limit(base.max_agent_calls, ov.max_agent_calls, allow_raise),
        max_tool_calls: clamp_limit(base.max_tool_calls, ov.max_tool_calls, allow_raise),
        max_concurrency: clamp_concurrency(base.max_concurrency, ov.max_concurrency, allow_raise),
        ..base
    };

    tracing::debug!(
        tier = ?tier,
        timeout_secs = secs,
        max_model_calls = policy.max_model_calls,
        max_agent_calls = policy.max_agent_calls,
        max_tool_calls = policy.max_tool_calls,
        max_concurrency = policy.max_concurrency,
        max_depth = policy.max_depth,
        "[rhai_workflows] resolved ReplPolicy"
    );
    Ok(policy)
}

/// Clamps a caller-requested call-count limit.
///
/// - No request → the crate default.
/// - `allow_raise` (full tier) → clamped to `[1, default * ceiling]`.
/// - Otherwise (supervised) → may only *tighten*: clamped to `[1, default]`.
fn clamp_limit(default: usize, requested: Option<usize>, allow_raise: bool) -> usize {
    match requested {
        None => default,
        Some(n) => {
            let ceiling = if allow_raise {
                default.saturating_mul(LIMIT_CEILING_MULTIPLIER)
            } else {
                default
            };
            n.clamp(1, ceiling.max(1))
        }
    }
}

/// Clamps batched concurrency to `[1, MAX_RHAI_CONCURRENCY]`, and to the default
/// unless the tier allows raising it.
fn clamp_concurrency(default: usize, requested: Option<usize>, allow_raise: bool) -> usize {
    let ceiling = if allow_raise {
        MAX_RHAI_CONCURRENCY
    } else {
        default.min(MAX_RHAI_CONCURRENCY)
    };
    match requested {
        None => default.min(MAX_RHAI_CONCURRENCY),
        Some(n) => n.clamp(1, ceiling.max(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_tier_is_refused() {
        let err =
            resolve_policy(AutonomyLevel::ReadOnly, None, None).expect_err("readonly refused");
        assert!(
            err.contains("read-only"),
            "reason should name the tier: {err}"
        );
    }

    #[test]
    fn default_timeout_when_unset_and_bounded_when_set() {
        let p = resolve_policy(AutonomyLevel::Supervised, None, None).expect("policy");
        assert_eq!(
            p.timeout,
            Some(Duration::from_secs(DEFAULT_RHAI_TIMEOUT_SECS))
        );

        // A caller request is clamped to [1, 3600].
        let p = resolve_policy(AutonomyLevel::Supervised, Some(10_000), None).expect("policy");
        assert_eq!(p.timeout, Some(Duration::from_secs(3600)));
        let p = resolve_policy(AutonomyLevel::Supervised, Some(0), None).expect("policy");
        assert_eq!(
            p.timeout,
            Some(Duration::from_secs(DEFAULT_RHAI_TIMEOUT_SECS))
        );
    }

    #[test]
    fn supervised_may_only_tighten_limits() {
        let base = ReplPolicy::default();
        let ov = RhaiLimitsOverride {
            max_tool_calls: Some(base.max_tool_calls * 10), // request a raise
            ..Default::default()
        };
        let p = resolve_policy(AutonomyLevel::Supervised, None, Some(&ov)).expect("policy");
        assert_eq!(
            p.max_tool_calls, base.max_tool_calls,
            "supervised cannot raise above the default"
        );

        let ov = RhaiLimitsOverride {
            max_tool_calls: Some(1),
            ..Default::default()
        };
        let p = resolve_policy(AutonomyLevel::Supervised, None, Some(&ov)).expect("policy");
        assert_eq!(p.max_tool_calls, 1, "supervised may tighten");
    }

    #[test]
    fn full_may_raise_up_to_the_ceiling() {
        let base = ReplPolicy::default();
        let ov = RhaiLimitsOverride {
            max_model_calls: Some(base.max_model_calls * 100), // way over ceiling
            ..Default::default()
        };
        let p = resolve_policy(AutonomyLevel::Full, None, Some(&ov)).expect("policy");
        assert_eq!(
            p.max_model_calls,
            base.max_model_calls * LIMIT_CEILING_MULTIPLIER,
            "full is capped at the 2x ceiling"
        );
    }

    #[test]
    fn concurrency_is_hard_capped() {
        let ov = RhaiLimitsOverride {
            max_concurrency: Some(1000),
            ..Default::default()
        };
        let p = resolve_policy(AutonomyLevel::Full, None, Some(&ov)).expect("policy");
        assert_eq!(p.max_concurrency, MAX_RHAI_CONCURRENCY);
    }

    #[test]
    fn depth_respects_the_spawn_ceiling() {
        let p = resolve_policy(AutonomyLevel::Full, None, None).expect("policy");
        assert!(p.max_depth <= RHAI_MAX_DEPTH);
    }
}
