use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::error::AgentError;
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::cron::{
    due_jobs, next_run_for_schedule, record_last_run, record_run, remove_job, reschedule_after_run,
    update_job, CronJob, CronJobPatch, DeliveryConfig, JobType, Schedule, SessionTarget,
};
use crate::openhuman::security::SecurityPolicy;
use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::{stream, StreamExt};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::time::{self, Duration};

const MIN_POLL_SECONDS: u64 = 5;
const SHELL_JOB_TIMEOUT_SECS: u64 = 120;
const AGENT_JOB_USER_FAILURE_MESSAGE: &str = "Something went wrong. Please try again.\nThis error has been reported. You can also report it on Discord.\n<openhuman-link path=\"community/discord-report\">Report on Discord</openhuman-link>";
// Actionable, static failure copy for the three permanent cron halt states
// (TAURI-RUST-514 / -BMW / -HCK). Surfaced verbatim in the alerts tab + run
// history in place of the generic `AGENT_JOB_USER_FAILURE_MESSAGE`, so a user
// whose job halts on a permanent config/billing state sees the exact next step
// instead of "Something went wrong". Static `&'static str` only — they carry no
// `err` fields, honouring the no-leak contract on `agent_error_to_user_message`.
const CRON_HALT_API_KEY_UNSET_MESSAGE: &str =
    "No API key is set for your AI provider. Add it in Connections \u{2192} API keys \u{2192} LLM, then re-run.";
const CRON_HALT_INSUFFICIENT_CREDITS_MESSAGE: &str =
    "Your AI provider is out of credits. Top it up or update its key in Connections \u{2192} API keys \u{2192} LLM.";
const CRON_HALT_BUDGET_EXHAUSTED_MESSAGE: &str =
    "You've reached your managed AI budget. Raise it in Settings \u{2192} Billing.";
const MORNING_BRIEFING_AGENT_ID: &str = "morning_briefing";
const MORNING_BRIEFING_FAILURE_NOTIFICATION: &str = "Morning briefing could not run. Check your AI provider, API key, and connected apps, then run it again from Settings > Cron Jobs.";
/// Recency window the morning briefing installs around its turn so Composio
/// task-fetch tools only surface tasks created/changed in the last day. Read
/// by the `composio_execute` handler via `current_task_recency_window`.
const MORNING_BRIEFING_TASK_RECENCY_SECS: u64 = 24 * 60 * 60;

/// Map a typed [`AgentError`] to a canned, user-facing message for cron-job
/// failure notifications.
///
/// **Contract (load-bearing — see `scheduler_tests::classifier_does_not_leak_error_content`):**
/// this function returns only static `&'static str` constants. It MUST NEVER
/// interpolate any field of `err` into its output (no `format!`, no
/// `err.to_string()`, no `Debug`/`Display`). `last_agent_error` carries stack
/// traces, provider URLs with query tokens, partial response bodies and
/// occasionally user input — routing any of that into a user-visible
/// notification would be a data-exposure regression.
///
/// Variants for which we have no concrete user action (e.g.
/// [`AgentError::ToolExecutionError`], [`AgentError::Other`]) fall back to
/// [`AGENT_JOB_USER_FAILURE_MESSAGE`], preserving today's behaviour.
fn agent_error_to_user_message(err: &AgentError) -> &'static str {
    match err {
        AgentError::ProviderError { retryable: true, .. } => {
            "The model provider is temporarily unavailable. The next run will retry automatically."
        }
        AgentError::ProviderError { retryable: false, .. } => {
            "The model provider rejected the request. Check provider credentials in Connections \u{2192} API keys \u{2192} LLM."
        }
        AgentError::ContextLimitExceeded { .. } => {
            "The conversation grew too long for the model. Start a new session or pick a model with a larger context window."
        }
        AgentError::CostBudgetExceeded { .. } => {
            "You've reached the daily cost budget for this agent. Raise it in Settings \u{2192} Billing or wait for the next budget window."
        }
        AgentError::MaxIterationsExceeded { .. } => {
            "Too many tool iterations. Raise the iteration cap in Connections \u{2192} API keys \u{2192} LLM or simplify the task."
        }
        AgentError::EmptyProviderResponse { .. } => {
            // Issue #3335: the prior copy named a "local provider"
            // remedy that doesn't exist on the Managed route. This
            // shorter form (≤120 chars per the
            // `agent_error_to_user_message_canned_strings_are_short`
            // contract, for clean notification-drawer rendering) names
            // the two highest-signal remedies — credits and model
            // configuration. The richer three-remedy copy lives on the
            // chat-surface side (`channels/providers/web_errors.rs`'s
            // empty_response arm) where there's no drawer-width limit.
            "Empty model response. Out of credits (Settings \u{2192} Billing) or try another model in Connections \u{2192} API keys \u{2192} LLM."
        }
        AgentError::CompactionFailed { .. } => {
            "Automatic history compaction failed. The next run will start with a fresh context."
        }
        AgentError::PermissionDenied { .. } => {
            "The agent needs a tool that isn't allowed on this channel. Adjust the permissions in Settings."
        }
        // ToolExecutionError and Other have no actionable canned message —
        // their error bodies are too freeform to summarise safely without
        // interpolating contents. Fall back to the generic copy.
        // RegistryValidationFailed carries diagnostic message bodies that name
        // internal tool/component identifiers — too freeform to summarise safely
        // without interpolation, so fall back to the generic copy like the other
        // non-actionable variants.
        AgentError::ToolExecutionError { .. }
        | AgentError::RegistryValidationFailed { .. }
        | AgentError::Other(_) => AGENT_JOB_USER_FAILURE_MESSAGE,
    }
}

