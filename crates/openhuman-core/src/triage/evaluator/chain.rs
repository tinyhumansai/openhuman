//! The tiered cloud → retry → local fallback orchestration.

use crate::core::runtime::DispatchCtx;
use std::future::Future;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Context;

use crate::platform::llm_gate::LlmPermit;
use tinyagents_harness::retry::{
    decide, BreakerRejection, FailoverDecision, FailoverReason, FailoverState,
    PersistentFailoverAttempt, PersistentFailoverBreaker, PersistentFailoverBreakerConfig,
};

use super::super::envelope::TriggerEnvelope;
use super::super::events;
use super::super::routing::{
    build_local_provider_with_config, resolve_provider_with_config, ResolvedProvider,
};
use super::arm::{try_arm, ArmError};
use super::outcome::{TriageOutcome, TriageResolutionPath};

/// Cap on how long to wait for a server-supplied `Retry-After` before
/// giving up on the cloud arm and falling through to local. Mirrors
/// the cap in `ReliableProvider::compute_backoff`.
const RETRY_AFTER_CAP: Duration = Duration::from_millis(30_000);

/// Default backoff for transient (non-rate-limit) cloud failures
/// before the single retry. Short enough to keep tail latency
/// bounded; long enough for a wedged TCP connection to give up.
const TRANSIENT_BACKOFF: Duration = Duration::from_millis(500);

const OUTAGE_BACKOFF_BASE_MS: i64 = 30_000;
static TRIAGE_MONOTONIC_START: OnceLock<Instant> = OnceLock::new();
pub(crate) type RetryState = PersistentFailoverBreaker<String>;

pub(crate) fn new_retry_state() -> RetryState {
    PersistentFailoverBreaker::new(PersistentFailoverBreakerConfig {
        base_backoff_ms: OUTAGE_BACKOFF_BASE_MS as u64,
        max_backoff_ms: (15 * 60_000) as u64,
        failure_limit: 8,
    })
}

static TRIAGE_RETRY_STATE: OnceLock<RetryState> = OnceLock::new();

fn retry_state() -> &'static RetryState {
    TRIAGE_RETRY_STATE.get_or_init(new_retry_state)
}

