use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::openhuman::config::schema::{CapabilityProviderTrustState, Config};

use super::types::CapabilityProviderDiagnostics;

const MAX_PROVIDER_ID_LEN: usize = 96;

/// Normalized provider metadata used by policy and diagnostics callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityProviderMetadata {
    pub id: String,
    pub display_name: String,
    pub source_uri: Option<String>,
    pub source_digest: Option<String>,
    pub trust_state: CapabilityProviderTrustState,
    pub enabled: bool,
}

/// In-memory view of configured external capability providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityProviderRegistry {
    providers: BTreeMap<String, CapabilityProviderMetadata>,
}

impl CapabilityProviderRegistry {
    /// Build a normalized provider registry from the current config snapshot.
    pub fn from_config(config: &Config) -> Result<Self, CapabilityProviderRegistryError> {
        log::trace!(
            "[tool_registry] capability_provider_registry start configured_providers={}",
            config.capability_providers.len()
        );

        let mut providers = BTreeMap::new();

        for provider in &config.capability_providers {
            let id = match normalize_capability_provider_id(&provider.id) {
                Ok(id) => id,
                Err(err) => {
                    log::debug!(
                        "[tool_registry] capability_provider_registry invalid_provider_id raw_id={:?} error={}",
                        provider.id,
                        err
                    );
                    return Err(err);
                }
            };
            log::debug!(
                "[tool_registry] capability_provider_registry normalized_provider raw_id={:?} provider_id={} enabled={} trust_state={:?}",
                provider.id,
                id,
                provider.enabled,
                provider.trust_state
            );

            if providers.contains_key(&id) {
                log::debug!(
                    "[tool_registry] capability_provider_registry duplicate_provider_id provider_id={}",
                    id
                );
                return Err(CapabilityProviderRegistryError::DuplicateId { id });
            }

            let display_name = match clean_string(&provider.display_name) {
                Some(display_name) => display_name,
                None => {
                    log::debug!(
                        "[tool_registry] capability_provider_registry display_name_empty provider_id={} fallback=provider_id",
                        id
                    );
                    id.clone()
                }
            };
            let metadata = CapabilityProviderMetadata {
                id: id.clone(),
                display_name,
                source_uri: clean_string_opt(provider.source_uri.as_deref()),
                source_digest: clean_string_opt(provider.source_digest.as_deref()),
                trust_state: provider.trust_state.clone(),
                enabled: provider.enabled,
            };
            providers.insert(id, metadata);
        }

        log::debug!(
            "[tool_registry] capability_provider_registry completed providers={}",
            providers.len()
        );
        Ok(Self { providers })
    }

    /// Return all providers sorted by normalized id.
    pub fn list(&self) -> Vec<CapabilityProviderMetadata> {
        self.providers.values().cloned().collect()
    }

    /// Look up a provider by raw or normalized id.
    pub fn get(&self, id: &str) -> Option<&CapabilityProviderMetadata> {
        let normalized = normalize_capability_provider_id(id).ok()?;
        self.providers.get(&normalized)
    }

    /// True only when the provider is both enabled and explicitly trusted.
    pub fn is_trusted_enabled(&self, id: &str) -> bool {
        self.get(id).is_some_and(|provider| {
            provider.enabled && provider.trust_state == CapabilityProviderTrustState::Trusted
        })
    }
}