/// Classify an [`anyhow::Error`] returned by the agent runtime into a canned
/// user-facing message. If the underlying error is a typed [`AgentError`],
/// route through [`agent_error_to_user_message`]; otherwise fall back to the
/// generic message.
fn classify_agent_anyhow_for_user(err: &anyhow::Error) -> &'static str {
    match err.downcast_ref::<AgentError>() {
        Some(agent_err) => agent_error_to_user_message(agent_err),
        None => AGENT_JOB_USER_FAILURE_MESSAGE,
    }
}

fn agent_session_target_tag(target: &SessionTarget) -> &'static str {
    match target {
        SessionTarget::Main => "main",
        SessionTarget::Isolated => "isolated",
    }
}

fn is_morning_briefing_job(job: &CronJob) -> bool {
    job.name.as_deref() == Some(MORNING_BRIEFING_AGENT_ID)
        || job.agent_id.as_deref() == Some(MORNING_BRIEFING_AGENT_ID)
}

fn strip_openhuman_link_markup(input: &str) -> String {
    const OPEN_TAG: &str = "<openhuman-link";
    const CLOSE_TAG: &str = "</openhuman-link>";

    let mut output = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find(OPEN_TAG) {
        output.push_str(&rest[..start]);
        let tag_and_after = &rest[start..];

        let Some(open_end) = tag_and_after.find('>') else {
            output.push_str(tag_and_after);
            return output;
        };
        let label_and_after = &tag_and_after[open_end + 1..];

        let Some(close_start) = label_and_after.find(CLOSE_TAG) else {
            output.push_str(tag_and_after);
            return output;
        };

        output.push_str(&label_and_after[..close_start]);
        rest = &label_and_after[close_start + CLOSE_TAG.len()..];
    }

    output.push_str(rest);
    output
}

fn cron_alert_body(job: &CronJob, output: &str) -> String {
    let trimmed = output.trim();
    if matches!(job.job_type, JobType::Agent)
        && trimmed == AGENT_JOB_USER_FAILURE_MESSAGE
        && is_morning_briefing_job(job)
    {
        return MORNING_BRIEFING_FAILURE_NOTIFICATION.to_string();
    }

    let body = strip_openhuman_link_markup(output);
    crate::openhuman::util::truncate_with_ellipsis(&body, 512)
}

pub async fn run(config: Config) -> Result<()> {
    // Ensure the global event bus is initialized so cron delivery events
    // are not silently dropped. This is a no-op if already initialized.
    crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
    crate::openhuman::health::bus::register_health_subscriber();

    let poll_secs = config.reliability.scheduler_poll_secs.max(MIN_POLL_SECONDS);
    let mut interval = time::interval(Duration::from_secs(poll_secs));
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));

    publish_global(DomainEvent::SystemStartup {
        component: "scheduler".to_string(),
    });

    // Track the most recently *emitted* scheduler health so we only
    // publish `HealthChanged` on a state transition. Without this the
    // bus would carry a steady `healthy: true` event every poll
    // interval — typically 30 s, forever — churn for any subscriber
    // that logs / persists / reacts to health events. `None` means
    // "nothing emitted yet for this run", so the first successful tick
    // is treated as a transition and emits.
    let mut last_emitted_health: Option<bool> = None;

    loop {
        interval.tick().await;
        tick_once(&config, &security, &mut last_emitted_health).await;
    }
}

/// Single poll cycle of the scheduler loop, extracted so tests can drive
/// it without owning `tokio::time::interval`.
///
/// Emits a `scheduler` health signal in three cases:
/// - Poll itself failed (DB read) → `healthy: false` with the DB error.
/// - Poll succeeded, queue empty or not → `healthy: true` (#3312
///   recovery signal). Without this, a single transient job failure
///   that flipped the component to `error` via [`process_due_jobs`]
///   would stay there indefinitely while the queue was idle — no later
///   event would clear it, the health endpoint would keep returning
///   503, and Docker would mark the container `unhealthy` for hours
///   until a manual restart. Tick-level "still polling" beats
///   job-level success as the recovery signal because the queue is
///   empty most of the time.
/// - Per-job results (handled inside `process_due_jobs`) continue to
///   flip the component back to `healthy: false` on a failure; the
///   next tick that survives the DB read will re-flip it to
///   `healthy: true`, exactly the auto-recovery behaviour the Docker
///   health check needs.
pub(crate) async fn tick_once(
    config: &Config,
    security: &Arc<SecurityPolicy>,
    last_emitted_health: &mut Option<bool>,
) {
    tracing::debug!("[cron:scheduler] tick poll begin");
    let jobs = match due_jobs(config, Utc::now()) {
        Ok(jobs) => jobs,
        Err(e) => {
            tracing::warn!("[cron:scheduler] tick poll db_error: {e}");
            // Transition-only emission: only publish on the first
            // failure after a previous healthy (or unknown) state.
            // Repeat DB failures stay quiet so subscribers don't see
            // an event-storm during a long outage.
            if *last_emitted_health != Some(false) {
                publish_global(DomainEvent::HealthChanged {
                    component: "scheduler".to_string(),
                    healthy: false,
                    message: Some(e.to_string()),
                });
                *last_emitted_health = Some(false);
            }
            return;
        }
    };

    let due_count = jobs.len();
    // Transition-only emission for the recovery / healthy signal: a
    // long idle stretch with no transitions stays silent on the bus,
    // so subscribers don't pay per-poll work for a steady `healthy:
    // true` event every poll interval — the nit oxoxDev caught on
    // #3329. The very first successful tick after boot (or after a
    // failure) is the one that fires; subsequent successful ticks
    // are no-ops on the wire.
    if *last_emitted_health != Some(true) {
        tracing::debug!(
            "[cron:scheduler] tick poll ok due_count={due_count} (recovery signal: healthy=true)"
        );
        publish_global(DomainEvent::HealthChanged {
            component: "scheduler".to_string(),
            healthy: true,
            message: None,
        });
        *last_emitted_health = Some(true);
    } else {
        tracing::trace!(
            "[cron:scheduler] tick poll ok due_count={due_count} (steady state, no event)"
        );
    }

    if due_count == 0 {
        tracing::trace!("[cron:scheduler] tick end (no due jobs)");
        return;
    }

    process_due_jobs(config, security, jobs).await;
    tracing::debug!("[cron:scheduler] tick end due_count={due_count} (jobs processed)");

    // `process_due_jobs` itself may have published `healthy: false` on
    // a job failure, but it does so directly on the bus without
    // touching our local tracker. Reset so the next successful tick
    // is again treated as a transition and re-emits `healthy: true` —
    // exactly the auto-recovery behaviour #3312 requires.
    *last_emitted_health = None;
}

