
/// Transient backend API failures (gateway hiccups, scheduled downtime).
/// Match by event tags written by report_error at the authed_json call site.
#[cfg(feature = "crash-reporting")]
pub fn is_transient_backend_api_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "backend_api")
}

/// Defense-in-depth `before_send` filter for skill-install fetch 4xx statuses.
///
/// A user/catalog-supplied `SKILL.md` URL returning 4xx means the remote skill
/// path is missing, private, or otherwise unavailable to that user. The install
/// RPC still returns the error so the UI can surface it, but Sentry should keep
/// reporting server-side and transport failures only.
#[cfg(feature = "crash-reporting")]
pub fn is_skill_install_user_fetch_failure(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("skills") {
        return false;
    }
    if tags.get("operation").map(String::as_str) != Some("install_fetch") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }

    tags.get("status")
        .and_then(|status| status.parse::<u16>().ok())
        .is_some_and(|status| (400..500).contains(&status))
}

/// Transient integrations / Composio failures (timeout, connection reset,
/// gateway hiccups).
///
/// Accepts both `domain="integrations"` (the shared
/// [`crate::integrations::IntegrationClient`] HTTP wrapper that
/// fronts every backend-proxied integration) and `domain="composio"` (errors
/// reported from the Composio op layer in
/// [`crate::integrations::composio::ops`]). Composio routes through the same
/// `IntegrationClient`, so the failure shape is identical — but op-level
/// reporters that wrap and re-emit those errors with their own domain tag
/// would otherwise escape the integrations-scoped filter (OPENHUMAN-TAURI-35
/// ~139ev, -2H ~26ev: `[composio] list_connections failed: Backend returned
/// 502 …` events that landed in Sentry under `domain=composio`).
#[cfg(feature = "crash-reporting")]
pub fn is_transient_integrations_failure(event: &sentry::protocol::Event<'_>) -> bool {
    is_transient_domain_failure(event, "integrations")
        || is_transient_domain_failure(event, "composio")
}

/// Skill-install fetch **client errors** (4xx, esp. 404/410).
///
/// `install_workflow_from_url_with_home` fetches a user/catalog-supplied
/// `SKILL.md`; a 4xx means the requested URL is gone or wrong — expected
/// user-input state surfaced to the UI as "skill not found", not a
/// Sentry-actionable defect. The primary suppression lives at that emit site
/// (it no longer calls `report_error` for 4xx); this is the defense-in-depth
/// net mirroring the `is_transient_*` filters, catching any future skills call
/// site that reports a 4xx. Matched by the tags `report_error` writes:
/// `domain=skills`, `failure=non_2xx`, and a 4xx `status`. A 5xx is a genuine
/// remote failure and stays reportable. Drops TAURI-RUST-CGE (~1,446 events /
/// 72 users on `openhuman@0.57.53`).
#[cfg(feature = "crash-reporting")]
pub fn is_skills_install_client_error_event(event: &sentry::protocol::Event<'_>) -> bool {
    let tags = &event.tags;
    if tags.get("domain").map(String::as_str) != Some("skills") {
        return false;
    }
    if tags.get("failure").map(String::as_str) != Some("non_2xx") {
        return false;
    }
    tags.get("status")
        .and_then(|status| status.parse::<u16>().ok())
        .is_some_and(|code| (400..500).contains(&code))
}

/// Transient updater failures from GitHub release probes/downloads.
///
/// Core-side reports carry structured tags (`domain=update`, often
/// `operation=check_releases`, plus `failure/status`). Tauri's updater plugin
/// can also emit message-only events such as
/// `"failed to check for updates: error sending request for url (...latest.json)"`.
/// Match both shapes, but never drop an arbitrary update-domain event unless
/// it also has a transient status/transport marker.
#[cfg(feature = "crash-reporting")]
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

/// Sentinel prefix stamped on a team-usage probe error that the
/// failure-backoff in `crate::integrations::client::budget_gate` short-circuited — i.e. an
/// already-reported repeat within the backoff window. The FIRST failure of a
/// streak propagates its real error string and reports normally; only the
/// suppressed repeats carry this prefix so the JSON-RPC boundary can demote
/// them (no re-report) instead of re-flooding Sentry. See GH #4153.
///
/// Single source of truth: the producer (`team::ops::get_usage_with_cache`)
/// builds its sentinel from this constant, and [`is_suppressed_usage_probe_backoff`]
/// matches it — coupled by a unit test so the two cannot drift.
pub const USAGE_PROBE_BACKOFF_PREFIX: &str = "USAGE_PROBE_BACKOFF:";

