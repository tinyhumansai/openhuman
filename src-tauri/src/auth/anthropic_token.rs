use serde::{Deserialize, Serialize};

/// How Anthropic credentials should be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnthropicAuthKind {
    /// Standard Anthropic API key via `x-api-key`.
    ApiKey,
    /// Subscription / setup token via `Authorization: Bearer ...`.
    Authorization,
}

impl AnthropicAuthKind {
    pub fn as_metadata_value(self) -> &'static str {
        match self {
            Self::ApiKey => "api-key",
            Self::Authorization => "authorization",
        }
    }

    pub fn from_metadata_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "api-key" | "x-api-key" | "apikey" => Some(Self::ApiKey),
            "authorization" | "bearer" | "auth-token" | "oauth" => Some(Self::Authorization),
            _ => None,
        }
    }
}

/// Detect auth kind with explicit override support.
pub fn detect_auth_kind(token: &str, explicit: Option<&str>) -> AnthropicAuthKind {
    if let Some(kind) = explicit.and_then(AnthropicAuthKind::from_metadata_value) {
        return kind;
    }

    let trimmed = token.trim();

    // JWT-like shape strongly suggests bearer token mode.
    if trimmed.matches('.').count() >= 2 {
        return AnthropicAuthKind::Authorization;
    }

    // Anthropic platform keys commonly start with this prefix.
    if trimmed.starts_with("sk-ant-api") {
        return AnthropicAuthKind::ApiKey;
    }

    // Default to API key for backward compatibility unless explicitly configured.
    AnthropicAuthKind::ApiKey
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kind_from_metadata() {
        assert_eq!(
            AnthropicAuthKind::from_metadata_value("authorization"),
            Some(AnthropicAuthKind::Authorization)
        );
        assert_eq!(
            AnthropicAuthKind::from_metadata_value("x-api-key"),
            Some(AnthropicAuthKind::ApiKey)
        );
        assert_eq!(AnthropicAuthKind::from_metadata_value("nope"), None);
    }

    #[test]
    fn detect_prefers_override() {
        let kind = detect_auth_kind("sk-ant-api-123", Some("authorization"));
        assert_eq!(kind, AnthropicAuthKind::Authorization);
    }

    #[test]
    fn detect_jwt_like_as_authorization() {
        let kind = detect_auth_kind("aaa.bbb.ccc", None);
        assert_eq!(kind, AnthropicAuthKind::Authorization);
    }

    #[test]
    fn detect_default_for_api_prefix() {
        let kind = detect_auth_kind("sk-ant-api-123", None);
        assert_eq!(kind, AnthropicAuthKind::ApiKey);
    }
}