/// Public entry point for delivering a job's output via the configured
/// delivery mode (proactive / announce). Called by `cron_run` ("Run Now")
/// so manual runs also push notifications and alerts. Manual runs are treated
/// as `success = true` so the user always sees the result they explicitly
/// triggered (empty output is still skipped).
pub async fn deliver_job(config: &Config, job: &CronJob, output: &str) {
    if let Err(e) = deliver_if_configured(config, job, output, true).await {
        if job.delivery.best_effort {
            tracing::warn!("[cron] delivery failed (best_effort, Run Now): {e}");
        } else {
            tracing::warn!("[cron] delivery failed (Run Now): {e}");
        }
    }
}

pub async fn execute_job_now(config: &Config, job: &CronJob) -> (bool, String) {
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    execute_job_with_retry(config, &security, job).await
}

/// Did this failed agent-job attempt hit the backend session-expired state?
///
/// When the OpenHuman backend returns 401 because the user's app JWT has
/// lapsed, [`inference::provider::ops::api_error`] already publishes
/// [`crate::core::event_bus::DomainEvent::SessionExpired`] (via
/// `publish_backend_session_expired`) and the credentials subscriber clears
/// the stored session + flips the scheduler-gate `signed_out` override. The
/// gate then halts downstream LLM work until the user re-auths.
///
/// The cron retry loop pre-dates that gate handshake: it sleeps with
/// exponential backoff and retries the same job N times, every attempt
/// hitting the same global 401, then calls `report_error` with
/// `failure=retries_exhausted`. That generated TAURI-RUST-N (7,038 events /
/// 5 users): a cron-fired `morning_briefing` agent grinding through retries
/// after a single JWT lapse, every retries-exhausted capture pointing at a
/// problem the user can only fix from the UI.
///
/// The right move is the same halt-on-first-occurrence pattern as the
/// legacy tool loop's `BACKEND_USER_STATE_MARKER` convention (#3334, the
/// loop itself was retired in the tinyagents migration, #4249): the
/// condition is global and retries can't recover it, so we stop after the
/// first attempt. Skipping the `report_error` call too is correct because
/// the existing classifier
/// [`crate::core::observability::is_session_expired_message`] already
/// considers this expected user state (`observability.rs` — anchored on
/// `OpenHuman API error (401` + `"error":"Invalid token"`).
///
/// We match on `last_agent_error` first because cron's `run_agent_job`
/// routes the raw anyhow chain there (containing the provider's wire
/// message), while `last_output` only carries the canned user-facing
/// notification (`AGENT_JOB_USER_FAILURE_MESSAGE` / per-variant copy). For
/// the canned-message branch we still fall back to `last_output` so a
/// future code path that surfaces the raw error there isn't a silent miss.
///
/// Restricted to `JobType::Agent`: shell jobs that happen to echo a
/// 401-shaped string don't go through the inference layer's
/// `SessionExpired` publish, so halting them based on stdout would skip
/// retries the operator may want.
fn is_session_expired_failure(
    job_type: &JobType,
    last_agent_error: Option<&str>,
    last_output: &str,
) -> bool {
    if !matches!(job_type, JobType::Agent) {
        return false;
    }
    let signal = last_agent_error.unwrap_or(last_output);
    crate::core::observability::is_session_expired_message(signal)
}

/// Did this failed agent-job attempt hit a provider **insufficient-credits
/// 402** state (BYO account out of balance, e.g. OpenRouter)?
///
/// Same shape as [`is_session_expired_failure`], for the same reason: the
/// condition is a deterministic, permanent user-state error with no local
/// lever — retrying it across the backoff loop cannot recover, it only burns
/// cycles and (pre-this-guard) multiplied the per-attempt
/// `report_error` events that flooded Sentry (TAURI-RUST-514: the residual
/// after #3617 capped the extraction path, surfacing here via the cron
/// `agent_job` `report_error` which the desktop `before_send` chain did not
/// yet filter). So we halt after the first occurrence and skip the report,
/// matching the source demotion already applied at the provider emit site
/// (`is_provider_insufficient_credits_402`).
///
/// Routes on `last_agent_error` first (the raw anyhow chain carrying the
/// provider's 402 wire body), falling back to `last_output`, identical to
/// [`is_session_expired_failure`]. Restricted to `JobType::Agent`.
fn is_insufficient_credits_failure(
    job_type: &JobType,
    last_agent_error: Option<&str>,
    last_output: &str,
) -> bool {
    if !matches!(job_type, JobType::Agent) {
        return false;
    }
    let signal = last_agent_error.unwrap_or(last_output);
    crate::core::observability::is_insufficient_credits_message(signal)
}

