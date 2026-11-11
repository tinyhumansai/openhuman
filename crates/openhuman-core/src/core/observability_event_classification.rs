
/// Returns true when a Sentry event is a per-attempt provider HTTP failure
/// that the reliable-provider layer already handles via retry + fallback.
///
/// The primary suppression lives at the call site
/// (`crate::inference::provider::http_error::should_report_provider_http_failure`),
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
#[cfg(feature = "crash-reporting")]
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
    tinyinference_llm::failure::TRANSIENT_PROVIDER_HTTP_STATUSES.contains(&status_u16)
}

/// Defense-in-depth `before_send` filter for managed-backend `errorCode`
/// events (#870): drops any Sentry event whose message/exception text carries a
/// backend `errorCode` that the backend owns (F2/F4) — *except* a backend-flagged
/// malformed `BAD_REQUEST`, which the client caused and so still pages (F8).
///
/// Primary suppression lives at the emit sites (`api_error` /
/// `compatible_*` streaming gates) and at the higher-layer re-report
/// classifier (`expected_error_kind` → [`ExpectedErrorKind::BackendErrorCodeOwned`]).
/// This catches any future call site that re-emits the same flattened error
/// without routing through those funnels. Delegates the decision to the
/// single-source [`crate::inference::provider::managed_error_skips_sentry`]
/// (managed-envelope gated, so a BYO payload carrying an `errorCode`-shaped
/// field is not wrongly dropped) so the layers can't drift.
#[cfg(feature = "crash-reporting")]
pub fn is_backend_error_code_event(event: &sentry::protocol::Event<'_>) -> bool {
    let direct = event.message.as_deref();
    let from_logentry = event.logentry.as_ref().map(|log| log.message.as_str());
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_logentry, from_exception]
        .into_iter()
        .flatten()
        .any(|message| {
            crate::inference::registry::classify_backend_error(None, message).is_some_and(
                |classification| classification.backend_owned && !classification.reportable,
            )
        })
}

/// Defense-in-depth `before_send` filter for transient streaming **transport**
/// failures (F7): drops `domain=llm_provider, failure=transport` events whose
/// body is a transient transport phrase (timeout / reset / TLS-handshake EOF /
/// "error sending request"). Flaky-network blips on the streaming send are
/// recovered by retry/fallback and carry no actionable Sentry signal; a
/// non-transient transport failure (DNS misconfig, unexpected protocol error)
/// is not matched and still pages.
///
/// Primary gate lives at the two streaming emit sites in
/// `compatible_provider_impl.rs` (`stream_chat` / `stream_chat_history`); this
/// catches any future site that reports the same shape without gating.
///
/// Scoped to the **streaming** operations on purpose (CodeRabbit): only the
/// `stream_chat` / `stream_chat_history` transport emits are meant to be
/// suppressed. A non-streaming `domain=llm_provider, failure=transport` event
/// carries a different `operation` tag and must keep paging, so the
/// observability blind spot stays as narrow as F7 intends.
#[cfg(feature = "crash-reporting")]
pub fn is_transient_provider_transport_failure(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("llm_provider") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("transport") {
        return false;
    }
    if !matches!(
        tags.get("operation").map(String::as_str),
        Some("stream_chat") | Some("stream_chat_history")
    ) {
        return false;
    }
    event_has_transient_transport_phrase(event)
}

/// Defense-in-depth filter for aggregate provider exhaustion events where the
/// aggregate only restates transient attempt failures.
///
/// Keep ordinary `failure=all_exhausted` events: they are the useful "every
/// fallback failed" signal. Drop only the narrow shape observed in #3542,
/// where the aggregate body starts with the reliable-provider exhaustion
/// prefix and contains transient HTTP/transport wording already classified by
/// [`is_transient_message_failure`].
#[cfg(feature = "crash-reporting")]
pub fn is_all_transient_provider_exhaustion_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("llm_provider") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("all_exhausted") {
        return false;
    }

    let direct = event.message.as_deref();
    let from_logentry = event.logentry.as_ref().map(|log| log.message.as_str());
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_logentry, from_exception]
        .into_iter()
        .flatten()
        .any(all_provider_attempts_are_transient)
}

#[cfg(feature = "crash-reporting")]
fn all_provider_attempts_are_transient(message: &str) -> bool {
    let Some(attempts) = message.strip_prefix("All providers/models failed. Attempts:") else {
        return false;
    };
    let mut saw_attempt = false;
    for attempt in attempts.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        saw_attempt = true;
        if !is_transient_message_failure(attempt) {
            return false;
        }
    }
    saw_attempt
}

/// Returns true when a Sentry event's message/exception text contains the
/// canonical max-tool-iterations cap phrase (see
/// `tinyagents_harness::agent_error::MAX_ITERATIONS_ERROR_PREFIX`).
///
/// Defense-in-depth filter for the Sentry `before_send` hook: the primary
/// suppression lives at the call sites in `agent::session_host::
/// runtime::run_single`, `channels::runtime::dispatch`, and
/// `web_chat::run_chat_task`, all of which now skip
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
#[cfg(feature = "crash-reporting")]
pub fn is_max_iterations_event(event: &sentry::protocol::Event<'_>) -> bool {
    let direct = event.message.as_deref();
    let from_exception = event.exception.last().and_then(|e| e.value.as_deref());
    [direct, from_exception]
        .into_iter()
        .flatten()
        .any(tinyagents_harness::agent_error::is_max_iterations_error)
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
#[cfg(feature = "crash-reporting")]
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

#[cfg(feature = "crash-reporting")]
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

#[cfg(feature = "crash-reporting")]
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

#[cfg(feature = "crash-reporting")]
fn event_has_updater_domain(event: &sentry::protocol::Event<'_>) -> bool {
    matches!(
        event.tags.get("domain").map(String::as_str),
        Some("update") | Some("update.check_releases") | Some("updater")
    )
}

#[cfg(feature = "crash-reporting")]
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
