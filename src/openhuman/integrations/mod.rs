//! Agent integration tools that proxy through the backend API.
//!
//! Each tool calls a backend endpoint (authenticated via JWT Bearer token) which
//! handles external API calls, billing, rate limiting, and markup. The client
//! never talks to external services directly.

pub mod client;
pub mod google_places;
pub mod parallel;
pub mod twilio;
pub mod types;

pub use client::{build_client, IntegrationClient};
pub use google_places::{GooglePlacesDetailsTool, GooglePlacesSearchTool};
pub use parallel::{ParallelExtractTool, ParallelSearchTool};
pub use twilio::TwilioCallTool;
pub use types::{
    BackendResponse, IntegrationPricing, IntegrationPricingEntry, PricingIntegrations, ToolScope,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_scope_equality() {
        assert_eq!(ToolScope::All, ToolScope::All);
        assert_ne!(ToolScope::All, ToolScope::CliRpcOnly);
        assert_ne!(ToolScope::AgentOnly, ToolScope::CliRpcOnly);
    }

    #[test]
    fn backend_response_deserializes() {
        let json = r#"{"success": true, "data": {"foo": 42}}"#;
        let resp: BackendResponse<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert_eq!(resp.data.unwrap()["foo"], 42);
    }

    #[test]
    fn backend_response_without_data() {
        let json = r#"{"success": true}"#;
        let resp: BackendResponse<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert!(resp.data.is_none());
    }

    #[test]
    fn integration_pricing_defaults_on_missing_fields() {
        let json = r#"{"integrations": {}}"#;
        let pricing: IntegrationPricing = serde_json::from_str(json).unwrap();
        assert!(pricing.integrations.twilio.is_none());
        assert!(pricing.integrations.google_places.is_none());
        assert!(pricing.integrations.parallel.is_none());
    }

    #[test]
    fn build_client_returns_none_when_no_auth_token() {
        let mut config = crate::openhuman::config::Config::default();
        config.api_key = None;
        assert!(build_client(&config).is_none());
    }

    #[test]
    fn build_client_uses_core_api_key() {
        // No per-integration config exists any more — the client is
        // built solely from the core `config.api_key` / `config.api_url`.
        let mut config = crate::openhuman::config::Config::default();
        config.api_key = Some("root-token".into());
        config.api_url = Some("https://api.example.test".into());
        assert!(build_client(&config).is_some());
    }

    #[test]
    fn build_client_rejects_whitespace_only_api_key() {
        let mut config = crate::openhuman::config::Config::default();
        config.api_key = Some("   ".into());
        assert!(build_client(&config).is_none());
    }
}