/// Did this failed agent-job attempt hit a managed-backend **budget-exhausted
/// 400** state (`USER_INSUFFICIENT_CREDITS` — the OpenHuman account is out of
/// its spend budget)?
///
/// The sibling of [`is_insufficient_credits_failure`] for the managed-backend
/// billing 400 instead of the BYO provider 402. Same rationale: a permanent
/// user-state error with no local lever, so retrying across the backoff loop
/// cannot recover and the per-attempt `report_error` floods Sentry
/// (TAURI-RUST-BMW). The existing `before_send` filter
/// [`crate::core::observability::is_budget_event`] is **tag-gated**
/// (`failure=non_2xx` + `status=400`), tags the cron `agent_job` re-report
/// does not carry — so the residual leaks here. Halt on the first occurrence
/// and skip the report, reusing the same body classifier as that filter
/// (`provider::is_budget_exhausted_message`). Restricted to `JobType::Agent`.
fn is_budget_exhausted_failure(
    job_type: &JobType,
    last_agent_error: Option<&str>,
    last_output: &str,
) -> bool {
    if !matches!(job_type, JobType::Agent) {
        return false;
    }
    let signal = last_agent_error.unwrap_or(last_output);
    crate::openhuman::inference::provider::is_budget_exhausted_message(signal)
}

/// TAURI-RUST-HCK — a cron **agent** job pinned to a provider with no
/// configured API key fails deterministically at the credential guard
/// (`credential_for_request`), before any HTTP, with "<provider> API key not
/// set. Configure via the web UI …". This is a permanent user-config state: it
/// cannot recover across the backoff loop, so the loop should halt on the first
/// occurrence instead of burning every retry and then emitting the
/// `failure=retries_exhausted` `report_error` on every cron cycle (3428 events
/// / 1 user). The bare cron `report_error` bypasses the `ApiKeyMissing`
/// `expected_error_kind` demotion (that only runs on the `report_error_or_expected`
/// path), so we suppress at source here — mirroring -514 / -BMW. Delegates to
/// the single-source matcher so the wording cannot drift from the emit site.
fn is_api_key_unset_failure(
    job_type: &JobType,
    last_agent_error: Option<&str>,
    last_output: &str,
) -> bool {
    if !matches!(job_type, JobType::Agent) {
        return false;
    }
    let signal = last_agent_error.unwrap_or(last_output);
    crate::core::observability::is_api_key_unset_message(signal)
}

/// TAURI-RUST-12K — a cron **agent** job pinned to a **local** LLM provider
/// (LM Studio / Ollama / llama.cpp on `localhost:<port>`) fails because the
/// user's local runtime is unavailable or reachable-but-idle with no model
/// loaded. This is a genuinely unpreventable user-environment state: the app
/// has no lever to start a user's local model server or load a model there, and
/// retrying across the backoff loop cannot fix it within one cron cycle.
///
/// The provider / agent emit sites already demote this via
/// `report_error_or_expected` (the `expected_error_kind` classifier routes it
/// to `LoopbackUnavailable`), so it never reaches Sentry there. But the bare
/// cron `report_error` below bypasses that demotion and re-emitted the
/// `failure=retries_exhausted` capture on every cron cycle — 2802 events / 29
/// users. So we halt on the first occurrence and skip the report, mirroring
/// the source demotion and the sibling billing / api-key guards
/// (TAURI-RUST-514 / -BMW / -HCK).
///
/// Delegates loopback-unreachable detection to the single-source matcher
/// [`crate::core::observability::is_local_provider_unreachable_message`] so
/// the wording cannot drift from the classifier emit site. Also recognizes the
/// inference provider's stable local-runtime "no model loaded" user message.
/// Narrow by design: a transient *remote* provider / backend network error
/// still retries and still reports. Checks both `last_agent_error` (the raw
/// anyhow chain carrying the wire message) and `last_output` (the surfaced user
/// message), because some provider paths preserve only one of those shapes.
/// Restricted to `JobType::Agent`.
fn is_local_provider_unreachable_failure(
    job_type: &JobType,
    last_agent_error: Option<&str>,
    last_output: &str,
) -> bool {
    if !matches!(job_type, JobType::Agent) {
        return false;
    }
    let raw_signal = last_agent_error.unwrap_or("");
    crate::core::observability::is_local_provider_unreachable_message(raw_signal)
        || crate::core::observability::is_local_provider_unreachable_message(last_output)
        || is_local_provider_no_model_loaded_message(raw_signal)
        || is_local_provider_no_model_loaded_message(last_output)
}

fn is_local_provider_no_model_loaded_message(signal: &str) -> bool {
    let lower = signal.to_ascii_lowercase();
    (lower.contains("local inference server") && lower.contains("no model loaded"))
        || lower.contains("no models loaded")
}