fn monotonic_ms() -> u64 {
    TRIAGE_MONOTONIC_START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

/// Run the triage classifier with the full tiered fallback chain.
///
/// 1. Resolve the cloud provider.
/// 2. Try cloud; on 429 / transient, sleep and retry once.
/// 3. On a second 429 / transient, build the local provider and
///    fall back to it (acquiring the global LLM permit).
/// 4. If no local arm exists, return `TriageOutcome::Terminal` so the
///    caller does not create another fixed-interval cloud invocation.
/// 5. On local failure, return `TriageOutcome::Deferred` so the
///    caller can retry when a local arm is available.
pub async fn run_triage(
    ctx: &DispatchCtx,
    envelope: &TriggerEnvelope,
) -> anyhow::Result<TriageOutcome> {
    let config = crate::config::ops::load_current_or_init(ctx)
        .await
        .context("loading config for triage turn")?;
    let cloud = resolve_provider_with_config(ctx, &config)
        .await
        .context("resolving provider for triage turn")?;
    let local = build_local_provider_with_config(ctx, &config);

    let outcome = run_triage_with_arms_inner(cloud, local, envelope, Some(retry_state()), || {
        crate::platform::llm_gate::wait_for_capacity()
    })
    .await;
    if let Err(err) = &outcome {
        events::publish_failed(envelope, &format!("{err}"));
    }
    outcome
}

/// Test-only entry point: skip the global LLM permit acquisition so the
/// triage tests don't contend with `llm_gate`'s process-wide
/// 1-slot semaphore or get trapped by a stale `Paused` policy left in
/// `STATE` by another test's `init_global` call.
#[cfg(test)]
pub async fn run_triage_with_arms_for_test(
    cloud: ResolvedProvider,
    local: Option<ResolvedProvider>,
    envelope: &TriggerEnvelope,
) -> anyhow::Result<TriageOutcome> {
    run_triage_with_arms_inner(cloud, local, envelope, None, || async { None }).await
}

#[cfg(test)]
pub(crate) async fn run_triage_with_arms_for_test_with_state(
    cloud: ResolvedProvider,
    local: Option<ResolvedProvider>,
    envelope: &TriggerEnvelope,
    state: &RetryState,
) -> anyhow::Result<TriageOutcome> {
    run_triage_with_arms_inner(cloud, local, envelope, Some(state), || async { None }).await
}

/// Core implementation of the tiered cloud→retry→local fallback.
///
/// `acquire_permit` is called exactly once, on the local-fallback arm,
/// to obtain the global LLM permit. Production callers pass
/// `llm_gate::wait_for_capacity`; tests pass `|| async { None }`
/// to skip the shared semaphore.
async fn run_triage_with_arms_inner<F, Fut>(
    cloud: ResolvedProvider,
    local: Option<ResolvedProvider>,
    envelope: &TriggerEnvelope,
    retry_state: Option<&RetryState>,
    acquire_permit: F,
) -> anyhow::Result<TriageOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<LlmPermit>>,
{
    let retry_key = format!("{}:{}", cloud.provider_name, cloud.model);
    let mut outage_attempt = if local.is_none() {
        if let Some(state) = retry_state {
            match state.try_acquire(retry_key.clone(), monotonic_ms()) {
                Ok(attempt) => Some(attempt),
                Err(rejection) => {
                    let delta_ms = rejection_delay_ms(rejection, monotonic_ms());
                    return Ok(TriageOutcome::Deferred {
                        defer_until_ms: now_ms().saturating_add(delta_ms as i64),
                        reason: "managed backend paused; retry window not reached".to_string(),
                    });
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Track whether the cloud arm bailed because of user budget so the
    // eventual Deferred reason explains *why* we're sitting idle rather
    // than the generic "both arms failed" copy.
    let mut cloud_budget_exhausted: Option<anyhow::Error> = None;
    // Track whether the cloud arm bailed because the prompt-guard
    // flagged the content (OPENHUMAN-TAURI-X). The guard runs before
    // dispatch and is shared across arms, so a flag on cloud will
    // repeat on local — surface the verdict in the Deferred reason
    // for operator-facing telemetry.
    let mut cloud_safety_flagged: Option<anyhow::Error> = None;

    // ── Cloud arm ──────────────────────────────────────────────────
    match try_arm(&cloud, envelope, TriageResolutionPath::Cloud).await {
        Ok(run) => {
            clear_outage(outage_attempt.as_mut());
            return Ok(TriageOutcome::Decision(run));
        }
        Err(ArmError::Fatal(err)) => {
            clear_outage(outage_attempt.as_mut());
            return Err(err);
        }
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
        Err(ArmError::SafetyFlagged(err)) => {
            tracing::warn!(
                source = %envelope.source.slug(),
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                path = TriageResolutionPath::Cloud.as_str(),
                error = %err,
                "[triage::evaluator] cloud rejected by prompt-guard; \
                 skipping retry and falling back to local arm"
            );
            cloud_safety_flagged = Some(err);
        }
        Err(ArmError::Retryable {
            retry_after_ms,
            source,
        }) => {
            let reason = FailoverReason::classify(
                &tinyagents_harness::error::TinyAgentsError::Model(format!("{source:#}")),
            );
            let decision = decide(
                reason,
                FailoverState {
                    retryable: true,
                    attempts_remaining: true,
                    larger_window_available: false,
                },
            );
            if decision != FailoverDecision::RetrySame {
                tracing::info!(
                    failover_reason = reason.as_str(),
                    "owner failover policy skipped a same-provider retry"
                );
            } else {
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
                    Ok(run) => {
                        clear_outage(outage_attempt.as_mut());
                        return Ok(TriageOutcome::Decision(run));
                    }
                    Err(ArmError::Fatal(err)) => {
                        clear_outage(outage_attempt.as_mut());
                        return Err(err);
                    }
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
                    Err(ArmError::SafetyFlagged(err)) => {
                        tracing::warn!(
                            source = %envelope.source.slug(),
                            label = %envelope.display_label,
                            external_id = %envelope.external_id,
                            path = TriageResolutionPath::CloudAfterRetry.as_str(),
                            error = %err,
                            "[triage::evaluator] cloud rejected by prompt-guard on retry; \
                             falling back to local arm"
                        );
                        cloud_safety_flagged = Some(err);
                    }
                    Err(ArmError::Retryable { .. }) => {
                        // Exhausted cloud budget — fall through to local.
                        tracing::warn!(
                            "[triage::evaluator] cloud retry budget exhausted; \
                         evaluating fallback"
                        );
                    }
                }
            }
        }
    }

    // ── Local fallback ─────────────────────────────────────────────
    let Some(local) = local else {
        // No local arm is available (runtime disabled, no model configured).
        // This event has no executable fallback, so returning Deferred would
        // invite callers to repeat the same cloud outage indefinitely.
        //
        // `reason` is part of `TriageOutcome::Deferred` and may be
        // forwarded into telemetry / UI, so it must stay a stable,
        // scrubbed string. Raw upstream error text goes to the debug
        // log instead, where it is operator-visible but not surfaced.
        let reason = if let Some(err) = cloud_safety_flagged.as_ref() {
            tracing::debug!(
                target: "[triage::evaluator]",
                source = %envelope.source.slug(),
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                error = %err,
                "prompt-guard rejected on cloud; no local arm — full guard verdict"
            );
            "prompt-guard rejection; local arm unavailable".to_string()
        } else if let Some(err) = cloud_budget_exhausted.as_ref() {
            tracing::debug!(
                target: "[triage::evaluator]",
                source = %envelope.source.slug(),
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                error = %err,
                "cloud budget exhausted; no local arm — full upstream error"
            );
            "cloud budget exhausted; local arm unavailable".to_string()
        } else {
            "cloud retry exhausted; local arm unavailable".to_string()
        };
        if cloud_budget_exhausted.is_some() || cloud_safety_flagged.is_some() {
            clear_outage(outage_attempt.as_mut());
            return Ok(TriageOutcome::Terminal { reason });
        }

        return match record_outage(outage_attempt.as_mut()) {
            Some(TriageOutcome::Terminal { reason }) => Ok(TriageOutcome::Terminal { reason }),
            Some(TriageOutcome::Deferred {
                defer_until_ms,
                reason,
            }) => Ok(TriageOutcome::Deferred {
                defer_until_ms,
                reason,
            }),
            Some(TriageOutcome::Decision(_)) => unreachable!("outage recording cannot decide"),
            None => Ok(TriageOutcome::Terminal { reason }),
        };
    };

    // Hold the global LLM permit for the lifetime of the local turn —
    // protects laptop RAM from concurrent local model calls (#1073).
    let _gate_permit = acquire_permit().await;

    match try_arm(&local, envelope, TriageResolutionPath::LocalFallback).await {
        Ok(run) => {
            clear_outage(outage_attempt.as_mut());
            Ok(TriageOutcome::Decision(run))
        }
        Err(ArmError::Fatal(err))
        | Err(ArmError::BudgetExhausted(err))
        | Err(ArmError::SafetyFlagged(err))
        | Err(ArmError::Retryable { source: err, .. }) => {
            // Local also failed — defer rather than surface a hard
            // error. Today's "hard fail" is the wrong default for a
            // transient blocker per #1257.
            //
            // `reason` is part of the public Deferred outcome and may
            // flow into telemetry / UI, so keep it scrubbed. Raw error
            // text from cloud + local lives in the structured warn
            // fields below — visible to operators, not callers.
            let reason = if cloud_safety_flagged.is_some() {
                "prompt-guard rejection; local arm also failed".to_string()
            } else if cloud_budget_exhausted.is_some() {
                "cloud budget exhausted; local arm also failed".to_string()
            } else {
                "cloud retry exhausted; local arm also failed".to_string()
            };
            tracing::warn!(
                target: "[triage::evaluator]",
                source = %envelope.source.slug(),
                label = %envelope.display_label,
                external_id = %envelope.external_id,
                local_error = %err,
                cloud_error = cloud_budget_exhausted
                    .as_ref()
                    .or(cloud_safety_flagged.as_ref())
                    .map(|e| e.to_string())
                    .unwrap_or_default(),
                defer_ms = OUTAGE_BACKOFF_BASE_MS,
                reason = %reason,
                "both arms failed; deferring"
            );
            Ok(TriageOutcome::Deferred {
                defer_until_ms: now_ms().saturating_add(OUTAGE_BACKOFF_BASE_MS),
                reason,
            })
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn clear_outage(attempt: Option<&mut PersistentFailoverAttempt<String>>) {
    if let Some(attempt) = attempt {
        attempt.record_success();
    }
}

fn rejection_delay_ms(rejection: BreakerRejection, now_ms: u64) -> u64 {
    match rejection {
        BreakerRejection::Backoff {
            next_attempt_ms, ..
        } => next_attempt_ms.saturating_sub(now_ms).max(1_000),
        BreakerRejection::InFlight => 1_000,
    }
}

#[cfg(test)]
pub(crate) fn begin_outage_attempt(
    state: Option<&RetryState>,
    key: &str,
) -> Option<PersistentFailoverAttempt<String>> {
    state?.try_acquire(key.to_string(), monotonic_ms()).ok()
}

pub(crate) fn record_outage(
    attempt: Option<&mut PersistentFailoverAttempt<String>>,
) -> Option<TriageOutcome> {
    let attempt = attempt?;
    let recorded_at_ms = monotonic_ms();
    let Some(failure) = attempt.record_failure(recorded_at_ms) else {
        return Some(TriageOutcome::Deferred {
            defer_until_ms: now_ms().saturating_add(OUTAGE_BACKOFF_BASE_MS),
            reason: "managed backend recovered during another attempt".to_string(),
        });
    };
    let delay_ms = failure
        .next_attempt_ms
        .saturating_sub(recorded_at_ms)
        .min(i64::MAX as u64) as i64;
    if failure.failure_limit_reached {
        return Some(TriageOutcome::Terminal {
            reason: format!(
                "managed backend outage reached retry limit ({})",
                failure.failure_count
            ),
        });
    }
    Some(TriageOutcome::Deferred {
        defer_until_ms: now_ms().saturating_add(delay_ms),
        reason: format!("managed backend outage; retry {delay_ms}ms backoff"),
    })
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod outage_attempt_tests;
