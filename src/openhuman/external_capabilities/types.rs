use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Config entry for one external capability provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ExternalCapabilityProviderConfig {
    /// Stable provider id used in generated tool provenance.
    pub id: String,
    /// Human-readable display name for diagnostics.
    pub name: String,
    /// Optional source URI for trust/debugging.
    pub source_uri: Option<String>,
    /// Optional source digest, e.g. `sha256:<hex>`.
    pub source_digest: Option<String>,
    /// Whether this provider is trusted to register generated tools.
    pub trusted: bool,
    /// Whether this provider is currently enabled.
    pub enabled: bool,
}

impl Default for ExternalCapabilityProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            source_uri: None,
            source_digest: None,
            trusted: false,
            enabled: true,
        }
    }
}

/// Top-level config section for external capability providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ExternalCapabilityProvidersConfig {
    /// Known external capability providers.
    pub providers: Vec<ExternalCapabilityProviderConfig>,
}

/// Normalized runtime provider record used by registries and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalCapabilityProvider {
    /// Normalized provider id.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Optional source URI for trust/debugging.
    pub source_uri: Option<String>,
    /// Optional source digest, e.g. `sha256:<hex>`.
    pub source_digest: Option<String>,
    /// Whether this provider is trusted to register generated tools.
    pub trusted: bool,
    /// Whether this provider is currently enabled.
    pub enabled: bool,
}

impl ExternalCapabilityProvider {
    /// Whether this provider can currently register generated tools.
    pub fn can_register_tools(&self) -> bool {
        self.enabled && self.trusted
    }
}