/// Returns true when a message is the usage-probe failure-backoff sentinel
/// (see [`USAGE_PROBE_BACKOFF_PREFIX`]). Anchored on the exact prefix so a real
/// backend error string can never match.
pub fn is_suppressed_usage_probe_backoff(msg: &str) -> bool {
    msg.starts_with(USAGE_PROBE_BACKOFF_PREFIX)
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
#[cfg(feature = "crash-reporting")]
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

/// Whether a raw error / message string is a provider **insufficient-credits
/// 402** — the BYO account (e.g. OpenRouter) genuinely lacks the balance to
/// satisfy the request. Anchored on BOTH a 402-status shape AND a credit
/// phrase, so a bare `402` (or a non-402 error whose body merely contains the
/// digits `402`) is not swallowed and keeps reaching Sentry.
///
/// Single source of truth shared by the message-level cron halt
/// (`crate::cron::scheduler`'s `is_insufficient_credits_failure`, which
/// stops retrying a permanent 402 and skips its `report_error`) and the
/// event-level `before_send` filter [`is_insufficient_credits_event`] — the
/// same split as [`is_session_expired_message`] ↔ [`is_session_expired_event`].
/// TAURI-RUST-514 / -C62.
pub fn is_insufficient_credits_message(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // Anchor the 402 to a status shape — the emit sites format the message
    // as "<provider> API error (402 Payment Required): <body>". Matching a
    // bare "402" would false-positive on body digits (e.g. a 400 error
    // whose body says "can only afford 402 tokens"), which is NOT this
    // user-state and must keep reaching Sentry.
    let is_402_status = lower.contains("(402") || lower.contains("402 payment required");
    if !is_402_status {
        return false;
    }
    // Check the credit/balance signal against the BODY only. The status prefix
    // "(402 Payment Required)" itself contains the phrase "payment required",
    // which `body_indicates_insufficient_credits` matches — so feeding it the
    // whole string would classify ANY 402 (even one whose body is an unrelated
    // condition) as insufficient-credits and suppress it (codex P2 on #3913).
    // Slice off everything up to and including the formatted "): " status
    // separator first; fall back to the whole text when the separator is
    // absent (non-standard shape) so a credit phrase there still matches.
    let body = lower
        .split_once("): ")
        .map_or(lower.as_str(), |(_, body)| body);
    tinyinference_llm::failure::body_indicates_insufficient_credits(body)
}

/// Defense-in-depth `before_send` filter for **insufficient-credits 402**
/// provider events (TAURI-RUST-C62): the user's own BYO provider account
/// (e.g. OpenRouter) is out of balance — a billing state OpenHuman has no
/// lever over once the request already caps `max_tokens`.
///
/// The primary emit-site demotion lives in the `Provider::chat()` native_chat
/// cascade (`is_provider_insufficient_credits_402`), but the compatible
/// provider reports the same failure from several other paths
/// (`chat_with_system`, `chat_with_history`, the streaming gates, and the
/// shared `api_error` helper) that don't run that cascade. This filter is the
/// single outermost net that catches all of them, keyed on the formatted
/// message rather than tags so it matches regardless of which path emitted it.
///
/// Match criteria (all required):
/// - the event message or any exception value names a 402 / payment-required
///   failure (`"402"` or `"payment required"`), AND
/// - that same text carries an insufficient-credits phrase
///   (`provider::body_indicates_insufficient_credits`).
#[cfg(feature = "crash-reporting")]
pub fn is_insufficient_credits_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(is_insufficient_credits_message)
    {
        return true;
    }
    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(is_insufficient_credits_message)
    })
}

/// Message-level matcher for a provider **monthly-quota / usage-limit
/// exhausted** failure. Status-agnostic by design — unlike
/// [`is_insufficient_credits_message`] it does NOT anchor on a 402 status,
/// because the Kiro IDE proxy wraps its 402 inside a 500 envelope
/// (TAURI-RUST-C9A). Delegates to the single-source quota-phrase set in
/// [`tinyinference_llm::failure::body_indicates_quota_exhausted`], so
/// the emit-site guard and this `before_send` net can't drift. Shared with the
/// event-level filter [`is_quota_exhausted_event`].
#[cfg(any(feature = "crash-reporting", test))]
fn is_quota_exhausted_message(body: &str) -> bool {
    tinyinference_llm::failure::body_indicates_quota_exhausted(body)
}

