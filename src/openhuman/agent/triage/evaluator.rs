//! Build the turn, dispatch `agent.run_turn`, parse the reply.
//!
//! This is the core of the triage pipeline. It implements a tiered
//! fallback chain (issue #1257):
//!
//! ```text
//! cloud (initial)
//!   ├── 429 / transient (5xx / timeout / connection) ──► retry once
//!   │       └── still failing ──► local fallback
//!   └── ok ──► resolution_path = Cloud | CloudAfterRetry
//!
//! local fallback
//!   ├── ok ──► resolution_path = LocalFallback
//!   └── failed ──► TriageOutcome::Deferred { until_ms, reason }
//! ```
//!
//! Non-transient cloud failures (auth, malformed prompt, model not
//! found, parse failure) bubble up immediately — there's no point
//! retrying them and the local arm wouldn't help either.
//!
//! ## Why `run_tool_call_loop` doesn't care about `tools_registry = []`
//!
//! The triage agent has `named = []` in its TOML (zero tools). The
//! `run_tool_call_loop` implementation in
//! `src/openhuman/agent/harness/tool_loop.rs` handles an empty registry
//! by just doing a plain `chat_with_history` under the hood — no tool
//! schemas are sent to the backend.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};

use crate::core::event_bus::{request_native_global, NativeRequestError};
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::harness::definition::{AgentDefinition, PromptSource};
use crate::openhuman::agent::harness::AgentDefinitionRegistry;
use crate::openhuman::config::Config;
use crate::openhuman::config::MultimodalConfig;
use crate::openhuman::providers::reliable::{
    is_rate_limited, is_upstream_unhealthy, parse_retry_after_ms,
};
use crate::openhuman::providers::ChatMessage;
use crate::openhuman::scheduler_gate::LlmPermit;

use super::decision::{parse_triage_decision, ParseError, TriageDecision};
use super::envelope::TriggerEnvelope;
use super::events;
use super::routing::{
    build_local_provider_with_config, resolve_provider_with_config, ResolvedProvider,
};

/// Agent definition id for the built-in triage classifier.
pub const TRIGGER_TRIAGE_AGENT_ID: &str = "trigger_triage";

/// How much of the raw payload we inline into the user message.
const PAYLOAD_INLINE_LIMIT_BYTES: usize = 8 * 1024;

/// Cap on how long to wait for a server-supplied `Retry-After` before
/// giving up on the cloud arm and falling through to local. Mirrors
/// the cap in `ReliableProvider::compute_backoff`.
const RETRY_AFTER_CAP: Duration = Duration::from_millis(30_000);

/// Default backoff for transient (non-rate-limit) cloud failures
/// before the single retry. Short enough to keep tail latency
/// bounded; long enough for a wedged TCP connection to give up.
const TRANSIENT_BACKOFF: Duration = Duration::from_millis(500);

/// How far in the future a Deferred outcome asks the caller to retry.
/// A short tick mirrors the issue's "next tick retries the whole
/// chain" language — long enough to shed a thundering herd, short
/// enough that user-visible latency on transient outages stays in the
/// tens of seconds.
const DEFER_WAKEUP_MS: i64 = 30_000;

/// Which arm produced this triage decision. Surfaced on `TriageRun`
/// so the orchestrator can colour-code degraded turns and show the
/// state in `/debug` views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageResolutionPath {
    /// Cloud succeeded on the initial attempt.
    Cloud,
    /// Cloud succeeded on the retry after a 429 / transient failure.
    CloudAfterRetry,
    /// Cloud failed twice; the local arm produced the decision.
    LocalFallback,
}

impl TriageResolutionPath {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::CloudAfterRetry => "cloud-after-retry",
            Self::LocalFallback => "local-fallback",
        }
    }
}

/// Final output of a single triage run when a decision was produced.
#[derive(Debug, Clone)]
pub struct TriageRun {
    pub decision: TriageDecision,
    /// `true` when the producing arm was local — kept for telemetry
    /// compatibility with subscribers that read this field. Equivalent
    /// to `resolution_path == LocalFallback`.
    pub used_local: bool,
    pub latency_ms: u64,
    pub resolution_path: TriageResolutionPath,
}