async fn execute_job_with_retry(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    let mut last_output = String::new();
    let mut last_agent_error: Option<String> = None;
    let retries = config.reliability.scheduler_retries;
    let mut backoff_ms = config.reliability.provider_backoff_ms.max(200);
    let mut session_expired = false;
    let mut credits_exhausted = false;
    let mut budget_exhausted = false;
    let mut key_unset = false;
    let mut local_unreachable = false;

    for attempt in 0..=retries {
        let (success, output, agent_error) = match job.job_type {
            JobType::Shell => {
                let (success, output) = run_job_command(config, security, job).await;
                (success, output, None)
            }
            JobType::Agent => run_agent_job(config, job).await,
            JobType::Flow => {
                let (success, output) = run_flow_schedule_job(job);
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

/// Static, leak-safe actionable alert copy for a permanent cron halt state.
/// Returns the user-facing `/notifications` body matching the halt reason —
/// `&'static str` only, so it can never carry a raw error field (the no-leak
/// contract that governs [`agent_error_to_user_message`]). Precedence mirrors
/// the halt classifiers' evaluation order: credits → budget → missing key.
fn permanent_halt_message(credits_exhausted: bool, budget_exhausted: bool) -> &'static str {
    if credits_exhausted {
        CRON_HALT_INSUFFICIENT_CREDITS_MESSAGE
    } else if budget_exhausted {
        CRON_HALT_BUDGET_EXHAUSTED_MESSAGE
    } else {
        CRON_HALT_API_KEY_UNSET_MESSAGE
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
///
/// The frontend `socketService` listens for `user_error` and routes it through
/// the same classifier the chat runtime uses, so a background (no-delivery) job
/// failure is no longer silent — it lands in the shell's UserErrorCenter with a
/// deep-link action even though no chat thread is active.
fn publish_cron_user_error(kind: &str) {
    log::debug!("[cron] action=surface_user_error kind={kind}");
    crate::openhuman::channels::providers::web::publish_web_channel_event(
        crate::core::socketio::WebChannelEvent {
            event: "user_error".to_string(),
            client_id: "system".to_string(),
            error_type: Some(kind.to_string()),
            error_source: Some("cron".to_string()),
            ..Default::default()
        },
    );
}

async fn process_due_jobs(config: &Config, security: &Arc<SecurityPolicy>, jobs: Vec<CronJob>) {
    let max_concurrent = config.scheduler.max_concurrent.max(1);
    let mut in_flight = stream::iter(jobs.into_iter().map(|job| {
        let config = config.clone();
        let security = Arc::clone(security);
        async move { execute_and_persist_job(&config, security.as_ref(), &job).await }
    }))
    .buffer_unordered(max_concurrent);

    while let Some((job_id, success, failure_message)) = in_flight.next().await {
        if success {
            publish_global(DomainEvent::HealthChanged {
                component: "scheduler".to_string(),
                healthy: true,
                message: None,
            });
        } else {
            publish_global(DomainEvent::HealthChanged {
                component: "scheduler".to_string(),
                healthy: false,
                message: Some(failure_message.unwrap_or_else(|| format!("job {job_id} failed"))),
            });
        }
    }
}

async fn execute_and_persist_job(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (String, bool, Option<String>) {
    warn_if_high_frequency_agent_job(job);

    let started_at = Utc::now();

    publish_global(DomainEvent::CronJobTriggered {
        job_id: job.id.clone(),
        job_name: job.name.clone().unwrap_or_default(),
        job_type: format!("{:?}", job.job_type),
    });

    let (execution_success, output) = execute_job_with_retry(config, security, job).await;
    let finished_at = Utc::now();
    let success = persist_job_result(
        config,
        job,
        execution_success,
        &output,
        started_at,
        finished_at,
    )
    .await;

    publish_global(DomainEvent::CronJobCompleted {
        job_id: job.id.clone(),
        success,
        output: crate::openhuman::util::truncate_with_ellipsis(&output, 512),
    });
    let failure_message =
        (!success).then(|| crate::openhuman::util::truncate_with_ellipsis(&output, 256));

    (job.id.clone(), success, failure_message)
}

async fn run_agent_job(config: &Config, job: &CronJob) -> (bool, String, Option<String>) {
    let name = job.name.clone().unwrap_or_else(|| "cron-job".to_string());
    let prompt = job.prompt.clone().unwrap_or_default();
    let prefixed_prompt = format!("[cron:{} {name}] {prompt}", job.id);

    // Apply per-job model override onto a cloned Config, so the Agent
    // sees it through the normal `default_model` path without mutating
    // the caller's config.
    let mut effective = config.clone();
    if let Some(model) = job.model.clone() {
        effective.default_model = Some(model);
    }

    // When an agent_id is set, resolve the built-in definition and apply
    // its model hint, iteration cap, and prompt body so the cron job
    // runs with the definition's constraints instead of the generic
    // Agent::from_config defaults.
    if let Some(ref agent_id) = job.agent_id {
        if let Some(registry) =
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
        {
            if let Some(def) = registry.get(agent_id) {
                tracing::debug!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    max_iterations = def.max_iterations,
                    "[cron] applying agent definition overrides"
                );
                // Resolve the agent definition's model spec into an
                // exact model id. `ModelSpec::resolve` synthesises
                // `{hint}-v1` for Hint specs, which only the OpenHuman
                // backend understands as a tier hint — Anthropic and
                // every other provider 404 on names like `agentic-v1`.
                // Route Hint specs through the per-workload factory so
                // we get the exact model the user has configured for
                // that workload, regardless of which provider it lives
                // on. Inherit / Exact keep their literal `resolve()`
                // behaviour because neither relies on the `-v1` trick.
                use crate::openhuman::agent::harness::definition::ModelSpec;
                let fallback_model = effective
                    .default_model
                    .clone()
                    .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string());
                let resolved_model = match &def.model {
                    ModelSpec::Hint(workload) => {
                        // Resolve the workload's configured model id via the crate
                        // `ChatModel` factory (#4249 Phase 1). We only need the
                        // resolved model string here, so the built model is
                        // discarded — `create_chat_model_with_model_id` wraps the
                        // same `create_chat_provider` resolution, so the model id is
                        // identical; temperature is irrelevant to id resolution.
                        match crate::openhuman::inference::provider::create_chat_model_with_model_id(
                            workload,
                            &effective,
                            effective.default_temperature,
                        ) {
                            Ok((_, m)) => {
                                tracing::debug!(
                                    job_id = %job.id,
                                    agent_id = %agent_id,
                                    workload = %workload,
                                    model = %m,
                                    "[cron] resolved Hint via workload factory"
                                );
                                m
                            }
                            Err(e) => {
                                tracing::warn!(
                                    job_id = %job.id,
                                    agent_id = %agent_id,
                                    workload = %workload,
                                    error = %e,
                                    fallback = %fallback_model,
                                    "[cron] workload factory failed; using fallback model"
                                );
                                fallback_model.clone()
                            }
                        }
                    }
                    ModelSpec::Inherit => fallback_model.clone(),
                    ModelSpec::Exact(name) => name.clone(),
                };
                effective.default_model = Some(resolved_model);
                effective.agent.max_tool_iterations = def.max_iterations;
            } else {
                tracing::warn!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    "[cron] agent_id not found in registry — falling back to generic agent"
                );
            }
        } else {
            tracing::warn!(
                job_id = %job.id,
                "[cron] AgentDefinitionRegistry not initialized — falling back to generic agent"
            );
        }
    }

    let run_result = match job.session_target {
        SessionTarget::Main | SessionTarget::Isolated => {
            tracing::debug!(
                job_id = %job.id,
                target = ?job.session_target,
                "[cron] building isolated agent for scheduled job"
            );
            match build_agent_for_cron_job(&effective, job) {
                Ok(mut agent) => {
                    // Tag events so downstream subscribers can correlate
                    // cron-triggered turns. `cron` is the channel so the
                    // event bus can filter from other flows (`cli`, `web`…).
                    agent.set_event_context(format!("cron:{}", job.id), "cron");
                    // Scope a `TrustedAutomation { Cron }` origin around the
                    // turn. The approval gate treats this as user-authorized
                    // automation and lets external_effect tools run without
                    // an in-app prompt — the user explicitly created this
                    // cron job and authorized its prompt at the same time.
                    let origin =
                        crate::openhuman::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
                            job_id: job.id.clone(),
                            source:
                                crate::openhuman::agent::turn_origin::TrustedAutomationSource::Cron,
                        };
                    let turn = crate::openhuman::agent::turn_origin::with_origin(
                        origin,
                        agent.run_single(&prefixed_prompt),
                    );
                    // Morning briefing only: install a 24h task-recency window
                    // so Composio task-fetch tools (Linear/ClickUp/Notion/Asana)
                    // surface only recently created/changed tasks. Other cron
                    // agents and all chat turns leave the window unset.
                    if is_morning_briefing_job(job) {
                        tracing::debug!(
                            job_id = %job.id,
                            recency_window_secs = MORNING_BRIEFING_TASK_RECENCY_SECS,
                            "[cron] applying morning-briefing task recency window"
                        );
                        crate::openhuman::agent::harness::with_task_recency_window(
                            std::time::Duration::from_secs(MORNING_BRIEFING_TASK_RECENCY_SECS),
                            turn,
                        )
                        .await
                    } else {
                        tracing::trace!(
                            job_id = %job.id,
                            "[cron] task recency window not applied for this job"
                        );
                        turn.await
                    }
                }
                Err(e) => Err(e),
            }
        }
    };

    match run_result {
        Ok(response) => (
            true,
            if response.trim().is_empty() {
                EMPTY_AGENT_OUTPUT.to_string()
            } else {
                response
            },
            None,
        ),
        Err(e) => {
            // Classify into a canned user-facing message *before* logging
            // anything that touches `e`. The classifier output is a
            // `&'static str` — it never contains any data derived from `e`.
            // The raw error is preserved as `last_agent_error` for the
            // observability pipeline (`report_error`), where stack traces
            // and provider URLs are appropriate; it must NOT reach the
            // user-visible notification body.
            let user_message = classify_agent_anyhow_for_user(&e);
            (false, user_message.to_string(), Some(e.to_string()))
        }
    }
}

/// Fires a `JobType::Flow` job: publishes `DomainEvent::FlowScheduleTick` for
/// the bound flow id (stored in `job.command`, see `JobType::Flow`'s doc) and
/// returns immediately. This job type does no work itself — dispatching the
/// actual `flows::ops::flows_run` happens asynchronously in
/// `flows::bus::FlowTriggerSubscriber`, which is the sole consumer of this
/// event (kept out of the cron domain so cron stays flow-agnostic).
fn run_flow_schedule_job(job: &CronJob) -> (bool, String) {
    let flow_id = job.command.clone();
    tracing::info!(
        target: "flows",
        job_id = %job.id,
        %flow_id,
        "[cron] flow schedule tick — publishing FlowScheduleTick"
    );
    publish_global(DomainEvent::FlowScheduleTick {
        flow_id: flow_id.clone(),
    });
    (
        true,
        format!("flow schedule tick emitted for flow {flow_id}"),
    )
}

/// Placeholder recorded in run history when an agent job succeeds but returns
/// no text. Never delivered to chat — used only for the run-history record.
const EMPTY_AGENT_OUTPUT: &str = "agent job executed";

fn build_agent_for_cron_job(config: &Config, job: &CronJob) -> anyhow::Result<Agent> {
    if let Some(agent_id) = job.agent_id.as_deref() {
        match Agent::from_config_for_agent(config, agent_id) {
            Ok(agent) => {
                tracing::debug!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    "[cron] built scheduled job agent from definition"
                );
                Ok(agent)
            }
            Err(e) => {
                tracing::warn!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    error = %e,
                    "[cron] failed to build agent from definition; falling back to generic agent"
                );
                Agent::from_config(config)
            }
        }
    } else {
        Agent::from_config(config)
    }
}

