//! Minimal native-cloud adapter for multi-key BYOK requests.
//! The ordinary single-key model stays unchanged. Inspection constructs only
//! the first credential's model; it must not perform runtime failover.
use std::sync::Arc;

use tinyinference_llm::model::ChatModel;

use super::credential_fleet::CredentialFleetModel;
use super::key_pool::RotationStrategy;

/// Health and cooldown belong to a destination, not merely a provider alias:
/// changing the endpoint must not carry its previous 429 or invalid-key state.
pub(super) fn rotation_pool_scope(
    workspace: &std::path::Path,
    slug: &str,
    model: &str,
    endpoint: &str,
) -> String {
    format!(
        "{}:{}:{}:{:016x}",
        workspace.display(),
        slug,
        model,
        super::key_pool::scope_fingerprint(endpoint),
    )
}

pub(super) fn build_cloud_model_with_keys(
    scope: String,
    keys: Vec<String>,
    enable_rotation: bool,
    mut build: impl FnMut(&str) -> anyhow::Result<Arc<dyn ChatModel<()>>>,
) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    let keys = if keys.is_empty() {
        vec![String::new()]
    } else {
        keys
    };
    if !enable_rotation || keys.len() == 1 {
        return build(&keys[0]);
    }

    let models = keys
        .iter()
        .map(|key| build(key))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Arc::new(CredentialFleetModel::new(
        scope,
        keys,
        models,
        None,
        RotationStrategy::RoundRobin,
    )))
}

#[cfg(test)]
#[path = "cloud_fleet_tests.rs"]
mod tests;
