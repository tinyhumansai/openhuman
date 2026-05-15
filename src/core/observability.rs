//! Centralised error reporting for the core, plus a Sentry
//! `before_send` filters that drop deterministic provider noise:
//! per-attempt transient-upstream failures, budget-exhausted user-state,
//! and transient updater failures.
//!
//! Wraps `tracing::error!` (which the global subscriber forwards to Sentry via
//! `sentry-tracing`) inside a `sentry::with_scope` so each captured event
//! carries consistent tags identifying the failing domain/operation plus any
//! callsite-specific context (session id, request id, tool name, …).
//!
//! Why this helper exists: errors that bubble up as `Result::Err` without ever
//! being logged at error level never reach Sentry. The agent-turn path is the
//! canonical example — `run_single` used to publish a `DomainEvent::AgentError`
//! and return `Err(_)`, but Sentry never saw it. Funnel error sites through
//! `report_error` so they show up tagged and grep-friendly in Sentry.

use std::fmt::Display;

/// A `(key, value)` pair attached as a Sentry tag. Tags are short, indexed,
/// and filterable in the Sentry UI — prefer them over free-form fields for
/// anything you'd want to facet on (`error_kind`, `tool_name`, `method`).
pub type Tag<'a> = (&'a str, &'a str);

/// HTTP status codes that the reliable-provider layer already handles via
/// retry + fallback, so per-attempt Sentry reports add noise without signal:
///
/// - **408** Request Timeout
/// - **429** Too Many Requests
/// - **502** Bad Gateway
/// - **503** Service Unavailable
/// - **504** Gateway Timeout
///
/// Single source of truth for both the call-site classifier
/// (`openhuman::providers::ops::should_report_provider_http_failure`) and the
/// `before_send` filter (`is_transient_provider_http_failure`). Update here
/// and both sites pick it up — keeps the two layers from drifting.
pub const TRANSIENT_PROVIDER_HTTP_STATUSES: &[u16] = &[408, 429, 502, 503, 504, 520];

/// HTTP status codes that represent transient backend / integration transport
/// failures rather than application bugs. Keep this as strings because Sentry
/// tags are strings, and the before_send classifiers match tag values exactly.
pub const TRANSIENT_HTTP_STATUSES: &[&str] = &["408", "429", "502", "503", "504", "520"];

/// Transport-layer phrases observed from reqwest / hyper for temporary
/// upstream interruptions. Keep these specific so rare configuration failures
/// still reach Sentry.
pub const TRANSIENT_TRANSPORT_PHRASES: &[&str] = &[
    "timeout",
    "operation timed out",
    "connection forcibly closed",
    "connection reset",
    "tls handshake eof",
    "error sending request",
];

/// HTTP statuses from updater probes that are expected GitHub/network noise:
/// unauthenticated GitHub API rate-limit / policy 403s plus gateway/server
/// hiccups. Scoped to updater domains/messages by [`is_updater_transient_event`].
const UPDATER_TRANSIENT_HTTP_STATUSES: &[u16] = &[403, 500, 502, 503, 504];

/// Message fragments observed from Tauri/core updater transient failures.
/// Keep these updater-specific so unrelated GitHub or generic transport
/// failures still reach Sentry.
const UPDATER_TRANSIENT_MESSAGE_PHRASES: &[&str] = &[
    "failed to check for updates: error sending request",
    "github api error: 403",
    "github api error: 5",
    "error sending request for url (https://github.com/tinyhumansai/openhuman/releases/",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedErrorKind {
    LocalAiDisabled,
    ApiKeyMissing,
    NetworkUnreachable,
    TransientUpstreamHttp,
    LocalAiBinaryMissing,
    BackendUserError,
    /// Third-party provider (composio, gmail OAuth, …) surfaced a user-state
    /// validation failure: a trigger registry mismatch, a toolkit that was
    /// never enabled, an OAuth scope that the user did not grant, or a
    /// required field that was left blank. The UI already shows an
    /// actionable error and Sentry has no remediation path — see
    /// [`is_provider_user_state_message`] for the exact body shapes.
    ///
    /// Drops OPENHUMAN-TAURI-3R / -3S / -33 / -34 / -97 (~54 events): the
    /// composio backend wraps several of these as HTTP 500 with the real
    /// 4xx body embedded, which would otherwise escape the
    /// [`is_backend_user_error_message`] 4xx-only matcher.
    ProviderUserState,
    LocalAiCapabilityUnavailable,
    BudgetExhausted,
    SessionExpired,
}

pub fn expected_error_kind(message: &str) -> Option<ExpectedErrorKind> {
    let lower = message.to_ascii_lowercase();
    if lower.contains("local ai is disabled") {
        return Some(ExpectedErrorKind::LocalAiDisabled);
    }
    if lower.contains("api key not set") || lower.contains("missing api key") {
        return Some(ExpectedErrorKind::ApiKeyMissing);
    }
    if is_network_unreachable_message(&lower) {
        return Some(ExpectedErrorKind::NetworkUnreachable);
    }
    if is_transient_upstream_http_message(&lower) {
        return Some(ExpectedErrorKind::TransientUpstreamHttp);
    }
    if lower.contains("binary not found") {
        return Some(ExpectedErrorKind::LocalAiBinaryMissing);
    }
    // Check `is_provider_user_state_message` BEFORE `is_backend_user_error_message`:
    // composio's "Toolkit X is not enabled" lands as a 4xx that both would
    // match, and the more specific `ProviderUserState` bucket is the right
    // home — see the variant doc-comment for OPENHUMAN-TAURI-… coverage.
    if is_provider_user_state_message(&lower) {
        return Some(ExpectedErrorKind::ProviderUserState);
    }
    if is_backend_user_error_message(&lower) {
        return Some(ExpectedErrorKind::BackendUserError);
    }
    if is_local_ai_capability_unavailable_message(&lower) {
        return Some(ExpectedErrorKind::LocalAiCapabilityUnavailable);
    }
    if crate::openhuman::providers::is_budget_exhausted_message(message) {
        return Some(ExpectedErrorKind::BudgetExhausted);
    }
    if is_session_expired_message(message) {
        return Some(ExpectedErrorKind::SessionExpired);
    }
    None
}

/// Detect **app-session-expired** boundary errors that bubble up from any
/// backend-touching call site (agent, web channel, cron, integrations).
///
/// Deliberately stricter than the dispatch-site classifier in
/// [`crate::core::jsonrpc`]: the dispatch-site predicate matches a generic
/// "401 + unauthorized" pair to trigger token cleanup on *any* 401 (even an
/// OpenAI / Anthropic BYO-key 401 that means a misconfigured key — see
/// `providers::ops::api_error`). Replicating that loose match here would
/// silence BYO-key configuration errors at the agent layer, where they
/// *are* actionable and should reach Sentry as errors.
///
/// The canonical OpenHuman session-expired wire shapes:
///
/// - `"OpenHuman API error (401 Unauthorized): {…\"Session expired. Please
///   log in again.\"…}"` — emitted by `providers::ops::api_error` from the
///   OpenHuman backend and re-raised through `agent::run_single` /
///   `channels::providers::web::run_chat_task` (OPENHUMAN-TAURI-26). The
///   `"session expired"` substring anchors the match to the OpenHuman
///   backend's session-renewal body, not the bare numeric status.
/// - `"SESSION_EXPIRED: backend session not active — sign in to resume LLM work"`
///   — the `scheduler_gate::is_signed_out` sentinel from
///   `providers::openhuman_backend::resolve_bearer`.
/// - `"no backend session token; run auth_store_session first"` and
///   `"session JWT required"` — local pre-flight guards that fire when the
///   stored profile is empty (`#1465`-ish onboarding spam) or has been
///   cleared by a previous 401 cycle. Both shapes are OpenHuman-specific.
///
/// At the JSON-RPC dispatch boundary the looser classifier in
/// `crate::core::jsonrpc::is_session_expired_error` keeps its existing
/// generic "401 + unauthorized" match so token cleanup + `DomainEvent::SessionExpired`
/// publish still fires for every 401. Adding the demote here therefore does
/// **not** silence the auto-cleanup teardown — it only stops the duplicate
/// per-attempt error event that escaped via `report_error_or_expected` from
/// the agent / web-channel layers (OPENHUMAN-TAURI-26).
pub fn is_session_expired_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("session expired")
        || lower.contains("no backend session token")
        || lower.contains("session jwt required")
        || msg.contains("SESSION_EXPIRED")
}

/// Detect transport-level connection failures that fire before any HTTP status
/// is observed — DNS resolution failures, TCP connect refused/reset, TLS
/// handshake failures, or ISP/firewall blocks. The canonical shape is
/// reqwest's `"error sending request for url (…)"`, which surfaces from any
/// HTTP call site (provider chat, embeddings, backend RPC) when the request
/// can't reach the server at all.
///
/// These are user-environment problems — VPN drop, captive portal, ISP-level
/// block (OPENHUMAN-TAURI-32: user in RU couldn't reach `api.tinyhumans.ai`),
/// firewall — that no amount of retry / fallback on our side can resolve.
/// Sentry has no signal to act on (no status, no trace, no payload), so each
/// occurrence is pure noise. Classify them as expected so the report site
/// logs a breadcrumb rather than spawning an error event.
fn is_network_unreachable_message(lower: &str) -> bool {
    lower.contains("error sending request for url")
        || lower.contains("dns error")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("network is unreachable")
        || lower.contains("no route to host")
        || lower.contains("tls handshake")
        || lower.contains("certificate verify failed")
}

