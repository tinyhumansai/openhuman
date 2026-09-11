use once_cell::sync::Lazy;
use regex::Regex;

static BUDGET_ERROR_NORMALIZE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[-_\s]+").expect("budget normalize regex"));
static BUDGET_ERROR_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"budget.*exceed").expect("budget exceeded regex"),
        Regex::new(r"top up").expect("top up regex"),
        Regex::new(r"add.*credits").expect("add credits regex"),
        Regex::new(r"out of credits").expect("out of credits regex"),
        Regex::new(r"no remaining credits").expect("no remaining credits regex"),
    ]
});

pub(crate) fn is_inference_budget_exceeded_error(message: &str) -> bool {
    let normalized = BUDGET_ERROR_NORMALIZE_RE
        .replace_all(&message.trim().to_ascii_lowercase(), " ")
        .into_owned();
    if BUDGET_ERROR_PATTERNS
        .iter()
        .any(|pattern| pattern.is_match(&normalized))
    {
        return true;
    }
    // Align with the canonical OpenHuman-backend budget detector
    // (`billing_error::is_budget_exhausted_message`) so the managed
    // no-credits response — a 400 carrying "Insufficient budget" /
    // "Insufficient balance" — surfaces the actionable budget message
    // below instead of the generic "Something went wrong" apology
    // (issue #3088). Without this, an Ollama user with zero credits and
    // routing still on Managed sees an opaque "provider error" and has no
    // way to self-diagnose that they must top up or switch routing.
    crate::openhuman::inference::provider::is_budget_exhausted_message(message)
}

pub(crate) fn inference_budget_exceeded_user_message() -> &'static str {
    // Keep the literal "top up" / "credits" tokens (asserted by
    // `budget_exceeded_copy_mentions_top_up`) and add the self-diagnosis
    // path for issue #3088: a user who enabled a local model but left
    // routing on Managed needs to know they can switch to their own model
    // rather than being stuck. We guide, never auto-switch — the user's
    // routing choice in Settings is respected.
    "You're out of credits, so I can't run the managed (cloud) model right now. \
     You can top up your credits or pick a plan to continue — or, if you've enabled a \
     local model like Ollama, switch routing to \"Use Your Own Models\" in Connections → API keys → LLM."
}

pub(crate) fn generic_inference_error_user_message() -> &'static str {
    "Something went wrong. Please try again.\nThis error has been reported. You can also report it on Discord.\n<openhuman-link path=\"community/discord-report\">Report on Discord</openhuman-link>"
}

/// Stable marker embedded in the synthetic error a web turn raises when it
/// exceeds its wall-clock backstop (`OPENHUMAN_WEB_TURN_TIMEOUT_SECS`). Kept as
/// a grep-friendly anchor so [`classify_inference_error`] routes it to the
/// dedicated `turn_timeout` branch instead of the generic catch-all.
pub(crate) const TURN_TIMEOUT_MARKER: &str = "openhuman_turn_wall_clock_timeout";

/// Build the synthetic error string a wedged web turn raises when its
/// wall-clock backstop fires. Carries [`TURN_TIMEOUT_MARKER`] so the error
/// classifier surfaces a graceful, retryable `turn_timeout` chat_error rather
/// than letting the turn hang forever with no terminal event (issue #4746).
pub(crate) fn turn_timeout_error_message(secs: u64) -> String {
    format!(
        "{TURN_TIMEOUT_MARKER}: agent turn exceeded its {secs}s wall-clock budget \
         without producing a terminal event (a tool or delegated sub-agent likely stalled)"
    )
}

/// True when `err` is a turn wall-clock timeout — either the synthetic marker
/// raised by the web turn driver's outer backstop ([`TURN_TIMEOUT_MARKER`]), or
/// the tinyagents harness's own `TinyAgentsError::Timeout` (issue #4746). The
/// harness renders that as `run timed out: <model|tool> call for run `..`
/// exceeded its remaining wall-clock budget (.. ms)` / `.. exceeded its
/// wall-clock deadline`, so both wall-clock phrasings are anchored here. This
/// routes the loop's graceful budget-exhaustion terminal event to the dedicated
/// `turn_timeout` copy instead of the generic catch-all.
pub(crate) fn is_turn_timeout_error(err: &str) -> bool {
    err.contains(TURN_TIMEOUT_MARKER)
        || err.contains("run timed out:")
        || err.contains("exceeded its remaining wall-clock budget")
        || err.contains("exceeded its wall-clock deadline")
}