async fn persist_job_result(
    config: &Config,
    job: &CronJob,
    mut success: bool,
    output: &str,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
) -> bool {
    let duration_ms = (finished_at - started_at).num_milliseconds();

    if let Err(e) = deliver_if_configured(config, job, output, success).await {
        if job.delivery.best_effort {
            tracing::warn!("Cron delivery failed (best_effort): {e}");
        } else {
            success = false;
            tracing::warn!("Cron delivery failed: {e}");
        }
    }

    let _ = record_run(
        config,
        &job.id,
        started_at,
        finished_at,
        if success { "ok" } else { "error" },
        Some(output),
        duration_ms,
    );

    if is_one_shot_auto_delete(job) {
        if success {
            if let Err(e) = remove_job(config, &job.id) {
                tracing::warn!("Failed to remove one-shot cron job after success: {e}");
            }
        } else {
            let _ = record_last_run(config, &job.id, finished_at, false, output);
            if let Err(e) = update_job(
                config,
                &job.id,
                CronJobPatch {
                    enabled: Some(false),
                    ..CronJobPatch::default()
                },
            ) {
                tracing::warn!("Failed to disable failed one-shot cron job: {e}");
            }
        }
        return success;
    }

    if let Err(e) = reschedule_after_run(config, job, success, output) {
        tracing::warn!("Failed to persist scheduler run result: {e}");
    }

    success
}

