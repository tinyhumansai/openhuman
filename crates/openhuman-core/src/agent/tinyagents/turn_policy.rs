//! Harness [`RunPolicy`] construction for an openhuman turn, plus the small
//! constants/helpers that shape it (wall-clock ceilings, the model-call cap
//! normalization, and the builder-configured tool-policy enforcement handle).

use tinyagents_harness::retry::RetryPolicy;
use tinyagents_harness::runtime::{InvalidArgsPolicy, RunPolicy, UnknownToolPolicy};

use crate::agent::harness::MAX_SPAWN_DEPTH;

/// The builder-configured [`ToolPolicy`](crate::agent::tool_policy::ToolPolicy)
/// plus the session context a policy check needs, handed to the shared turn seam
/// so it can install the [`ToolPolicyMiddleware`](super::middleware::ToolPolicyMiddleware).
/// `None` means "no policy enforcement on this turn" (the channel/CLI + sub-agent
/// paths, which carry their own gating).
pub(crate) struct ToolPolicyEnforcement {
    pub policy: std::sync::Arc<dyn crate::agent::tool_policy::ToolPolicy>,
    /// The session's channel-permission snapshot — enforces the per-channel
    /// permission ceiling (deny + per-call permission-level gate) the in-house
    /// engine ran in `agent_tool_exec`.
    pub session: crate::tools::agent_policy::ToolPolicySession,
    pub session_id: String,
    pub channel: String,
    pub agent_definition_id: String,
}

/// Default per-turn wall-clock ceiling for an openhuman agent turn, in seconds
/// (issue #4746). Applied as the harness `RunLimits::max_wall_clock_ms` so the
/// loop interrupts a call instead of parking forever with no terminal event.
///
/// 60 minutes (#5766, was 10). With hang detection now owned by the per-call
/// ceiling below, this is a pure runaway guard over the whole turn — its old
/// 600s value doubled as the per-call bound (every call got the turn's
/// *remainder*), which killed long *productive* turns: a turn that had
/// legitimately spent ~543s across many successful calls handed its next model
/// call a 56s budget and died. A turn's real bounds are the model/tool call
/// caps times the per-call ceiling; this only catches what escapes those.
pub(super) const DEFAULT_AGENT_TURN_TIMEOUT_SECS: u64 = 3_600;

/// Default wall-clock ceiling for a **single model call** within a turn, in
/// seconds (#5766). Applied as the harness `RunLimits::max_model_call_ms`, so
/// every model call (and every retry attempt) gets a fresh
/// `min(this, turn remainder)` budget instead of only the shrinking remainder.
/// This is the hang detector the turn ceiling used to double as — scoped to
/// the one call that wedged, not the whole turn's history.
///
/// Deliberately generous — 15 minutes: a hidden-reasoning model call can be
/// legitimately app-silent for several minutes, so this is a backstop for
/// calls that will never return, not a latency target. Tool calls (including
/// sub-agent delegations, which wrap entire child turns) are exempt by design
/// in the harness and stay bounded by the turn remainder plus their own
/// per-tool timeouts.
pub(super) const DEFAULT_MODEL_CALL_TIMEOUT_SECS: u64 = 900;

/// Resolve the per-turn wall-clock ceiling in milliseconds for the harness
/// policy. Reads `OPENHUMAN_AGENT_TURN_TIMEOUT_SECS` (falling back to
/// [`DEFAULT_AGENT_TURN_TIMEOUT_SECS`]); `0` means "no ceiling" → `None`, which
/// restores the previous unbounded behavior for callers that deliberately opt
/// out (e.g. very long autonomous runs).
pub(crate) fn agent_turn_wall_clock_ms() -> Option<u64> {
    parse_agent_turn_wall_clock_ms(
        std::env::var("OPENHUMAN_AGENT_TURN_TIMEOUT_SECS")
            .ok()
            .as_deref(),
    )
}