/// Outcome of [`run_triage`]. Either a parsed decision or a
/// deferral asking the caller to retry the whole chain after
/// `defer_until_ms` (Unix epoch millis).
#[derive(Debug, Clone)]
pub enum TriageOutcome {
    Decision(TriageRun),
    Deferred {
        /// Unix epoch millis at which the caller should re-run the
        /// triage chain.
        defer_until_ms: i64,
        /// Short human-readable reason — already scrubbed; safe to log.
        reason: String,
    },
}

impl TriageOutcome {
    pub fn into_decision(self) -> Option<TriageRun> {
        match self {
            TriageOutcome::Decision(run) => Some(run),
            TriageOutcome::Deferred { .. } => None,
        }
    }
}

/// Run the triage classifier with the full tiered fallback chain.
///
/// 1. Resolve the cloud provider.
/// 2. Try cloud; on 429 / transient, sleep and retry once.
/// 3. On a second 429 / transient, build the local provider and
///    fall back to it (acquiring the global LLM permit).
/// 4. On local failure, return `TriageOutcome::Deferred` so the
///    caller (typically a trigger-handler RPC) can reschedule.
pub async fn run_triage(envelope: &TriggerEnvelope) -> anyhow::Result<TriageOutcome> {
    let config = Config::load_or_init()
        .await
        .context("loading config for triage turn")?;
    let cloud = resolve_provider_with_config(&config)
        .await
        .context("resolving provider for triage turn")?;
    let local = build_local_provider_with_config(&config);

    let outcome = run_triage_with_arms_inner(cloud, local, envelope, || {
        crate::openhuman::scheduler_gate::wait_for_capacity()
    })
    .await;
    if let Err(err) = &outcome {
        events::publish_failed(envelope, &format!("{err}"));
    }
    outcome
}

/// Production entry point that takes already-resolved arms and acquires
/// the global LLM permit via [`scheduler_gate::wait_for_capacity`].
///
/// Use [`run_triage_with_arms_for_test`] in tests to bypass the shared
/// semaphore. This function is `pub` for integration callers outside
/// this module that supply pre-resolved providers.
pub async fn run_triage_with_arms(
    cloud: ResolvedProvider,
    local: Option<ResolvedProvider>,
    envelope: &TriggerEnvelope,
) -> anyhow::Result<TriageOutcome> {
    run_triage_with_arms_inner(cloud, local, envelope, || {
        crate::openhuman::scheduler_gate::wait_for_capacity()
    })
    .await
}

/// Test-only entry point: skip the global LLM permit acquisition so the
/// triage tests don't contend with `scheduler_gate`'s process-wide
/// 1-slot semaphore or get trapped by a stale `Paused` policy left in
/// `STATE` by another test's `init_global` call.
#[cfg(test)]
pub async fn run_triage_with_arms_for_test(
    cloud: ResolvedProvider,
    local: Option<ResolvedProvider>,
    envelope: &TriggerEnvelope,
) -> anyhow::Result<TriageOutcome> {
    run_triage_with_arms_inner(cloud, local, envelope, || async { None }).await
}

