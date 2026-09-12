use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::error::AgentError;
use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::cron::{
    due_jobs, record_last_run, record_run, remove_job, reschedule_after_run, runs_closer_than,
    update_job, CronJob, CronJobPatch, DeliveryConfig, JobType, Schedule, SessionTarget,
    TooFrequent, MIN_AGENT_JOB_INTERVAL,
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
            // chat-surface side (`web_chat/web_errors.rs`'s
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
    crate::core::bus::init().await.expect("bus init");
    crate::openhuman::platform::health::bus::register_health_subscriber();

    let poll_secs = config.reliability.scheduler_poll_secs.max(MIN_POLL_SECONDS);
    let mut interval = time::interval(Duration::from_secs(poll_secs));
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));

    BUS.publish(DomainEvent::SystemStartup {
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
                BUS.publish(DomainEvent::HealthChanged {
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
        BUS.publish(DomainEvent::HealthChanged {
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
/// [`crate::core::events::DomainEvent::SessionExpired`] (via
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
