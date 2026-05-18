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
    RateLimited,
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
            Self::RateLimited => "rate_limited",
            Self::UpstreamProvider => "upstream_provider",
            Self::ComposioPlatform => "composio_platform",
            Self::Gateway => "gateway",
            Self::Other => "other",
        }
    }
}

pub fn classify_composio_error(tool: &str, message: &str) -> ComposioErrorClass {
    let lower = message.to_ascii_lowercase();
    let class = if is_validation_shape(&lower) {
        ComposioErrorClass::Validation
    } else if is_insufficient_scope_shape(&lower) {
        ComposioErrorClass::InsufficientScope
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
        ComposioErrorClass::RateLimited => format_rate_limited_message(tool, detail),
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
        ComposioErrorClass::RateLimited => format_rate_limited_message(tool, &detail),
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

fn format_insufficient_scope_message(tool: &str, detail: &str) -> String {
    let toolkit = tool
        .split('_')
        .next()
        .unwrap_or("integration")
        .to_ascii_lowercase();
    format!(
        "`{tool}` was rejected because the connected {toolkit} account is missing required \
         permissions ({detail}). Reconnect the integration in Settings → Skills and grant the \
         scopes requested during OAuth."
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

fn is_rate_limited_shape(lower: &str) -> bool {
    lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("ratelimited")
        || lower.contains("too many requests")
        || lower.contains("429")
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
        || is_rate_limited_shape(lower)
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
