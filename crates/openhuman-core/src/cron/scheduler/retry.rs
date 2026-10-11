//! The per-job retry loop with exponential backoff, its permanent-halt
//! short-circuits, and the retries-exhausted observability report.

use super::agent_run::{run_agent_job_for_run, run_flow_schedule_job};
use super::failure_classification::{
    is_api_key_unset_failure, is_budget_exhausted_failure, is_insufficient_credits_failure,
    is_local_provider_unreachable_failure, is_session_expired_failure, permanent_halt_message,
};
use super::shell_job::run_job_command;
use crate::config::Config;
use crate::security::SecurityPolicy;
use chrono::Utc;
use tokio::time::{self, Duration};
use {
    tinyflows_schedule::types::CronJob, tinyflows_schedule::types::JobType,
    tinyflows_schedule::types::SessionTarget,
};

pub(super) fn agent_session_target_tag(target: &SessionTarget) -> &'static str {
    match target {
        SessionTarget::Main => "main",
        SessionTarget::Isolated => "isolated",
        SessionTarget::Current => "current",
    }
}

/// Run `job` now under the caller's `run_id` (also the id its delivery and
/// run record use).
pub async fn execute_job_now(config: &Config, job: &CronJob, run_id: &str) -> (bool, String) {
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    execute_job_with_retry_for_run(config, &security, job, run_id).await
}

/// [`execute_job_with_retry_for_run`] under a fresh run id.
#[cfg(test)]
pub(super) async fn execute_job_with_retry(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    let run_id = uuid::Uuid::new_v4().to_string();
    execute_job_with_retry_for_run(config, security, job, &run_id).await
}

