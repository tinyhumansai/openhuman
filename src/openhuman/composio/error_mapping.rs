//! Classify and format Composio tool failures so validation, scope, and
//! upstream-provider errors are not surfaced as generic gateway (502) failures.
//!
//! Issue #1797 — Composio support found tool-level failures on their side while
//! OpenHuman was bucketing them as HTTP 502 / gateway instability.

/// Stable, grep-friendly error classes for metrics and UI routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposioErrorClass {
    Validation,
    InsufficientScope,
    /// The connection lacks permission to enable/manage triggers — a 403 whose
    /// body does **not** mention "scope" (so [`Self::InsufficientScope`] never
    /// matches it). Distinct so the user gets reconnect guidance. See #2913.
    TriggerPermission,
    RateLimited,
    /// Composio answered the execute call with an HTTP 404/410 — the action
    /// endpoint or slug is unknown (or the endpoint is deprecated/Gone), NOT a
    /// broken connection. Distinct so the user is told their integration is
    /// still connected and is **not** advised to re-authenticate a healthy
    /// account. See #3219 (direct-mode sent the lowercase toolkit slug to the
    /// wrong execute path → 404/410 → misclassified as [`Self::ComposioPlatform`]).
    ActionNotFound,
    UpstreamProvider,
    ComposioPlatform,
    Gateway,
    Other,
}

impl ComposioErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::InsufficientScope => "insufficient_scope",
            Self::TriggerPermission => "trigger_permission",
            Self::RateLimited => "rate_limited",
            Self::ActionNotFound => "action_not_found",
            Self::UpstreamProvider => "upstream_provider",
            Self::ComposioPlatform => "composio_platform",
            Self::Gateway => "gateway",
            Self::Other => "other",
        }
    }
}

pub fn classify_composio_error(tool: &str, message: &str) -> ComposioErrorClass {
    let lower = message.to_ascii_lowercase();
    // EARLY status arm (#3219): a Composio HTTP 404/410 means the action
    // endpoint/slug is unknown or deprecated — NOT a broken connection. It must
    // win over the substring arms below, because the 404 body frequently carries
    // the phrase "connection error, try to authenticate" that
    // `is_composio_platform_shape` would otherwise match, telling the user to
    // re-authenticate a perfectly healthy connection.
    let class = if is_action_not_found_shape(&lower) {
        ComposioErrorClass::ActionNotFound
    } else if is_validation_shape(&lower) {
        ComposioErrorClass::Validation
    } else if is_insufficient_scope_shape(&lower) {
        ComposioErrorClass::InsufficientScope
    } else if is_trigger_permission_shape(&lower) {
        ComposioErrorClass::TriggerPermission
    } else if is_rate_limited_shape(&lower) {
        ComposioErrorClass::RateLimited
    } else if is_gateway_transport_shape(&lower) && !is_embedded_provider_failure(&lower) {
        ComposioErrorClass::Gateway
    } else if is_composio_platform_shape(&lower) {
        ComposioErrorClass::ComposioPlatform
    } else if tool.starts_with("GMAIL_")
        || tool.starts_with("SLACK_")
        || tool.starts_with("NOTION_")
        || tool.starts_with("GOOGLECALENDAR_")
    {
        ComposioErrorClass::UpstreamProvider
    } else {
        ComposioErrorClass::Other
    };
    tracing::debug!(
        tool = %tool,
        class = class.as_str(),
        "[composio][classify] error classified"
    );
    class
}

pub fn format_provider_error(tool: &str, raw: &str) -> String {
    let class = classify_composio_error(tool, raw);
    let detail = raw.trim();
    let body = match class {
        ComposioErrorClass::Validation => format!("Invalid arguments for `{tool}`: {detail}"),
        ComposioErrorClass::InsufficientScope => format_insufficient_scope_message(tool, detail),
        ComposioErrorClass::TriggerPermission => format_trigger_permission_message(tool),
        ComposioErrorClass::RateLimited => format_rate_limited_message(tool, detail),
        ComposioErrorClass::ActionNotFound => format_action_not_found_message(tool),
        ComposioErrorClass::UpstreamProvider => {
            format!("`{tool}` failed at the connected provider: {detail}")
        }
        ComposioErrorClass::ComposioPlatform => {
            format!("Composio connection issue for `{tool}`: {detail}")
        }
        ComposioErrorClass::Gateway => {
            format!("Temporary gateway error while calling `{tool}`: {detail}")
        }
        ComposioErrorClass::Other => format!("`{tool}` failed: {detail}"),
    };
    prefix_class(class, &body)
}