/// Core implementation of the tiered cloud→retry→local fallback.
///
/// `acquire_permit` is called exactly once, on the local-fallback arm,
/// to obtain the global LLM permit. Production callers pass
/// `scheduler_gate::wait_for_capacity`; tests pass `|| async { None }`
/// to skip the shared semaphore.
async fn run_triage_with_arms_inner<F, Fut>(
    cloud: ResolvedProvider,
    local: Option<ResolvedProvider>,
    envelope: &TriggerEnvelope,
    acquire_permit: F,
) -> anyhow::Result<TriageOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<LlmPermit>>,
{
    // Track whether the cloud arm bailed because of user budget so the
    // eventual Deferred reason explains *why* we're sitting idle rather
    // than the generic "both arms failed" copy.
    let mut cloud_budget_exhausted: Option<anyhow::Error> = None;

    // ── Cloud arm ──────────────────────────────────────────────────
    match try_arm(&cloud, envelope, TriageResolutionPath::Cloud).await {
        Ok(run) => return Ok(TriageOutcome::Decision(run)),
        Err(ArmError::Fatal(err)) => return Err(err),
        Err(ArmError::BudgetExhausted(err)) => {
            tracing::warn!(
                source = %envelope.source.slug(),
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                path = TriageResolutionPath::Cloud.as_str(),
                error = %err,
                "[triage::evaluator] cloud rejected for budget; \
                 skipping retry and falling back to local arm"
            );
            cloud_budget_exhausted = Some(err);
        }
        Err(ArmError::Retryable { retry_after_ms, .. }) => {
            // Sleep before the cloud retry. Honour Retry-After when
            // present; otherwise use a short backoff so the second
            // attempt has a real chance of finding the upstream
            // recovered.
            let sleep_ms = retry_after_ms
                .map(|ms| Duration::from_millis(ms).min(RETRY_AFTER_CAP))
                .unwrap_or(TRANSIENT_BACKOFF);
            tracing::info!(
                sleep_ms = sleep_ms.as_millis() as u64,
                had_retry_after = retry_after_ms.is_some(),
                "[triage::evaluator] cloud retry pending after retryable failure"
            );
            tokio::time::sleep(sleep_ms).await;

            match try_arm(&cloud, envelope, TriageResolutionPath::CloudAfterRetry).await {
                Ok(run) => return Ok(TriageOutcome::Decision(run)),
                Err(ArmError::Fatal(err)) => return Err(err),
                Err(ArmError::BudgetExhausted(err)) => {
                    tracing::warn!(
                        source = %envelope.source.slug(),
                        label = %envelope.display_label,
                        external_id = %envelope.external_id,
                        path = TriageResolutionPath::CloudAfterRetry.as_str(),
                        error = %err,
                        "[triage::evaluator] cloud rejected for budget on retry; \
                         falling back to local arm"
                    );
                    cloud_budget_exhausted = Some(err);
                }
                Err(ArmError::Retryable { .. }) => {
                    // Exhausted cloud budget — fall through to local.
                    tracing::warn!(
                        "[triage::evaluator] cloud retry budget exhausted; \
                         falling back to local arm"
                    );
                }
            }
        }
    }

    // ── Local fallback ─────────────────────────────────────────────
    let Some(local) = local else {
        // No local arm available at all (runtime disabled, no model
        // configured) — the only honest outcome is a deferral so the
        // next tick retries the whole chain.
        //
        // `reason` is part of `TriageOutcome::Deferred` and may be
        // forwarded into telemetry / UI, so it must stay a stable,
        // scrubbed string. Raw upstream error text goes to the debug
        // log instead, where it is operator-visible but not surfaced.
        let reason = match &cloud_budget_exhausted {
            Some(err) => {
                tracing::debug!(
                    target: "[triage::evaluator]",
                    source = %envelope.source.slug(),
                    label = %envelope.display_label,
                    external_id = %envelope.external_id,
                    error = %err,
                    "cloud budget exhausted; no local arm — full upstream error"
                );
                "cloud budget exhausted; local arm unavailable".to_string()
            }
            None => "cloud retry exhausted; local arm unavailable".to_string(),
        };
        return Ok(TriageOutcome::Deferred {
            defer_until_ms: now_ms().saturating_add(DEFER_WAKEUP_MS),
            reason,
        });
    };

    // Hold the global LLM permit for the lifetime of the local turn —
    // protects laptop RAM from concurrent local model calls (#1073).
    let _gate_permit = acquire_permit().await;

    match try_arm(&local, envelope, TriageResolutionPath::LocalFallback).await {
        Ok(run) => Ok(TriageOutcome::Decision(run)),
        Err(ArmError::Fatal(err))
        | Err(ArmError::BudgetExhausted(err))
        | Err(ArmError::Retryable { source: err, .. }) => {
            // Local also failed — defer rather than surface a hard
            // error. Today's "hard fail" is the wrong default for a
            // transient blocker per #1257.
            //
            // `reason` is part of the public Deferred outcome and may
            // flow into telemetry / UI, so keep it scrubbed. Raw error
            // text from cloud + local lives in the structured warn
            // fields below — visible to operators, not callers.
            let reason = match cloud_budget_exhausted.as_ref() {
                Some(_) => "cloud budget exhausted; local arm also failed".to_string(),
                None => "cloud retry exhausted; local arm also failed".to_string(),
            };
            tracing::warn!(
                target: "[triage::evaluator]",
                source = %envelope.source.slug(),
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                local_error = %err,
                cloud_error = cloud_budget_exhausted
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_default(),
                defer_ms = DEFER_WAKEUP_MS,
                reason = %reason,
                "both arms failed; deferring"
            );
            Ok(TriageOutcome::Deferred {
                defer_until_ms: now_ms().saturating_add(DEFER_WAKEUP_MS),
                reason,
            })
        }
    }
}

