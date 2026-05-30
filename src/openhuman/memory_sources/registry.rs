//! CRUD operations for memory sources.
//!
//! Reads and writes `Config.memory_sources` via the config load/save
//! cycle. Each mutation reloads the live config, applies the change,
//! and persists atomically.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};

pub async fn list_sources() -> Result<Vec<MemorySourceEntry>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    Ok(config.memory_sources.clone())
}

pub async fn list_enabled_by_kind(kind: SourceKind) -> Result<Vec<MemorySourceEntry>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    Ok(config
        .memory_sources
        .iter()
        .filter(|s| s.kind == kind && s.enabled)
        .cloned()
        .collect())
}

pub async fn get_source(id: &str) -> Result<Option<MemorySourceEntry>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    Ok(config.memory_sources.iter().find(|s| s.id == id).cloned())
}

pub async fn add_source(entry: MemorySourceEntry) -> Result<MemorySourceEntry, String> {
    entry.validate()?;
    let mut config = config_rpc::load_config_with_timeout().await?;

    if config.memory_sources.iter().any(|s| s.id == entry.id) {
        return Err(format!("source with id '{}' already exists", entry.id));
    }

    tracing::info!(
        id = %entry.id,
        kind = %entry.kind.as_str(),
        label = %entry.label,
        "[memory_sources] adding source"
    );

    config.memory_sources.push(entry.clone());
    config
        .save()
        .await
        .map_err(|e| format!("failed to save config: {e:#}"))?;

    Ok(entry)
}

pub async fn update_source(
    id: &str,
    patch: MemorySourcePatch,
) -> Result<MemorySourceEntry, String> {
    let mut config = config_rpc::load_config_with_timeout().await?;

    let entry = config
        .memory_sources
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("source '{id}' not found"))?;

    if let Some(label) = patch.label {
        entry.label = label;
    }
    if let Some(enabled) = patch.enabled {
        entry.enabled = enabled;
    }
    if let Some(toolkit) = patch.toolkit {
        entry.toolkit = Some(toolkit);
    }
    if let Some(connection_id) = patch.connection_id {
        entry.connection_id = Some(connection_id);
    }
    if let Some(path) = patch.path {
        entry.path = Some(path);
    }
    if let Some(glob) = patch.glob {
        entry.glob = Some(glob);
    }
    if let Some(url) = patch.url {
        entry.url = Some(url);
    }
    if let Some(branch) = patch.branch {
        entry.branch = Some(branch);
    }
    if let Some(paths) = patch.paths {
        entry.paths = paths;
    }
    if let Some(query) = patch.query {
        entry.query = Some(query);
    }
    if let Some(since_days) = patch.since_days {
        entry.since_days = Some(since_days);
    }
    if let Some(max_items) = patch.max_items {
        entry.max_items = Some(max_items);
    }
    if let Some(selector) = patch.selector {
        entry.selector = Some(selector);
    }

    entry.validate()?;
    let updated = entry.clone();

    tracing::info!(
        id = %id,
        kind = %updated.kind.as_str(),
        "[memory_sources] updated source"
    );

    config
        .save()
        .await
        .map_err(|e| format!("failed to save config: {e:#}"))?;

    Ok(updated)
}

pub async fn remove_source(id: &str) -> Result<bool, String> {
    let mut config = config_rpc::load_config_with_timeout().await?;
    let before = config.memory_sources.len();
    config.memory_sources.retain(|s| s.id != id);
    let removed = config.memory_sources.len() < before;

    if removed {
        tracing::info!(id = %id, "[memory_sources] removed source");
        config
            .save()
            .await
            .map_err(|e| format!("failed to save config: {e:#}"))?;
    }

    Ok(removed)
}

/// Upsert a composio source — used by the auto-registration path.
/// If a source with the same `connection_id` already exists, updates
/// the label; otherwise inserts a new entry.
pub async fn upsert_composio_source(
    toolkit: &str,
    connection_id: &str,
    label: &str,
) -> Result<MemorySourceEntry, String> {
    let mut config = config_rpc::load_config_with_timeout().await?;

    if let Some(existing) = config.memory_sources.iter_mut().find(|s| {
        s.kind == SourceKind::Composio && s.connection_id.as_deref() == Some(connection_id)
    }) {
        existing.label = label.to_string();
        let updated = existing.clone();
        config
            .save()
            .await
            .map_err(|e| format!("failed to save config: {e:#}"))?;
        tracing::debug!(
            connection_id = %connection_id,
            toolkit = %toolkit,
            "[memory_sources] upserted composio source (update)"
        );
        return Ok(updated);
    }

    let entry = MemorySourceEntry {
        id: format!("src_{}", uuid::Uuid::new_v4().as_simple()),
        kind: SourceKind::Composio,
        label: label.to_string(),
        enabled: true,
        toolkit: Some(toolkit.to_string()),
        connection_id: Some(connection_id.to_string()),
        path: None,
        glob: None,
        url: None,
        branch: None,
        paths: Vec::new(),
        query: None,
        since_days: None,
        max_items: None,
        selector: None,
    };
    config.memory_sources.push(entry.clone());
    config
        .save()
        .await
        .map_err(|e| format!("failed to save config: {e:#}"))?;

    tracing::info!(
        connection_id = %connection_id,
        toolkit = %toolkit,
        "[memory_sources] upserted composio source (insert)"
    );

    Ok(entry)
}

/// Partial update payload for a source entry.
#[derive(Debug, Default, serde::Deserialize)]
pub struct MemorySourcePatch {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub toolkit: Option<String>,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub glob: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub since_days: Option<u32>,
    #[serde(default)]
    pub max_items: Option<u32>,
    #[serde(default)]
    pub selector: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_source_patch_deserializes_partial() {
        let json = serde_json::json!({ "label": "New label", "enabled": false });
        let patch: MemorySourcePatch = serde_json::from_value(json).unwrap();
        assert_eq!(patch.label.as_deref(), Some("New label"));
        assert_eq!(patch.enabled, Some(false));
        assert!(patch.toolkit.is_none());
    }
}
