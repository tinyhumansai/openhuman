//! External capability-provider trust metadata.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configured external capability provider metadata.
///
/// The registry layer normalizes and validates `id` before policy code uses it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct CapabilityProviderConfig {
    /// Human-configured provider id. Registry normalization makes this stable.
    pub id: String,
    /// Display name for diagnostics and future UI surfaces.
    pub display_name: String,
    /// Optional source URI for provenance, such as a GitHub repo or MCP catalog URL.
    pub source_uri: Option<String>,
    /// Optional source digest, for example `sha256:<hex>`.
    pub source_digest: Option<String>,
    /// Explicit trust state. Defaults to `untrusted` for fail-closed behavior.
    pub trust_state: CapabilityProviderTrustState,
    /// Whether this provider is enabled for discovery/admission.
    pub enabled: bool,
}

impl Default for CapabilityProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            source_uri: None,
            source_digest: None,
            trust_state: CapabilityProviderTrustState::Untrusted,
            enabled: false,
        }
    }
}

/// Trust state for an external capability provider.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProviderTrustState {
    /// Provider metadata is accepted, but capabilities from it are not trusted.
    Untrusted,
    /// Provider is explicitly trusted by local config.
    Trusted,
}

impl Default for CapabilityProviderTrustState {
    fn default() -> Self {
        Self::Untrusted
    }
}