/// Defense-in-depth `before_send` filter for provider **monthly-quota
/// exhausted** events (TAURI-RUST-C9A): the user's third-party plan has spent
/// its allotment for the period — a billing/plan state OpenHuman has no lever
/// over.
///
/// The primary emit-site demotion lives in the `Provider::chat()` native_chat
/// cascade and the shared `api_error` helper (`is_provider_quota_exhausted`),
/// but the compatible provider reports the same failure from several other
/// paths that don't run those guards. This filter is the single outermost net
/// that catches all of them, keyed on the formatted message rather than tags so
/// it matches regardless of which path emitted it (and regardless of whether
/// the upstream wrapped the 402 in a 500 envelope).
#[cfg(feature = "crash-reporting")]
pub fn is_quota_exhausted_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(is_quota_exhausted_message)
    {
        return true;
    }
    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(is_quota_exhausted_message)
    })
}

/// Whether a raw error / message string is an Ollama **Cloud** hosted-inference
/// `500` (`Internal Server Error (ref: <uuid>)`). Matches either the raw emit
/// shape (`ollama API error (500 …): {"error":"Internal Server Error (ref: …)"}`)
/// or the actionable re-raise the emit sites swap in
/// (`is_ollama_cloud_internal_500_message`). The raw arm requires BOTH the
/// `ollama` provider name and the `internal server error (ref:` envelope, so a
/// generic 500 from another provider, or a local Ollama daemon crash (which
/// carries no `ref:` UUID), still reaches Sentry.
pub fn is_ollama_cloud_internal_500_message_any(text: &str) -> bool {
    if tinyinference_llm::failure::is_ollama_cloud_internal_500_message(text) {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    lower.contains("ollama") && lower.contains("internal server error (ref:")
}

/// Defense-in-depth `before_send` filter for **Ollama Cloud hosted-inference
/// 500s** (TAURI-RUST-5MV): ollama.com's `*:cloud` models intermittently
/// return an opaque `Internal Server Error (ref: <uuid>)` with no client lever
/// (non-deterministic, byte-identical request succeeds when healthy), retried +
/// fallen-back by the reliable-provider layer.
///
/// The primary demotion lives at the `native_chat` / `streaming_chat` /
/// `api_error` emit sites, and the agent re-report is demoted via
/// `expected_error_kind` → `TransientUpstreamHttp`. This is the single outermost
/// net for any other compatible-provider path (`chat_with_system`,
/// `chat_with_history`, the non-native cascades) that reports the same body,
/// keyed on the message rather than tags so it matches regardless of emitter.
#[cfg(feature = "crash-reporting")]
pub fn is_ollama_cloud_internal_500_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(is_ollama_cloud_internal_500_message_any)
    {
        return true;
    }
    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(is_ollama_cloud_internal_500_message_any)
    })
}

/// Defense-in-depth `before_send` filter for Windows `ERROR_FILE_SYSTEM_LIMITATION`
/// (os error 665) — a host-filesystem condition (fragmentation, USN journal
/// overflow, filter-driver resource cap) that is persistent, locale-independent
/// in the `(os error N)` suffix, and unrecoverable by the app.
///
/// The primary suppression lives at the emit site via `expected_error_kind` →
/// `ExpectedErrorKind::WindowsFileSystemLimitation` (called by
/// `report_error_or_expected`). This filter catches any future call site that
/// bypasses `report_error_or_expected` and emits the error via `report_error`
/// or `tracing::error!` directly — keeping TAURI-RUST-QT0 (6,050 events /
/// 1 user) permanently off Sentry.
///
/// Also catches `OS error 665` from the Tauri shell side
/// (`crates/openhuman-app/`) which is compiled into a separate crate and therefore
/// cannot route through the core's `expected_error_kind` classifier.
#[cfg(feature = "crash-reporting")]
pub fn is_windows_file_system_limitation_event(event: &sentry::protocol::Event<'_>) -> bool {
    let check = |text: &str| -> bool {
        let lower = text.to_ascii_lowercase();
        lower.contains("(os error 665)")
    };
    if event.message.as_deref().is_some_and(check) {
        return true;
    }
    if event
        .logentry
        .as_ref()
        .map(|log| log.message.as_str())
        .is_some_and(check)
    {
        return true;
    }
    event
        .exception
        .values
        .iter()
        .any(|exception| exception.value.as_deref().is_some_and(check))
}

