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
     local model like Ollama, switch routing to \"Use Your Own Models\" in Settings → AI Configuration."
}

pub(crate) fn generic_inference_error_user_message() -> &'static str {
    "Something went wrong. Please try again.\nThis error has been reported. You can also report it on Discord.\n<openhuman-link path=\"community/discord-report\">Report on Discord</openhuman-link>"
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
    /// `max_iterations`, `timeout`, `auth_error`, `budget_exhausted`,
    /// `provider_error`, `context_overflow`, `model_unavailable`,
    /// `inference`.
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
/// provider::reliable::parse_retry_after_ms`] helper but operates on
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
/// `polymarket`, `composio`, etc.) — see `src/openhuman/security/
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

pub(crate) fn classify_inference_error(err: &str) -> ClassifiedError {
    let lower = err.to_lowercase();
    let provider = extract_provider_name(err);
    let fallback_available = if is_fallback_chain_exhausted(err) {
        Some(false)
    } else {
        None
    };

    // Order matters: the SecurityPolicy hourly cap and the
    // agent-loop max-iterations error both surface as strings that
    // contain "rate limit" / "iteration", so they MUST be checked
    // before the generic provider-429 branch — otherwise users see
    // a confusing "your AI provider is rate-limiting you" message
    // for limits OpenHuman itself enforced (issue #2364).
    if is_action_budget_exhausted(&lower) {
        ClassifiedError {
            error_type: "action_budget_exceeded",
            message: with_provider_detail(
                "You've hit OpenHuman's per-hour action budget — this is a local safety cap, \
                 not your AI provider. The window decays gradually; you can keep chatting in \
                 this thread and tool-heavy steps will resume as the budget refills.",
                err,
            ),
            source: "openhuman_budget",
            // The window decays gradually so the same thread CAN recover
            // — we just can't predict the exact wait.
            retryable: true,
            retry_after_ms: None,
            // OpenHuman's own cap — provider name (if any was in the
            // surrounding error chain) is irrelevant; the limit isn't
            // from a provider.
            provider: None,
            fallback_available: None,
        }
    } else if crate::openhuman::agent::error::is_max_iterations_error(err) {
        ClassifiedError {
            error_type: "max_iterations",
            message: with_provider_detail(
                "The agent ran the maximum number of tool steps for one turn without \
                 finishing. This usually means a tool kept failing (often a rate limit on a \
                 web fetch). You can retry the same question in this thread once the \
                 underlying limit clears.",
                err,
            ),
            source: "agent_loop",
            retryable: true,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        }
    } else if lower.contains("rate limit") || lower.contains("429") {
        let retry_secs = parse_retry_after_secs_from_str(err);
        // Non-retryable business 429s ("plan does not include", balance
        // exhausted, known provider business codes like Z.AI 1311/1113)
        // also surface here — mark them non-retryable so the FE can hide
        // the "Retry" button and route the user to settings/billing.
        let non_retryable = is_non_retryable_rate_limit_text(&lower);
        let summary = if non_retryable {
            "Your AI provider is rejecting requests for billing or plan reasons \
             (out of credits, plan limit, or unavailable model). Retrying won't \
             help — open Settings to top up, upgrade your plan, or pick a \
             different model."
                .to_string()
        } else {
            format!(
                "Your AI provider is rate-limiting requests. This is a transient upstream \
                 limit, not a thread-level block — you can retry in this thread.{}",
                retry_after_hint(retry_secs)
            )
        };
        ClassifiedError {
            error_type: "rate_limited",
            message: with_provider_detail(summary.as_str(), err),
            source: "provider",
            retryable: !non_retryable,
            retry_after_ms: retry_secs.map(|s| s.saturating_mul(1000)),
            provider,
            fallback_available,
        }
    } else if lower.contains("timeout") || lower.contains("timed out") {
        ClassifiedError {
            error_type: "timeout",
            message: with_provider_detail(
                "The request timed out. Please check your connection and try again.",
                err,
            ),
            source: "transport",
            retryable: true,
            retry_after_ms: None,
            provider,
            fallback_available,
        }
    } else if lower.contains("401") || lower.contains("unauthorized") || lower.contains("api key") {
        ClassifiedError {
            error_type: "auth_error",
            message: with_provider_detail(
                "There's an authentication issue with the AI provider. Please check your API key in settings.",
                err,
            ),
            source: "config",
            retryable: false,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        }
    } else if lower.contains("402")
        || lower.contains("payment required")
        || lower.contains("insufficient balance")
        // Issue #3088: the OpenHuman managed backend reports no-credits as a
        // 400 with "Insufficient budget" (not a 402), which previously fell
        // through to the generic "Something went wrong" branch. Catch the
        // canonical budget phrases here so the user gets the actionable
        // top-up / switch-to-your-own-model guidance instead.
        || is_inference_budget_exceeded_error(err)
    {
        // `openhuman_billing` means OpenHuman's own credit/quota system —
        // a 402 carrying the "openhuman" envelope (or no envelope at all,
        // since OpenHuman's backend is the only origin without one in
        // practice). When the 402 comes from an upstream provider envelope
        // (`<provider> API error (402)`), the limit belongs to that
        // provider, not OpenHuman billing, so tag the source as `provider`.
        let source: &'static str = match provider.as_deref() {
            Some("openhuman") | None => "openhuman_billing",
            Some(_) => "provider",
        };
        ClassifiedError {
            error_type: "budget_exhausted",
            message: with_provider_detail(inference_budget_exceeded_user_message(), err),
            source,
            retryable: false,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        }
    } else if lower.contains("500")
        || lower.contains("internal server")
        || lower.contains("service unavailable")
        || lower.contains("503")
    {
        ClassifiedError {
            error_type: "provider_error",
            message: with_provider_detail(
                "The AI provider is temporarily unavailable. Please try again later.",
                err,
            ),
            source: "provider",
            retryable: true,
            retry_after_ms: None,
            provider,
            fallback_available,
        }
    } else if lower.contains("context")
        && (lower.contains("length")
            || lower.contains("limit")
            || lower.contains("exceed")
            || lower.contains("token"))
    {
        ClassifiedError {
            error_type: "context_overflow",
            message: with_provider_detail(
                "The conversation is too long. Please start a new chat.",
                err,
            ),
            source: "config",
            retryable: false,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        }
    } else if crate::openhuman::inference::provider::is_provider_config_rejection_message(err) {
        // #2079 / #2076 / #2202: an OpenHuman abstract tier alias leaked to
        // a custom provider, a stale model pin, or a model-specific
        // temperature constraint. Checked BEFORE the generic
        // model-unavailable arm so config-rejection bodies that also
        // contain "model"/"does not exist"/"does not have access" get the
        // specific "Settings → LLM" remediation instead of the generic
        // copy. Shared predicate keeps this in lockstep with the
        // Sentry-demotion classifier.
        ClassifiedError {
            error_type: "model_unavailable",
            message: with_provider_detail(
                "Your AI provider rejected the request's model or temperature setting. \
                 Check your model and routing in Settings → LLM.",
                err,
            ),
            source: "config",
            retryable: false,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        }
    } else if lower.contains("model")
        && (lower.contains("not found")
            || lower.contains("unavailable")
            || lower.contains("does not exist")
            || lower.contains("does not have access"))
    {
        ClassifiedError {
            error_type: "model_unavailable",
            message: with_provider_detail(
                "The selected model isn't available on your provider. Check your model settings.",
                err,
            ),
            source: "config",
            retryable: false,
            retry_after_ms: None,
            provider,
            fallback_available: None,
        }
    } else {
        ClassifiedError {
            error_type: "inference",
            message: with_provider_detail(generic_inference_error_user_message(), err),
            source: "provider",
            retryable: true,
            retry_after_ms: None,
            provider,
            fallback_available,
        }
    }
}

