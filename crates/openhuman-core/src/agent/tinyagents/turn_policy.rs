//! Harness [`RunPolicy`] construction for an openhuman turn.
//!
//! The policy itself (ceilings, retry schedule, unknown-tool and invalid-arg
//! recovery, cache and capture settings) is owned by
//! `tinyagents_harness::turn_policy`. This module keeps the host half: the
//! `OPENHUMAN_*` environment overrides, provider locality, the per-turn tool
//! budget and the spawn-depth cap.

use tinyagents_harness::runtime::RunPolicy;
use tinyagents_harness::turn_policy::{
    local_turn_backstop_secs, run_policy, turn_wall_clock_ms, TurnPolicyOptions,
    TurnPolicyOverrides,
};

use crate::agent::tinyagents::budget::MAX_SPAWN_DEPTH;

/// The builder-configured [`ToolCallPolicy`](tinyagents_harness::middleware::ToolCallPolicy)
/// plus the session context a policy check needs, handed to the shared turn seam
/// so it can install the [`ToolPolicyMiddleware`](super::middleware::tool_policy::ToolPolicyMiddleware).
/// `None` means "no policy enforcement on this turn" (the channel/CLI + sub-agent
/// paths, which carry their own gating).
#[derive(Clone)]
pub(crate) struct ToolPolicyEnforcement {
    pub policy: std::sync::Arc<
        dyn tinyagents_harness::middleware::ToolCallPolicy<
            crate::agent::tinyagents::host::OpenHumanRunContext,
        >,
    >,
    /// The session's channel-permission snapshot — enforces the per-channel
    /// permission ceiling (deny + per-call permission-level gate) the in-house
    /// engine ran in `agent_tool_exec`.
    pub session: tinytools::ToolPolicySession,
    pub channel: String,
    pub agent_definition_id: String,
}

/// Whether `provider` (a provider string such as `ollama:llama3`) with the
/// resolved `endpoint` is a local / self-hosted runtime, which gets the longer
/// ceilings (#6042).
///
/// Do not use `tinyinference_local::profile::is_local_provider_string` for this
/// on its own: it maps the bare name `openai` onto the generic local
/// OpenAI-compatible kind, so hosted OpenAI would be classed as local and lose
/// its tight hang detection. Locality comes from the runtime kind and the
/// endpoint instead:
/// - Ollama / LM Studio / MLX / oMLX are self-hosted by definition.
/// - The generic OpenAI-compatible kinds (`local-openai`, `llamacpp`, `vllm`,
///   bare `openai`...) can point anywhere, so they are local only when the
///   endpoint host is loopback or on a private network.
/// - `openai:<model>` and every other cloud provider are never local.
pub(crate) fn provider_is_self_hosted(provider: &str, endpoint: Option<&str>) -> bool {
    use tinyinference_local::profile::{kind_from_provider_string, LocalProviderKind};
    let p = provider.trim().to_ascii_lowercase();
    let Some(kind) = kind_from_provider_string(&p) else {
        return false;
    };
    match kind {
        LocalProviderKind::LocalOpenai => endpoint
            .and_then(|url| url::Url::parse(url).ok())
            .is_some_and(|parsed| crate::util::url::host_is_local(&parsed)),
        _ => true,
    }
}

/// Whether `config` routes the chat workload to a local / self-hosted provider.
pub(crate) fn chat_provider_is_local(config: &crate::config::Config) -> bool {
    let provider = crate::inference::provider::provider_for_role("chat", config);
    provider_is_self_hosted(&provider, local_openai_endpoint(config).as_deref())
}

