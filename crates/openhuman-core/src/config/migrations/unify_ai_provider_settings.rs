//! Migration 1 → 2: derive per-workload provider strings from legacy settings.

use crate::config::schema::cloud_providers::{seed_cloud_providers, MigrationStats};
use crate::config::Config;

/// Run the AI-provider unification migration on the given `Config`.
///
/// Provider entries are seeded by the config schema helper, which is also
/// called when initializing a new workspace. This migration additionally
/// derives the workload routes that replace the legacy local-AI flags.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let mut stats = seed_cloud_providers(config);
    derive_workload_providers(config, &mut stats);

    log::info!(
        "[migrations][unify-ai] done seeded_providers={} primary_set={} workload_fields_filled={}",
        stats.cloud_providers_seeded,
        stats.primary_cloud_set,
        stats.workload_fields_filled,
    );
    Ok(stats)
}

/// Derive each per-workload `*_provider` field from the legacy flags.
///
/// All fields are gated on `is_none()` — a partially-migrated config skips
/// fields that were already set by a previous run or a hand-edit.
fn derive_workload_providers(config: &mut Config, stats: &mut MigrationStats) {
    let runtime_on = config.local_ai.runtime_enabled;
    let embed_model = config.local_ai.embedding_model_id.clone();

    let set_field = |field: &mut Option<String>, value: String, stats: &mut MigrationStats| {
        if field.is_none() {
            *field = Some(value);
            stats.workload_fields_filled += 1;
        }
    };

    set_field(&mut config.memory_provider, "cloud".to_string(), stats);

    let embeddings_value =
        if config.local_ai.usage.embeddings && runtime_on && !embed_model.is_empty() {
            format!("ollama:{embed_model}")
        } else {
            "cloud".to_string()
        };
    set_field(&mut config.embeddings_provider, embeddings_value, stats);
}

#[cfg(test)]
#[path = "unify_ai_provider_settings_tests.rs"]
mod tests;