/// Detect transient upstream HTTP failures that have bubbled up out of the
/// provider layer and into higher-level domains (`agent`, `web_channel`, …).
///
/// The reliable-provider stack already retries / falls back on
/// [`TRANSIENT_PROVIDER_HTTP_STATUSES`] (408/429/502/503/504), and the
/// `before_send` filter drops the per-attempt provider events that carry
/// `domain=llm_provider`. But the same error is *also* returned via
/// `Result::Err` and re-reported by callers that wrap the provider — e.g.
/// `agent.run_single` (OPENHUMAN-TAURI-5Z), `web_channel.run_chat_task`,
/// scheduler tick handlers — under a different `domain` tag, escaping the
/// provider-scoped filter and producing one Sentry event per failed turn.
///
/// The canonical wire format from `providers::ops::api_error` is:
/// `"<provider> API error (<status>): <sanitized>"` — e.g.
/// `"OpenHuman API error (504 Gateway Timeout): error code: 504"`. Pin the
/// match to that exact `"api error (<status>"` prefix so an unrelated message
/// that merely mentions "504" (a log line, a doc URL) is not silenced.
///
/// Also matches the second canonical wire shape: tungstenite's
/// `WsError::Http(response)` Display, which renders as `"HTTP error: <status>"`
/// (and which `socket::ws_loop::run_connection` wraps as
/// `"WebSocket connect: HTTP error: 502 Bad Gateway"`). Per
/// OPENHUMAN-TAURI-5P (~110 events) and -EZ (~51 events), backend
/// staging/production load balancers emit HTTP 502/504 during the WebSocket
/// upgrade handshake; tungstenite surfaces those as `WsError::Http` and the
/// socket reconnect loop already handles them via exponential backoff. Each
/// `FAIL_ESCALATE_THRESHOLD` escalation fires `report_error_or_expected` with
/// the formatted reason, which would land in Sentry as `domain=socket`
/// noise without this matcher (the existing `domain=integrations`
/// before_send filter scopes too narrowly).
///
/// Three separator variants cover every observed shape: trailing space
/// (`"HTTP error: 502 Bad Gateway"`), trailing newline (`"HTTP error: 502\n…"`
/// from chained errors), and trailing colon (`"HTTP error: 502: …"`). Bare
/// `"HTTP error: 502"` at end-of-string is not matched on purpose — the
/// status integer alone could collide with unrelated log lines containing
/// `"HTTP error: 5023"` (port number, runbook ID).
fn is_transient_upstream_http_message(lower: &str) -> bool {
    TRANSIENT_PROVIDER_HTTP_STATUSES.iter().any(|code| {
        lower.contains(&format!("api error ({code}"))
            || lower.contains(&format!("http error: {code} "))
            || lower.contains(&format!("http error: {code}\n"))
            || lower.contains(&format!("http error: {code}:"))
    })
}

/// Detect non-2xx HTTP failures returned from the backend integrations / composio
/// clients that are by definition user-input or user-auth-state problems — not
/// bugs Sentry can act on.
///
/// The canonical wire format from
/// [`crate::openhuman::integrations::client::IntegrationClient::post`] / `get`
/// and [`crate::openhuman::composio::client::ComposioClient`] is:
/// `"Backend returned <status> <reason> for <METHOD> <url>: <detail>"` — e.g.
/// `"Backend returned 400 Bad Request for POST https://api.tinyhumans.ai/agent-integrations/composio/authorize: Composio authorization failed: 400 …"`
/// (OPENHUMAN-TAURI-BC: user submitted SharePoint authorize without filling in
/// the required Tenant Name field). The backend correctly returned a 4xx; the
/// UI already surfaces the structured error to the user via toast — Sentry has
/// no remediation path because the request was malformed *by the user's
/// input*, not by our code.
///
/// We pin the match to the `"backend returned "` prefix so an unrelated
/// message merely mentioning "400" (a log line, doc URL) is not silenced.
///
/// We classify only 4xx codes, with **two exclusions**:
/// - `408 Request Timeout` and `429 Too Many Requests` are *transient* — they
///   are surfaced via [`is_transient_upstream_http_message`] for the provider
///   path and stay actionable for the backend path so a sustained 429 (rate
///   limit cliff) still pages.
///
/// 5xx is intentionally **not** classified here — server-side failures from
/// our backend are real bugs that should reach Sentry. The transient
/// 502/503/504 deduplication is handled by the threshold logic in callers
/// (see e.g. `openhuman::socket::ws_loop::FAIL_ESCALATE_THRESHOLD`).
fn is_backend_user_error_message(lower: &str) -> bool {
    let Some(rest) = lower.split_once("backend returned ").map(|(_, r)| r) else {
        return false;
    };
    let status_digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let Ok(status) = status_digits.parse::<u16>() else {
        return false;
    };
    // 4xx (except transient 408 / 429 which are handled separately).
    matches!(status, 400..=499) && status != 408 && status != 429
}

/// Detect third-party provider validation failures that bubble up as
/// user-state errors — composio trigger registry mismatch, toolkit not
/// enabled, OAuth scopes missing, required fields left blank.
///
/// Unlike [`is_backend_user_error_message`], this classifier is **body-text
/// shape-based** rather than HTTP-status-based, so it catches the cases
/// where the composio backend wraps a Composio API 4xx as a 500 with the
/// real validation message embedded in the body (OPENHUMAN-TAURI-3R / -3S
/// / -97 — `"Backend returned 500 … Trigger type GITHUB_PUSH_EVENT not
/// found"`, `"Backend returned 500 … Missing required fields: Your
/// Subdomain"`). These would otherwise escape the 4xx-only matcher and
/// fire as actionable Sentry events even though the underlying condition
/// is user-state (the trigger slug isn't in composio's registry, the
/// toolkit wasn't enabled by the user, the form field was left blank, …).
///
/// Also handles the gmail-sync 403 (OPENHUMAN-TAURI-33) where the
/// composio sync loop surfaces the upstream Google OAuth scopes error as
/// `"HTTP 403: Request had insufficient authentication scopes."`. The
/// remediation is "user re-authorizes with the right scope" — nothing
/// Sentry can act on.
///
/// All matches are substring-based against the lower-cased message so the
/// classifier survives caller wrapping (rpc.invoke_method, agent.run_single,
/// `[composio:gmail]` prefixes, anyhow chains, …).
fn is_provider_user_state_message(lower: &str) -> bool {
    // OPENHUMAN-TAURI-3R / -3S: composio enable_trigger when the slug isn't
    // in the trigger registry (e.g. user clicked a stale UI option).
    // Backend returns 500 with `"Trigger type GITHUB_PUSH_EVENT not found"`.
    // Also covers the alternate phrasing `"Cannot enable trigger … not found"`.
    if (lower.contains("trigger type ") && lower.contains("not found"))
        || (lower.contains("cannot enable trigger") && lower.contains("not found"))
    {
        return true;
    }

    // OPENHUMAN-TAURI-34: composio rejected a tool call because the user
    // hasn't enabled the toolkit yet. Wire shape:
    // `Backend returned 400 … Toolkit "get" is not enabled`.
    if lower.contains("toolkit ") && lower.contains("is not enabled") {
        return true;
    }

    // OPENHUMAN-TAURI-97: composio authorize with a blank required field —
    // SharePoint Subdomain, WhatsApp WABA ID, Tenant Name, etc.
    // Backend returns 500 with `"Missing required fields: …"` body.
    //
    // **Intentionally broad** — unlike the trigger/toolkit arms, this is a
    // single substring with no second anchor. Composio's wire shape varies
    // per provider (`Missing required fields: Tenant Name`, `Missing
    // required fields: Your Subdomain (example: 'your-subdomain' for…)`,
    // `Missing required fields: WABA ID (WhatsApp Business Account ID…)`)
    // and embedding every variant would be brittle. Accepted false-positive
    // surface: a non-composio caller whose error happens to contain
    // `"missing required fields"` (e.g. `"Internal error: missing required
    // fields in config"`) will also demote to info. This is fine — every
    // current emit site routed through `report_error_or_expected` is scoped
    // to composio / integrations envelopes, so a stray collision would have
    // to come from a brand-new call site that explicitly opts in.
    // See `unrelated_missing_required_fields_classifies_as_accepted_false_positive`
    // for the documented surface.
    if lower.contains("missing required fields") {
        return true;
    }

    // OPENHUMAN-TAURI-33: gmail sync hit an OAuth scope wall —
    // `HTTP 403: Request had insufficient authentication scopes.`
    // (or any sibling OAuth scope rejection from composio's toolkits).
    if lower.contains("insufficient authentication scopes") {
        return true;
    }

    false
}

/// Detect "<capability> is disabled / unavailable for this RAM tier" errors
/// emitted by the local-AI service when the user's hardware tier doesn't
/// support a capability (OPENHUMAN-TAURI-3B: vision asset download invoked
/// on a 0–4 GB tier). These are pure user-state conditions — the local-AI
/// service surfaces them so the UI can prompt the user to switch tiers —
/// and carry no remediable signal for Sentry.
///
/// The two canonical wire shapes today both contain `"for this ram tier"`:
///
/// - `"Vision is disabled for this RAM tier. Switch to the 4-8 GB tier or
///   above to enable it."` — from `local_ai/service/assets.rs::ensure_capability_ready`
/// - `"vision summaries are unavailable for this RAM tier. Use OCR-only
///   summarization or switch to a higher local AI tier."` —
///   from `local_ai/service/vision_embed.rs::summarize`
///
/// Anchor the classifier to that exact substring so an unrelated message
/// that merely mentions "RAM tier" out of context is not silenced.
fn is_local_ai_capability_unavailable_message(lower: &str) -> bool {
    lower.contains("for this ram tier")
}