pub fn remap_transport_error(tool: &str, raw: &str) -> String {
    let detail = extract_transport_detail(raw);
    let class = if is_embedded_provider_failure(&detail) {
        classify_composio_error(tool, &detail)
    } else if is_gateway_transport_shape(raw) {
        ComposioErrorClass::Gateway
    } else {
        classify_composio_error(tool, raw)
    };
    let body = match class {
        ComposioErrorClass::InsufficientScope => format_insufficient_scope_message(tool, &detail),
        ComposioErrorClass::TriggerPermission => format_trigger_permission_message(tool),
        ComposioErrorClass::RateLimited => format_rate_limited_message(tool, &detail),
        ComposioErrorClass::ActionNotFound => format_action_not_found_message(tool),
        ComposioErrorClass::Gateway => format!(
            "Temporary gateway error while calling `{tool}`: {}",
            summarize_gateway(raw)
        ),
        ComposioErrorClass::Validation => format!("Invalid arguments for `{tool}`: {detail}"),
        ComposioErrorClass::UpstreamProvider => {
            format!("`{tool}` failed at the connected provider: {detail}")
        }
        ComposioErrorClass::ComposioPlatform => {
            format!("Composio connection issue for `{tool}`: {detail}")
        }
        ComposioErrorClass::Other => format!("`{tool}` failed: {detail}"),
    };
    prefix_class(class, &body)
}

fn prefix_class(class: ComposioErrorClass, body: &str) -> String {
    format!("[composio:error:{}] {}", class.as_str(), body)
}

