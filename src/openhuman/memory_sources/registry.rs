//! CRUD operations for memory sources.
//!
//! Reads and writes `Config.memory_sources` via the config load/save
//! cycle. Each mutation reloads the live config, applies the change,
//! and persists atomically.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use std::sync::OnceLock;

static MEMORY_SOURCES_WRITE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub(crate) async fn memory_sources_write_guard() -> tokio::sync::MutexGuard<'static, ()> {
    MEMORY_SOURCES_WRITE_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

/// Conservative default sync caps for a Composio toolkit, keyed by toolkit slug.
///
/// Single source of truth for the cheap out-of-the-box sync volume. Applied to a
/// source entry when it is first registered (`upsert_composio_source`, insert-only)
/// and by the one-time caps migration (`reconcile::apply_composio_source_caps_migration`)
/// for cap-less entries. Never overwrites a user-customised cap.
///
/// Returns `(max_items, sync_depth_days)`.
pub fn memory_sync_defaults_for_toolkit(toolkit: &str) -> (Option<u32>, Option<u32>) {
    match toolkit {
        "gmail" => (Some(100), Some(30)),
        "slack" => (Some(50), Some(14)),
        "notion" => (Some(30), Some(30)),
        "linear" => (Some(50), Some(30)),
        "clickup" => (Some(50), Some(30)),
        "github" => (Some(50), Some(30)),
        // Generic fallback for any toolkit not listed above.
        _ => (Some(30), Some(14)),
    }
}

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
    let _guard = memory_sources_write_guard().await;
    let mut config = config_rpc::load_config_with_timeout().await?;

    if config.memory_sources.iter().any(|s| s.id == entry.id) {
        return Err(format!("source with id '{}' already exists", entry.id));
    }

    tracing::info!(
        id = %entry.id,
        kind = %entry.kind.as_str(),
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
    let _guard = memory_sources_write_guard().await;
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
    if let Some(v) = patch.max_tokens_per_sync {
        entry.max_tokens_per_sync = Some(v);
    }
    if let Some(v) = patch.max_cost_per_sync_usd {
        entry.max_cost_per_sync_usd = Some(v);
    }
    if let Some(v) = patch.sync_depth_days {
        entry.sync_depth_days = Some(v);
    }
    if let Some(v) = patch.max_commits {
        entry.max_commits = Some(v);
    }
    if let Some(v) = patch.max_issues {
        entry.max_issues = Some(v);
    }
    if let Some(v) = patch.max_prs {
        entry.max_prs = Some(v);
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
    let _guard = memory_sources_write_guard().await;
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

/// Remove every composio source bound to `connection_id` — the disconnect path.
///
/// Mirrors [`upsert_composio_source`], which keys composio sources on
/// `connection_id`. [`remove_source`] keys on the `src_*` id, which the
/// connection-delete flow doesn't have, so this is the connection-keyed
/// counterpart. Returns the number of entries removed (0 if none matched).
pub async fn remove_composio_source_by_connection_id(connection_id: &str) -> Result<usize, String> {
    let _guard = memory_sources_write_guard().await;
    let mut config = config_rpc::load_config_with_timeout().await?;
    let before = config.memory_sources.len();
    config.memory_sources.retain(|s| {
        !(s.kind == SourceKind::Composio && s.connection_id.as_deref() == Some(connection_id))
    });
    let removed = before - config.memory_sources.len();

    if removed > 0 {
        tracing::info!(
            connection_id = %connection_id,
            removed,
            "[memory_sources] removed composio source(s) on connection disconnect"
        );
        config
            .save()
            .await
            .map_err(|e| format!("failed to save config: {e:#}"))?;
    }

    Ok(removed)
}

/// Apply a single composio upsert to an in-memory source list.
///
/// If a source with the same `connection_id` already exists, updates its label
/// and returns a clone of the updated entry with `was_insert = false`;
/// otherwise pushes a new entry (with conservative per-toolkit defaults) and
/// returns it with `was_insert = true`.
///
/// Pure (no I/O) so the single-call path, the batch path, and unit tests all
/// share one find-or-push predicate and cannot drift.
fn upsert_composio_entry_in_place(
    sources: &mut Vec<MemorySourceEntry>,
    toolkit: &str,
    connection_id: &str,
    label: &str,
) -> (MemorySourceEntry, bool) {
    if let Some(existing) = sources.iter_mut().find(|s| {
        s.kind == SourceKind::Composio && s.connection_id.as_deref() == Some(connection_id)
    }) {
        existing.label = label.to_string();
        return (existing.clone(), false);
    }

    let (default_max_items, default_sync_depth_days) = memory_sync_defaults_for_toolkit(toolkit);
    tracing::debug!(
        toolkit = %toolkit,
        max_items = ?default_max_items,
        sync_depth_days = ?default_sync_depth_days,
        "[memory_sources] applying conservative defaults for new composio source"
    );

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
        max_commits: None,
        max_issues: None,
        max_prs: None,
        query: None,
        since_days: None,
        max_items: default_max_items,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: default_sync_depth_days,
    };
    sources.push(entry.clone());
    (entry, true)
}

/// Upsert a composio source — used by the auto-registration path.
/// If a source with the same `connection_id` already exists, updates
/// the label; otherwise inserts a new entry.
pub async fn upsert_composio_source(
    toolkit: &str,
    connection_id: &str,
    label: &str,
) -> Result<MemorySourceEntry, String> {
    let _guard = memory_sources_write_guard().await;
    let mut config = config_rpc::load_config_with_timeout().await?;

    let (entry, was_insert) =
        upsert_composio_entry_in_place(&mut config.memory_sources, toolkit, connection_id, label);

    config
        .save()
        .await
        .map_err(|e| format!("failed to save config: {e:#}"))?;

    if was_insert {
        tracing::info!(
            connection_id = %connection_id,
            toolkit = %toolkit,
            "[memory_sources] upserted composio source (insert)"
        );
    } else {
        tracing::debug!(
            connection_id = %connection_id,
            toolkit = %toolkit,
            "[memory_sources] upserted composio source (update)"
        );
    }

    Ok(entry)
}

/// Target for a batch composio upsert: `(toolkit, connection_id, label)`.
pub type ComposioUpsertTarget = (String, String, String);

/// Batch-upsert many composio sources with a **single** config load + save.
///
/// The per-call [`upsert_composio_source`] does its own load-modify-save, so
/// calling it in a loop costs `2N` config I/O round-trips and — worse — cannot
/// be parallelised safely (concurrent calls each load the same snapshot and
/// their saves clobber each other). This batch path loads the config once,
/// applies every upsert in-memory via the shared [`upsert_composio_entry_in_place`]
/// predicate, and saves once. Returns the number of targets applied.
///
/// Targets are applied in order; a later target sharing a `connection_id` with
/// an earlier one updates the same (already inserted/updated) entry's label,
/// matching the sequential single-call semantics.
pub async fn upsert_composio_sources_batch(
    targets: &[ComposioUpsertTarget],
) -> Result<u32, String> {
    if targets.is_empty() {
        return Ok(0);
    }

    let _guard = memory_sources_write_guard().await;
    let mut config = config_rpc::load_config_with_timeout().await?;

    let mut inserted = 0u32;
    let mut updated = 0u32;
    for (toolkit, connection_id, label) in targets {
        let (_entry, was_insert) = upsert_composio_entry_in_place(
            &mut config.memory_sources,
            toolkit,
            connection_id,
            label,
        );
        if was_insert {
            inserted += 1;
        } else {
            updated += 1;
        }
    }

    config
        .save()
        .await
        .map_err(|e| format!("failed to save config: {e:#}"))?;

    tracing::info!(
        targets = targets.len(),
        inserted,
        updated,
        "[memory_sources] batch-upserted composio sources (single save)"
    );

    Ok(inserted + updated)
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
    #[serde(default)]
    pub max_tokens_per_sync: Option<u64>,
    #[serde(default)]
    pub max_cost_per_sync_usd: Option<f64>,
    #[serde(default)]
    pub sync_depth_days: Option<u32>,
    // ── GithubRepo-specific caps (previously missing from patch) ──
    #[serde(default)]
    pub max_commits: Option<u32>,
    #[serde(default)]
    pub max_issues: Option<u32>,
    #[serde(default)]
    pub max_prs: Option<u32>,
}

/// Enable ALL configured memory sources and clear every per-source cap,
/// giving the user unrestricted access ("All In" mode).
///
/// For each source in `config.memory_sources`:
/// - Sets `enabled = true`.
/// - Clears `max_items`, `since_days`, `sync_depth_days`,
///   `max_commits`, `max_issues`, `max_prs`,
///   `max_tokens_per_sync`, `max_cost_per_sync_usd` to `None`.
///
/// Saves config once after all mutations and returns the updated entries.
pub async fn apply_all_in() -> Result<Vec<MemorySourceEntry>, String> {
    let _guard = memory_sources_write_guard().await;
    let mut config = config_rpc::load_config_with_timeout().await?;

    tracing::info!(
        count = config.memory_sources.len(),
        "[memory_sources] apply_all_in: enabling all sources and clearing caps"
    );

    for source in &mut config.memory_sources {
        tracing::debug!(
            id = %source.id,
            kind = %source.kind.as_str(),
            "[memory_sources] apply_all_in: enabling source and clearing caps"
        );
        source.enabled = true;
        source.max_items = None;
        source.since_days = None;
        source.sync_depth_days = None;
        source.max_commits = None;
        source.max_issues = None;
        source.max_prs = None;
        source.max_tokens_per_sync = None;
        source.max_cost_per_sync_usd = None;
    }

    let updated = config.memory_sources.clone();

    config
        .save()
        .await
        .map_err(|e| format!("apply_all_in: failed to save config: {e:#}"))?;

    tracing::info!(
        count = updated.len(),
        "[memory_sources] apply_all_in: complete — all sources enabled, all caps cleared"
    );

    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composio_defaults_for_known_toolkits() {
        assert_eq!(
            memory_sync_defaults_for_toolkit("gmail"),
            (Some(100), Some(30))
        );
        assert_eq!(
            memory_sync_defaults_for_toolkit("slack"),
            (Some(50), Some(14))
        );
        assert_eq!(
            memory_sync_defaults_for_toolkit("notion"),
            (Some(30), Some(30))
        );
        assert_eq!(
            memory_sync_defaults_for_toolkit("linear"),
            (Some(50), Some(30))
        );
        assert_eq!(
            memory_sync_defaults_for_toolkit("clickup"),
            (Some(50), Some(30))
        );
        assert_eq!(
            memory_sync_defaults_for_toolkit("github"),
            (Some(50), Some(30))
        );
    }

    #[test]
    fn composio_defaults_for_generic_fallback() {
        assert_eq!(
            memory_sync_defaults_for_toolkit("unknown_toolkit_xyz"),
            (Some(30), Some(14))
        );
        assert_eq!(memory_sync_defaults_for_toolkit(""), (Some(30), Some(14)));
    }

    #[test]
    fn in_place_upsert_inserts_new_entry_with_toolkit_defaults() {
        let mut sources: Vec<MemorySourceEntry> = vec![];
        let (entry, was_insert) =
            upsert_composio_entry_in_place(&mut sources, "gmail", "conn_a", "Gmail · conn_a");
        assert!(was_insert, "first upsert for a connection must insert");
        assert_eq!(sources.len(), 1);
        assert_eq!(entry.kind, SourceKind::Composio);
        assert_eq!(entry.connection_id.as_deref(), Some("conn_a"));
        assert_eq!(entry.toolkit.as_deref(), Some("gmail"));
        assert_eq!(entry.label, "Gmail · conn_a");
        assert!(entry.enabled);
        // Conservative gmail defaults are applied on insert.
        assert_eq!(entry.max_items, Some(100));
        assert_eq!(entry.sync_depth_days, Some(30));
    }

    #[test]
    fn in_place_upsert_updates_label_only_for_existing_connection() {
        let mut sources: Vec<MemorySourceEntry> = vec![];
        upsert_composio_entry_in_place(&mut sources, "gmail", "conn_a", "old label");
        // User-customised a cap after the initial insert.
        sources[0].max_items = Some(7);

        let (entry, was_insert) =
            upsert_composio_entry_in_place(&mut sources, "gmail", "conn_a", "new label");
        assert!(!was_insert, "second upsert for same connection must update");
        assert_eq!(sources.len(), 1, "must not duplicate the entry");
        assert_eq!(entry.label, "new label");
        assert_eq!(
            entry.max_items,
            Some(7),
            "update must not clobber user-set caps"
        );
    }

    #[test]
    fn in_place_upsert_keeps_distinct_connections_separate() {
        let mut sources: Vec<MemorySourceEntry> = vec![];
        upsert_composio_entry_in_place(&mut sources, "gmail", "conn_a", "A");
        upsert_composio_entry_in_place(&mut sources, "gmail", "conn_b", "B");
        assert_eq!(sources.len(), 2);
        let ids: Vec<_> = sources
            .iter()
            .filter_map(|s| s.connection_id.as_deref())
            .collect();
        assert!(ids.contains(&"conn_a") && ids.contains(&"conn_b"));
    }

    #[test]
    fn memory_source_patch_deserializes_partial() {
        let json = serde_json::json!({ "label": "New label", "enabled": false });
        let patch: MemorySourcePatch = serde_json::from_value(json).unwrap();
        assert_eq!(patch.label.as_deref(), Some("New label"));
        assert_eq!(patch.enabled, Some(false));
        assert!(patch.toolkit.is_none());
    }

    #[test]
    fn memory_source_patch_round_trips_github_limit_fields() {
        let json = serde_json::json!({
            "max_commits": 100,
            "max_issues": 50,
            "max_prs": 25
        });
        let patch: MemorySourcePatch = serde_json::from_value(json).unwrap();
        assert_eq!(patch.max_commits, Some(100));
        assert_eq!(patch.max_issues, Some(50));
        assert_eq!(patch.max_prs, Some(25));
        // Unset fields must be None (serde(default))
        assert!(patch.label.is_none());
        assert!(patch.enabled.is_none());
    }

    #[test]
    fn memory_source_patch_defaults_github_fields_to_none() {
        let json = serde_json::json!({ "enabled": true });
        let patch: MemorySourcePatch = serde_json::from_value(json).unwrap();
        assert!(patch.max_commits.is_none());
        assert!(patch.max_issues.is_none());
        assert!(patch.max_prs.is_none());
    }
}