/// Single-arm execution result. `Retryable` lets the orchestrator
/// decide whether to sleep + retry on the same arm (cloud) or to fall
/// through (local). `Fatal` short-circuits the whole chain.
enum ArmError {
    /// 429 / 5xx / timeout / connection — the kind of failure where
    /// trying again later might help.
    Retryable {
        retry_after_ms: Option<u64>,
        source: anyhow::Error,
    },
    /// Auth failure, missing model, prompt parse error, registry
    /// missing, etc. — retry / fallback would not change the result.
    Fatal(anyhow::Error),
    /// Cloud upstream rejected the call because the user is out of
    /// budget / credits. Retrying the cloud arm would just burn the
    /// same wall, but the local arm has no upstream cost — so we
    /// skip cloud retry, try local, and defer if local also fails.
    /// This is **not** a fatal error: the user takes an explicit
    /// action (top up) to fix it, so it must not page Sentry.
    BudgetExhausted(anyhow::Error),
}

/// Run a single arm: dispatch the agent turn through the native bus
/// and parse the reply. Classifies any error so the caller can decide
/// what to do next.
async fn try_arm(
    resolved: &ResolvedProvider,
    envelope: &TriggerEnvelope,
    intended_path: TriageResolutionPath,
) -> Result<TriageRun, ArmError> {
    let started = Instant::now();

    tracing::debug!(
        source = %envelope.source.slug(),
        label = %envelope.display_label,
        external_id = %envelope.external_id,
        provider = %resolved.provider_name,
        used_local = resolved.used_local,
        path = intended_path.as_str(),
        "[triage::evaluator] starting triage turn"
    );

    let registry = AgentDefinitionRegistry::global().ok_or_else(|| {
        ArmError::Fatal(anyhow!(
            "AgentDefinitionRegistry not initialised — did startup wiring \
             skip `init_global`?"
        ))
    })?;
    let definition = registry.get(TRIGGER_TRIAGE_AGENT_ID).ok_or_else(|| {
        ArmError::Fatal(anyhow!(
            "built-in `{TRIGGER_TRIAGE_AGENT_ID}` definition missing from registry"
        ))
    })?;

    let system_prompt = extract_inline_prompt(definition).ok_or_else(|| {
        ArmError::Fatal(anyhow!(
            "trigger_triage agent definition must ship an inline prompt body"
        ))
    })?;
    let user_message = render_user_message(envelope);
    let history = vec![
        ChatMessage::system(&system_prompt),
        ChatMessage::user(&user_message),
    ];

    let request = AgentTurnRequest {
        provider: Arc::clone(&resolved.provider),
        history,
        tools_registry: Arc::new(Vec::new()),
        provider_name: resolved.provider_name.clone(),
        model: resolved.model.clone(),
        temperature: definition.temperature,
        silent: true,
        channel_name: "triage".to_string(),
        multimodal: MultimodalConfig::default(),
        max_tool_iterations: 1,
        on_delta: None,
        target_agent_id: Some("trigger_triage".to_string()),
        visible_tool_names: None,
        extra_tools: Vec::new(),
        on_progress: None,
    };

    let response = match request_native_global::<AgentTurnRequest, AgentTurnResponse>(
        AGENT_RUN_TURN_METHOD,
        request,
    )
    .await
    {
        Ok(r) => r,
        Err(err) => {
            let message = match &err {
                NativeRequestError::HandlerFailed { message, .. } => message.clone(),
                other => format!("[agent.run_turn dispatch] {other}"),
            };
            tracing::warn!(
                error = %message,
                path = intended_path.as_str(),
                "[triage::evaluator] agent turn dispatch failed"
            );
            return Err(classify_error(message));
        }
    };

    let decision = match parse_triage_decision(&response.text) {
        Ok(d) => d,
        Err(parse_err) => {
            tracing::warn!(
                error = %parse_err,
                reply_chars = response.text.chars().count(),
                path = intended_path.as_str(),
                "[triage::evaluator] classifier reply did not parse"
            );
            // A parse failure means the model produced unusable
            // output. Retrying the same arm with the same prompt
            // won't help, but on the *cloud* arm a parse failure is
            // worth retrying once because the cloud model can be
            // non-deterministic across calls. On the local arm we've
            // already exhausted cloud and would just spin — treat it
            // as fatal so the chain progresses to Deferred.
            return Err(match intended_path {
                TriageResolutionPath::Cloud => ArmError::Retryable {
                    retry_after_ms: None,
                    source: anyhow!(
                        "classifier reply did not parse: {}",
                        format_parse_error(&parse_err)
                    ),
                },
                _ => ArmError::Fatal(anyhow!(
                    "classifier reply did not parse on {} arm: {}",
                    intended_path.as_str(),
                    format_parse_error(&parse_err)
                )),
            });
        }
    };

    let latency_ms = started.elapsed().as_millis() as u64;
    let used_local = matches!(intended_path, TriageResolutionPath::LocalFallback);
    tracing::info!(
        source = %envelope.source.slug(),
        action = %decision.action.as_str(),
        path = intended_path.as_str(),
        latency_ms = latency_ms,
        "[triage::evaluator] classifier decision produced"
    );

    Ok(TriageRun {
        decision,
        used_local,
        latency_ms,
        resolution_path: intended_path,
    })
}

