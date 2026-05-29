//! Tests for migration 5 → 6 (`reconcile_orphaned_providers`).

use super::*;
use crate::openhuman::config::schema::cloud_providers::CloudProviderCreds;
use crate::openhuman::config::Config;

/// Minimal cloud-provider entry for tests — only `id` and `slug` matter here.
fn provider(id: &str, slug: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        ..Default::default()
    }
}

#[test]
fn scrubs_orphaned_chat_provider_to_managed() {
    // The reported bug: chat points at OpenAI, but OpenAI was removed.
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4o".to_string());
    // cloud_providers has no `openai` entry.

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 1);
    assert_eq!(
        config.chat_provider, None,
        "orphaned chat_provider -> managed"
    );
}

#[test]
fn keeps_provider_when_slug_still_present() {
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.cloud_providers = vec![provider("p_openai_1", "openai")];

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 0);
    assert_eq!(config.chat_provider.as_deref(), Some("openai:gpt-4o"));
}

#[test]
fn scrubs_when_stored_slug_has_whitespace_factory_would_reject() {
    // The factory compares `e.slug == slug` exactly (no trim). A provider stored
    // as " openai" does NOT resolve a "openai:gpt-4o" route, so the migration
    // must scrub it rather than leaving a reference the factory rejects.
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.cloud_providers = vec![provider("p1", " openai")];

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 1);
    assert_eq!(config.chat_provider, None);
}

#[test]
fn preserves_temperature_suffix_on_valid_slug() {
    let mut config = Config::default();
    config.reasoning_provider = Some("openai:gpt-4o@0.3".to_string());
    config.cloud_providers = vec![provider("p_openai_1", "openai")];

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 0);
    assert_eq!(
        config.reasoning_provider.as_deref(),
        Some("openai:gpt-4o@0.3")
    );
}

#[test]
fn normalizes_openhuman_colon_to_none() {
    let mut config = Config::default();
    config.memory_provider = Some("openhuman:".to_string());
    config.embeddings_provider = Some("openhuman:reasoning-v1".to_string());

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 2);
    assert_eq!(config.memory_provider, None);
    assert_eq!(config.embeddings_provider, None);
}

#[test]
fn leaves_sentinels_and_local_providers_untouched() {
    let mut config = Config::default();
    config.chat_provider = Some("openhuman".to_string());
    config.reasoning_provider = Some("cloud".to_string());
    config.agentic_provider = Some(String::new());
    config.coding_provider = None;
    config.memory_provider = Some("ollama:llama3".to_string());
    config.embeddings_provider = Some("lmstudio:nomic-embed".to_string());

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 0);
    assert_eq!(config.chat_provider.as_deref(), Some("openhuman"));
    assert_eq!(config.reasoning_provider.as_deref(), Some("cloud"));
    assert_eq!(config.agentic_provider.as_deref(), Some(""));
    assert_eq!(config.coding_provider, None);
    assert_eq!(config.memory_provider.as_deref(), Some("ollama:llama3"));
    assert_eq!(
        config.embeddings_provider.as_deref(),
        Some("lmstudio:nomic-embed")
    );
}

#[test]
fn scrubs_bare_unresolvable_string() {
    // A bare non-sentinel like "openai" (no colon) is rejected by the factory.
    let mut config = Config::default();
    config.chat_provider = Some("openai".to_string());

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 1);
    assert_eq!(config.chat_provider, None);
}

#[test]
fn scrubs_every_orphaned_workload() {
    let mut config = Config::default();
    for field in [
        &mut config.chat_provider,
        &mut config.reasoning_provider,
        &mut config.agentic_provider,
        &mut config.coding_provider,
        &mut config.memory_provider,
        &mut config.embeddings_provider,
        &mut config.heartbeat_provider,
        &mut config.learning_provider,
        &mut config.subconscious_provider,
    ] {
        *field = Some("ghost:model-x".to_string());
    }

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 9);
    assert_eq!(config.chat_provider, None);
    assert_eq!(config.subconscious_provider, None);
}

#[test]
fn clears_dangling_primary_cloud() {
    let mut config = Config::default();
    config.primary_cloud = Some("p_openhuman_missing".to_string());
    config.cloud_providers = vec![provider("p_openai_1", "openai")];

    let stats = run(&mut config).expect("migration should succeed");

    assert!(stats.primary_cloud_cleared);
    assert_eq!(config.primary_cloud, None);
}

#[test]
fn keeps_valid_primary_cloud() {
    let mut config = Config::default();
    config.primary_cloud = Some("p_openai_1".to_string());
    config.cloud_providers = vec![provider("p_openai_1", "openai")];

    let stats = run(&mut config).expect("migration should succeed");

    assert!(!stats.primary_cloud_cleared);
    assert_eq!(config.primary_cloud.as_deref(), Some("p_openai_1"));
}

#[test]
fn clean_config_is_a_no_op() {
    let mut config = Config::default();

    let stats = run(&mut config).expect("migration should succeed");

    assert_eq!(stats.workload_fields_scrubbed, 0);
    assert!(!stats.primary_cloud_cleared);
}

#[test]
fn redact_provider_for_log_masks_model_keeps_slug() {
    // Slug retained as a diagnostic; model segment masked.
    assert_eq!(
        redact_provider_for_log("openai:gpt-4o"),
        "openai:<redacted>"
    );
    assert_eq!(
        redact_provider_for_log("  ghost:secret-model@0.7  "),
        "ghost:<redacted>"
    );
    // Bare string has no slug to keep — fully masked.
    assert_eq!(redact_provider_for_log("openai"), "<redacted>");
}

#[test]
fn idempotent_second_run_scrubs_nothing() {
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.primary_cloud = Some("p_missing".to_string());

    let first = run(&mut config).expect("first run should succeed");
    assert_eq!(first.workload_fields_scrubbed, 1);
    assert!(first.primary_cloud_cleared);

    let second = run(&mut config).expect("second run should succeed");
    assert_eq!(second.workload_fields_scrubbed, 0);
    assert!(!second.primary_cloud_cleared);
}
