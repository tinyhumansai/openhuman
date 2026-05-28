use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::store;

/// Default page size for `ai_list_artifacts`.
const DEFAULT_LIMIT: usize = 50;
/// Maximum page size cap for `ai_list_artifacts`.
const MAX_LIMIT: usize = 200;

/// List artifacts in the workspace with pagination.
///
/// Returns `{ "artifacts": [...], "total": N, "offset": M, "limit": L }`.
pub async fn ai_list_artifacts(
    config: &Config,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<RpcOutcome<Value>, String> {
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    log::debug!(
        "[artifacts] ai_list_artifacts: workspace={:?} offset={offset} limit={limit}",
        config.workspace_dir
    );

    let (artifacts, total) = store::list_artifacts(&config.workspace_dir, offset, limit).await?;

    log::debug!(
        "[artifacts] ai_list_artifacts: returning {} of {total} total",
        artifacts.len()
    );

    let value = json!({
        "artifacts": artifacts,
        "total": total,
        "offset": offset,
        "limit": limit,
    });
    Ok(RpcOutcome::new(value, vec![]))
}

/// Retrieve a single artifact by ID.
///
/// Returns the serialized `ArtifactMeta` plus an `absolute_path` field
/// pointing to the full on-disk location of the artifact files.
pub async fn ai_get_artifact(
    config: &Config,
    artifact_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    log::debug!(
        "[artifacts] ai_get_artifact: id={artifact_id} workspace={:?}",
        config.workspace_dir
    );

    if artifact_id.is_empty() {
        return Err("[artifacts] artifact_id must not be empty".to_string());
    }

    let meta = store::get_artifact(&config.workspace_dir, artifact_id).await?;

    // Compute absolute path for the caller's convenience.
    // Guard against a corrupt or adversarial meta.path that escapes the artifacts root.
    let artifacts_root = config.workspace_dir.join("artifacts");
    let resolved = artifacts_root.join(&meta.path);
    if !resolved.starts_with(&artifacts_root) {
        return Err(format!(
            "[artifacts] meta.path {:?} escapes artifacts root for id={artifact_id}",
            meta.path
        ));
    }
    let absolute_path = resolved.to_string_lossy().into_owned();

    let mut value =
        serde_json::to_value(&meta).map_err(|e| format!("[artifacts] serialization error: {e}"))?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "absolute_path".to_string(),
            Value::String(absolute_path.clone()),
        );
    }

    log::debug!(
        "[artifacts] ai_get_artifact: found id={artifact_id} absolute_path={absolute_path}"
    );
    Ok(RpcOutcome::new(value, vec![]))
}

/// Delete an artifact and all associated files.
///
/// Returns `{ "artifact_id": "...", "deleted": true }`.
pub async fn ai_delete_artifact(
    config: &Config,
    artifact_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    log::debug!(
        "[artifacts] ai_delete_artifact: id={artifact_id} workspace={:?}",
        config.workspace_dir
    );

    if artifact_id.is_empty() {
        return Err("[artifacts] artifact_id must not be empty".to_string());
    }

    store::delete_artifact(&config.workspace_dir, artifact_id).await?;

    log::debug!("[artifacts] ai_delete_artifact: deleted id={artifact_id}");
    let value = json!({
        "artifact_id": artifact_id,
        "deleted": true,
    });
    Ok(RpcOutcome::new(value, vec![]))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
