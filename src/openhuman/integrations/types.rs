//! Shared types for agent integration tools.

use serde::Deserialize;

// Re-export ToolScope from the canonical definition in tools::traits.
pub use crate::openhuman::tools::traits::ToolScope;

// ── Pricing types (fetched from backend) ────────────────────────────

/// Per-integration pricing returned by `GET /agent-integrations/pricing`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct IntegrationPricing {
    #[serde(default)]
    pub integrations: PricingIntegrations,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PricingIntegrations {
    #[serde(default)]
    pub twilio: Option<IntegrationPricingEntry>,
    #[serde(default)]
    pub google_places: Option<IntegrationPricingEntry>,
    #[serde(default)]
    pub parallel: Option<IntegrationPricingEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntegrationPricingEntry {
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub pricing: serde_json::Value,
}

// ── Backend response envelope ───────────────────────────────────────

/// Standard `{ success, data, error }` envelope from the backend.
#[derive(Debug, Deserialize)]
pub struct BackendResponse<T> {
    #[allow(dead_code)]
    pub success: bool,
    pub data: Option<T>,
    #[serde(default)]
    #[allow(dead_code)]
    pub error: Option<String>,
}