/// Classify a handler-failure message string from the agent bus into
/// either a retryable (sleep + try again) or fatal (give up) error.
fn classify_error(message: String) -> ArmError {
    let err = anyhow!("{message}");
    if is_rate_limited(&err) {
        return ArmError::Retryable {
            retry_after_ms: parse_retry_after_ms(&err),
            source: err,
        };
    }
    if is_upstream_unhealthy(&err) || is_transient_string(&message) {
        return ArmError::Retryable {
            retry_after_ms: None,
            source: err,
        };
    }
    // Budget-exceeded is technically a 400 (not 5xx/429), so the
    // generic transient checks above won't catch it — but it is a
    // user-actionable upstream blocker, not a code bug, so we route
    // it through `BudgetExhausted` to avoid Sentry pages.
    if is_inference_budget_exceeded(&message) {
        return ArmError::BudgetExhausted(err);
    }
    ArmError::Fatal(err)
}

/// Returns `true` when `message` signals that the upstream rejected the
/// call because the user's inference budget or credit balance is empty —
/// meaning a retry would hit the same wall.
///
/// The vocabulary matches the OpenHuman backend's error copy and common
/// third-party provider phrasing. It does **not** mirror the
/// *semantics* of `channels/providers/web.rs` (a different code path);
/// it is an independent, conservative allowlist evaluated inline so the
/// triage evaluator carries no cross-domain import.
///
/// Kept conservative on purpose: a false positive would silently
/// reclassify a real `Fatal` error as `BudgetExhausted`, hiding it from
/// Sentry.
fn is_inference_budget_exceeded(message: &str) -> bool {
    // Normalize: lowercase, replace non-alphanumeric with spaces, then
    // split into whitespace-separated tokens. This lets us do
    // whole-word matching: a raw `contains("top up")` against the
    // normalized text would also fire on "stop updating" (which
    // contains the substring "top up" across word boundaries).
    let normalized: String = message
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = normalized.split_whitespace().collect();
    const NEEDLES: &[&str] = &[
        "budget exceeded",
        "budget exceeds",
        "top up",
        "add credits",
        "out of credits",
        "no remaining credits",
    ];
    NEEDLES.iter().any(|needle| {
        let needle_tokens: Vec<&str> = needle.split_whitespace().collect();
        if needle_tokens.is_empty() || words.len() < needle_tokens.len() {
            return false;
        }
        words
            .windows(needle_tokens.len())
            .any(|window| window == needle_tokens.as_slice())
    })
}