/// Normalize provider ids to a stable, policy-safe slug.
pub fn normalize_capability_provider_id(
    raw: &str,
) -> Result<String, CapabilityProviderRegistryError> {
    let raw = raw.trim();
    if raw.is_empty() {
        log::trace!("[tool_registry] normalize_capability_provider_id invalid_empty");
        return Err(CapabilityProviderRegistryError::InvalidId {
            raw: raw.to_string(),
        });
    }

    let mut normalized = String::new();
    let mut previous_separator = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            previous_separator = false;
        } else if ch == '-' || ch == '_' || ch == '.' || ch.is_ascii_whitespace() {
            push_separator(&mut normalized, &mut previous_separator, ch);
        } else {
            push_separator(&mut normalized, &mut previous_separator, '-');
        }
    }

    let normalized = normalized
        .trim_matches(|ch| ch == '-' || ch == '_' || ch == '.')
        .to_string();

    if normalized.is_empty() || normalized.len() > MAX_PROVIDER_ID_LEN {
        log::trace!(
            "[tool_registry] normalize_capability_provider_id invalid raw_id={:?} normalized_id={:?} normalized_len={}",
            raw,
            normalized,
            normalized.len()
        );
        return Err(CapabilityProviderRegistryError::InvalidId {
            raw: raw.to_string(),
        });
    }

    log::trace!(
        "[tool_registry] normalize_capability_provider_id completed raw_id={:?} normalized_id={}",
        raw,
        normalized
    );
    Ok(normalized)
}

/// Build the configured provider registry.
pub fn capability_provider_registry(
    config: &Config,
) -> Result<CapabilityProviderRegistry, CapabilityProviderRegistryError> {
    let registry = CapabilityProviderRegistry::from_config(config);
    if let Err(err) = &registry {
        log::debug!(
            "[tool_registry] capability_provider_registry failed configured_providers={} error={}",
            config.capability_providers.len(),
            err
        );
    }
    registry
}

/// List configured external capability providers.
pub fn list_capability_providers(
    config: &Config,
) -> Result<Vec<CapabilityProviderMetadata>, CapabilityProviderRegistryError> {
    Ok(capability_provider_registry(config)?.list())
}

/// Look up one configured external capability provider.
pub fn capability_provider_by_id(
    config: &Config,
    id: &str,
) -> Result<Option<CapabilityProviderMetadata>, CapabilityProviderRegistryError> {
    Ok(capability_provider_registry(config)?.get(id).cloned())
}

/// Return whether a configured provider may be treated as trusted and enabled.
pub fn is_capability_provider_trusted_enabled(config: &Config, id: &str) -> bool {
    capability_provider_registry(config)
        .map(|registry| registry.is_trusted_enabled(id))
        .unwrap_or(false)
}

