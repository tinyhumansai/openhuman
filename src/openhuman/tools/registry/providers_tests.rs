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

    let err = CapabilityProviderRegistry::from_config(&config).expect_err("duplicate should fail");

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

    let err = CapabilityProviderRegistry::from_config(&config).expect_err("invalid id should fail");

    assert!(err.to_string().contains("invalid provider id"));
}
