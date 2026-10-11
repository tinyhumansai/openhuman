use super::*;
use crate::{ModelDefaults, Provider, RuntimeBuilder, RuntimeModule, SkillsPolicy, WeightClass};
#[test]
fn capability_json_is_versioned_and_never_contains_credentials() {
    let info = RuntimeBuilder::new()
        .provider(
            Provider::openai_compatible(
                "https://user:password@host.example/v1?key=secret-query",
                "secret-api-key",
            )
            .model("sample-model"),
        )
        .api_key("secret-backend-key")
        .token(crate::TokenSource::Fixed(std::sync::Arc::new(
            "secret-rpc-token".into(),
        )))
        .describe();
    let json = serde_json::to_string(&info).unwrap();
    for secret in [
        "password",
        "secret-query",
        "secret-api-key",
        "secret-backend-key",
        "secret-rpc-token",
        "host.example",
    ] {
        assert!(!json.contains(secret), "{secret} escaped introspection");
    }
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["schema_version"], 1);
    let roundtrip: RuntimeInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(
        roundtrip.defaults.provider_model.as_deref(),
        Some("sample-model")
    );
    assert_eq!(
        info.compiled_features,
        openhuman_core::core::runtime::compiled_features()
    );
}
#[test]
fn describe_includes_effective_host_limits_and_agent_defaults() {
    let info = RuntimeBuilder::new()
        .workspace(crate::Workspace::Inherit)
        .max_agents(3)
        .model_defaults(ModelDefaults {
            temperature: Some(0.4),
            max_tokens: Some(100),
            top_p: Some(0.8),
            max_iterations: Some(5),
        })
        .skills(SkillsPolicy {
            root: Some("skills-root".into()),
            include_user_skills: true,
        })
        .describe();
    assert_eq!(info.host_kind, "Cli");
    assert_eq!(info.max_agents, 3);
    assert_eq!(info.defaults.model.temperature, Some(0.4));
    assert_eq!(info.defaults.model.max_tokens, Some(100));
    assert!(info.defaults.include_user_skills);
    assert!(info.defaults.has_skills_root);
}
#[test]
fn typed_configuration_groups_apply_on_top_of_supplied_config() {
    use openhuman_core::config::schema::{
        AutonomyConfig, CronConfig, PrivacyConfig, PrivacyMode, SecretsConfig,
    };
    let info = RuntimeBuilder::new()
        .privacy(PrivacyConfig {
            mode: PrivacyMode::Sensitive,
        })
        .secrets(SecretsConfig { encrypt: false })
        .cron(CronConfig {
            enabled: false,
            ..Default::default()
        })
        .learning(LearningSettings {
            build_beliefs_every: 30,
            learnings_limit: 2,
        })
        .autonomy(AutonomyConfig {
            max_actions_per_hour: 7,
            ..Default::default()
        })
        .describe();
    assert_eq!(info.configuration.privacy, "Sensitive");
    assert!(!info.configuration.secrets_encrypted);
    assert!(!info.configuration.cron_enabled);
    assert_eq!(info.configuration.learning.build_beliefs_every, 30);
    assert_eq!(info.configuration.learning.learnings_limit, 2);
}
#[test]
fn presets_report_consistent_module_weight_and_optional_gates() {
    let lean = RuntimeBuilder::lean().describe();
    assert_eq!(lean.weight, WeightClass::Lean);
    assert_eq!(
        lean.modules,
        vec![
            RuntimeModule::Agent,
            RuntimeModule::Memory,
            RuntimeModule::Inference
        ]
    );
    let standard = RuntimeBuilder::standard().describe();
    assert_eq!(standard.weight, WeightClass::Standard);
    assert!(!standard.modules.contains(&RuntimeModule::Voice));
    assert!(!standard.modules.contains(&RuntimeModule::Automation));
    let full = RuntimeBuilder::full().describe();
    assert_eq!(full.weight, WeightClass::Full);
    for module in [
        RuntimeModule::Integrations,
        RuntimeModule::Automation,
        RuntimeModule::Runtimes,
    ] {
        assert!(full.modules.contains(&module));
    }
    assert!(full.services.contains(&"cron".into()));
    assert!(lean.services.is_empty());
    assert!(!lean.domains.contains(&"modules".into()));
    assert!(!lean.domains.contains(&"flows".into()));
    for info in [
        RuntimeBuilder::standard().describe(),
        RuntimeBuilder::full().describe(),
    ] {
        for module in info.modules {
            if let Some(feature) = module.required_feature() {
                assert!(info.compiled_features[feature]);
            }
        }
    }
}
#[test]
fn unavailable_module_error_names_its_cargo_feature() {
    let features = openhuman_core::core::runtime::compiled_features();
    for module in [
        RuntimeModule::Voice,
        RuntimeModule::Web3,
        RuntimeModule::Documents,
        RuntimeModule::Hosting,
        RuntimeModule::Skills,
        RuntimeModule::Mcp,
    ] {
        if let Some(feature) = module.required_feature() {
            let builder = RuntimeBuilder::new().modules([module]);
            if !features[feature] {
                assert!(
                    matches!(builder.validate(), Err(crate::RuntimeError::MissingFeature{feature:actual,..}) if actual==feature)
                );
            } else {
                assert!(builder.validate().is_ok());
            }
        }
    }
}
#[test]
fn discovered_config_still_rejects_typed_boot_edits() {
    let builder = RuntimeBuilder::desktop().cron(Default::default());
    assert!(builder.validate().is_err());
    assert!(RuntimeBuilder::desktop()
        .model_defaults(ModelDefaults {
            max_tokens: Some(300),
            ..Default::default()
        })
        .validate()
        .is_ok());
}

#[test]
fn storage_config_and_explicit_sources_are_credential_safe() {
    let mut config = openhuman_core::config::Config::default();
    config.storage.url =
        Some("mongodb://app:secret-password@private-host/database?authSource=secret-query".into());
    let builder = RuntimeBuilder::new().config(config);
    let info = builder.describe();
    assert_eq!(info.storage.driver.as_deref(), Some("mongodb"));
    assert_eq!(info.storage.source, "configuration");
    assert!(info.storage.configured);
    let json = serde_json::to_string(&info).unwrap();
    for secret in ["secret-password", "private-host", "secret-query"] {
        assert!(!json.contains(secret));
    }
    if !info.compiled_features["storage-mongodb"] {
        assert!(matches!(
            builder.validate(),
            Err(crate::RuntimeError::MissingStorageFeature {
                feature: "storage-mongodb",
                ..
            })
        ));
    }
    let info = RuntimeBuilder::new().storage("memory").describe();
    assert_eq!(info.storage.driver.as_deref(), Some("memory"));
    assert_eq!(info.storage.source, "explicit");
}
