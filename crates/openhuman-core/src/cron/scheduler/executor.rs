//! The host's answer to the module's `Execute` and `Failed` callbacks: one
//! attempt of a shell or agent job under the security policy and operator
//! limits, classified for the runtime.
//!
//! The module (`tinyflows_bus::schedule`) owns the retry loop and its backoff.
//! This executor runs the attempt through the job-kind registry
//! ([`crate::cron::handlers`]) and decides whether a failure is worth another
//! one: a policy block or a permanent user-config / billing state (expired
//! session, no credits, exhausted budget, unset key, unreachable local
//! provider) is returned `retryable: false`, and the final failure is reported
//! once through [`HostExecutor::failed`].

use super::failure_classification::{
    is_api_key_unset_failure, is_budget_exhausted_failure, is_insufficient_credits_failure,
    is_local_provider_unreachable_failure, is_session_expired_failure, permanent_halt_message,
};
use crate::cron::bus_host::{kind_refusal, Binding};
use crate::cron::handlers::{kind_handler, JobRunContext};
use tinyflows_bus::schedule::types::{CronJob, JobType, SessionTarget};
use tinyflows_bus::schedule::Attempt;

pub(crate) struct HostExecutor {
    binding: Binding,
}

impl HostExecutor {
    pub(crate) fn new(binding: Binding) -> Self {
        Self { binding }
    }

    async fn attempt(&self, job: &CronJob, run_id: &str) -> Attempt {
        let config = self.binding.config();
        self.binding.publish_triggered_once(job, run_id);
        // A hosted-profile sweep runs agent jobs and trusted memory jobs.
        if let Some(refusal) = kind_refusal(&self.binding, job) {
            return Attempt {
                success: false,
                output: refusal,
                retryable: false,
                raw_detail: None,
            };
        }
        // Operator limits (`cron::limits`) also bind rows stored before they
        // were switched on.
        if let Err(error) = crate::cron::limits::check_run(&config, job) {
            return Attempt {
                success: false,
                output: error.to_string(),
                retryable: false,
                raw_detail: None,
            };
        }
        let Some(handler) = kind_handler(&job.job_type) else {
            return Attempt {
                success: false,
                output: format!(
                    "no handler is registered for {} jobs",
                    job.job_type.as_str()
                ),
                retryable: false,
                raw_detail: None,
            };
        };
        tracing::debug!(
            job_id = %job.id,
            kind = job.job_type.as_str(),
            "[cron:executor] running attempt"
        );
        let outcome = handler
            .run(&JobRunContext::new(self.binding.clone()), job, run_id)
            .await;
        let (success, output, agent_error, permanent) = (
            outcome.success,
            outcome.output,
            outcome.agent_error,
            outcome.permanent,
        );
        if success {
            return Attempt {
                success: true,
                output,
                retryable: true,
                raw_detail: None,
            };
        }
        if permanent {
            return Attempt {
                success: false,
                output,
                retryable: false,
                raw_detail: agent_error,
            };
        }
        Self::classify_failure(job, output, agent_error.as_deref())
    }

    /// Whether a failed attempt may be retried, and the output to record.
    fn classify_failure(job: &CronJob, output: String, agent_error: Option<&str>) -> Attempt {
        let permanent = |output: String| Attempt {
            success: false,
            output,
            retryable: false,
            raw_detail: agent_error.map(str::to_string),
        };
        if output.starts_with("blocked by security policy:") {
            // Deterministic policy violations are not retryable.
            return permanent(output);
        }
        let job_type = &job.job_type;
        if is_session_expired_failure(job_type, agent_error, &output) {
            // The inference layer already published `SessionExpired`; retries
            // cannot recover until the user re-auths (TAURI-RUST-N).
            return permanent(output);
        }
        let user_error_kind = if is_insufficient_credits_failure(job_type, agent_error, &output) {
            Some("insufficient_credits")
        } else if is_budget_exhausted_failure(job_type, agent_error, &output) {
            Some("budget_exceeded")
        } else if is_api_key_unset_failure(job_type, agent_error, &output) {
            Some("api_key_missing")
        } else {
            None
        };
        if let Some(user_error_kind) = user_error_kind {
            tracing::debug!(job_id = %job.id, "[cron:executor] permanent user-config halt; not retrying");
            // The specific, actionable (static, leak-safe) copy replaces the
            // generic agent-failure text; the raw error chain is never shown.
            let message = permanent_halt_message(
                user_error_kind == "insufficient_credits",
                user_error_kind == "budget_exceeded",
            );
            // Also reach the UserErrorCenter so the user can fix it with no
            // chat thread open (metadata-only; #4165 / TAURI-RUST-HCK).
            publish_cron_user_error(user_error_kind);
            return permanent(message.to_string());
        }
        if is_local_provider_unreachable_failure(job_type, agent_error, &output) {
            // A local LLM server refusing the loopback connection cannot
            // recover across the backoff loop (TAURI-RUST-12K).
            return permanent(output);
        }
        Attempt {
            success: false,
            output,
            retryable: true,
            raw_detail: agent_error.map(str::to_string),
        }
    }
}