/// Defense-in-depth `before_send` filter for host disk-full failures
/// (ENOSPC / `ERROR_DISK_FULL` / `ERROR_HANDLE_DISK_FULL` / `StorageFull` /
/// `SQLITE_FULL`, any OS locale — see [`is_disk_full_error`]).
///
/// The primary suppression lives at the emit site via `expected_error_kind` →
/// `ExpectedErrorKind::DiskFull` (called by `report_error_or_expected`). This
/// filter catches a call site that bypasses it with `report_error` or a bare
/// `tracing::error!` — keeping TAURI-RUST-15PJ (`transcript persistence
/// failed: … (os error 112)`, 2,278 events / 1 user, #8508) and its siblings
/// off Sentry. Mirrors [`is_windows_file_system_limitation_event`].
#[cfg(feature = "crash-reporting")]
pub fn is_disk_full_event(event: &sentry::protocol::Event<'_>) -> bool {
    if event.message.as_deref().is_some_and(is_disk_full_error) {
        return true;
    }
    if event
        .logentry
        .as_ref()
        .map(|log| log.message.as_str())
        .is_some_and(is_disk_full_error)
    {
        return true;
    }
    event
        .exception
        .values
        .iter()
        .any(|exception| exception.value.as_deref().is_some_and(is_disk_full_error))
}

#[cfg(feature = "crash-reporting")]
fn event_contains_budget_exhausted_message(event: &sentry::protocol::Event<'_>) -> bool {
    if event
        .message
        .as_deref()
        .is_some_and(tinyinference_providers::is_budget_exhausted_message)
    {
        return true;
    }

    event.exception.values.iter().any(|exception| {
        exception
            .value
            .as_deref()
            .is_some_and(tinyinference_providers::is_budget_exhausted_message)
    })
}

/// Defense-in-depth `before_send` filter for **user-config provider error
/// patterns** — 4xx client errors, model-not-found, subscription/payment
/// issues, and other misconfigurations that are not application bugs.
///
/// Matches on `event.message` or exception values against known user-config
/// patterns. The primary suppression lives at the `report_error` / emit sites;
/// this catches any future call site that bypasses those classifiers.
///
/// Returns true (should be dropped) when the event message matches any known
/// user-config provider pattern. Target: ~22 Sentry issues / ~26k events.
#[cfg(feature = "crash-reporting")]
pub fn is_user_config_provider_event(event: &sentry::protocol::Event<'_>) -> bool {
    // Collect all text sources to check against
    let texts: Vec<&str> = {
        let mut v = Vec::with_capacity(2 + event.exception.values.len());
        if let Some(msg) = event.message.as_deref() {
            v.push(msg);
        }
        if let Some(log) = event.logentry.as_ref().map(|l| l.message.as_str()) {
            v.push(log);
        }
        for exc in &event.exception.values {
            if let Some(val) = exc.value.as_deref() {
                v.push(val);
            }
        }
        v
    };
    if texts.is_empty() {
        return false;
    }

    // Provider 4xx patterns — user config errors like:
    //   "HTTP 400", "HTTP 401", "HTTP 403", "HTTP 404"
    // matching against llm_provider domain events
    let tags = &event.tags;
    let domain = tags.get("domain").map(String::as_str);
    if domain == Some("llm_provider") {
        if let Some(status) = tags.get("status") {
            // 4xx user config errors. 401/403/404 are unambiguously
            // auth/permission/config issues and safe to drop blanket.
            // 400 (Bad Request) may also be a client-side serialization
            // bug — only drop when the message content confirms it's
            // a user config error (handled by message patterns below
            // and the earlier `is_budget_event` / `is_insufficient_credits_event`
            // filters in the before_send chain).
            if matches!(status.as_str(), "401" | "403" | "404") {
                return true;
            }
        }
        // Message-level patterns
        let lower = texts.join(" ").to_ascii_lowercase();
        if lower.contains("401 payment required")
            || lower.contains("subscription")
            || lower.contains("max monthly spend")
            || lower.contains("context length exceeded")
            || lower.contains("context size exceeded")
        {
            return true;
        }
    }

    // Embedding API 4xx / unauthorized — user's embedding provider config
    if domain == Some("llm_provider") || domain == Some("local_ai") {
        let lower = texts.join(" ").to_ascii_lowercase();
        if (lower.contains("embed") || lower.contains("embedding"))
            && (lower.contains("401")
                || lower.contains("404")
                || lower.contains("unauthorized")
                || lower.contains("invalid model"))
        {
            return true;
        }
    }

    false
}

#[cfg(feature = "crash-reporting")]
include!("observability_connectivity.rs");