/// True when `err` is the **outer** web-turn backstop firing —
/// [`TURN_TIMEOUT_MARKER`], raised by [`drive_turn_with_deadline`] — as opposed
/// to the harness's own `Timeout`.
///
/// The two are structurally different events and only look alike once
/// stringified, which is why they were treated alike and why one of them went
/// unnoticed for as long as it did (#5804):
///
/// * The marker is raised when the turn future produced **no terminal event at
///   all** inside the channel's ceiling. By construction nothing was completing
///   — the turn wedged outside the harness run (session assembly, persistence
///   plumbing). There is no in-flight work to report and the user already has a
///   graceful `turn_timeout`, so a Sentry event would be noise.
///
/// * The harness `Timeout` is raised while bounding a **real, in-flight model
///   or tool call** against the run's remaining wall-clock budget. Reaching it
///   means the run consumed its budget doing work, and everything that work
///   produced is discarded with the turn. That is a defect signal, and
///   suppressing it is what made the discarded-turn failure invisible.
///
/// Used only for the Sentry suppression decision. [`is_turn_timeout_error`]
/// still covers both for the *user-facing* classification, which is unchanged:
/// either way the turn ran out of time and the graceful `turn_timeout` copy is
/// the right thing to show.
pub(crate) fn is_outer_backstop_timeout(err: &str) -> bool {
    err.contains(TURN_TIMEOUT_MARKER)
}

/// Pull the structured provider error message out of a raw error string.
///
/// Provider error chains from OpenAI/Anthropic/OpenRouter/etc. arrive looking
/// like `custom_openai API error (404 Not Found): {"error":{"message":"...","type":"..."}}`.
/// We extract the `error.message` value so the UI can show the *real* reason
/// — e.g. "Project ... does not have access to model `gpt-5.5`" — instead of
/// a generic apology.
///
/// Returns `None` for transport-level failures (DNS, TLS, connect refused)
/// where there is no provider body to quote — those have no actionable
/// detail and the raw error text can leak internal infrastructure URLs,
/// which the chat surface deliberately does not expose to end users.
pub(crate) fn extract_provider_error_detail(err: &str) -> Option<String> {
    const MAX_DETAIL_CHARS: usize = 300;

    // Find the first `"message"` JSON field anywhere in the error chain.
    let key = "\"message\"";
    let idx = err.find(key)?;
    let after_key = &err[idx + key.len()..];
    // Skip whitespace and the colon to the opening quote of the value.
    let after_colon = after_key.trim_start_matches(|c: char| c != '"');
    let stripped = after_colon.strip_prefix('"')?;

    // Manual unescape — handle `\"` and `\\` only; everything else passes
    // through. Sufficient for OpenAI/Anthropic/etc. error bodies.
    let mut out = String::new();
    let mut chars = stripped.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let trimmed = out.trim();
                if trimmed.is_empty() {
                    return None;
                }
                let sanitized = crate::openhuman::inference::provider::sanitize_api_error(trimmed);
                return Some(crate::openhuman::util::truncate_with_ellipsis(
                    &sanitized,
                    MAX_DETAIL_CHARS,
                ));
            }
            '\\' => {
                if let Some(esc) = chars.next() {
                    match esc {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        other => out.push(other),
                    }
                }
            }
            other => out.push(other),
        }
    }

    None
}

/// Append the upstream provider detail to a user-facing message, if a useful
/// one can be extracted. Keeps the friendly summary first and the verbatim
/// provider reason below as a quotable block.
pub(crate) fn with_provider_detail(summary: &str, err: &str) -> String {
    match extract_provider_error_detail(err) {
        Some(detail) => format!("{summary}\n\n> {detail}"),
        None => summary.to_string(),
    }
}