/// Capture an error to Sentry with structured tags.
///
/// `domain` and `operation` are required and become tags `domain:<…>` and
/// `operation:<…>`. `extra` is an optional list of extra tag pairs. The error
/// itself is rendered via `Display` and emitted as a `tracing::error!` event,
/// which the Sentry tracing layer turns into a Sentry event under the active
/// scope.
///
/// Use stable, low-cardinality values for tag keys/values so Sentry can group
/// and aggregate. High-cardinality data (full IDs, payloads) belongs in the
/// error message body, not in tags.
pub fn report_error<E: Display + ?Sized>(
    err: &E,
    domain: &str,
    operation: &str,
    extra: &[Tag<'_>],
) {
    // Use the alternate format specifier so `anyhow::Error` renders its full
    // context chain (outer context + every wrapped cause, joined by ": ").
    // Plain `Display` impls fall back to the standard representation. Without
    // this, anyhow's default `to_string()` only emits the outermost context
    // and the underlying cause (e.g. a `toml::de::Error` with line/column) is
    // dropped — making the captured Sentry event undiagnosable. See
    // OPENHUMAN-TAURI-B2 for an instance where this masked the real failure.
    let message = format!("{err:#}");
    report_error_message(&message, domain, operation, extra);
}

/// Report an error unless it is an expected user-state/config condition.
///
/// Expected conditions are logged at `info` or `warn` so the Sentry tracing
/// layer records at most a breadcrumb, not an error event.
pub fn report_error_or_expected<E: Display + ?Sized>(
    err: &E,
    domain: &str,
    operation: &str,
    extra: &[Tag<'_>],
) {
    let message = format!("{err:#}");
    if let Some(kind) = expected_error_kind(&message) {
        report_expected_message(kind, &message, domain, operation);
        return;
    }
    report_error_message(&message, domain, operation, extra);
}

fn report_expected_message(kind: ExpectedErrorKind, message: &str, domain: &str, operation: &str) {
    match kind {
        ExpectedErrorKind::LocalAiDisabled => {
            tracing::info!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected local-ai disabled error: {message}"
            );
        }
        ExpectedErrorKind::ApiKeyMissing => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected API-key configuration error: {message}"
            );
        }
        ExpectedErrorKind::NetworkUnreachable => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected network-unreachable error: {message}"
            );
        }
        ExpectedErrorKind::TransientUpstreamHttp => {
            tracing::warn!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped transient upstream HTTP error: {message}"
            );
        }
        ExpectedErrorKind::LocalAiBinaryMissing => {
            // User-state condition: piper / whisper.cpp / Ollama binary
            // isn't installed on this host. The error message itself is
            // the user-facing instruction ("Set PIPER_BIN or install
            // piper.") — Sentry has nothing to act on, since we can't
            // install the binary for them. OPENHUMAN-TAURI-9N is the
            // canonical instance: `local_ai_tts` fails immediately
            // (elapsed_ms=1) on a Windows host without piper installed.
            tracing::info!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected local-ai binary-missing error: {message}"
            );
        }
        ExpectedErrorKind::BackendUserError => {
            // 4xx from the integrations / composio backend client —
            // user-input or auth-state failure that the backend already
            // surfaced to the user via the structured error toast.
            // OPENHUMAN-TAURI-BC: SharePoint authorize 400 because the
            // user didn't fill in the required Tenant Name field.
            tracing::warn!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected backend user-error response: {message}"
            );
        }
        ExpectedErrorKind::ProviderUserState => {
            // Third-party provider (composio, gmail OAuth, …) rejected the
            // request for a user-state reason: trigger slug missing from
            // composio's registry (OPENHUMAN-TAURI-3R / -3S), toolkit not
            // enabled (OPENHUMAN-TAURI-34), OAuth scopes missing
            // (OPENHUMAN-TAURI-33), or a required form field was left blank
            // (OPENHUMAN-TAURI-97). The UI already surfaces the actionable
            // error to the user — Sentry has no remediation path.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "provider_user_state",
                error = %message,
                "[observability] {domain}.{operation} skipped expected provider-user-state error: {message}"
            );
        }
        ExpectedErrorKind::LocalAiCapabilityUnavailable => {
            // User-state condition: the local-AI service refused a
            // capability (vision summarization, vision asset download)
            // because the user's RAM tier doesn't support it. The
            // error message itself is the user-facing remediation
            // ("Switch to the 4-8 GB tier or above to enable it.") —
            // Sentry has nothing to act on. OPENHUMAN-TAURI-3B: 28
            // hits in 4 days from `local_ai_download_asset` on a
            // 0–4 GB tier requesting vision.
            tracing::info!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected local-ai capability-unavailable error: {message}"
            );
        }
        ExpectedErrorKind::BudgetExhausted => {
            // User-state condition: the backend reports the user is out of
            // budget / credits / balance (HTTP 400 from the OpenHuman backend,
            // surfaced by `providers::is_budget_exhausted_message`). The UI
            // already surfaces this as an actionable toast — Sentry would
            // turn each affected turn into noise (OPENHUMAN-TAURI-3M / -12 /
            // -13). Demote to info so it still appears in breadcrumbs but
            // never spawns a Sentry error event.
            tracing::info!(
                domain = domain,
                operation = operation,
                kind = "budget",
                error = %message,
                "[observability] {domain}.{operation} skipped expected budget-exhausted error: {message}"
            );
        }
        ExpectedErrorKind::SessionExpired => {
            // Auth-boundary condition: the user's JWT expired (or was never
            // present). The JSON-RPC dispatch layer already handles the
            // teardown — `Err` propagation publishes `DomainEvent::SessionExpired`
            // which clears the stored token and flips the scheduler-gate
            // signed-out override so background workers stand down — and the
            // UI re-auths the user. The per-attempt error event from the
            // upstream call site (agent.run_single, web_channel.run_chat_task)
            // adds noise without signal: every mid-conversation 401 would
            // emit one event before the cascade dampener kicks in
            // (OPENHUMAN-TAURI-26, and the same upstream gap that
            // OPENHUMAN-TAURI-1T's #1516 cascade fix dampened but did not
            // close). Demote to info so the breadcrumb survives for trace
            // correlation but Sentry sees no error event.
            tracing::info!(
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} skipped expected session-expired error: {message}"
            );
        }
    }
}

/// Distinct `tracing::Metadata::target()` we set on the diagnostic
/// `tracing::error!` emitted from [`report_error_message`].
///
/// Sentry capture for this helper happens via an explicit
/// `sentry::capture_message` call below — not via the `sentry-tracing`
/// layer scooping up the `tracing::error!` event. The production
/// `sentry_tracing_layer()` in `core::logging` filters events with this
/// target to `EventFilter::Ignore` so we never double-report (one direct
/// `capture_message`, one tracing-bridge capture of the same condition).
///
/// Why direct capture instead of relying on the bridge: the bridge worked
/// in steady-state but flaked under parallel test scheduling
/// (`thread_not_found_rpc_error_does_not_report_to_sentry` repeatedly hit
/// `events.len() == 0` in CI even with a thread-default subscriber wired
/// up — likely a Linux-only thread-local ordering quirk in
/// `sentry-tracing`'s `Hub::current()` lookup at event-emit time). Direct
/// `sentry::capture_message` synchronously routes through the active hub
/// and is deterministic, which keeps both production reporting and tests
/// honest.
pub const REPORT_ERROR_TRACING_TARGET: &str = "openhuman::observability::report_error";

pub(crate) fn report_error_message(
    message: &str,
    domain: &str,
    operation: &str,
    extra: &[Tag<'_>],
) {
    sentry::with_scope(
        |scope| {
            scope.set_tag("domain", domain);
            scope.set_tag("operation", operation);
            for (k, v) in extra {
                scope.set_tag(k, v);
            }
        },
        || {
            // Direct, synchronous Sentry capture — see
            // `REPORT_ERROR_TRACING_TARGET` for why we don't rely on the
            // `sentry-tracing` layer for this call site.
            sentry::capture_message(message, sentry::Level::Error);
            // Diagnostic log line for stderr / file appenders. Tagged with
            // the marker target so the production sentry-tracing layer
            // skips it (no double Sentry event).
            tracing::error!(
                target: REPORT_ERROR_TRACING_TARGET,
                domain = domain,
                operation = operation,
                error = %message,
                "[observability] {domain}.{operation} failed: {message}"
            );
        },
    );
}

/// Returns true when a Sentry event is a per-attempt provider HTTP failure
/// that the reliable-provider layer already handles via retry + fallback.
///
/// The primary suppression lives at the call site
/// (`openhuman::providers::ops::should_report_provider_http_failure`),
/// which short-circuits transient codes before `report_error` ever fires.
/// This helper is intended for use inside the `sentry::ClientOptions`
/// `before_send` hook as defense-in-depth — it catches any future call
/// site that emits a `tracing::error!` with the same shape but bypasses
/// the classifier.
///
/// Match criteria (all required):
/// - tag `domain == "llm_provider"` — pins the filter to provider-originated
///   events so an unrelated subsystem emitting `failure=non_2xx`/`status=503`
///   for its own reasons doesn't get silently dropped
/// - tag `failure == "non_2xx"` (the marker set by `ops::api_error`)
/// - tag `status` parses to one of [`TRANSIENT_PROVIDER_HTTP_STATUSES`]
pub fn is_transient_provider_http_failure(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("llm_provider") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    let Some(status_u16) = tags.get("status").and_then(|s| s.parse::<u16>().ok()) else {
        return false;
    };
    TRANSIENT_PROVIDER_HTTP_STATUSES.contains(&status_u16)
}

/// Returns true when a Sentry event's message/exception text contains the
/// canonical max-tool-iterations cap phrase (see
/// `openhuman::agent::error::MAX_ITERATIONS_ERROR_PREFIX`).
///
/// Defense-in-depth filter for the Sentry `before_send` hook: the primary
/// suppression lives at the call sites in `agent::harness::session::
/// runtime::run_single`, `channels::runtime::dispatch`, and
/// `channels::providers::web::run_chat_task`, all of which now skip
/// `report_error` when this variant is detected. This filter catches any
/// future call site that re-emits the message without going through those
/// funnels — e.g. a new wrapper that calls `tracing::error!` directly with
/// the typed error rendering — and keeps OPENHUMAN-TAURI-99 / -98
/// permanently off Sentry without requiring touch-ups at each new site.
///
/// Match strategy: scans `event.message` first (the path used by
/// `report_error_message` → `sentry::capture_message`) and falls back to
/// the last exception's `value` (the shape `sentry-tracing` produces when
/// stacktraces are attached). Both fields are checked for the canonical
/// prefix so the filter stays robust to future Sentry plumbing changes.
pub fn is_max_iterations_event(event: &sentry::protocol::Event<'_>) -> bool {
    let direct = event.message.as_deref();
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_exception]
        .into_iter()
        .flatten()
        .any(crate::openhuman::agent::error::is_max_iterations_error)
}

/// Tag + body classifier for the `before_send` chain — drops Sentry events
/// emitted at the OpenHuman backend / rpc layers for "401 Session
/// expired" or the pre-flight "no session token stored" guards.
///
/// Pairs with [`is_session_expired_message`] (which classifies the
/// message body at the emit site via `report_error_or_expected`). This
/// fn runs in `before_send` so it catches any future call site that
/// re-emits the same shape without routing through the classifier —
/// keeps OPENHUMAN-TAURI-25 / -1Q / -27 / -1G permanently off Sentry
/// (~185 events/day combined).
///
/// Scope: only the three domains that surface session-expired today
/// (`llm_provider`, `backend_api`, `rpc`). Composio's OAuth-state 401
/// is excluded — that's actionable and must reach Sentry.
pub fn is_session_expired_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    let Some(domain) = tags.get("domain").map(String::as_str) else {
        return false;
    };
    if !matches!(domain, "llm_provider" | "backend_api" | "rpc") {
        return false;
    }

    let status_is_401 = tags
        .get("status")
        .and_then(|s| s.parse::<u16>().ok())
        .is_some_and(|code| code == 401);

    let direct = event.message.as_deref();
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    let body_matches = [direct, from_exception]
        .into_iter()
        .flatten()
        .any(is_session_expired_message);

    if status_is_401 && body_matches {
        return true;
    }

    // Pre-flight rpc guard has no status tag — accept on body alone,
    // scoped to the rpc dispatcher (other domains don't emit the
    // "no session token stored" sentinel).
    if domain == "rpc" && body_matches {
        return true;
    }

    false
}