/// Pure core of [`agent_turn_wall_clock_ms`]: map an optional
/// `OPENHUMAN_AGENT_TURN_TIMEOUT_SECS` value to a wall-clock ceiling in
/// milliseconds. An absent/unparseable value falls back to
/// [`DEFAULT_AGENT_TURN_TIMEOUT_SECS`]; `0` yields `None` (unbounded opt-out).
/// Kept env-free so it is deterministically unit-testable.
pub(super) fn parse_agent_turn_wall_clock_ms(env_value: Option<&str>) -> Option<u64> {
    let secs = env_value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_AGENT_TURN_TIMEOUT_SECS);
    (secs > 0).then(|| secs.saturating_mul(1_000))
}

/// Resolve the per-model-call wall-clock ceiling in milliseconds for the
/// harness policy. Reads `OPENHUMAN_MODEL_CALL_TIMEOUT_SECS` (falling back to
/// [`DEFAULT_MODEL_CALL_TIMEOUT_SECS`]); `0` means "no per-call ceiling" →
/// `None`, leaving calls bounded only by the turn's remaining wall clock as
/// before #5766.
pub(super) fn model_call_wall_clock_ms() -> Option<u64> {
    parse_model_call_wall_clock_ms(
        std::env::var("OPENHUMAN_MODEL_CALL_TIMEOUT_SECS")
            .ok()
            .as_deref(),
    )
}

/// Pure core of [`model_call_wall_clock_ms`]: map an optional
/// `OPENHUMAN_MODEL_CALL_TIMEOUT_SECS` value to a per-call ceiling in
/// milliseconds. An absent/unparseable value falls back to
/// [`DEFAULT_MODEL_CALL_TIMEOUT_SECS`]; `0` yields `None` (opt-out). Kept
/// env-free so it is deterministically unit-testable — the same shape as
/// [`parse_agent_turn_wall_clock_ms`].
pub(super) fn parse_model_call_wall_clock_ms(env_value: Option<&str>) -> Option<u64> {
    let secs = env_value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_MODEL_CALL_TIMEOUT_SECS);
    (secs > 0).then(|| secs.saturating_mul(1_000))
}

