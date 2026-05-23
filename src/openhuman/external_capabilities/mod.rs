//! Registry for external capability providers.
//!
//! This module keeps provider identity and trust metadata generic. It does not
//! know how any provider packages, loads, or executes capabilities; it only
//! normalizes the provider records OpenHuman can use for admission, policy, and
//! diagnostics.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
pub struct ExternalCapabilityProvidersConfig {
    /// Known external capability providers.
    pub providers: Vec<ExternalCapabilityProviderConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalCapabilityProvider {
    pub id: String,
    pub name: String,
    pub source_uri: Option<String>,
    pub source_digest: Option<String>,
    pub trusted: bool,
    pub enabled: bool,
}

impl ExternalCapabilityProvider {
    fn from_config(config: &ExternalCapabilityProviderConfig) -> Result<Self, String> {
        let id = normalize_provider_id(&config.id)
            .ok_or_else(|| format!("invalid external capability provider id `{}`", config.id))?;
        let name = config.name.trim();
        if name.is_empty() {
            return Err(format!(
                "external capability provider `{id}` name must be non-empty"
            ));
        }

        Ok(Self {
            id,
            name: name.to_string(),
            source_uri: trim_optional(&config.source_uri),
            source_digest: trim_optional(&config.source_digest),
            trusted: config.trusted,
            enabled: config.enabled,
        })
    }

    pub fn can_register_tools(&self) -> bool {
        self.enabled && self.trusted
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExternalCapabilityProviderRegistry {
    providers: BTreeMap<String, ExternalCapabilityProvider>,
    errors: Vec<String>,
}

impl ExternalCapabilityProviderRegistry {
    pub fn from_config(config: &ExternalCapabilityProvidersConfig) -> Self {
        let mut providers = BTreeMap::new();
        let mut errors = Vec::new();

        for provider in &config.providers {
            match ExternalCapabilityProvider::from_config(provider) {
                Ok(provider) => {
                    if providers.contains_key(&provider.id) {
                        errors.push(format!(
                            "duplicate external capability provider id `{}`",
                            provider.id
                        ));
                    } else {
                        providers.insert(provider.id.clone(), provider);
                    }
                }
                Err(err) => errors.push(err),
            }
        }

        Self { providers, errors }
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn list(&self) -> Vec<&ExternalCapabilityProvider> {
        self.providers.values().collect()
    }

    pub fn get(&self, provider_id: &str) -> Option<&ExternalCapabilityProvider> {
        normalize_provider_id(provider_id).and_then(|id| self.providers.get(&id))
    }

    pub fn can_register_tools(&self, provider_id: &str) -> bool {
        self.get(provider_id)
            .map(ExternalCapabilityProvider::can_register_tools)
            .unwrap_or(false)
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }
}

pub fn normalize_provider_id(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    let valid = normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.'));
    if !valid {
        return None;
    }
    let starts_or_ends_with_sep = normalized
        .chars()
        .next()
        .zip(normalized.chars().last())
        .map(|(first, last)| is_separator(first) || is_separator(last))
        .unwrap_or(true);
    if starts_or_ends_with_sep {
        return None;
    }
    Some(normalized)
}

fn is_separator(ch: char) -> bool {
    matches!(ch, '-' | '_' | '.')
}

fn trim_optional(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(id: &str) -> ExternalCapabilityProviderConfig {
        ExternalCapabilityProviderConfig {
            id: id.to_string(),
            name: "Local Runtime".to_string(),
            source_uri: Some(" file:///runtime ".to_string()),
            source_digest: Some(" sha256:abc ".to_string()),
            trusted: true,
            enabled: true,
        }
    }

    #[test]
    fn normalizes_valid_provider_ids() {
        assert_eq!(
            normalize_provider_id("  Local.Runtime_1 "),
            Some("local.runtime_1".to_string())
        );
        assert_eq!(
            normalize_provider_id("provider-1"),
            Some("provider-1".to_string())
        );
    }

    #[test]
    fn rejects_invalid_provider_ids() {
        assert_eq!(normalize_provider_id(""), None);
        assert_eq!(normalize_provider_id(".provider"), None);
        assert_eq!(normalize_provider_id("provider."), None);
        assert_eq!(normalize_provider_id("provider id"), None);
        assert_eq!(normalize_provider_id("provider/id"), None);
    }

    #[test]
    fn registry_loads_trusted_enabled_provider() {
        let registry =
            ExternalCapabilityProviderRegistry::from_config(&ExternalCapabilityProvidersConfig {
                providers: vec![config("runtime.local")],
            });

        assert!(registry.errors().is_empty());
        assert_eq!(registry.list().len(), 1);
        assert!(registry.can_register_tools("RUNTIME.LOCAL"));
        let provider = registry.get("runtime.local").unwrap();
        assert_eq!(provider.source_uri.as_deref(), Some("file:///runtime"));
        assert_eq!(provider.source_digest.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn disabled_or_untrusted_provider_cannot_register_tools() {
        let mut disabled = config("disabled.runtime");
        disabled.enabled = false;
        let mut untrusted = config("untrusted.runtime");
        untrusted.trusted = false;
        let registry =
            ExternalCapabilityProviderRegistry::from_config(&ExternalCapabilityProvidersConfig {
                providers: vec![disabled, untrusted],
            });

        assert!(!registry.can_register_tools("disabled.runtime"));
        assert!(!registry.can_register_tools("untrusted.runtime"));
    }

    #[test]
    fn registry_reports_duplicates_and_invalid_records() {
        let mut unnamed = config("unnamed.runtime");
        unnamed.name = " ".to_string();
        let registry =
            ExternalCapabilityProviderRegistry::from_config(&ExternalCapabilityProvidersConfig {
                providers: vec![config("runtime.local"), config("RUNTIME.LOCAL"), unnamed],
            });

        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.errors().len(), 2);
        assert!(registry.errors()[0].contains("duplicate"));
        assert!(registry.errors()[1].contains("name must be non-empty"));
    }
}