/// Structured chat-error envelope produced by [`classify_inference_error`].
///
/// Carries the typed metadata the frontend needs to render a recovery UI
/// (retry-after countdown, retry button, fallback CTA) without having to
/// regex the human-readable `message`. Issue #2606.
///
/// `error_type` and `message` preserve the wire shape PR #2371 established
/// — existing FE handlers that read those fields keep working. The new
/// fields are additive and `Option`-typed where the value isn't always
/// known at the classifier layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassifiedError {
    /// Stable token: `rate_limited`, `action_budget_exceeded`,
    /// `max_iterations`, `turn_timeout`, `timeout`, `auth_error`,
    /// `session_expired`, `budget_exhausted`, `provider_error`,
    /// `context_overflow`, `model_unavailable`, `payload_too_large`,
    /// `provider_request_rejected`, `capability_unsupported`,
    /// `chat_template_rejected`, `empty_response`, `network`, `inference`.
    pub(crate) error_type: &'static str,
    /// User-facing copy (already includes provider detail block and the
    /// retry-after countdown sentence when available).
    pub(crate) message: String,
    /// Where the limit originated. One of:
    /// - `"provider"`         — upstream LLM provider 429 / rate limit
    /// - `"openhuman_budget"` — local SecurityPolicy per-hour action cap
    /// - `"agent_loop"`       — agent ran out of tool iterations
    /// - `"openhuman_billing"` — OpenHuman credit/quota exhaustion
    /// - `"transport"`        — network / DNS / TLS / timeout
    /// - `"config"`           — auth, model, context, generic
    pub(crate) source: &'static str,
    /// Can the user retry the same prompt in the same thread? `false` for
    /// non-retryable business 429s, auth failures, model_unavailable,
    /// context_overflow, and OpenHuman billing exhaustion.
    pub(crate) retryable: bool,
    /// Milliseconds the upstream asked us to wait. Surfaced verbatim from
    /// `Retry-After:` / `retry_after:` headers when present; `None` when
    /// the upstream didn't supply one OR the error class doesn't have a
    /// concept of retry-after (auth, config, etc.).
    pub(crate) retry_after_ms: Option<u64>,
    /// Provider name extracted from the leading
    /// `"<provider> API error (...)"` envelope emitted by
    /// `inference::provider::ops::api_error`. `None` for non-provider
    /// errors (OpenHuman budget cap, agent loop) and for transport
    /// failures that don't carry an identifiable provider prefix.
    pub(crate) provider: Option<String>,
    /// `Some(false)` once the reliable-provider chain has exhausted every
    /// configured `model_fallbacks` entry (the aggregate "All
    /// providers/models failed" branch). `None` means the classifier
    /// can't tell from the error string alone — the FE should treat it
    /// as "unknown, don't promise a fallback".
    pub(crate) fallback_available: Option<bool>,
}

/// Best-effort extraction of the provider name from an error string.
///
/// `inference::provider::ops::api_error` formats upstream failures as
/// `"<provider> API error (<status>): <body>"`, e.g.
/// `"openrouter API error (429 Too Many Requests): ..."`. We pull the
/// leading word and lowercase it so the wire value is stable across
/// providers' own capitalisation.
///
/// Returns `None` when:
/// - The error string doesn't carry the `" API error"` infix.
/// - The candidate word contains characters that wouldn't appear in a
///   provider name (slashes, colons, etc. — guards against transport
///   error prefixes that happen to be followed by " API error").
pub(crate) fn extract_provider_name(err: &str) -> Option<String> {
    const INFIX: &str = " API error";
    let idx = err.find(INFIX)?;
    let prefix = err[..idx].trim_end();
    let candidate = prefix
        .rsplit_once(char::is_whitespace)
        .map_or(prefix, |(_, last)| last);
    if candidate.is_empty()
        || !candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some(candidate.to_ascii_lowercase())
}

/// Detect the reliable-provider aggregate that fires once every
/// configured `model_fallbacks` entry has been tried.
///
/// `reliable.rs::format_failure_aggregate` always opens with
/// `"All providers/models failed. Attempts:"`. When that marker is
/// present the FE should NOT offer a fallback retry — there is none
/// left to try.
pub(crate) fn is_fallback_chain_exhausted(err: &str) -> bool {
    err.contains("All providers/models failed")
}