/// String-flat mirror of
/// [`crate::openhuman::inference::provider::reliable::is_non_retryable_rate_limit`].
///
/// The reliable provider already classifies 429s into retryable vs
/// non-retryable based on business-quota markers ("plan does not
/// include", "insufficient balance", Z.AI codes 1311/1113, …) — but
/// that typed `anyhow::Error` is collapsed to a `String` at the
/// native-bus boundary before reaching this layer. We re-detect the
/// same markers in the flattened string so the FE knows whether to
/// offer a "Retry" button.
///
/// Caller passes the already-lowercased error string to avoid double
/// allocation.
pub(crate) fn is_non_retryable_rate_limit_text(lower: &str) -> bool {
    const BUSINESS_HINTS: &[&str] = &[
        "plan does not include",
        "doesn't include",
        "not include",
        "insufficient balance",
        "insufficient_balance",
        "insufficient quota",
        "insufficient_quota",
        "quota exhausted",
        "out of credits",
        "no available package",
        "package not active",
        "purchase package",
        "model not available for your plan",
    ];
    if BUSINESS_HINTS.iter().any(|hint| lower.contains(hint)) {
        return true;
    }
    // Known provider business codes observed for 429 where retry is
    // futile (mirrors reliable.rs). Scan integer-like tokens.
    for token in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if matches!(code, 1113 | 1311) {
                return true;
            }
        }
    }
    false
}