/// Build the harness [`RunPolicy`] for an openhuman turn.
///
/// The loop enforces limits from `self.policy.limits` (not the per-run
/// `RunConfig`), so the model-call cap **must** be set here or it falls back to
/// the tinyagents default of 25 — far more than openhuman's `max_iterations`.
/// The recursion depth cap is also set here so TinyAgents uses OpenHuman's
/// existing sub-agent spawn depth instead of the SDK default.
/// Retry is now owned by the crate [`RetryPolicy`] (issue #4249, Phase 3a): the
/// turn path no longer wraps its provider in `ReliableProvider` (removed in
/// `session/builder/factory.rs`), so the single retry layer is here, at the
/// harness model call. The schedule mirrors the former `ReliableProvider`
/// defaults — 2 retries (3 attempts) with 500 ms exponential backoff — so
/// transient 429/5xx behavior is preserved. Retryability is decided by the crate
/// `is_retryable`, which the [`native model adapter`](super::model) adapter feeds
/// correctly: a permanent config/auth/quota/context error is mapped to a
/// non-retryable `TinyAgentsError::Validation`, a transient blip to a retryable
/// `Model` error. The crate caps `max_attempts` at
/// `RunLimits::max_retries_per_call + 1` (default 3 retries), so this stays
/// within the loop's own bound.
///
/// (Config parity note: the former `config.reliability.provider_retries` /
/// `provider_backoff_ms` / `model_fallbacks` no longer drive the turn path —
/// retry is the fixed schedule below and cross-route fallback is the crate
/// registry `FallbackPolicy` from [`routes::route_fallback_policy`](super::routes::route_fallback_policy).
/// Those config knobs still apply to the non-seam `ReliableProvider` paths.)
///
/// Cross-route **fallback** (`RunPolicy.fallback`) is orthogonal to retry and is
/// populated per-turn by the caller ([`assemble_turn_harness`](super::harness_assembly::assemble_turn_harness)
/// via [`routes::route_fallback_policy`](super::routes::route_fallback_policy)); it is safe to enable now because
/// `ReliableProvider` does *not* fail over across the registered workload-tier
/// routes (chat→burst, reasoning→agentic, …) the way the harness registry can.
pub(crate) fn run_policy_for(max_iterations: usize, response_cache_enabled: bool) -> RunPolicy {
    let mut policy = RunPolicy::default();
    policy.limits.max_model_calls = max_iterations;
    policy.limits.max_tool_calls = crate::agent::stop_hooks::tool_call_limit(max_iterations);
    policy.limits.max_depth = MAX_SPAWN_DEPTH;
    // Wall-clock ceiling for the whole turn (issue #4746). The harness bounds
    // every individual model AND tool call by the run's *remaining* wall-clock
    // budget (`with_call_budget` → `tokio::time::timeout`), but ONLY when a
    // deadline is configured — with `max_wall_clock_ms = None` (the default)
    // `call_budget()` returns `None` and each call is awaited UNBOUNDED. That is
    // exactly how a turn shipped an empty reply: a hung/slow model stream or a
    // delegated sub-agent tool call that never returned left the loop parked
    // inside an await, so the between-call deadline check never ran and no
    // terminal event (loop `Timeout` → chat_error) ever fired. Setting the cap
    // here arms the harness's per-call timeout so a wedged call is interrupted
    // mid-flight and the turn degrades gracefully. It also bounds sub-agents:
    // the parent's remaining-budget wraps the sub-agent tool call, and a child
    // turn with no per-run timeout inherits this policy-level cap. Generous by
    // design (a backstop, not a UX deadline); env-overridable, `0` disables.
    policy.limits.max_wall_clock_ms = agent_turn_wall_clock_ms();
    // Per-model-call ceiling (#5766): each model call (and retry attempt) gets
    // a fresh `min(ceiling, turn remainder)` budget, so hang detection is
    // per-call instead of riding the turn deadline — which let the turn
    // ceiling above grow from 10 to 60 minutes without a wedged call being
    // able to hold a turn for more than this. Tool calls (incl. sub-agent
    // delegations) are exempt in the harness and keep the remainder-only
    // budget. Env-overridable, `0` disables.
    policy.limits.max_model_call_ms = model_call_wall_clock_ms();
    // Crate-owned retry (Phase 3a): mirror the former `ReliableProvider` schedule
    // (2 retries, 500 ms exponential backoff). `backoff_sleep` is on so a
    // transient 429/5xx actually waits before retrying, as it did before.
    policy.retry = RetryPolicy {
        max_attempts: 3,
        initial_backoff_ms: 500,
        max_backoff_ms: 30_000,
        multiplier: 2.0,
        jitter: false,
        backoff_sleep: true,
        max_retry_after_ms: RetryPolicy::DEFAULT_MAX_RETRY_AFTER_MS,
        retry_on: None,
    };
    // Unknown-tool recovery (01.2 / C3): the crate policy owns this end to end —
    // the `__openhuman_unknown_tool__` sentinel tool + `UnknownToolRewriteMiddleware`
    // were already deleted. We deliberately keep `ReturnToolError` rather than
    // `Rewrite { tool_name }`: Rewrite requires a real catch-all target tool (the
    // deleted sentinel was exactly that) and, when it hits, *silently* executes
    // that tool and emits `AgentEvent::UnknownToolCall { recovery: "rewrite:.." }`
    // WITHOUT injecting a tool message. `ReturnToolError` instead injects a
    // recoverable `unknown tool `<name>` (arguments: ..); valid tools: [..]`
    // result naming the originally-requested tool. Two live consumers depend on
    // that message: (1) the #4419 attempted-tool-name UX and (2) the failure
    // classifier in `agent::hooks::sanitize_tool_output`, which labels the result
    // `unknown_tool` by matching the "unknown tool" substring. Flipping to Rewrite
    // would drop both. The original name + args are also preserved verbatim on
    // `AgentEvent::UnknownToolCall` and projected by `OpenhumanEventBridge`.
    policy.unknown_tool = UnknownToolPolicy::ReturnToolError;
    // Registered tools with schema-invalid arguments should produce a tool
    // error the model can correct, not abort the entire run. TinyAgents 2.1
    // owns this admission behavior directly; the former host SchemaGuard had
    // to manufacture valid stub arguments only because this policy was left at
    // its historical fail-fast default.
    policy.invalid_args = InvalidArgsPolicy::ReturnToolError;
    // Prompt-prefix protection is always on (issue #4249, 03.2). Two things
    // ride on it, and both were inert until the harness started stamping this
    // effective policy onto the outgoing request (tinyagents `model_call`):
    //   * the `PromptCacheGuardMiddleware` records a `CacheLayoutEvent` whenever
    //     volatile content busts the provider KV-cache prefix (diagnostic), and
    //   * the loop injects a `prompt_cache_key` routing hint derived from the
    //     stable prefix into `provider_options`, and the provider adapters see
    //     `protect_prompt_prefix` and emit explicit `cache_control` breakpoints
    //     where the provider needs them (native Anthropic, OpenRouter relays).
    // The stable prefix itself is declared per request by the host
    // `PromptCacheSegmentMiddleware`.
    policy.cache.protect_prompt_prefix = true;
    // Response caching is gated: it is enabled only for deterministic internal
    // runs (which additionally attach a `ResponseCache`). Interactive chat turns
    // pass `false` here AND attach no cache, so a live user turn can never be
    // served a cached model response (double fail-safe).
    policy.cache.response_cache_enabled = response_cache_enabled;
    // Payload capture ON: the loop stamps request messages + completion onto
    // `ModelCompleted` and tool arguments + result onto `ToolCompleted`, which
    // the `OpenhumanEventBridge` projects into content-bearing `AgentProgress`
    // events (generation/tool span input+output in trace exports). Privacy
    // posture is unchanged off-device: the durable journal passes through a
    // `RedactingSink` (on-device, same data class as the threads DB, which
    // already persists full conversations + tool output), and the Langfuse
    // exporter withholds all content unless
    // `observability.agent_tracing.capture_content` is on.
    policy.capture = tinyagents_harness::runtime::PayloadCapture::all();
    policy
}