/// Extract a Retry-After / retry_after seconds hint from a free-form
/// error string. Mirrors the typed [`crate::openhuman::inference::
/// provider::error_classify::parse_retry_after_ms`] helper but operates on
/// the already-flattened `String` that reaches the channel-classifier
/// layer.
///
/// Returns `Some(n)` when a non-negative integer or fractional value
/// follows one of the canonical headers; fractional values are
/// rounded up so the user is never told to retry sooner than the
/// upstream actually allows.
pub(crate) fn parse_retry_after_secs_from_str(err: &str) -> Option<u64> {
    // Normalise quoted JSON-key wrappers ("retry_after": 30) by
    // stripping double quotes before scanning for prefixes
    // (CodeRabbit review on #2371). A serialised provider body like
    // `{"retry_after": 30}` would otherwise miss every prefix and
    // the user would lose the retry hint the provider supplied.
    let normalized = err.to_ascii_lowercase().replace('"', "");
    for prefix in &[
        "retry-after:",
        "retry_after:",
        "retry-after ",
        "retry_after ",
        // Managed backend (#870) emits the structured `retryAfter` field
        // (camelCase). After lower-casing + quote-stripping above it
        // collapses to `retryafter: 30` / `retryafter 30`, so the
        // separator-bearing prefixes here let the same parser surface the
        // structured field the spec asks us to prefer (F5).
        "retryafter:",
        "retryafter ",
    ] {
        if let Some(pos) = normalized.find(prefix) {
            let after = &normalized[pos + prefix.len()..];
            let num_str: String = after
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(secs) = num_str.parse::<f64>() {
                if secs.is_finite() && secs >= 0.0 {
                    return Some(secs.ceil() as u64);
                }
            }
        }
    }
    None
}

/// Format the retry-after hint as a short user-friendly suffix
/// (`" Try again in 30 seconds."`). Returns an empty string when no
/// hint is available so callers can `format!("{summary}{hint}")`
/// without branching on `Option`.
pub(crate) fn retry_after_hint(secs: Option<u64>) -> String {
    match secs {
        Some(0) => " You can retry immediately.".to_string(),
        Some(1) => " Try again in 1 second.".to_string(),
        Some(n) if n < 90 => format!(" Try again in {n} seconds."),
        Some(n) => {
            // Round UP — never tell the user to retry sooner than
            // the upstream actually allows. 90–119s used to render
            // as "about 1 minutes" both because of integer flooring
            // and missing singular/plural handling (CodeRabbit
            // review on #2371).
            let mins = (n / 60) + u64::from(n % 60 != 0);
            let unit = if mins == 1 { "minute" } else { "minutes" };
            format!(" Try again in about {mins} {unit}.")
        }
        None => String::new(),
    }
}

/// Detect the SecurityPolicy global hourly action-budget signal
/// emitted by the built-in tools (`web_fetch`, `curl`, `http_request`,
/// `composio`, etc.) — see `src/openhuman/security/
/// policy.rs::SecurityPolicy::is_rate_limited`.
///
/// We match the canonical English strings those tools emit. This is
/// load-bearing for issue #2364: before this check ran, any string
/// containing "rate limit" was misclassified as a provider 429 and
/// the user saw the generic "You're being rate-limited" copy, which
/// hides that the cap is OpenHuman's own per-hour safety budget,
/// not the upstream LLM provider.
pub(crate) fn is_action_budget_exhausted(err_lower: &str) -> bool {
    err_lower.contains("rate limit exceeded: action budget exhausted")
        || err_lower.contains("rate limit exceeded: too many actions in the last hour")
        || err_lower.contains("action blocked: rate limit exceeded")
}

