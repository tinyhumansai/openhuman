//! Classification of agent-job failures into static, leak-safe user copy and
//! the permanent-halt predicates that stop the retry loop early.

use tinyagents_harness::agent_error::AgentError;
use tinyflows_bus::schedule::types::JobType;

pub(super) const AGENT_JOB_USER_FAILURE_MESSAGE: &str = "Something went wrong. Please try again.";
// Actionable, static failure copy for the three permanent cron halt states
// (TAURI-RUST-514 / -BMW / -HCK). Surfaced verbatim in the alerts tab + run
// history in place of the generic `AGENT_JOB_USER_FAILURE_MESSAGE`, so a user
// whose job halts on a permanent config/billing state sees the exact next step
// instead of "Something went wrong". Static `&'static str` only — they carry no
// `err` fields, honouring the no-leak contract on `agent_error_to_user_message`.
pub(super) const CRON_HALT_API_KEY_UNSET_MESSAGE: &str =
    "No API key is set for your AI provider. Add it in Connections \u{2192} API keys \u{2192} LLM, then re-run.";
pub(super) const CRON_HALT_INSUFFICIENT_CREDITS_MESSAGE: &str =
    "Your AI provider is out of credits. Top it up or update its key in Connections \u{2192} API keys \u{2192} LLM.";
pub(super) const CRON_HALT_BUDGET_EXHAUSTED_MESSAGE: &str =
    "You've reached your managed AI budget. Raise it in Settings \u{2192} Billing.";

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
pub(super) fn agent_error_to_user_message(err: &AgentError) -> &'static str {
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
            // Empty content alone cannot establish a billing problem. Keep
            // the notification short and point to a retry or model change.
            "Empty model response. Retry the job or try another model in Connections \u{2192} API keys \u{2192} LLM."
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
pub(super) fn classify_agent_anyhow_for_user(err: &anyhow::Error) -> &'static str {
    match err.downcast_ref::<AgentError>() {
        Some(agent_err) => agent_error_to_user_message(agent_err),
        None => AGENT_JOB_USER_FAILURE_MESSAGE,
    }
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
pub(super) fn is_session_expired_failure(
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
pub(super) fn is_insufficient_credits_failure(
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
pub(super) fn is_budget_exhausted_failure(
    job_type: &JobType,
    last_agent_error: Option<&str>,
    last_output: &str,
) -> bool {
    if !matches!(job_type, JobType::Agent) {
        return false;
    }
    let signal = last_agent_error.unwrap_or(last_output);
    tinyinference_providers::is_budget_exhausted_message(signal)
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
pub(super) fn is_api_key_unset_failure(
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
pub(super) fn is_local_provider_unreachable_failure(
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

pub(super) fn is_local_provider_no_model_loaded_message(signal: &str) -> bool {
    let lower = signal.to_ascii_lowercase();
    (lower.contains("local inference server") && lower.contains("no model loaded"))
        || tinyinference_llm::failure::body_indicates_no_model_loaded(signal)
}

/// Static, leak-safe actionable alert copy for a permanent cron halt state.
/// Returns the user-facing `/notifications` body matching the halt reason —
/// `&'static str` only, so it can never carry a raw error field (the no-leak
/// contract that governs [`agent_error_to_user_message`]). Precedence mirrors
/// the halt classifiers' evaluation order: credits → budget → missing key.
pub(super) fn permanent_halt_message(
    credits_exhausted: bool,
    budget_exhausted: bool,
) -> &'static str {
    if credits_exhausted {
        CRON_HALT_INSUFFICIENT_CREDITS_MESSAGE
    } else if budget_exhausted {
        CRON_HALT_BUDGET_EXHAUSTED_MESSAGE
    } else {
        CRON_HALT_API_KEY_UNSET_MESSAGE
    }
}
