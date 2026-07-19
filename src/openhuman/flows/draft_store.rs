//! File-based persistence for [`FlowDraft`]s (audit F5).
//!
//! Drafts are plain JSON files under `{workspace_dir}/flows/drafts/<id>.json`,
//! one file per draft — deliberately NOT a SQLite table (no schema/migration,
//! trivially inspectable and deletable). The draft is the shared working copy
//! the agent tools (Rust core) and the canvas both read/write by id across
//! turns and reloads, which rules out frontend-only `localStorage`.
//!
//! This module is the thin storage layer; business logic (promote → the
//! existing create/update gates) lives in [`super::ops`]. The same RPC contract
//! (`flows_draft_*`) would hold if drafts later migrate into a table.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::flows::types::{DraftOrigin, FlowDraft};

/// The directory holding draft files, `{workspace_dir}/flows/drafts`.
fn drafts_dir(config: &Config) -> PathBuf {
    config.workspace_dir.join("flows").join("drafts")
}

/// Whether `id` is a safe draft-file stem — guards `get`/`update`/`delete`
/// against path traversal (`..`, separators) since the id reaches the
/// filesystem. Server-minted ids are UUIDs; this only accepts that shape.
fn is_safe_draft_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The on-disk path for draft `id` (validated).
fn draft_path(config: &Config, id: &str) -> Result<PathBuf> {
    if !is_safe_draft_id(id) {
        bail!("invalid draft id: {id:?}");
    }
    Ok(drafts_dir(config).join(format!("{id}.json")))
}

/// Creates a new draft, writes it to disk, and returns it.
pub fn create_draft(
    config: &Config,
    flow_id: Option<String>,
    name: String,
    graph: Value,
    origin: DraftOrigin,
) -> Result<FlowDraft> {
    let now = Utc::now().to_rfc3339();
    let draft = FlowDraft {
        id: Uuid::new_v4().to_string(),
        flow_id,
        name,
        graph,
        origin,
        created_at: now.clone(),
        updated_at: now,
    };
    write_draft(config, &draft)?;
    tracing::debug!(
        target: "flows",
        draft_id = %draft.id,
        origin = draft.origin.as_str(),
        "[flows] draft_store: created draft"
    );
    Ok(draft)
}

/// Reads a draft by id, or `None` if no such file exists.
pub fn get_draft(config: &Config, id: &str) -> Result<Option<FlowDraft>> {
    let path = draft_path(config, id)?;
    match std::fs::read(&path) {
        Ok(bytes) => {
            let draft: FlowDraft =
                serde_json::from_slice(&bytes).with_context(|| format!("draft {id} is corrupt"))?;
            Ok(Some(draft))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading draft {id}")),
    }
}

/// Patches a draft's mutable fields (any `Some` is applied), bumps
/// `updated_at`, persists, and returns the updated draft. Errors if the draft
/// does not exist.
pub fn update_draft(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph: Option<Value>,
    flow_id: Option<Option<String>>,
) -> Result<FlowDraft> {
    let mut draft = get_draft(config, id)?.with_context(|| format!("draft {id} not found"))?;
    if let Some(name) = name {
        draft.name = name;
    }
    if let Some(graph) = graph {
        draft.graph = graph;
    }
    if let Some(flow_id) = flow_id {
        draft.flow_id = flow_id;
    }
    draft.updated_at = Utc::now().to_rfc3339();
    write_draft(config, &draft)?;
    tracing::debug!(target: "flows", draft_id = %id, "[flows] draft_store: updated draft");
    Ok(draft)
}

/// Lists all drafts, newest-updated first. Skips (and logs) any corrupt file
/// rather than failing the whole listing.
pub fn list_drafts(config: &Config) -> Result<Vec<FlowDraft>> {
    let dir = drafts_dir(config);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("listing drafts in {}", dir.display())),
    };
    let mut drafts = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read(&path).map(|b| serde_json::from_slice::<FlowDraft>(&b)) {
            Ok(Ok(draft)) => drafts.push(draft),
            Ok(Err(e)) => {
                tracing::warn!(target: "flows", path = %path.display(), error = %e, "[flows] draft_store: skipping corrupt draft file");
            }
            Err(e) => {
                tracing::warn!(target: "flows", path = %path.display(), error = %e, "[flows] draft_store: could not read draft file");
            }
        }
    }
    // Newest-updated first (RFC3339 with a fixed +00:00 offset sorts lexically).
    drafts.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(drafts)
}

