//! Centralised error reporting for the core, plus a Sentry
//! `before_send` filters that drop deterministic provider noise:
//! per-attempt transient-upstream failures and budget-exhausted user-state.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedErrorKind {
    LocalAiDisabled,
    ApiKeyMissing,
    NetworkUnreachable,
    TransientUpstreamHttp,
    LocalAiBinaryMissing,
    BackendUserError,
    LocalAiCapabilityUnavailable,
    BudgetExhausted,
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
    if is_backend_user_error_message(&lower) {
        return Some(ExpectedErrorKind::BackendUserError);
    }
    if is_local_ai_capability_unavailable_message(&lower) {
        return Some(ExpectedErrorKind::LocalAiCapabilityUnavailable);
    }
    if crate::openhuman::providers::is_budget_exhausted_message(message) {
        return Some(ExpectedErrorKind::BudgetExhausted);
    }
    None
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
fn is_transient_upstream_http_message(lower: &str) -> bool {
    TRANSIENT_PROVIDER_HTTP_STATUSES
        .iter()
        .any(|code| lower.contains(&format!("api error ({code}")))
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
pub fn is_transient_integrations_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "integrations")
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
        // didn't fill in the required Tenant Name field. The exact wire
        // shape `IntegrationClient::post` builds — must classify as
        // expected so the Sentry event is suppressed.
        let bc = "Backend returned 400 Bad Request for POST \
                  https://api.tinyhumans.ai/agent-integrations/composio/authorize: \
                  Composio authorization failed: 400 \
                  {\"error\":{\"message\":\"Missing required fields: Tenant Name\",\
                  \"slug\":\"ConnectedAccount_MissingRequiredFields\",\"status\":400}}";
        assert_eq!(
            expected_error_kind(bc),
            Some(ExpectedErrorKind::BackendUserError),
            "OPENHUMAN-TAURI-BC wire shape must classify"
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

        let wrong_domain = event_with_tags(&[
            ("domain", "composio"),
            ("failure", "non_2xx"),
            ("status", "503"),
        ]);
        assert!(
            !is_transient_integrations_failure(&wrong_domain),
            "domain scoping must keep composio-tagged events visible"
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