/// The endpoint the generic local OpenAI-compatible runtime resolves to:
/// `LOCAL_OPENAI_URL`, else `local_ai.base_url`, else the profile default.
pub(crate) fn local_openai_endpoint(config: &crate::config::Config) -> Option<String> {
    std::env::var("LOCAL_OPENAI_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| config.local_ai.base_url.clone())
        .or_else(|| {
            Some(
                tinyinference_local::profile::LOCAL_OPENAI_PROFILE
                    .default_base_url
                    .to_string(),
            )
        })
}

/// Resolve the per-turn wall-clock ceiling in milliseconds for a turn on a
/// local provider when `local`. `OPENHUMAN_AGENT_TURN_TIMEOUT_SECS` still wins;
/// `0` means "no ceiling" and yields `None`.
pub(crate) fn agent_turn_wall_clock_ms_for(local: bool) -> Option<u64> {
    turn_wall_clock_ms(
        env_str("OPENHUMAN_AGENT_TURN_TIMEOUT_SECS").as_deref(),
        local,
    )
}

/// Outer web-turn backstop for a turn on a local provider (#6042): the local
/// turn ceiling plus the harness grace, or `None` when the ceiling is disabled.
pub(crate) fn local_web_turn_backstop_secs() -> Option<u64> {
    local_turn_backstop_secs(env_str("OPENHUMAN_AGENT_TURN_TIMEOUT_SECS").as_deref())
}

fn env_str(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The `OPENHUMAN_*` overrides for the harness ceilings, read from the process
/// environment.
fn env_overrides() -> TurnPolicyOverrides {
    TurnPolicyOverrides {
        turn_timeout_secs: env_str("OPENHUMAN_AGENT_TURN_TIMEOUT_SECS"),
        model_call_timeout_secs: env_str("OPENHUMAN_MODEL_CALL_TIMEOUT_SECS"),
        stream_idle_timeout_secs: env_str("OPENHUMAN_STREAM_IDLE_TIMEOUT_SECS"),
        stream_first_event_timeout_secs: env_str("OPENHUMAN_STREAM_FIRST_EVENT_TIMEOUT_SECS"),
        max_consecutive_stream_idle_timeouts: env_str(
            "OPENHUMAN_MAX_CONSECUTIVE_STREAM_IDLE_TIMEOUTS",
        ),
    }
}

/// Build the harness [`RunPolicy`] for an openhuman turn (hosted provider).
pub(crate) fn run_policy_for(max_iterations: usize, response_cache_enabled: bool) -> RunPolicy {
    run_policy_for_provider(max_iterations, response_cache_enabled, false, None)
}

/// [`run_policy_for`] with the turn's provider locality: a local / self-hosted
/// provider gets the longer per-call and per-turn ceilings (#6042); hosted
/// providers keep the tight defaults so a wedged call is still caught fast.
///
/// Cross-route fallback (`RunPolicy.fallback`) is orthogonal and populated
/// per-turn by [`turn_run_policy`](super::turn_plan::turn_run_policy).
pub(crate) fn run_policy_for_provider(
    max_iterations: usize,
    response_cache_enabled: bool,
    local_provider: bool,
    tool_call_cap: Option<usize>,
) -> RunPolicy {
    run_policy(&TurnPolicyOptions {
        max_model_calls: max_iterations,
        max_tool_calls: crate::agent::stop_hooks::tool_call_limit(max_iterations, tool_call_cap),
        max_depth: MAX_SPAWN_DEPTH,
        local_provider,
        response_cache_enabled,
        overrides: env_overrides(),
    })
}

/// True when `name` is a sub-agent spawn/delegation tool that a **child** run
/// must never be able to invoke (issue #4452), re-asserted at registration as
/// defense-in-depth so a misconfigured allowlist cannot reintroduce sub-agent
/// spawning into a nested run. Kept local to this seam (rather than importing
/// the `pub(super)` runner helper) so the invariant travels with the
/// registration site that enforces it.
///
/// **This is a strict subset of the caller-side strip**, not a mirror of it
/// (issue #6157). `subagent_host::tool_prep::is_subagent_spawn_tool` also
/// resolves each archetype's `delegate_name` override through the definition
/// registry — `manage_tasks`, `create_image`, `setup_skills`, … — none of which
/// carry the `delegate_` prefix this match relies on. Matching them here would
/// put a registry lookup on the per-tool registration loop, so the caller
/// stays responsible for the override names: every path that feeds `allowed`
/// runs `is_subagent_spawn_tool` first, including the dynamic per-spawn tools
/// (`subagent_host::ops::runner`). Widen this predicate in lockstep if that
/// ever stops being true.
pub(crate) fn is_subagent_spawn_or_delegate_tool(name: &str) -> bool {
    name == "spawn_subagent" || name.starts_with("delegate_") || name == "spawn_worker_thread"
}

#[cfg(test)]
#[path = "turn_policy_budget_tests.rs"]
mod budget_tests;