pub(super) async fn execute_job_with_retry_for_run(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
    run_id: &str,
) -> (bool, String) {
    let mut last_output = String::new();
    let mut last_agent_error: Option<String> = None;
    // A per-job override (`cron::policy`) wins; `Some(0)` is one attempt. An
    // unreadable policy refuses the run (and any later attempt) outright.
    if let Err(error) = crate::cron::limits::check_run(config, job) {
        return (false, error.to_string());
    }
    let policy = match crate::cron::policy::get_policy(config, &job.id) {
        Ok(policy) => policy,
        Err(error) => return (false, error.to_string()),
    };
    let retries = crate::cron::policy::effective_retries(config, &policy);
    let mut backoff_ms = config.reliability.provider_backoff_ms.max(200);
    let mut session_expired = false;
    let mut credits_exhausted = false;
    let mut budget_exhausted = false;
    let mut key_unset = false;
    let mut local_unreachable = false;

    for attempt in 0..=retries {
        if attempt > 0 {
            if let Err(error) = crate::cron::policy::get_policy(config, &job.id) {
                return (false, error.to_string());
            }
        }
        let (success, output, agent_error) = match job.job_type {
            JobType::Shell => {
                let (success, output) = run_job_command(config, security, job).await;
                (success, output, None)
            }
            JobType::Agent => run_agent_job_for_run(config, job, run_id).await,
            JobType::Flow => {
                let (success, output) = run_flow_schedule_job(job).await;
                (success, output, None)
            }
        };
        last_output = output;
        if agent_error.is_some() {
            last_agent_error = agent_error;
        }

        if success {
            return (true, last_output);
        }

        if last_output.starts_with("blocked by security policy:") {
            // Deterministic policy violations are not retryable.
            return (false, last_output);
        }

        if is_session_expired_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — the inference layer already
            // published `SessionExpired`, retries cannot recover until the
            // user re-auths, and the classifier considers this expected
            // user state (TAURI-RUST-N). See `is_session_expired_failure`
            // for the full rationale.
            session_expired = true;
            break;
        }

        if is_insufficient_credits_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — a BYO provider 402 (out of
            // balance) is permanent across the backoff loop, and the
            // provider emit site already demoted it from Sentry. Skipping
            // the retries-exhausted `report_error` below keeps the residual
            // off Sentry at source, independent of the `before_send` chain
            // (TAURI-RUST-514). See `is_insufficient_credits_failure`.
            // Metadata-only log (no raw provider body — see CLAUDE.md).
            log::debug!(
                "[cron] action=halt_on_insufficient_credits_402 job_id={} attempt={} retries={}",
                job.id.as_str(),
                attempt,
                retries
            );
            credits_exhausted = true;
            break;
        }

        if is_budget_exhausted_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — a managed-backend budget 400
            // (USER_INSUFFICIENT_CREDITS) is permanent across the backoff
            // loop. The tag-gated `is_budget_event` before_send filter never
            // matches this cron re-report, so suppressing the report here
            // keeps it off Sentry at source (TAURI-RUST-BMW). See
            // `is_budget_exhausted_failure`. Metadata-only log (no raw body).
            log::debug!(
                "[cron] action=halt_on_budget_exhausted_400 job_id={} attempt={} retries={}",
                job.id.as_str(),
                attempt,
                retries
            );
            budget_exhausted = true;
            break;
        }

        if is_api_key_unset_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — a configured provider with no
            // API key fails deterministically at the credential guard before
            // any HTTP, so the missing key is permanent across the backoff
            // loop. The bare cron `report_error` below bypasses the
            // `ApiKeyMissing` `expected_error_kind` demotion, so suppressing
            // here keeps the residual off Sentry at source (TAURI-RUST-HCK).
            // The failure stays visible to the user via the alerts tab
            // (`push_cron_alert`) + run history. See `is_api_key_unset_failure`.
            // Metadata-only log (no raw provider body — see CLAUDE.md).
            log::debug!(
                "[cron] action=halt_on_api_key_unset job_id={} attempt={} retries={}",
                job.id.as_str(),
                attempt,
                retries
            );
            key_unset = true;
            break;
        }

        if is_local_provider_unreachable_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — a local LLM provider refusing the
            // loopback connection (LM Studio / Ollama not running) cannot
            // recover across the backoff loop, and the provider/agent emit
            // sites already demoted it from Sentry (`LoopbackUnavailable`).
            // The bare cron `report_error` below bypasses that demotion, so
            // suppressing here keeps the residual off Sentry at source
            // (TAURI-RUST-12K). The failure stays visible via the run history
            // + cron alert. See `is_local_provider_unreachable_failure`.
            // Metadata-only log (no raw provider body — see CLAUDE.md).
            log::debug!(
                "[cron] action=halt_on_local_provider_unreachable job_id={} attempt={} retries={}",
                job.id.as_str(),
                attempt,
                retries
            );
            local_unreachable = true;
            break;
        }

        if attempt < retries {
            let jitter_ms = u64::from(Utc::now().timestamp_subsec_millis() % 250);
            time::sleep(Duration::from_millis(backoff_ms + jitter_ms)).await;
            backoff_ms = (backoff_ms.saturating_mul(2)).min(30_000);
        }
    }

    // Permanent user-config / billing states are demoted at source: halt the
    // loop and skip the retries-exhausted report, independent of the tag-gated
    // before_send filters that the cron re-report does not match. Covers BYO
    // 402 out-of-credit + managed-backend 400 out-of-budget (TAURI-RUST-514 /
    // -BMW) and a configured provider with no API key (TAURI-RUST-HCK). The
    // `session_expired` (TAURI-RUST-N) and `local_unreachable` (a local LLM
    // server refusing the loopback connection, TAURI-RUST-12K) halts are the
    // same shape — suppress the bypassing bare report — but carry no
    // user-config remediation surface, so they gate the report directly rather
    // than routing through `permanent_config_halt`'s UserErrorCenter swap.
    let permanent_config_halt = credits_exhausted || budget_exhausted || key_unset;
    if matches!(job.job_type, JobType::Agent)
        && !session_expired
        && !local_unreachable
        && !permanent_config_halt
    {
        let report_message = last_agent_error.as_deref().unwrap_or(last_output.as_str());
        crate::core::observability::report_error(
            report_message,
            "cron",
            "agent_job",
            &[
                ("job_id", job.id.as_str()),
                ("agent_id", job.agent_id.as_deref().unwrap_or("none")),
                (
                    "session_target",
                    agent_session_target_tag(&job.session_target),
                ),
                ("failure", "retries_exhausted"),
            ],
        );
    } else if matches!(job.job_type, JobType::Agent) && permanent_config_halt {
        // Suppressed the retries-exhausted Sentry report for a permanent
        // user-config / billing state. Metadata-only breadcrumb so the
        // suppression is diagnosable in production without the raw provider body.
        let (reason, user_error_kind) = if credits_exhausted {
            ("insufficient_credits_402", "insufficient_credits")
        } else if budget_exhausted {
            ("budget_exhausted_400", "budget_exceeded")
        } else {
            ("api_key_unset", "api_key_missing")
        };
        log::debug!(
            "[cron] action=suppress_retries_exhausted_report reason={reason} job_id={} retries={}",
            job.id.as_str(),
            retries
        );
        // Replace the generic agent-failure copy with the specific, actionable
        // (static, leak-safe) reason so the hoisted /notifications alert + run
        // history tell the user the exact next step rather than "Something went
        // wrong" (CodeRabbit #4169). The raw `last_agent_error` chain is NEVER
        // surfaced here — only the `&'static str` constants from
        // `permanent_halt_message`.
        last_output = permanent_halt_message(credits_exhausted, budget_exhausted).to_string();
        // Also surface the actionable state to the UserErrorCenter so the user
        // can fix it (add an API key / top up credits / raise the budget) even
        // with no chat thread open. Broadcast-only + metadata-only — see
        // `publish_cron_user_error` (#4165 / TAURI-RUST-HCK follow-up).
        publish_cron_user_error(user_error_kind);
    }

    (false, last_output)
}

/// Surface a permanent cron user-config / billing halt to every connected
/// client's UserErrorCenter.
///
/// Broadcasts a metadata-only `user_error` web-channel event to the `"system"`
/// room (which every socket auto-joins). The payload carries only the stable
/// `kind` token in `error_type` — one of `api_key_missing` / `insufficient_credits`
/// / `budget_exceeded`, mirroring the frontend `UserErrorKind` discriminator —
/// plus `error_source = "cron"`. It NEVER carries the raw provider body (see the
/// metadata-only rule in CLAUDE.md), so no secrets / PII leave the core.
///
/// The frontend `socketService` listens for `user_error` and routes it through
/// the same classifier the chat runtime uses, so a background (no-delivery) job
/// failure is no longer silent — it lands in the shell's UserErrorCenter with a
/// deep-link action even though no chat thread is active.
pub(super) fn publish_cron_user_error(kind: &str) {
    log::debug!("[cron] action=surface_user_error kind={kind}");
    crate::web_chat::publish_web_channel_event(crate::web_chat::WebChannelEvent {
        event: "user_error".to_string(),
        client_id: "system".to_string(),
        error_type: Some(kind.to_string()),
        error_source: Some("cron".to_string()),
        ..Default::default()
    });
}