/// Return a redacted provider summary for tool-policy diagnostics.
pub fn capability_provider_diagnostics(config: &Config) -> CapabilityProviderDiagnostics {
    match capability_provider_registry(config) {
        Ok(registry) => {
            let providers = registry.list();
            log::debug!(
                "[tool_registry] capability_provider_diagnostics completed providers={}",
                providers.len()
            );
            CapabilityProviderDiagnostics {
                total_providers: providers.len(),
                enabled_providers: providers.iter().filter(|provider| provider.enabled).count(),
                trusted_providers: providers
                    .iter()
                    .filter(|provider| {
                        provider.trust_state == CapabilityProviderTrustState::Trusted
                    })
                    .count(),
                trusted_enabled_providers: providers
                    .iter()
                    .filter(|provider| {
                        provider.enabled
                            && provider.trust_state == CapabilityProviderTrustState::Trusted
                    })
                    .count(),
                registry_errors: Vec::new(),
            }
        }
        Err(err) => {
            log::debug!(
                "[tool_registry] capability_provider_diagnostics registry_error configured_providers={} error={}",
                config.capability_providers.len(),
                err
            );
            CapabilityProviderDiagnostics {
                total_providers: config.capability_providers.len(),
                registry_errors: vec![err.to_string()],
                ..CapabilityProviderDiagnostics::default()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityProviderRegistryError {
    InvalidId { raw: String },
    DuplicateId { id: String },
}

impl fmt::Display for CapabilityProviderRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapabilityProviderRegistryError::InvalidId { raw } => {
                write!(formatter, "invalid provider id: {raw:?}")
            }
            CapabilityProviderRegistryError::DuplicateId { id } => {
                write!(formatter, "duplicate provider id after normalization: {id}")
            }
        }
    }
}

impl std::error::Error for CapabilityProviderRegistryError {}

fn push_separator(normalized: &mut String, previous_separator: &mut bool, separator: char) {
    if normalized.is_empty() || *previous_separator {
        return;
    }
    normalized.push(match separator {
        '_' => '_',
        '.' => '.',
        _ => '-',
    });
    *previous_separator = true;
}

fn clean_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn clean_string_opt(value: Option<&str>) -> Option<String> {
    value.and_then(clean_string)
}

#[cfg(test)]
mod tests {
    use crate::openhuman::config::schema::{
        CapabilityProviderConfig, CapabilityProviderTrustState, Config,
    };

    use super::CapabilityProviderRegistry;

    fn config_with(providers: Vec<CapabilityProviderConfig>) -> Config {
        Config {
            capability_providers: providers,
            ..Config::default()
        }
    }

    fn provider(
        id: &str,
        trust_state: CapabilityProviderTrustState,
        enabled: bool,
    ) -> CapabilityProviderConfig {
        CapabilityProviderConfig {
            id: id.to_string(),
            display_name: format!("{id} Provider"),
            source_uri: Some(format!("https://example.com/{id}")),
            source_digest: Some("sha256:abc123".to_string()),
            trust_state,
            enabled,
        }
    }

    #[test]
    fn default_config_has_no_capability_providers() {
        let registry =
            CapabilityProviderRegistry::from_config(&Config::default()).expect("empty registry");

        assert!(registry.list().is_empty());
        assert!(registry.get("anything").is_none());
    }

    #[test]
    fn valid_provider_registration_normalizes_metadata() {
        let config = config_with(vec![CapabilityProviderConfig {
            id: "Acme Tools".to_string(),
            display_name: "Acme Tools".to_string(),
            source_uri: Some("https://example.com/openhuman/acme-tools".to_string()),
            source_digest: Some("sha256:abc123".to_string()),
            trust_state: CapabilityProviderTrustState::Trusted,
            enabled: true,
        }]);

        let registry = CapabilityProviderRegistry::from_config(&config).expect("valid provider");
        let providers = registry.list();

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "acme-tools");
        assert_eq!(providers[0].display_name, "Acme Tools");
        assert_eq!(
            providers[0].source_uri.as_deref(),
            Some("https://example.com/openhuman/acme-tools")
        );
        assert_eq!(providers[0].source_digest.as_deref(), Some("sha256:abc123"));
        assert_eq!(
            providers[0].trust_state,
            CapabilityProviderTrustState::Trusted
        );
        assert!(providers[0].enabled);
        assert!(registry.is_trusted_enabled("ACME Tools"));
    }

    #[test]
    fn disabled_or_untrusted_providers_are_not_trusted_enabled() {
        let config = config_with(vec![
            provider(
                "trusted-disabled",
                CapabilityProviderTrustState::Trusted,
                false,
            ),
            provider(
                "untrusted-enabled",
                CapabilityProviderTrustState::Untrusted,
                true,
            ),
        ]);

        let registry =
            CapabilityProviderRegistry::from_config(&config).expect("providers should parse");

        assert_eq!(registry.list().len(), 2);
        assert!(!registry.is_trusted_enabled("trusted-disabled"));
        assert!(!registry.is_trusted_enabled("untrusted-enabled"));
    }

    #[test]
    fn duplicate_provider_ids_are_rejected_after_normalization() {
        let config = config_with(vec![
            provider("Acme Tools", CapabilityProviderTrustState::Trusted, true),
            provider("acme-tools", CapabilityProviderTrustState::Trusted, true),
        ]);

        let err =
            CapabilityProviderRegistry::from_config(&config).expect_err("duplicate should fail");

        assert!(err.to_string().contains("duplicate"));
        assert!(err.to_string().contains("acme-tools"));
    }

    #[test]
    fn invalid_provider_ids_are_rejected() {
        let config = config_with(vec![provider(
            "!!!",
            CapabilityProviderTrustState::Trusted,
            true,
        )]);

        let err =
            CapabilityProviderRegistry::from_config(&config).expect_err("invalid id should fail");

        assert!(err.to_string().contains("invalid provider id"));
    }
}