impl HostExecutor {
    /// One attempt of `job`, run under the scheduler's context.
    pub(crate) async fn execute(&self, job: &CronJob, run_id: &str) -> Attempt {
        self.binding.within(self.attempt(job, run_id)).await
    }

    /// The retries-exhausted (or permanent) report, once per failed run.
    pub(crate) async fn failed(&self, job: &CronJob, run_id: &str, attempt: &Attempt) {
        self.binding.finish_triggered_run(run_id);
        if !matches!(job.job_type, JobType::Agent) || !attempt.retryable {
            return;
        }
        let message = attempt
            .raw_detail
            .as_deref()
            .unwrap_or(attempt.output.as_str());
        crate::core::observability::report_error(
            message,
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
    }
}

pub(super) fn agent_session_target_tag(target: &SessionTarget) -> &'static str {
    match target {
        SessionTarget::Main => "main",
        SessionTarget::Isolated => "isolated",
        SessionTarget::Current => "current",
    }
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
pub(super) fn publish_cron_user_error(kind: &str) {
    tracing::debug!("[cron] action=surface_user_error kind={kind}");
    crate::web_chat::publish_web_channel_event(crate::web_chat::WebChannelEvent::UserError {
        context: crate::web_chat::event_context("system", "", ""),
        error_type: Some(kind.to_string()),
        error_source: Some("cron".to_string()),
    });
}

/// Why an agent job counts as scheduled more often than every five minutes,
/// if it does. `None` for shell and flow jobs, for one-shot `At` schedules,
/// for anything at or above [`MIN_AGENT_JOB_INTERVAL`], and for an expression
/// that cannot be read (that is not evidence of anything).
///
/// Returns the verdict rather than only logging it, because the verdict is the
/// part worth testing. The gap is the shortest one between consecutive runs
/// (`RunsCloserThan`), so an irregular expression is judged by its tightest
/// pair and not by whichever pair happens to follow the instant of the check.
pub(crate) async fn agent_job_too_frequent(
    config: &crate::config::Config,
    job: &CronJob,
) -> Option<tinyflows_bus::schedule::TooFrequent> {
    if !matches!(job.job_type, JobType::Agent) {
        return None;
    }
    match crate::cron::client::runs_closer_than(
        config,
        &job.schedule,
        chrono::Utc::now(),
        tinyflows_bus::schedule::MIN_AGENT_JOB_INTERVAL,
    )
    .await
    {
        Ok(verdict) => verdict,
        Err(error) => {
            tracing::debug!(job_id = %job.id, %error, "[cron:executor] cadence check unavailable");
            None
        }
    }
}

/// New agent jobs cannot be created below the floor (`ValidateSchedule` with
/// `agent` rejects them), so a job that trips this predates the rule. It keeps
/// running (silently skipping runs would turn its schedule into a lie) and
/// this line is the operator's signal to edit it.
pub(crate) async fn warn_if_high_frequency_agent_job(
    config: &crate::config::Config,
    job: &CronJob,
) {
    if let Some(too_frequent) = agent_job_too_frequent(config, job).await {
        tracing::warn!(
            "Cron agent job '{}' is scheduled more frequently than every 5 minutes: it \
             {too_frequent}. It was created before that floor was enforced; edit its \
             schedule to silence this.",
            job.id
        );
    }
}