fn is_one_shot_auto_delete(job: &CronJob) -> bool {
    job.delete_after_run && matches!(job.schedule, Schedule::At { .. })
}

fn warn_if_high_frequency_agent_job(job: &CronJob) {
    if !matches!(job.job_type, JobType::Agent) {
        return;
    }
    let too_frequent = match &job.schedule {
        Schedule::Every { every_ms } => *every_ms < 5 * 60 * 1000,
        Schedule::Cron { .. } => {
            let now = Utc::now();
            match (
                next_run_for_schedule(&job.schedule, now),
                next_run_for_schedule(&job.schedule, now + chrono::Duration::seconds(1)),
            ) {
                (Ok(a), Ok(b)) => (b - a).num_minutes() < 5,
                _ => false,
            }
        }
        Schedule::At { .. } => false,
    };

    if too_frequent {
        tracing::warn!(
            "Cron agent job '{}' is scheduled more frequently than every 5 minutes",
            job.id
        );
    }
}

/// True when an agent job produced no meaningful text — blank output or the
/// [`EMPTY_AGENT_OUTPUT`] placeholder. Such runs are never injected into chat.
fn cron_output_is_empty(output: &str) -> bool {
    output.trim().is_empty() || output == EMPTY_AGENT_OUTPUT
}

/// Whether a cron job's output should be injected into the user's chat thread.
/// Skips failed runs and empty/placeholder output; failures still surface in
/// the alerts tab and run history (handled separately by the caller).
fn should_deliver_cron_output_to_chat(success: bool, output: &str) -> bool {
    success && !cron_output_is_empty(output)
}

/// Whether a completed cron run should surface in the alerts tab
/// (`/notifications`). Failures stay visible even when they produce no output;
/// only successful-but-empty runs are dropped entirely.
fn cron_result_should_alert(success: bool, output: &str) -> bool {
    !success || !cron_output_is_empty(output)
}

async fn deliver_if_configured(
    config: &Config,
    job: &CronJob,
    output: &str,
    success: bool,
) -> Result<()> {
    let delivery: &DeliveryConfig = &job.delivery;

    // Don't post failed or empty cron runs into the user's chat: a failed turn
    // (e.g. a transient network error) would otherwise deliver a canned
    // "Something went wrong" message into the conversation with no user
    // message behind it. Failures still reach the alerts tab (`push_cron_alert`)
    // and the run-history / health signals, which are recorded elsewhere.
    let is_empty = cron_output_is_empty(output);
    let deliver_to_chat = should_deliver_cron_output_to_chat(success, output);
    if !deliver_to_chat {
        tracing::debug!(
            job_id = %job.id,
            success,
            is_empty,
            "[cron] skipping chat delivery for failed/empty cron run"
        );
    }

    // A failed run must stay visible in /notifications regardless of delivery
    // mode — a no-delivery agent job that halts on a permanent config/billing
    // state (e.g. a keyless provider, TAURI-RUST-HCK) would otherwise fail
    // silently. A *successful* non-empty run only alerts in the delivering
    // modes (proactive/announce); a `none`-mode success stays silent (its
    // output lives in last_output only — the cron contract), so we don't spam
    // explicitly-silent background jobs with an unread alert every interval
    // (Codex #4166).
    let mode = delivery.mode.trim().to_ascii_lowercase();
    let delivers = matches!(mode.as_str(), "proactive" | "announce");
    let alert_to_notifications =
        cron_result_should_alert(success, output) && (!success || delivers);
    let alert_body = if is_empty {
        "Scheduled job failed without output."
    } else {
        output
    };

    match mode.as_str() {
        // Proactive delivery — the channels module decides where to send.
        // Used by morning briefings, welcome messages, and other
        // user-facing proactive agents.
        "proactive" => {
            if deliver_to_chat {
                let source = format!("cron:{}", job.id);
                tracing::debug!(
                    job_id = %job.id,
                    source = %source,
                    "[cron] publishing ProactiveMessageRequested event"
                );
                publish_global(DomainEvent::ProactiveMessageRequested {
                    source,
                    message: output.to_string(),
                    job_name: job.name.clone(),
                });
            }
        }

        // Announce delivery — the cron job specifies the exact channel
        // and target. Used for explicit channel-targeted output.
        "announce" if deliver_to_chat => {
            let channel = delivery
                .channel
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("delivery.channel is required for announce mode"))?;
            let target = delivery
                .to
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("delivery.to is required for announce mode"))?;

            tracing::debug!(
                job_id = %job.id,
                channel = %channel,
                target = %target,
                "[cron] publishing CronDeliveryRequested event"
            );
            publish_global(DomainEvent::CronDeliveryRequested {
                job_id: job.id.clone(),
                channel: channel.to_string(),
                target: target.to_string(),
                output: output.to_string(),
            });
        }

        // No delivery configured — output is stored in last_output only.
        // The failure still reaches the alerts tab via the hoisted
        // `push_cron_alert` below.
        _ => {}
    }

    // Surface in the alerts tab (/notifications) for any result that isn't a
    // successful-but-empty run — INDEPENDENT of delivery mode. A failed cron
    // job must stay visible to the user even when it has no chat delivery
    // configured (the common case: a keyless agent job failing "API key not
    // set", TAURI-RUST-HCK). Previously this fired only inside the proactive /
    // announce arms, so no-delivery jobs failed silently in /notifications.
    if alert_to_notifications {
        push_cron_alert(config, job, alert_body);
    }

    Ok(())
}