pub fn is_transient_http_status(status: &str) -> bool {
    TRANSIENT_HTTP_STATUSES.contains(&status)
}

pub fn is_transient_http_status_code(status: u16) -> bool {
    let status = status.to_string();
    is_transient_http_status(status.as_str())
}

pub fn contains_transient_transport_phrase(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    TRANSIENT_TRANSPORT_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

pub fn is_updater_transient_http_status(status: u16) -> bool {
    UPDATER_TRANSIENT_HTTP_STATUSES.contains(&status)
}

pub fn is_updater_transient_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    UPDATER_TRANSIENT_MESSAGE_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
}

fn event_has_transient_transport_phrase(event: &sentry::protocol::Event<'_>) -> bool {
    event
        .message
        .as_deref()
        .is_some_and(contains_transient_transport_phrase)
        || event
            .logentry
            .as_ref()
            .is_some_and(|log| contains_transient_transport_phrase(&log.message))
        || event.exception.values.iter().any(|exception| {
            exception
                .value
                .as_deref()
                .is_some_and(contains_transient_transport_phrase)
        })
}

fn event_has_updater_transient_message(event: &sentry::protocol::Event<'_>) -> bool {
    event
        .message
        .as_deref()
        .is_some_and(is_updater_transient_message)
        || event
            .logentry
            .as_ref()
            .is_some_and(|log| is_updater_transient_message(&log.message))
        || event.exception.values.iter().any(|exception| {
            exception
                .value
                .as_deref()
                .is_some_and(is_updater_transient_message)
        })
}

fn event_has_updater_domain(event: &sentry::protocol::Event<'_>) -> bool {
    matches!(
        event.tags.get("domain").map(String::as_str),
        Some("update") | Some("update.check_releases") | Some("updater")
    )
}

fn is_transient_domain_failure(event: &sentry::protocol::Event<'_>, domain: &str) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some(domain) {
        return false;
    }

    match tags.get("failure").map(String::as_str) {
        Some("non_2xx") => tags
            .get("status")
            .is_some_and(|status| is_transient_http_status(status)),
        Some("transport") => event_has_transient_transport_phrase(event),
        _ => false,
    }
}

/// Transient backend API failures (gateway hiccups, scheduled downtime).
/// Match by event tags written by report_error at the authed_json call site.
pub fn is_transient_backend_api_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "backend_api")
}

/// Transient integrations / Composio failures (timeout, connection reset,
/// gateway hiccups).
///
/// Accepts both `domain="integrations"` (the shared
/// [`crate::openhuman::integrations::IntegrationClient`] HTTP wrapper that
/// fronts every backend-proxied integration) and `domain="composio"` (errors
/// reported from the Composio op layer in
/// [`crate::openhuman::composio::ops`]). Composio routes through the same
/// `IntegrationClient`, so the failure shape is identical — but op-level
/// reporters that wrap and re-emit those errors with their own domain tag
/// would otherwise escape the integrations-scoped filter (OPENHUMAN-TAURI-35
/// ~139ev, -2H ~26ev: `[composio] list_connections failed: Backend returned
/// 502 …` events that landed in Sentry under `domain=composio`).
pub fn is_transient_integrations_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "integrations")
        || is_transient_domain_failure(event, "composio")
}

/// Transient updater failures from GitHub release probes/downloads.
///
/// Core-side reports carry structured tags (`domain=update`, often
/// `operation=check_releases`, plus `failure/status`). Tauri's updater plugin
/// can also emit message-only events such as
/// `"failed to check for updates: error sending request for url (...latest.json)"`.
/// Match both shapes, but never drop an arbitrary update-domain event unless
/// it also has a transient status/transport marker.
pub fn is_updater_transient_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event_has_updater_transient_message(event) {
        return true;
    }

    if !event_has_updater_domain(event) {
        return false;
    }

    match event.tags.get("failure").map(String::as_str) {
        Some("non_2xx") => event
            .tags
            .get("status")
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(is_updater_transient_http_status),
        Some("transport") => event_has_transient_transport_phrase(event),
        _ => false,
    }
}

/// String tokens that mark a formatted error message as a transient HTTP
/// failure. Used at upstream emit sites (`rpc.invoke_method`,
/// `web_channel.run_chat_task`) where the error has already been stringified
/// and the original `status` / `failure` tag context is gone.
///
/// Each token combines a status code with a non-numeric anchor (parenthesis
/// or canonical reason phrase) so bare numeric coincidences ("process 502
/// exited") do not match.
const TRANSIENT_STATUS_MESSAGE_TOKENS: &[&str] = &[
    "(408 ",
    "(429 ",
    "(502 ",
    "(503 ",
    "(504 ",
    "(520 ",
    "408 request timeout",
    "429 too many requests",
    "502 bad gateway",
    "503 service unavailable",
    "504 gateway timeout",
    "520 <unknown status code>",
];

/// Returns true when a formatted error message describes a transient HTTP
/// or transport-layer failure that has already been demoted further down the
/// stack. Use at upstream re-emit sites (`rpc.invoke_method`,
/// `web_channel.run_chat_task`) where `report_error` is called with the
/// stringified downstream error and no `failure` / `status` tag context.
pub fn is_transient_message_failure(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    TRANSIENT_STATUS_MESSAGE_TOKENS
        .iter()
        .any(|token| lower.contains(token))
        || contains_transient_transport_phrase(&lower)
}

/// Returns true when a Sentry event is a budget-exhausted 400 that should be
/// dropped from `before_send`.
///
/// Match criteria (all required):
/// - tag `failure == "non_2xx"`
/// - tag `status == "400"`
/// - the event message or any exception value contains one of the tight
///   budget-exhaustion phrases
///
/// Note: `domain` is intentionally not gated here as defense-in-depth over
/// the emit-site classifier — any non_2xx/400 event that carries the
/// budget-exhausted phrasing is dropped regardless of which domain produced
/// it, so a future re-emitter under a different tag still gets filtered.
pub fn is_budget_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    if tags.get("status").map(String::as_str) != Some("400") {
        return false;
    }
    event_contains_budget_exhausted_message(event)
}