/// Heuristic for transient cloud failures the provider stack didn't
/// already classify — connection resets, timeouts, generic 5xx text.
/// Mirrors the conservative match shape used by `is_upstream_unhealthy`.
fn is_transient_string(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    let hints = [
        "timed out",
        "timeout",
        "connection",
        "connect error",
        "broken pipe",
        "reset by peer",
        "deadline exceeded",
        "temporarily unavailable",
    ];
    if hints.iter().any(|h| lower.contains(h)) {
        return true;
    }
    // Bare 5xx in the message body. Be careful not to match arbitrary
    // numerals — only treat 5xx as transient.
    for token in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if (500..600).contains(&code) {
                return true;
            }
        }
    }
    false
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn extract_inline_prompt(def: &AgentDefinition) -> Option<String> {
    match &def.system_prompt {
        PromptSource::Inline(body) if !body.is_empty() => Some(body.clone()),
        PromptSource::Dynamic(build) => {
            use crate::openhuman::context::prompt::{
                ConnectedIntegration, LearnedContextData, PromptContext, PromptTool, ToolCallFormat,
            };
            let empty_tools: Vec<PromptTool<'_>> = Vec::new();
            let empty_integrations: Vec<ConnectedIntegration> = Vec::new();
            let empty_visible: std::collections::HashSet<String> = std::collections::HashSet::new();
            let ctx = PromptContext {
                workspace_dir: std::path::Path::new("."),
                model_name: "",
                agent_id: &def.id,
                tools: &empty_tools,
                skills: &[],
                dispatcher_instructions: "",
                learned: LearnedContextData::default(),
                visible_tool_names: &empty_visible,
                tool_call_format: ToolCallFormat::PFormat,
                connected_integrations: &empty_integrations,
                connected_identities_md: String::new(),
                include_profile: false,
                include_memory_md: false,
                curated_snapshot: None,
                user_identity: None,
            };
            match build(&ctx) {
                Ok(body) if !body.is_empty() => Some(body),
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(
                        agent_id = %def.id,
                        error = %e,
                        "[triage::evaluator] dynamic prompt builder failed"
                    );
                    None
                }
            }
        }
        _ => None,
    }
}

fn render_user_message(envelope: &TriggerEnvelope) -> String {
    let payload_string = truncate_payload(&envelope.payload, PAYLOAD_INLINE_LIMIT_BYTES);
    format!(
        "SOURCE: {source}\n\
         DISPLAY_LABEL: {label}\n\
         EXTERNAL_ID: {eid}\n\
         PAYLOAD:\n{payload}",
        source = envelope.source.slug(),
        label = envelope.display_label,
        eid = envelope.external_id,
        payload = payload_string,
    )
}

fn format_parse_error(err: &ParseError) -> String {
    match err {
        ParseError::NoJsonObject => "classifier reply had no JSON object".to_string(),
        ParseError::InvalidJson(src) => format!("classifier JSON invalid: {src}"),
        ParseError::MissingTarget { action } => {
            format!("action `{action}` missing required target_agent/prompt")
        }
    }
}

fn truncate_payload(payload: &serde_json::Value, max_bytes: usize) -> String {
    let pretty = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    if pretty.len() <= max_bytes {
        return pretty;
    }
    let dropped = pretty.len() - max_bytes;
    let end = crate::openhuman::util::floor_char_boundary(&pretty, max_bytes);
    format!("{}\n[...truncated {dropped} bytes]", &pretty[..end])
}

#[cfg(test)]
#[path = "evaluator_tests.rs"]
mod tests;