/// Insert a notification into the alerts tab for a completed cron job.
fn push_cron_alert(config: &Config, job: &CronJob, output: &str) {
    use crate::openhuman::notifications::store as notif_store;
    use crate::openhuman::notifications::types::{IntegrationNotification, NotificationStatus};

    let name = job.name.as_deref().unwrap_or("Cron job");
    let body = cron_alert_body(job, output);

    let notification = IntegrationNotification {
        id: uuid::Uuid::new_v4().to_string(),
        provider: "cron".to_string(),
        account_id: Some(job.id.clone()),
        title: name.to_string(),
        body,
        raw_payload: serde_json::json!({
            "job_id": job.id,
            "job_name": job.name,
            "delivery_mode": job.delivery.mode,
        }),
        importance_score: Some(0.65),
        triage_action: Some("react".to_string()),
        triage_reason: Some("Scheduled delivery".to_string()),
        status: NotificationStatus::Unread,
        received_at: Utc::now(),
        scored_at: Some(Utc::now()),
    };

    match notif_store::insert_if_not_recent(config, &notification) {
        Ok(true) => {
            tracing::debug!(
                job_id = %job.id,
                "[cron] pushed notification alert to alerts tab"
            );
        }
        Ok(false) => {
            tracing::debug!(
                job_id = %job.id,
                "[cron] skipped duplicate notification alert"
            );
        }
        Err(e) => {
            tracing::warn!(
                job_id = %job.id,
                error = %e,
                "[cron] failed to push notification alert"
            );
        }
    }
}

fn is_env_assignment(word: &str) -> bool {
    word.contains('=')
        && word
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

fn strip_wrapping_quotes(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

fn forbidden_path_argument(security: &SecurityPolicy, command: &str) -> Option<String> {
    let mut normalized = command.to_string();
    for sep in ["&&", "||"] {
        normalized = normalized.replace(sep, "\x00");
    }
    for sep in ['\n', ';', '|'] {
        normalized = normalized.replace(sep, "\x00");
    }

    for segment in normalized.split('\x00') {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        // Skip leading env assignments and executable token.
        let mut idx = 0;
        while idx < tokens.len() && is_env_assignment(tokens[idx]) {
            idx += 1;
        }
        if idx >= tokens.len() {
            continue;
        }
        idx += 1;

        for token in &tokens[idx..] {
            let candidate = strip_wrapping_quotes(token);
            if candidate.is_empty() || candidate.starts_with('-') || candidate.contains("://") {
                continue;
            }

            let looks_like_path = candidate.starts_with('/')
                || candidate.starts_with("./")
                || candidate.starts_with("../")
                || candidate.starts_with("~/")
                || candidate.contains('/');

            if looks_like_path && !security.is_path_string_allowed(candidate) {
                return Some(candidate.to_string());
            }
        }
    }

    None
}

async fn run_job_command(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    run_job_command_with_timeout(
        config,
        security,
        job,
        Duration::from_secs(SHELL_JOB_TIMEOUT_SECS),
    )
    .await
}

async fn run_job_command_with_timeout(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
    timeout: Duration,
) -> (bool, String) {
    if !security.can_act() {
        return (
            false,
            "blocked by security policy: autonomy is read-only".to_string(),
        );
    }

    if security.is_rate_limited() {
        return (
            false,
            "blocked by security policy: rate limit exceeded".to_string(),
        );
    }

    if !security.is_command_allowed(&job.command) {
        return (
            false,
            format!(
                "blocked by security policy: command not allowed: {}",
                job.command
            ),
        );
    }

    if let Some(path) = forbidden_path_argument(security, &job.command) {
        return (
            false,
            format!("blocked by security policy: forbidden path argument: {path}"),
        );
    }

    if !security.record_action() {
        return (
            false,
            "blocked by security policy: action budget exhausted".to_string(),
        );
    }

    let child = match Command::new("sh")
        .arg("-lc")
        .arg(&job.command)
        .current_dir(&config.action_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return (false, format!("spawn error: {e}")),
    };

    match time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!(
                "status={}\nstdout:\n{}\nstderr:\n{}",
                output.status,
                stdout.trim(),
                stderr.trim()
            );
            (output.status.success(), combined)
        }
        Ok(Err(e)) => (false, format!("spawn error: {e}")),
        Err(_) => (
            false,
            format!("job timed out after {}s", timeout.as_secs_f64()),
        ),
    }
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