/// Consecutive identical tool failures that trip the repeated-failure circuit
/// breaker (see `middleware::RepeatedToolFailureMiddleware`). Three matches the
/// legacy progress-guard's tolerance before it halted a stuck loop.
pub(crate) const REPEATED_TOOL_FAILURE_THRESHOLD: usize = 3;

/// Legacy default model-call cap used when a caller passes `max_iterations == 0`
/// to request "unset" (native-bus / test callers relied on the old loop treating
/// `max_tool_iterations == 0` as the default of 10). Passing `0` straight through
/// would set the harness `max_model_calls` to zero and abort before the first
/// provider call, so the runners normalize `0` to this value.
const DEFAULT_MAX_ITERATIONS: usize = 10;

/// Normalize a caller-supplied iteration cap: `0` means "unset" → the default.
pub(crate) fn effective_max_iterations(max_iterations: usize) -> usize {
    if max_iterations == 0 {
        DEFAULT_MAX_ITERATIONS
    } else {
        max_iterations
    }
}

/// True when `name` is a sub-agent spawn/delegation tool that a **child** run
/// must never be able to invoke (issue #4452), re-asserted at registration as
/// defense-in-depth so a misconfigured allowlist cannot reintroduce sub-agent
/// spawning into a nested run. Kept local to this seam (rather than importing
/// the `pub(super)` runner helper) so the invariant travels with the
/// registration site that enforces it.
///
/// **This is a strict subset of the caller-side strip**, not a mirror of it
/// (issue #6157). `subagent_runner::tool_prep::is_subagent_spawn_tool` also
/// resolves each archetype's `delegate_name` override through the definition
/// registry — `plan`, `research`, `run_code`, `review_code`, … — none of which
/// carry the `delegate_` prefix this match relies on. Matching them here would
/// put a registry lookup on the per-tool registration loop, so the caller
/// stays responsible for the override names: every path that feeds `allowed`
/// runs `is_subagent_spawn_tool` first, including the dynamic per-spawn tools
/// (`subagent_runner::ops::runner`). Widen this predicate in lockstep if that
/// ever stops being true.
pub(crate) fn is_subagent_spawn_or_delegate_tool(name: &str) -> bool {
    name == "spawn_subagent"
        || name.starts_with("delegate_")
        || name == "agent_prepare_context"
        || name == "spawn_worker_thread"
}

#[cfg(test)]
#[path = "turn_policy_budget_tests.rs"]
mod budget_tests;