/// Classify a managed-backend error by its stable `errorCode` (#870).
///
/// Returns `Some` only when the flattened error string carries a *recognised*
/// backend `errorCode`. Because an `errorCode` is present **only** when the
/// error came through the managed backend, branching on it here lets us trust
/// the backend's verdict (operator faults route to the calm "temporarily
/// unavailable — we've been notified" copy, no user-blaming) instead of the
/// substring heuristics, which are tuned for the BYO / direct-provider path
/// (where no `errorCode` exists and "check your API key / model settings" is
/// the correct, user-actionable copy). See [`classify_inference_error`] (F2).
///
/// `None` falls through to the substring ladder, covering both the BYO path
/// (no code) and any future/unrecognised managed code we don't yet map.
fn classify_by_backend_error_code(
    err: &str,
    provider: Option<String>,
    fallback_available: Option<bool>,
) -> Option<ClassifiedError> {
    use crate::openhuman::inference::provider::{
        body_flags_malformed, extract_backend_error_code, is_managed_backend_envelope,
        BackendErrorCode,
    };

    // Managed-vs-BYO gate: an `errorCode` is only trustworthy on a
    // managed-backend envelope. A BYO / direct-provider body that merely
    // contains an `errorCode`-shaped field must fall through to the substring
    // ladder (CodeRabbit), keeping its user-actionable copy intact.
    if !is_managed_backend_envelope(err) {
        return None;
    }

    let code = extract_backend_error_code(err)?;

    // Verbose diagnostics on the new managed-code branch (per CLAUDE.md).
    // Low-cardinality only — the raw `err` may carry a provider payload / PII
    // and is logged at the caller, not here.
    log::debug!(
        "[chat-error][classify][errorCode] code={:?} provider={:?}",
        code,
        provider,
    );

    let classified = match code {
        BackendErrorCode::RateLimited => {
            let retry_secs = parse_retry_after_secs_from_str(err);
            ClassifiedError {
                error_type: "rate_limited",
                message: format!(
                    "Your AI provider is rate-limiting requests. You can retry in this thread.{}",
                    retry_after_hint(retry_secs)
                ),
                source: "provider",
                retryable: true,
                retry_after_ms: retry_secs.map(|s| s.saturating_mul(1000)),
                provider,
                fallback_available,
            }
        }
        BackendErrorCode::UserInsufficientCredits => ClassifiedError {
            error_type: "budget_exhausted",
            message: "You're out of credits. Top up, or switch to 'Use Your Own Models' \
                 in Settings."
                .to_string(),
            source: "openhuman_billing",
            retryable: false,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        },
        // Operator fault (our key/account/quota/5xx) OR operator registry /
        // routing misconfig — NOT user-actionable. Both route to the same
        // calm "we've been notified" copy; the backend already paged. We
        // deliberately DROP the "check your API key" (F4) and "pick a
        // different model" (F6) copy the BYO substring arms would emit.
        BackendErrorCode::UpstreamUnavailable | BackendErrorCode::ModelUnavailable => {
            ClassifiedError {
                error_type: "provider_error",
                message: "The AI service is temporarily unavailable — we've been notified. \
                     Please try again shortly."
                    .to_string(),
                source: "provider",
                retryable: true,
                retry_after_ms: None,
                provider,
                fallback_available,
            }
        }
        BackendErrorCode::PayloadTooLarge => ClassifiedError {
            error_type: "payload_too_large",
            message: "Your message or attachment is too large for this model. Shorten it \
                 or remove the attachment — or start a new thread."
                .to_string(),
            source: "config",
            retryable: false,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        },
        BackendErrorCode::ContextLengthExceeded => ClassifiedError {
            error_type: "context_overflow",
            message: "The conversation is too long. Please start a new chat.".to_string(),
            source: "config",
            retryable: false,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        },
        BackendErrorCode::BadRequest => {
            // Same code, three shapes. FIRST: a tool-ordering rejection
            // (`validateToolMessageOrdering` — an orphaned `role:'tool'` message
            // with no matching assistant `tool_call`) is *poisoned history*, not
            // a model/param problem. The de-poison guard in `run_task.rs` has
            // already evicted the offending warm session by the time this copy
            // is built, so the next turn cold-boots clean — tell the user
            // exactly that (and mark retryable, because resending now works).
            if is_malformed_tool_history_text(&err.to_lowercase()) {
                ClassifiedError {
                    error_type: "provider_request_rejected",
                    message: malformed_history_user_message().to_string(),
                    source: "provider",
                    retryable: true,
                    retry_after_ms: None,
                    provider,
                    fallback_available: None,
                }
            // Else two shapes (B8/F8): a backend-flagged *malformed*
            // payload is a client bug (the request was built wrong — it pages
            // Sentry at the FE layer, gated elsewhere), while a plain
            // user-parameter rejection is a model/param mismatch the user can
            // fix. The copy differs: don't tell the user to abandon the thread
            // for a one-off malformation (only this turn failed).
            } else if body_flags_malformed(err) {
                ClassifiedError {
                    error_type: "provider_request_rejected",
                    message: "Something went wrong with this message. Try rephrasing it — \
                         or start a new thread if it keeps happening."
                        .to_string(),
                    source: "provider",
                    retryable: false,
                    retry_after_ms: None,
                    provider,
                    fallback_available: None,
                }
            } else {
                ClassifiedError {
                    error_type: "provider_request_rejected",
                    message: "The request was rejected — usually a model or parameter \
                         mismatch. Try a different model in Connections → API keys → LLM."
                        .to_string(),
                    source: "provider",
                    retryable: false,
                    retry_after_ms: None,
                    provider,
                    fallback_available: None,
                }
            }
        }
        BackendErrorCode::InternalError => ClassifiedError {
            error_type: "inference",
            // Backend already paged its own 500; the FE must not double-report
            // (gated in the Sentry classifier) and the user just retries.
            message: "Something went wrong — we've been notified. Please try again.".to_string(),
            source: "provider",
            retryable: true,
            retry_after_ms: None,
            provider,
            fallback_available,
        },
    };

    Some(classified)
}
