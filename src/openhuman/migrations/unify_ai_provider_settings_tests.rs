//! Tests for the 1 → 2 AI-provider unification migration.

use super::*;
use crate::openhuman::config::schema::cloud_providers::CloudProviderType;
use crate::openhuman::config::schema::{LocalAiConfig, LocalAiUsage};
use crate::openhuman::config::Config;

fn make_legacy_config_local_on() -> Config {
    let mut c = Config::default();
    c.local_ai = LocalAiConfig {
        runtime_enabled: true,
        chat_model_id: "llama3.1:8b".into(),
        embedding_model_id: "bge-m3".into(),
        usage: LocalAiUsage {
            embeddings: true,
            heartbeat: true,
            learning_reflection: false,
            subconscious: true,
        },
        ..LocalAiConfig::default()
    };
    c.memory_tree.llm_backend = crate::openhuman::config::schema::LlmBackend::Local;
    c
}

#[test]
fn empty_config_seeds_openhuman_entry() {
    let mut c = Config::default();
    let stats = run(&mut c).expect("migration must succeed");

    assert_eq!(stats.cloud_providers_seeded, 1);
    assert_eq!(c.cloud_providers.len(), 1);
    assert_eq!(c.cloud_providers[0].r#type, CloudProviderType::Openhuman);
    assert!(c.cloud_providers[0].id.starts_with("p_openhuman_"));
}

#[test]
fn primary_cloud_defaults_to_openhuman_id() {
    let mut c = Config::default();
    let stats = run(&mut c).expect("migration must succeed");

    assert!(stats.primary_cloud_set);
    assert_eq!(c.primary_cloud, Some(c.cloud_providers[0].id.clone()));
}

#[test]
fn legacy_inference_url_becomes_custom_entry() {
    let mut c = Config::default();
    c.inference_url = Some("https://api.example.com/v1".into());
    c.model_routes
        .push(crate::openhuman::config::schema::ModelRouteConfig {
            hint: "reasoning".into(),
            model: "gpt-4o".into(),
        });

    let stats = run(&mut c).expect("migration must succeed");

    assert_eq!(stats.cloud_providers_seeded, 2);
    let custom = c
        .cloud_providers
        .iter()
        .find(|e| e.r#type == CloudProviderType::Custom)
        .expect("custom entry must be seeded");
    assert_eq!(custom.endpoint, "https://api.example.com/v1");
    assert_eq!(custom.default_model, "gpt-4o");
}

#[test]
fn openhuman_inference_url_does_not_seed_custom() {
    let mut c = Config::default();
    c.inference_url = Some("https://api.openhuman.ai/v1".into());
    let _ = run(&mut c).expect("migration must succeed");
    // Only the openhuman entry should be seeded — no Custom entry.
    assert_eq!(c.cloud_providers.len(), 1);
    assert_eq!(c.cloud_providers[0].r#type, CloudProviderType::Openhuman);
}

#[test]
fn embeddings_provider_derived_from_legacy_usage() {
    let mut c = make_legacy_config_local_on();
    let stats = run(&mut c).expect("migration must succeed");
    assert!(stats.workload_fields_filled >= 5);
    assert_eq!(c.embeddings_provider.as_deref(), Some("ollama:bge-m3"));
}

#[test]
fn heartbeat_provider_derived_from_legacy_usage() {
    let mut c = make_legacy_config_local_on();
    let _ = run(&mut c).unwrap();
    assert_eq!(c.heartbeat_provider.as_deref(), Some("ollama:llama3.1:8b"));
}

#[test]
fn subconscious_provider_derived_from_legacy_usage() {
    let mut c = make_legacy_config_local_on();
    let _ = run(&mut c).unwrap();
    assert_eq!(
        c.subconscious_provider.as_deref(),
        Some("ollama:llama3.1:8b")
    );
}

#[test]
fn learning_provider_defaults_to_cloud_when_flag_off() {
    // learning_reflection is `false` in our fixture.
    let mut c = make_legacy_config_local_on();
    let _ = run(&mut c).unwrap();
    assert_eq!(c.learning_provider.as_deref(), Some("cloud"));
}

#[test]
fn memory_provider_local_when_llm_backend_local() {
    let mut c = make_legacy_config_local_on();
    let _ = run(&mut c).unwrap();
    assert_eq!(c.memory_provider.as_deref(), Some("ollama:llama3.1:8b"));
}

#[test]
fn memory_provider_cloud_when_llm_backend_cloud() {
    let mut c = Config::default();
    // default backend is Cloud
    let _ = run(&mut c).unwrap();
    assert_eq!(c.memory_provider.as_deref(), Some("cloud"));
}

#[test]
fn chat_workload_providers_left_unset() {
    let mut c = make_legacy_config_local_on();
    let _ = run(&mut c).unwrap();
    // Reasoning/agentic/coding have no legacy equivalent — they stay None
    // and the factory defaults them to "cloud" at runtime.
    assert_eq!(c.reasoning_provider, None);
    assert_eq!(c.agentic_provider, None);
    assert_eq!(c.coding_provider, None);
}

#[test]
fn idempotent_second_run_is_noop() {
    let mut c = make_legacy_config_local_on();
    let first = run(&mut c).expect("first run must succeed");
    let providers_after_first = c.cloud_providers.len();
    let primary_after_first = c.primary_cloud.clone();
    let heartbeat_after_first = c.heartbeat_provider.clone();

    let second = run(&mut c).expect("second run must succeed");

    // Second run must not seed extras nor flip any field.
    assert_eq!(second.cloud_providers_seeded, 0);
    assert!(!second.primary_cloud_set);
    assert_eq!(second.workload_fields_filled, 0);
    assert_eq!(c.cloud_providers.len(), providers_after_first);
    assert_eq!(c.primary_cloud, primary_after_first);
    assert_eq!(c.heartbeat_provider, heartbeat_after_first);

    // Sanity: stats from the first run say we did do work.
    assert!(first.cloud_providers_seeded >= 1);
    assert!(first.workload_fields_filled >= 1);
}

#[test]
fn runtime_disabled_falls_back_to_cloud_even_with_usage_flags() {
    let mut c = make_legacy_config_local_on();
    c.local_ai.runtime_enabled = false;
    let _ = run(&mut c).unwrap();
    // With runtime off, every workload routes to cloud regardless of usage.*
    assert_eq!(c.heartbeat_provider.as_deref(), Some("cloud"));
    assert_eq!(c.subconscious_provider.as_deref(), Some("cloud"));
    assert_eq!(c.embeddings_provider.as_deref(), Some("cloud"));
    assert_eq!(c.memory_provider.as_deref(), Some("cloud"));
}