fn event_contains_budget_exhausted_message(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(crate::openhuman::providers::is_budget_exhausted_message)
    {
        return true;
    }

    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(crate::openhuman::providers::is_budget_exhausted_message)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper must accept `&anyhow::Error`, `&dyn std::error::Error`, and
    /// plain `&str` — the three shapes that show up at error sites today.
    #[test]
    fn report_error_accepts_common_error_shapes() {
        let anyhow_err = anyhow::anyhow!("boom");
        report_error(&anyhow_err, "test", "anyhow_shape", &[]);

        let io_err = std::io::Error::other("io failed");
        report_error(&io_err, "test", "io_shape", &[("kind", "io")]);

        report_error("plain message", "test", "str_shape", &[]);
    }

    #[test]
    fn anyhow_chain_is_rendered_in_full() {
        // Regression guard: `err.to_string()` on an anyhow chain only emits
        // the outermost context. Using `{:#}` joins every cause, which is
        // what Sentry needs to actually diagnose wrapped failures.
        let inner = std::io::Error::other("inner cause");
        let wrapped = anyhow::Error::from(inner).context("outer ctx");
        assert_eq!(format!("{wrapped:#}"), "outer ctx: inner cause");
    }

    #[test]
    fn classifies_expected_config_errors() {
        assert_eq!(
            expected_error_kind("rpc.invoke_method failed: local ai is disabled"),
            Some(ExpectedErrorKind::LocalAiDisabled)
        );
        assert_eq!(
            expected_error_kind(
                "agent.provider_chat failed: ollama API key not set. Configure via the web UI"
            ),
            Some(ExpectedErrorKind::ApiKeyMissing)
        );
        assert_eq!(
            expected_error_kind("ollama embed failed with status 500"),
            None
        );
    }

    #[test]
    fn classifies_local_ai_capability_unavailable_errors() {
        // OPENHUMAN-TAURI-3B: surfaced by `local_ai_download_asset` when a
        // user on a 0–4 GB RAM tier requests a vision asset. Both canonical
        // wire shapes — emitted from `assets.rs` and `vision_embed.rs` —
        // must classify as expected so they stop reaching Sentry.
        for raw in [
            "Vision is disabled for this RAM tier. Switch to the 4-8 GB tier or above to enable it.",
            "vision summaries are unavailable for this RAM tier. Use OCR-only summarization or switch to a higher local AI tier.",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::LocalAiCapabilityUnavailable),
                "should classify as local-ai capability unavailable: {raw}"
            );
        }

        // Wrapped by the RPC dispatch layer as it reaches `report_error_or_expected`
        // — the classifier is substring-based, so caller context must not defeat it.
        assert_eq!(
            expected_error_kind(
                "rpc.invoke_method failed: Vision is disabled for this RAM tier. Switch to the 4-8 GB tier or above to enable it."
            ),
            Some(ExpectedErrorKind::LocalAiCapabilityUnavailable)
        );
    }

    #[test]
    fn does_not_classify_unrelated_messages_as_capability_unavailable() {
        // The classifier anchors on the exact "for this RAM tier" substring.
        // Messages that talk about RAM in a different context (sizing the
        // tier list, doc references) must not be silenced.
        assert_eq!(expected_error_kind("ollama embed failed: out of RAM"), None);
        assert_eq!(
            expected_error_kind("local_ai_set_ram_tier failed: invalid tier value"),
            None
        );
    }

    #[test]
    fn classifies_network_unreachable_errors() {
        // OPENHUMAN-TAURI-32: reqwest's transport-level error wrapped by the
        // web_channel error site. The classifier must catch it even when
        // embedded in caller context, since `report_error_or_expected` runs
        // `expected_error_kind` on the full anyhow chain.
        assert_eq!(
            expected_error_kind(
                "run_chat_task failed client_id=abc thread_id=t1 request_id=r1 \
                 error=error sending request for url (https://api.tinyhumans.ai/openai/v1/chat/completions)"
            ),
            Some(ExpectedErrorKind::NetworkUnreachable)
        );
        for raw in [
            "error sending request for url (https://api.example.com/x)",
            "provider failed: dns error: failed to lookup address information",
            "tcp connect: connection refused (os error 61)",
            "stream closed: connection reset by peer",
            "network is unreachable (os error 51)",
            "no route to host",
            "tls handshake eof",
            "certificate verify failed: unable to get local issuer certificate",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::NetworkUnreachable),
                "should classify as network-unreachable: {raw}"
            );
        }
    }

    #[test]
    fn does_not_classify_unrelated_provider_errors_as_network() {
        // Status-bearing provider failures (404, 500, …) are surfaced via
        // their HTTP status path and must NOT be silenced by the
        // network-unreachable classifier — the body text doesn't hit any of
        // the transport-level markers.
        assert_eq!(
            expected_error_kind("OpenAI API error (404): model gpt-x not found"),
            None
        );
        assert_eq!(
            expected_error_kind("OpenAI API error (500): internal server error"),
            None
        );
    }

    #[test]
    fn classifies_transient_upstream_http_errors() {
        // OPENHUMAN-TAURI-5Z: the canonical shape emitted by
        // `providers::ops::api_error` and re-raised through `agent.run_single`.
        assert_eq!(
            expected_error_kind("OpenHuman API error (504 Gateway Timeout): error code: 504"),
            Some(ExpectedErrorKind::TransientUpstreamHttp)
        );

        // Every transient code must classify, whether the status renders as
        // bare digits or "<digits> <reason>".
        for raw in [
            "OpenHuman API error (408): request timeout",
            "OpenAI API error (429 Too Many Requests): rate limit",
            "Anthropic API error (502 Bad Gateway): upstream unhealthy",
            "OpenHuman API error (503): service unavailable",
            "Provider API error (504): upstream timed out",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::TransientUpstreamHttp),
                "should classify as transient upstream HTTP: {raw}"
            );
        }

        // Wrapped in an anyhow chain (as it reaches the agent layer) must
        // still classify — `expected_error_kind` is substring-based.
        assert_eq!(
            expected_error_kind(
                "agent turn failed: OpenHuman API error (504 Gateway Timeout): \
                 error code: 504"
            ),
            Some(ExpectedErrorKind::TransientUpstreamHttp)
        );
    }

    #[test]
    fn integrations_post_composio_timeout_dropped() {
        // OPENHUMAN-TAURI-18 / -G regression guard. The integrations
        // client at `crate::openhuman::integrations::client::IntegrationClient::post`
        // builds the reqwest error chain and routes it through
        // `report_error_or_expected(.., "integrations", "post", &[("failure",
        // "transport")])`. The chain text contains the
        // `"error sending request for url"` anchor so
        // `is_network_unreachable_message` matches first and demotes to
        // `NetworkUnreachable` (functionally equivalent to
        // `TransientUpstreamHttp` for Sentry suppression — both routes
        // skip the report path via `report_expected_message`).
        //
        // Pinning this exact wire shape catches a future refactor that
        // drops the URL anchor (e.g. a chain-flatten helper that strips
        // it for "PII safety"), which would silently re-open the leak.
        let chain = "error sending request for url \
                     (https://api.tinyhumans.ai/agent-integrations/composio/execute) → \
                     client error (SendRequest) → connection error → \
                     Operation timed out (os error 60)";
        assert_eq!(
            expected_error_kind(chain),
            Some(ExpectedErrorKind::NetworkUnreachable),
            "TAURI-18 chain shape must classify as NetworkUnreachable"
        );

        // If the URL anchor is ever dropped, the transport-phrase
        // fallback (`operation timed out` from
        // `TRANSIENT_TRANSPORT_PHRASES`) catches it via the message
        // classifier helper used at upstream re-emit sites — confirm
        // both paths so the regression surface is fully pinned.
        assert!(
            is_transient_message_failure(chain),
            "TAURI-18 chain must also satisfy upstream message classifier \
             (defense-in-depth for sites that lose the URL anchor)"
        );
    }

    #[test]
    fn channels_dispatch_re_emit_of_provider_502_classifies_as_transient() {
        // OPENHUMAN-TAURI-4F (~157 events) / -1C (~87 events) / -8F
        // (~39 events): the reliable provider layer retried 5xx, the
        // agent re-raised the error, and `channels::runtime::dispatch`
        // re-emitted it under `domain="channels", operation="dispatch_llm_error"`
        // via raw `report_error` (which skips classification). Switching
        // that site to `report_error_or_expected` routes the chain
        // through this classifier — but only works if the canonical
        // `"OpenHuman API error (NNN ...)"` substring still anchors the
        // match through the channels-layer wrapping.
        //
        // The wrapping shape at the dispatch site is the agent error
        // chain rendered via `format!("{e:#}")`. For a backend 502 from
        // `providers::ops::api_error`, that resolves to:
        //   "OpenHuman API error (502 Bad Gateway): error code: 502"
        // possibly prepended with a runner / iteration prefix. Both
        // shapes must classify as transient so the dispatch re-emit
        // gets demoted.
        for raw in [
            "OpenHuman API error (502 Bad Gateway): error code: 502",
            "agent.provider_chat failed: OpenHuman API error (503 Service Unavailable): retry budget exhausted",
            "all providers exhausted: OpenHuman API error (504 Gateway Timeout): error code: 504",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::TransientUpstreamHttp),
                "channels.dispatch re-emit of {raw:?} must classify as transient"
            );
        }
    }

    #[test]
    fn classifies_socket_transient_http_errors() {
        // OPENHUMAN-TAURI-5P / -EZ: tungstenite's `WsError::Http(response)`
        // surfaces during the WebSocket upgrade handshake when the backend
        // load balancer returns 502 / 504. The socket reconnect loop wraps
        // it as `format!("WebSocket connect: {e}")`, producing
        // `"WebSocket connect: HTTP error: <status> <reason>"`. Each
        // sustained-outage threshold escalation routes the formatted reason
        // through `report_error_or_expected`, which must classify as
        // transient so the per-client noise stops reaching Sentry.
        for raw in [
            "WebSocket connect: HTTP error: 502 Bad Gateway",
            "WebSocket connect: HTTP error: 503 Service Unavailable",
            "WebSocket connect: HTTP error: 504 Gateway Timeout",
            "[socket] Connection failed (sustained outage after 5 attempts): \
             WebSocket connect: HTTP error: 502 Bad Gateway",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::TransientUpstreamHttp),
                "should classify as transient upstream HTTP (socket shape): {raw}"
            );
        }

        // Trailing-colon separator (chained error formatting).
        // Note: avoid words like "connection refused" or "timeout" in the
        // suffix — those would also match `is_network_unreachable_message` /
        // `TRANSIENT_TRANSPORT_PHRASES` and the order in `expected_error_kind`
        // would route through `NetworkUnreachable` first, defeating the
        // assertion. Both classifications silence the event so production
        // behavior is identical, but the test is anchored on the canonical
        // socket shape so a future regression in `is_transient_upstream_http_message`
        // surfaces here, not behind another classifier.
        assert_eq!(
            expected_error_kind(
                "WebSocket connect: HTTP error: 502: upstream returned bad gateway"
            ),
            Some(ExpectedErrorKind::TransientUpstreamHttp)
        );

        // Trailing-newline separator (multi-line error chain).
        assert_eq!(
            expected_error_kind("WebSocket connect: HTTP error: 504\nupstream gateway"),
            Some(ExpectedErrorKind::TransientUpstreamHttp)
        );
    }

    #[test]
    fn does_not_classify_unrelated_http_error_text_as_transient_socket() {
        // Bare numeric "HTTP error: 5023" (port number, runbook ID) without
        // a separator must NOT silence — pin the matcher to space/newline/colon.
        assert_eq!(expected_error_kind("HTTP error: 5023"), None);
        // Non-transient HTTP statuses must not match — `WsError::Http` for
        // a 401 / 403 / 404 is genuinely actionable (auth / routing bug).
        for raw in [
            "WebSocket connect: HTTP error: 401 Unauthorized",
            "WebSocket connect: HTTP error: 403 Forbidden",
            "WebSocket connect: HTTP error: 404 Not Found",
            "WebSocket connect: HTTP error: 500 Internal Server Error",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                None,
                "must NOT silence actionable socket HTTP error: {raw}"
            );
        }
    }

    #[test]
    fn does_not_classify_actionable_provider_errors_as_transient_upstream() {
        // 4xx (other than 408/429) and non-transient 5xx must continue to
        // reach Sentry — those are real bugs (wrong model name, malformed
        // request, internal exception) that need to be triaged.
        for raw in [
            "OpenAI API error (400): bad request",
            "OpenAI API error (401): unauthorized",
            "OpenAI API error (403): forbidden",
            "OpenAI API error (404): model not found",
            "OpenAI API error (500): internal server error",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                None,
                "must NOT silence actionable provider error: {raw}"
            );
        }

        // A free-form message that merely mentions "504" without the
        // `api error (` prefix must not be classified — pin the match to
        // the canonical shape from `ops::api_error`.
        assert_eq!(
            expected_error_kind("see runbook for 504 handling at https://example.com/504"),
            None
        );
    }

    #[test]
    fn classifies_backend_user_error_responses() {
        // OPENHUMAN-TAURI-BC: SharePoint authorize 400 because the user
        // didn't fill in the required Tenant Name field. After the
        // ProviderUserState classifier was added (#1472 wave E), this
        // canonical shape now lands in the more specific
        // ProviderUserState bucket — `"missing required fields"` wins
        // over the generic 4xx matcher. Either expected-kind silences
        // Sentry; the dedicated bucket gives operators a finer-grained
        // `kind="provider_user_state"` info-log facet for triage.
        let bc = "Backend returned 400 Bad Request for POST \
                  https://api.tinyhumans.ai/agent-integrations/composio/authorize: \
                  Composio authorization failed: 400 \
                  {\"error\":{\"message\":\"Missing required fields: Tenant Name\",\
                  \"slug\":\"ConnectedAccount_MissingRequiredFields\",\"status\":400}}";
        assert_eq!(
            expected_error_kind(bc),
            Some(ExpectedErrorKind::ProviderUserState),
            "OPENHUMAN-TAURI-BC wire shape must classify as ProviderUserState (the \
             more specific bucket once #1472 wave E added it)"
        );

        // Cover the rest of the 4xx surface produced by integrations /
        // composio clients — all user-input / auth-state failures that
        // Sentry can't action.
        for raw in [
            "Backend returned 400 Bad Request for POST https://api.example.com/x: bad input",
            "Backend returned 401 Unauthorized for GET https://api.example.com/x: token expired",
            "Backend returned 403 Forbidden for GET https://api.example.com/x: permission denied",
            "Backend returned 404 Not Found for GET https://api.example.com/x: missing",
            "Backend returned 422 Unprocessable Entity for POST https://api.example.com/x: validation failed",
            "Backend returned 451 Unavailable for Legal Reasons for GET https://api.example.com/x: blocked",
            // Lowercased context wrapping is irrelevant — substring match is case-insensitive.
            "[observability] integrations.post failed: Backend returned 400 Bad Request for POST https://api.tinyhumans.ai/x: detail",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::BackendUserError),
                "must classify as backend user-error: {raw}"
            );
        }
    }

    #[test]
    fn does_not_classify_transient_or_server_backend_errors_as_user_error() {
        // 408 / 429 are transient — they belong to the
        // upstream-transient bucket (or are retried at the caller), not
        // the user-error bucket. A sustained 429 (rate limit cliff) MUST
        // still surface so we can react.
        for raw in [
            "Backend returned 408 Request Timeout for POST https://api.example.com/x: timeout",
            "Backend returned 429 Too Many Requests for POST https://api.example.com/x: slow down",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                None,
                "transient 4xx must NOT be classified as user-error: {raw}"
            );
        }

        // 5xx is always actionable — server bugs need to reach Sentry.
        for raw in [
            "Backend returned 500 Internal Server Error for POST https://api.example.com/x: oops",
            "Backend returned 502 Bad Gateway for POST https://api.example.com/x: upstream down",
            "Backend returned 503 Service Unavailable for POST https://api.example.com/x: maintenance",
            "Backend returned 504 Gateway Timeout for POST https://api.example.com/x: slow upstream",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                None,
                "5xx must NOT be classified as user-error: {raw}"
            );
        }

        // A free-form message that mentions "400" but doesn't follow the
        // `Backend returned <status>` prefix from the integrations /
        // composio clients must not be silenced.
        assert_eq!(
            expected_error_kind("see HTTP 400 specification at https://example.com/400"),
            None
        );
        assert_eq!(
            expected_error_kind("OpenAI API error (400): bad request"),
            None,
            "provider-formatted 4xx must keep going through the provider classifier path"
        );
    }

    #[test]
    fn classifies_trigger_type_not_found_as_provider_user_state() {
        // OPENHUMAN-TAURI-3R / -3S: composio enable_trigger when the slug
        // isn't in the trigger registry. Backend wraps the upstream
        // composio 4xx as 500, so this would otherwise escape the
        // 4xx-only `is_backend_user_error_message` matcher.
        assert_eq!(
            expected_error_kind(
                "Backend returned 500 Internal Server Error for POST \
                 https://api.tinyhumans.ai/agent-integrations/composio/triggers: \
                 Trigger type GITHUB_PUSH_EVENT not found"
            ),
            Some(ExpectedErrorKind::ProviderUserState)
        );

        // Wrapped by `rpc.invoke_method` / `[composio] sync(toolkit) failed: …`
        // — substring match must survive caller context.
        assert_eq!(
            expected_error_kind(
                "rpc.invoke_method failed: Backend returned 500 Internal Server Error \
                 for POST /agent-integrations/composio/triggers: \
                 Trigger type SLACK_NEW_MESSAGE not found"
            ),
            Some(ExpectedErrorKind::ProviderUserState)
        );

        // Alternate phrasing observed from the same cluster.
        assert_eq!(
            expected_error_kind(
                "composio: Cannot enable trigger 'GITHUB_PUSH_EVENT': trigger not found in registry"
            ),
            Some(ExpectedErrorKind::ProviderUserState)
        );
    }

    #[test]
    fn classifies_toolkit_not_enabled_as_provider_user_state() {
        // OPENHUMAN-TAURI-34: 400 from composio because the user hasn't
        // enabled the toolkit. Must classify as ProviderUserState (more
        // specific) rather than the generic BackendUserError bucket — the
        // ordering in `expected_error_kind` enforces that.
        let msg = "Backend returned 400 Bad Request for POST \
                   https://api.tinyhumans.ai/agent-integrations/composio/execute: \
                   Toolkit \"get\" is not enabled";
        assert_eq!(
            expected_error_kind(msg),
            Some(ExpectedErrorKind::ProviderUserState)
        );

        // Wrapped variant (anyhow chain through the agent runtime).
        assert_eq!(
            expected_error_kind(
                "tool.invoke failed: Backend returned 400 Bad Request for POST \
                 /agent-integrations/composio/execute: Toolkit \"linear\" is not enabled \
                 for this account"
            ),
            Some(ExpectedErrorKind::ProviderUserState)
        );
    }

    #[test]
    fn classifies_missing_required_fields_as_provider_user_state() {
        // OPENHUMAN-TAURI-97: composio authorize with a blank required
        // field. Backend wraps the composio 400 as 500 with the inner
        // body embedded as a JSON-stringified error message.
        assert_eq!(
            expected_error_kind(
                "Backend returned 500 Internal Server Error for POST \
                 https://api.tinyhumans.ai/agent-integrations/composio/authorize: \
                 400 {\"error\":{\"message\":\"Missing required fields: Your Subdomain\"}}"
            ),
            Some(ExpectedErrorKind::ProviderUserState)
        );

        // Sibling toolkits surface the same shape with different field names.
        for raw in [
            "Backend returned 500 Internal Server Error for POST /authorize: Missing required fields: WABA ID",
            "Backend returned 500 Internal Server Error for POST /authorize: Missing required fields: Tenant Name",
            "Backend returned 400 Bad Request for POST /authorize: Missing required fields: Domain URL",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::ProviderUserState),
                "missing-required-fields shape must classify: {raw}"
            );
        }
    }

    #[test]
    fn classifies_insufficient_scopes_as_provider_user_state() {
        // OPENHUMAN-TAURI-33: gmail sync surfaced the upstream Google
        // OAuth scopes error verbatim through composio. Reaches the RPC
        // dispatch site via `[composio] sync(gmail) failed: [composio:gmail]
        // GMAIL_FETCH_EMAILS page 0: HTTP 403: Request had insufficient
        // authentication scopes.`.
        assert_eq!(
            expected_error_kind(
                "[composio:gmail] GMAIL_FETCH_EMAILS page 0: HTTP 403: \
                 Request had insufficient authentication scopes."
            ),
            Some(ExpectedErrorKind::ProviderUserState)
        );

        // Bare upstream shape (in case any future caller forwards without
        // the gmail prefix).
        assert_eq!(
            expected_error_kind("HTTP 403: Request had insufficient authentication scopes."),
            Some(ExpectedErrorKind::ProviderUserState)
        );
    }

    #[test]
    fn does_not_classify_unrelated_500s_as_provider_user_state() {
        // Sanity check: a generic 500 with no provider-user-state body
        // shape must continue to reach Sentry as an actionable event.
        assert_eq!(
            expected_error_kind(
                "Backend returned 500 Internal Server Error for POST \
                 /agent-integrations/composio/triggers: random panic in handler"
            ),
            None
        );
        assert_eq!(
            expected_error_kind(
                "Backend returned 500 Internal Server Error for GET /teams: database connection lost"
            ),
            None
        );

        // Free-form text that mentions "not found" / "is not enabled" out
        // of context must not be silenced.
        assert_eq!(
            expected_error_kind("file not found at /tmp/x.json"),
            None,
            "bare 'not found' without 'trigger type' anchor must NOT classify"
        );
        assert_eq!(
            expected_error_kind("the cache is not enabled in this build"),
            None,
            "bare 'is not enabled' without 'toolkit ' anchor must NOT classify"
        );
    }

    #[test]
    fn unrelated_missing_required_fields_classifies_as_accepted_false_positive() {
        // Documents the breadth of the `"missing required fields"` arm —
        // unlike the trigger/toolkit arms it has no second anchor, so a
        // non-composio call site whose error happens to contain the phrase
        // will also demote. This is the accepted false-positive surface
        // per the classifier doc-comment (every current emit site is
        // scoped to composio/integrations envelopes, so a stray collision
        // would have to come from a brand-new opt-in call site).
        //
        // Pinning this assertion locks the breadth in so a future
        // narrowing of the matcher surfaces here instead of silently
        // re-bucketing the demote path.
        assert_eq!(
            expected_error_kind("Internal error: missing required fields in config"),
            Some(ExpectedErrorKind::ProviderUserState),
            "accepted false-positive: bare 'missing required fields' demotes by design"
        );
    }

    #[test]
    fn provider_user_state_takes_precedence_over_backend_user_error() {
        // Critical ordering guarantee: a 4xx body that contains the
        // toolkit-not-enabled phrasing must land in `ProviderUserState`
        // (more specific) — not in the generic `BackendUserError` bucket.
        // Without the ordering in `expected_error_kind`, the 4xx matcher
        // would win and the operator would see a different breadcrumb
        // kind than intended (and miss the `kind="provider_user_state"`
        // tag in info logs).
        let msg = "Backend returned 400 Bad Request for POST \
                   /agent-integrations/composio/execute: \
                   Toolkit \"github\" is not enabled";
        assert_eq!(
            expected_error_kind(msg),
            Some(ExpectedErrorKind::ProviderUserState),
            "4xx + toolkit-not-enabled must land in ProviderUserState, not BackendUserError"
        );
    }

    #[test]
    fn classifies_local_ai_binary_missing_errors() {
        // OPENHUMAN-TAURI-9N: `local_ai_tts` returns this exact string
        // from `service::speech::tts` when piper isn't on PATH or
        // `PIPER_BIN` isn't set.
        assert_eq!(
            expected_error_kind("piper binary not found. Set PIPER_BIN or install piper."),
            Some(ExpectedErrorKind::LocalAiBinaryMissing)
        );
        // Sibling shapes from the same service area share the anchor and
        // must classify the same way — the user-facing remediation is
        // identical (install / configure the binary).
        assert_eq!(
            expected_error_kind(
                "whisper.cpp binary not found. Set WHISPER_BIN or install whisper-cli."
            ),
            Some(ExpectedErrorKind::LocalAiBinaryMissing)
        );
        assert_eq!(
            expected_error_kind(
                "Ollama binary not found at '/usr/local/bin/ollama'. Provide a valid path to the ollama executable."
            ),
            Some(ExpectedErrorKind::LocalAiBinaryMissing)
        );
        assert_eq!(
            expected_error_kind("Ollama installed but binary not found on system"),
            Some(ExpectedErrorKind::LocalAiBinaryMissing)
        );
        // Wrapped by the RPC dispatcher in production:
        //   `"rpc.invoke_method failed: piper binary not found. …"`.
        // The classifier is substring-based, so caller context must not
        // defeat it.
        assert_eq!(
            expected_error_kind(
                "rpc.invoke_method failed: piper binary not found. Set PIPER_BIN or install piper."
            ),
            Some(ExpectedErrorKind::LocalAiBinaryMissing)
        );
    }

    #[test]
    fn does_not_classify_unrelated_messages_as_binary_missing() {
        // Pin the anchor: messages that talk about binaries in a
        // different context (download failures, version mismatches)
        // must not be silenced.
        assert_eq!(
            expected_error_kind("piper binary failed to spawn: permission denied"),
            None
        );
        assert_eq!(
            expected_error_kind("whisper.cpp returned empty transcript"),
            None
        );
    }

    #[test]
    fn classifies_session_expired_messages() {
        // OPENHUMAN-TAURI-26: the canonical wire shape that `agent.run_single`
        // and `web_channel.run_chat_task` re-emit via `report_error_or_expected`
        // when the user's JWT expires mid-conversation. The classifier
        // anchors on the literal `"session expired"` substring from the
        // OpenHuman backend's 401 body — NOT on the bare `(401 Unauthorized)`
        // status, which would also silence BYO-key OpenAI/Anthropic 401s
        // that are actionable.
        assert_eq!(
            expected_error_kind(
                r#"OpenHuman API error (401 Unauthorized): {"success":false,"error":"Session expired. Please log in again."}"#
            ),
            Some(ExpectedErrorKind::SessionExpired)
        );

        // Wrapped by the agent / web-channel report sites in production —
        // the classifier is substring-based so caller context must not
        // defeat it.
        assert_eq!(
            expected_error_kind(
                r#"run_chat_task failed client_id=abc thread_id=t1 request_id=r1 error=OpenHuman API error (401 Unauthorized): {"success":false,"error":"Session expired. Please log in again."}"#
            ),
            Some(ExpectedErrorKind::SessionExpired)
        );

        // Sentinel raised by `providers::openhuman_backend::resolve_bearer`
        // when the scheduler-gate signed-out override is set
        // (OPENHUMAN-TAURI-1T's cascade dampener returns this so callers
        // get the same teardown path as a real backend 401).
        assert_eq!(
            expected_error_kind(
                "SESSION_EXPIRED: backend session not active — sign in to resume LLM work"
            ),
            Some(ExpectedErrorKind::SessionExpired)
        );

        // Local pre-flight guards — OpenHuman-specific phrasing, safe to
        // match regardless of caller wrapping.
        for raw in [
            "no backend session token; run auth_store_session first",
            "session JWT required",
            "composio unavailable: no backend session token. Sign in first (auth_store_session).",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                Some(ExpectedErrorKind::SessionExpired),
                "should classify as session-expired: {raw}"
            );
        }
    }

    #[test]
    fn does_not_classify_byo_key_provider_401_as_session_expired() {
        // Critical: a BYO-key 401 from OpenAI / Anthropic etc. is an
        // actionable misconfiguration (wrong API key) that the user needs
        // to fix in settings. It must reach Sentry as an error and must
        // NOT be classified as session-expired at the agent layer — the
        // strict classifier requires the OpenHuman backend's
        // "session expired" body to anchor the match. The dispatch-site
        // classifier (`crate::core::jsonrpc::is_session_expired_error`)
        // still matches these for the `DomainEvent::SessionExpired`
        // auto-cleanup path, which clears stale local state defensively.
        for raw in [
            "OpenAI API error (401 Unauthorized): invalid_api_key",
            "Anthropic API error (401 Unauthorized): authentication_error",
            "OpenAI API error (401): unauthorized",
            r#"OpenAI API error (401 Unauthorized): {"error":{"code":"invalid_api_key","message":"Incorrect API key provided"}}"#,
            // Generic "invalid token" without OpenHuman session phrasing —
            // could mean a third-party provider rejected its own token.
            "Invalid token",
            "got an invalid token here",
        ] {
            assert_eq!(
                expected_error_kind(raw),
                None,
                "BYO-key / generic 401 must reach Sentry as actionable error: {raw}"
            );
        }
    }

    #[test]
    fn does_not_classify_unrelated_messages_as_session_expired() {
        // Bare numeric 401 (port number, runbook reference) must not be
        // silenced.
        assert_eq!(expected_error_kind("server returned 401"), None);
        assert_eq!(
            expected_error_kind("see runbook for 401 handling at https://example.com/401"),
            None
        );
        // Provider 5xx — must reach Sentry.
        assert_eq!(
            expected_error_kind("OpenAI API error (500): internal server error"),
            None
        );
        // Lowercase sentinel must NOT match — the SESSION_EXPIRED sentinel
        // is case-sensitive by design (matches the sentinel emitted by
        // `providers::openhuman_backend::resolve_bearer` exactly).
        assert_eq!(expected_error_kind("session_expired lowercase"), None);
    }

    #[test]
    fn report_error_does_not_panic_with_many_tags() {
        let err = anyhow::anyhow!("multi-tag");
        report_error(
            &err,
            "test",
            "multi_tag",
            &[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")],
        );
    }

    fn event_with_tags(pairs: &[(&str, &str)]) -> sentry::protocol::Event<'static> {
        let mut event = sentry::protocol::Event::default();
        let mut tags: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for (k, v) in pairs {
            tags.insert((*k).to_string(), (*v).to_string());
        }
        event.tags = tags;
        event
    }

    fn event_with_tags_and_message(
        pairs: &[(&str, &str)],
        message: &str,
    ) -> sentry::protocol::Event<'static> {
        let mut event = event_with_tags(pairs);
        event.message = Some(message.to_string());
        event
    }

    #[test]
    fn transient_filter_drops_429_408_502_503_504() {
        for status in ["429", "408", "502", "503", "504"] {
            let event = event_with_tags(&[
                ("domain", "llm_provider"),
                ("failure", "non_2xx"),
                ("status", status),
            ]);
            assert!(
                is_transient_provider_http_failure(&event),
                "status {status} must be classified as transient and filtered"
            );
        }
    }

    #[test]
    fn transient_filter_keeps_permanent_failures() {
        for status in ["400", "401", "403", "404", "500"] {
            let event = event_with_tags(&[
                ("domain", "llm_provider"),
                ("failure", "non_2xx"),
                ("status", status),
            ]);
            assert!(
                !is_transient_provider_http_failure(&event),
                "status {status} must NOT be filtered — it's actionable"
            );
        }
    }

    #[test]
    fn transient_filter_keeps_aggregate_all_exhausted() {
        let event = event_with_tags(&[
            ("domain", "llm_provider"),
            ("failure", "all_exhausted"),
            ("status", "503"),
        ]);
        assert!(
            !is_transient_provider_http_failure(&event),
            "aggregate all_exhausted events must surface (they are the cascade signal)"
        );
    }

    #[test]
    fn transient_filter_keeps_events_with_no_status_tag() {
        let event = event_with_tags(&[("domain", "llm_provider"), ("failure", "non_2xx")]);
        assert!(
            !is_transient_provider_http_failure(&event),
            "missing status tag must not be silently dropped"
        );
    }

    // Regression guard: the filter must scope to provider events only. Other
    // subsystems emit `failure=non_2xx` (e.g.
    // `providers/compatible.rs` uses the same marker for OAI-compatible
    // error paths, but every site goes through `report_error(..,
    // "llm_provider", ..)` so the domain tag is consistent), but the broader
    // point is: any future caller that re-uses the same tag set for a
    // different domain must NOT be silently dropped by this filter.
    #[test]
    fn transient_filter_keeps_events_with_no_domain_tag() {
        let event = event_with_tags(&[("failure", "non_2xx"), ("status", "503")]);
        assert!(
            !is_transient_provider_http_failure(&event),
            "missing domain tag means the event isn't provider-originated — must surface"
        );
    }

    #[test]
    fn transient_filter_keeps_events_from_other_domains() {
        let event = event_with_tags(&[
            ("domain", "scheduler"),
            ("failure", "non_2xx"),
            ("status", "503"),
        ]);
        assert!(
            !is_transient_provider_http_failure(&event),
            "non-provider domain must surface even if failure/status tags collide"
        );
    }

    #[test]
    fn backend_api_filter_drops_transient_statuses() {
        for status in TRANSIENT_HTTP_STATUSES {
            let event = event_with_tags(&[
                ("domain", "backend_api"),
                ("failure", "non_2xx"),
                ("status", status),
            ]);
            assert!(
                is_transient_backend_api_failure(&event),
                "backend status {status} must be classified as transient"
            );
        }
    }

    #[test]
    fn backend_api_filter_drops_transient_transport_phrases() {
        for phrase in TRANSIENT_TRANSPORT_PHRASES {
            let event = event_with_tags_and_message(
                &[("domain", "backend_api"), ("failure", "transport")],
                &format!("GET /teams failed: {phrase}"),
            );
            assert!(
                is_transient_backend_api_failure(&event),
                "backend transport phrase {phrase} must be classified as transient"
            );
        }
    }

    #[test]
    fn backend_api_filter_keeps_non_transient_failures() {
        for status in ["404", "500"] {
            let event = event_with_tags(&[
                ("domain", "backend_api"),
                ("failure", "non_2xx"),
                ("status", status),
            ]);
            assert!(
                !is_transient_backend_api_failure(&event),
                "backend status {status} must stay visible"
            );
        }

        let wrong_domain = event_with_tags(&[
            ("domain", "scheduler"),
            ("failure", "non_2xx"),
            ("status", "503"),
        ]);
        assert!(
            !is_transient_backend_api_failure(&wrong_domain),
            "domain scoping must keep unrelated transient-shaped events visible"
        );

        let non_matching_transport = event_with_tags_and_message(
            &[("domain", "backend_api"), ("failure", "transport")],
            "GET /teams failed: certificate verify failed",
        );
        assert!(
            !is_transient_backend_api_failure(&non_matching_transport),
            "transport failures without an allowlisted phrase must stay visible"
        );
    }

    #[test]
    fn integrations_filter_drops_transient_statuses() {
        for status in TRANSIENT_HTTP_STATUSES {
            let event = event_with_tags(&[
                ("domain", "integrations"),
                ("failure", "non_2xx"),
                ("status", status),
            ]);
            assert!(
                is_transient_integrations_failure(&event),
                "integrations status {status} must be classified as transient"
            );
        }
    }

    #[test]
    fn integrations_filter_drops_transient_transport_phrases() {
        for phrase in TRANSIENT_TRANSPORT_PHRASES {
            let event = event_with_tags_and_message(
                &[("domain", "integrations"), ("failure", "transport")],
                &format!("GET /agent-integrations/tools failed: {phrase}"),
            );
            assert!(
                is_transient_integrations_failure(&event),
                "integrations transport phrase {phrase} must be classified as transient"
            );
        }
    }

    #[test]
    fn integrations_filter_keeps_non_transient_failures() {
        for status in ["404", "500"] {
            let event = event_with_tags(&[
                ("domain", "integrations"),
                ("failure", "non_2xx"),
                ("status", status),
            ]);
            assert!(
                !is_transient_integrations_failure(&event),
                "integrations status {status} must stay visible"
            );
        }

        // Sibling-domain check: composio op-layer events MUST be silenced
        // by the integrations filter — composio routes through the same
        // `IntegrationClient` so the failure shape is identical, but
        // op-level reporters that wrap and re-emit with their own domain
        // tag would otherwise escape (OPENHUMAN-TAURI-35 / -2H).
        let scheduler_domain = event_with_tags(&[
            ("domain", "scheduler"),
            ("failure", "non_2xx"),
            ("status", "503"),
        ]);
        assert!(
            !is_transient_integrations_failure(&scheduler_domain),
            "domain scoping must keep unrelated transient-shaped events visible"
        );

        let non_matching_transport = event_with_tags_and_message(
            &[("domain", "integrations"), ("failure", "transport")],
            "GET /agent-integrations/tools failed: invalid certificate",
        );
        assert!(
            !is_transient_integrations_failure(&non_matching_transport),
            "transport failures without an allowlisted phrase must stay visible"
        );
    }

    #[test]
    fn composio_domain_routes_through_integrations_filter() {
        // OPENHUMAN-TAURI-35 (~139 events) / -2H (~26 events):
        // `[composio] list_connections failed: Backend returned 502 …` —
        // composio op-layer wrappers (e.g. `composio_list_connections`) emit
        // errors under `domain="composio"` so the original
        // `domain="integrations"` filter let them through. Routing the
        // composio domain through the same transient classifier closes
        // that gap; the underlying transport / non_2xx semantics are
        // identical because both layers share the same `IntegrationClient`.
        for status in TRANSIENT_HTTP_STATUSES {
            let event = event_with_tags(&[
                ("domain", "composio"),
                ("failure", "non_2xx"),
                ("status", status),
            ]);
            assert!(
                is_transient_integrations_failure(&event),
                "composio status {status} must be classified as transient"
            );
        }

        // Transport-phrase variant — composio also surfaces reqwest
        // transport failures (timeouts, connection resets) once the op
        // wrapper has tagged the event with `failure=transport`.
        for phrase in TRANSIENT_TRANSPORT_PHRASES {
            let event = event_with_tags_and_message(
                &[("domain", "composio"), ("failure", "transport")],
                &format!("[composio] execute failed: {phrase}"),
            );
            assert!(
                is_transient_integrations_failure(&event),
                "composio transport phrase {phrase} must be classified as transient"
            );
        }

        // Non-transient composio statuses (404 / 500) must still surface —
        // actionable bugs even when reported under the composio domain.
        for status in ["404", "500"] {
            let event = event_with_tags(&[
                ("domain", "composio"),
                ("failure", "non_2xx"),
                ("status", status),
            ]);
            assert!(
                !is_transient_integrations_failure(&event),
                "composio status {status} must stay visible"
            );
        }
    }

    #[test]
    fn updater_transient_403_is_dropped() {
        let event = event_with_tags_and_message(
            &[
                ("domain", "update"),
                ("operation", "check_releases"),
                ("failure", "non_2xx"),
                ("status", "403"),
            ],
            "[observability] update.check_releases failed: GitHub API error: 403 Forbidden",
        );
        assert!(
            is_updater_transient_event(&event),
            "GitHub 403 updater checks are unactionable transient/rate-limit noise"
        );
    }

    #[test]
    fn updater_transient_502_is_dropped() {
        let event = event_with_tags_and_message(
            &[
                ("domain", "update.check_releases"),
                ("failure", "non_2xx"),
                ("status", "502"),
            ],
            "GitHub API error: 502 Bad Gateway",
        );
        assert!(
            is_updater_transient_event(&event),
            "GitHub 5xx updater checks must be filtered as transient"
        );
    }

    #[test]
    fn updater_real_panic_still_reported() {
        let event = event_with_tags_and_message(
            &[("domain", "update"), ("operation", "check_releases")],
            "thread 'main' panicked at src/openhuman/update/core.rs: index out of bounds",
        );
        assert!(
            !is_updater_transient_event(&event),
            "update-domain events without a transient updater shape must still reach Sentry"
        );
    }

    #[test]
    fn message_failure_classifier_matches_canonical_status_phrases() {
        for msg in [
            "rpc.invoke_method failed: GET /teams failed (502 Bad Gateway)",
            "GET /teams/me/usage failed (503 Service Unavailable)",
            "downstream returned (504 Gateway Timeout): retry budget exhausted",
            "OpenHuman API error (520 <unknown status code>): cf",
            "POST /channels/telegram/typing failed (429 Too Many Requests)",
            "auth connect failed: 503 Service Unavailable",
        ] {
            assert!(
                is_transient_message_failure(msg),
                "{msg:?} must be classified as transient"
            );
        }
    }

    #[test]
    fn message_failure_classifier_matches_transport_phrases() {
        for msg in [
            "integrations.get failed: composio/tools → operation timed out",
            "GET https://api.example.com → connection forcibly closed (os 10054)",
            "POST /v1/foo → tls handshake eof",
            "error sending request for url (https://api.example.com)",
        ] {
            assert!(
                is_transient_message_failure(msg),
                "{msg:?} must be classified as transient"
            );
        }
    }

    #[test]
    fn message_failure_classifier_keeps_unrelated_messages() {
        for msg in [
            "rpc.invoke_method failed: schema validation error",
            "process 502 exited unexpectedly",
            "GET /teams failed (404 Not Found)",
            "GET /teams failed (500 Internal Server Error)",
            "unrelated error with port 5023",
            "",
        ] {
            assert!(
                !is_transient_message_failure(msg),
                "{msg:?} must not be classified as transient"
            );
        }
    }

    #[test]
    fn budget_filter_drops_budget_message_on_tagged_400() {
        let event = event_with_tags_and_message(
            &[("failure", "non_2xx"), ("status", "400")],
            r#"OpenHuman API error (400 Bad Request): {"success":false,"error":"Insufficient budget"}"#,
        );

        assert!(is_budget_event(&event));
    }

    #[test]
    fn budget_filter_drops_budget_exception_on_tagged_400() {
        let mut event = event_with_tags(&[("failure", "non_2xx"), ("status", "400")]);
        event.exception.values.push(sentry::protocol::Exception {
            value: Some("Budget exceeded — add credits to continue".to_string()),
            ..Default::default()
        });

        assert!(is_budget_event(&event));
    }

    #[test]
    fn budget_filter_keeps_non_budget_400() {
        let event = event_with_tags_and_message(
            &[("failure", "non_2xx"), ("status", "400")],
            "Bad request: missing field",
        );

        assert!(!is_budget_event(&event));
    }

    #[test]
    fn budget_filter_requires_non_2xx_failure_and_400_status() {
        let message = "Budget exceeded — add credits to continue";
        for tags in [
            vec![("failure", "transport"), ("status", "400")],
            vec![("failure", "non_2xx"), ("status", "500")],
            vec![("failure", "non_2xx")],
        ] {
            let event = event_with_tags_and_message(&tags, message);
            assert!(!is_budget_event(&event));
        }
    }

    #[test]
    fn report_error_or_expected_does_not_panic() {
        report_error_or_expected(
            "local ai is disabled",
            "rpc",
            "invoke_method",
            &[("method", "openhuman.local_ai_prompt")],
        );
        report_error_or_expected(
            "ollama API key not set",
            "agent",
            "provider_chat",
            &[("provider", "ollama")],
        );
    }

    fn event_with_message(msg: &str) -> sentry::protocol::Event<'static> {
        let mut event = sentry::protocol::Event::default();
        event.message = Some(msg.to_string());
        event
    }

    fn event_with_exception_value(value: &str) -> sentry::protocol::Event<'static> {
        let mut event = sentry::protocol::Event::default();
        event.exception = vec![sentry::protocol::Exception {
            value: Some(value.to_string()),
            ..Default::default()
        }]
        .into();
        event
    }

    #[test]
    fn max_iterations_filter_matches_message_path() {
        // `report_error_message` calls `sentry::capture_message`, which
        // populates `event.message`. The filter must see the canonical
        // phrase on that field path.
        let event = event_with_message("Agent exceeded maximum tool iterations (8)");
        assert!(is_max_iterations_event(&event));
    }

    #[test]
    fn max_iterations_filter_matches_exception_path() {
        // sentry-tracing with attach_stacktrace=true populates the
        // exception list instead of (or in addition to) `event.message`.
        // Filter must still catch the noise.
        let event = event_with_exception_value(
            "agent.run_single failed: Agent exceeded maximum tool iterations (10)",
        );
        assert!(is_max_iterations_event(&event));
    }

    #[test]
    fn max_iterations_filter_keeps_unrelated_events() {
        assert!(!is_max_iterations_event(&event_with_message(
            "provider returned 503"
        )));
        assert!(!is_max_iterations_event(&event_with_message("")));
        assert!(!is_max_iterations_event(&sentry::protocol::Event::default()));
    }
}