/// Deletes a draft file. Returns `true` if a file was removed, `false` if it
/// was already absent.
pub fn delete_draft(config: &Config, id: &str) -> Result<bool> {
    let path = draft_path(config, id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => {
            tracing::debug!(target: "flows", draft_id = %id, "[flows] draft_store: deleted draft");
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("deleting draft {id}")),
    }
}

/// Serializes a draft to its file, creating the drafts dir if needed. Writes to
/// a temp file then renames, so a crash mid-write never leaves a corrupt draft.
fn write_draft(config: &Config, draft: &FlowDraft) -> Result<()> {
    let dir = drafts_dir(config);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating drafts dir {}", dir.display()))?;
    let path = draft_path(config, &draft.id)?;
    let tmp = dir.join(format!(".{}.json.tmp", draft.id));
    let json = serde_json::to_vec_pretty(draft).context("serializing draft")?;
    std::fs::write(&tmp, &json).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    fn sample_graph() -> Value {
        json!({ "nodes": [ { "id": "t", "kind": "trigger", "name": "Manual" } ], "edges": [] })
    }

    #[test]
    fn create_get_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let draft = create_draft(
            &config,
            None,
            "My draft".into(),
            sample_graph(),
            DraftOrigin::Chat,
        )
        .unwrap();
        let loaded = get_draft(&config, &draft.id).unwrap().unwrap();
        assert_eq!(loaded, draft);
        assert_eq!(loaded.name, "My draft");
        assert_eq!(loaded.origin, DraftOrigin::Chat);
        assert!(loaded.flow_id.is_none());
    }

    #[test]
    fn get_missing_is_none() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        assert!(get_draft(&config, "does-not-exist").unwrap().is_none());
    }

    #[test]
    fn update_patches_fields_and_bumps_updated_at() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let draft = create_draft(
            &config,
            None,
            "Old".into(),
            sample_graph(),
            DraftOrigin::Canvas,
        )
        .unwrap();
        let updated = update_draft(
            &config,
            &draft.id,
            Some("New name".into()),
            Some(json!({ "nodes": [], "edges": [] })),
            Some(Some("flow-42".into())),
        )
        .unwrap();
        assert_eq!(updated.name, "New name");
        assert_eq!(updated.flow_id.as_deref(), Some("flow-42"));
        assert_eq!(updated.graph["nodes"].as_array().unwrap().len(), 0);
        assert!(updated.updated_at >= draft.updated_at);
        assert_eq!(updated.created_at, draft.created_at);
    }

    #[test]
    fn list_returns_newest_first_and_delete_removes() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let a = create_draft(&config, None, "A".into(), sample_graph(), DraftOrigin::Chat).unwrap();
        // Bump a second draft's updated_at by updating it after creation.
        let b = create_draft(&config, None, "B".into(), sample_graph(), DraftOrigin::Chat).unwrap();
        let b = update_draft(&config, &b.id, Some("B2".into()), None, None).unwrap();

        let list = list_drafts(&config).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id, "newest-updated first");

        assert!(delete_draft(&config, &a.id).unwrap());
        assert!(
            !delete_draft(&config, &a.id).unwrap(),
            "second delete is a no-op"
        );
        assert_eq!(list_drafts(&config).unwrap().len(), 1);
    }

    #[test]
    fn list_on_missing_dir_is_empty() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        assert!(list_drafts(&config).unwrap().is_empty());
    }

    #[test]
    fn rejects_path_traversal_ids() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        assert!(get_draft(&config, "../secret").is_err());
        assert!(draft_path(&config, "a/b").is_err());
        assert!(draft_path(&config, "..").is_err());
        assert!(draft_path(&config, "ok-123_ID").is_ok());
    }
}