/// Derive the lowercase toolkit slug from a Composio tool/trigger identifier.
///
/// Identifiers are upper-snake-case (e.g. `GMAIL_NEW_GMAIL_MESSAGE`); the leading
/// segment names the toolkit, so this returns e.g. `gmail`. `str::split('_').next()`
/// always yields `Some(_)` for `&str` input (empty input yields `Some("")`), so
/// `unwrap_or_default()` is a safe, equivalent terminator.
fn derive_toolkit_slug(tool: &str) -> String {
    tool.split('_')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn format_insufficient_scope_message(tool: &str, detail: &str) -> String {
    let toolkit = derive_toolkit_slug(tool);
    format!(
        "`{tool}` was rejected because the connected {toolkit} account is missing required \
         permissions ({detail}). Reconnect the integration in Settings → Connections → \
         {toolkit} and grant the scopes requested during OAuth."
    )
}

/// Build the trigger-permission guidance message (issue #2913).
///
/// The toolkit slug is derived from the tool/trigger identifier the same way
/// [`format_insufficient_scope_message`] does (e.g. `GMAIL_NEW_GMAIL_MESSAGE`
/// → `gmail`), so the copy is branded and points the user at reconnecting.
fn format_trigger_permission_message(tool: &str) -> String {
    let toolkit = derive_toolkit_slug(tool);
    format!(
        "Couldn't enable this trigger: the connected {toolkit} account doesn't have \
         permission to manage triggers. Reconnect {toolkit} in Settings → Connections → \
         {toolkit} and grant the permissions requested during OAuth, then try again."
    )
}

/// Build the action-not-found guidance (issue #3219).
///
/// Deliberately does **not** echo the raw provider `detail`: a Composio 404
/// body often literally reads "connection error, try to authenticate", and
/// surfacing that verbatim would re-introduce the exact misleading re-auth
/// nudge this class exists to suppress. The raw text is still captured in the
/// `tracing::debug!` line in [`classify_composio_error`] for diagnosis.
fn format_action_not_found_message(tool: &str) -> String {
    let toolkit = derive_toolkit_slug(tool);
    format!(
        "`{tool}` couldn't run: Composio reported this action as not found. Your {toolkit} \
         integration is still connected and working — this is not a sign-in problem. The action \
         name is likely out of date or Composio's API changed; try again with the current action \
         name, or report this if it keeps happening."
    )
}

fn format_rate_limited_message(tool: &str, detail: &str) -> String {
    format!(
        "`{tool}` hit an upstream rate limit ({detail}). Wait a minute and retry, or reduce \
         call frequency — this is not an OpenHuman gateway outage."
    )
}

fn is_validation_shape(lower: &str) -> bool {
    lower.contains("invalid arguments")
        || lower.contains("missing required")
        || lower.contains("must not be empty")
        || lower.contains("required field")
        || lower.contains("bad request")
        || lower.contains("invalid date")
        || lower.contains("rfc 3339")
        || lower.contains("timemax")
        || lower.contains("timemin")
}

fn is_insufficient_scope_shape(lower: &str) -> bool {
    lower.contains("insufficient authentication scopes")
        || lower.contains("insufficient scope")
        || lower.contains("insufficient permissions")
        || (lower.contains("403") && lower.contains("scope"))
        || lower.contains("invalid oauth scope")
}

/// Heuristic for a trigger-permission denial (issue #2913).
///
/// The backend 403 body reads "You do not have permission to enable triggers on
/// this connection" — note it has **no** "scope" token, so
/// [`is_insufficient_scope_shape`] never matches. We require a forbidden signal
/// (`403`/`forbidden`) AND a permission-denied phrase AND trigger context, so a
/// generic 403 won't be miscategorised.
fn is_trigger_permission_shape(lower: &str) -> bool {
    let forbidden = lower.contains("403") || lower.contains("forbidden");
    let permission_denied = lower.contains("do not have permission")
        || lower.contains("not have permission")
        || lower.contains("permission denied")
        || lower.contains("not permitted")
        || lower.contains("not allowed");
    let trigger_context = lower.contains("trigger");
    forbidden && permission_denied && trigger_context
}

fn is_rate_limited_shape(lower: &str) -> bool {
    lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("ratelimited")
        || lower.contains("too many requests")
        || lower.contains("429")
}

/// True when the message carries a Composio HTTP 404 or 410 status (#3219).
///
/// Both `response_error` shapes (`HTTP 404` and `HTTP 404: <body>`) and the
/// wrapped v3/v2 fallback string keep the literal `HTTP <code>` token, so a
/// substring scan is reliable. Scoped to 404 (action/endpoint unknown) and 410
/// (endpoint Gone/deprecated) on purpose — other 4xx/5xx keep their existing
/// validation / scope / rate-limit / gateway classifications.
fn is_action_not_found_shape(lower: &str) -> bool {
    lower.contains("http 404") || lower.contains("http 410")
}

fn is_composio_platform_shape(lower: &str) -> bool {
    lower.contains("connection error, try to authenticate")
        || lower.contains("not enabled")
        || lower.contains("not connected")
        || lower.contains("token revoked")
}

fn is_gateway_transport_shape(lower: &str) -> bool {
    lower.contains("backend returned 502")
        || lower.contains("502 bad gateway")
        || lower.contains("backend returned 503")
        || lower.contains("backend returned 504")
        || lower.contains("(502 ")
        || lower.contains("(503 ")
        || lower.contains("(504 ")
}

fn is_embedded_provider_failure(lower: &str) -> bool {
    is_validation_shape(lower)
        || is_insufficient_scope_shape(lower)
        || is_trigger_permission_shape(lower)
        || is_rate_limited_shape(lower)
        || is_action_not_found_shape(lower)
        || is_composio_platform_shape(lower)
        || lower.contains("composio")
        || lower.contains("google")
        || lower.contains("slack")
        || lower.contains("notion")
        || lower.contains("gmail")
        || lower.contains("fetch_type")
        || lower.contains("timemax")
        || lower.contains("timemin")
}

fn extract_transport_detail(raw: &str) -> String {
    raw.split_once(": ")
        .map(|(_, tail)| tail.to_string())
        .unwrap_or_else(|| raw.to_string())
}

fn summarize_gateway(raw: &str) -> String {
    if let Some(idx) = raw.find("Backend returned ") {
        let rest = &raw[idx..];
        if let Some(colon) = rest.rfind(": ") {
            return rest[colon + 2..].trim().to_string();
        }
        return rest.trim().to_string();
    }
    raw.trim().to_string()
}

#[cfg(test)]
#[path = "error_mapping_tests.rs"]
mod tests;
