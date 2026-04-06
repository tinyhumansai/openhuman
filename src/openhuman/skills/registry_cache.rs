//! Registry cache management — disk-based caching for the remote skill registry.
//!
//! This module provides utilities for fetching the remote skill registry and
//! caching it locally on disk to reduce network traffic and allow for offline
//! operation or faster startup.

use std::path::{Path, PathBuf};

use super::registry_types::{CachedRegistry, RemoteSkillRegistry, SkillCategory};

/// Time-to-live for the cached registry in seconds (1 hour).
pub(crate) const CACHE_TTL_SECS: i64 = 3600;

/// Default URL for fetching the remote skill registry.
pub(crate) const DEFAULT_REGISTRY_URL: &str = "https://raw.githubusercontent.com/tinyhumansai/openhuman-skills/refs/heads/build/skills/registry.json";

/// Resolves the URL to use for the skill registry, favoring the `SKILLS_REGISTRY_URL`
/// environment variable if set.
pub(crate) fn registry_url() -> String {
    std::env::var("SKILLS_REGISTRY_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string())
}

/// Returns the path to the local skills directory if specified by the `SKILLS_LOCAL_DIR`
/// environment variable.
pub(crate) fn local_skills_dir() -> Option<PathBuf> {
    std::env::var("SKILLS_LOCAL_DIR").ok().map(PathBuf::from)
}

/// Determines if a given URL represents a local file path or a `file://` URI.
pub(crate) fn is_local_path(url: &str) -> bool {
    url.starts_with('/') || url.starts_with("file://")
}

/// Reads a file from a local path or `file://` URI.
pub(crate) fn read_local_file(url: &str) -> Result<Vec<u8>, String> {
    let path = if let Some(stripped) = url.strip_prefix("file://") {
        PathBuf::from(stripped)
    } else {
        PathBuf::from(url)
    };
    std::fs::read(&path).map_err(|e| format!("failed to read local file {}: {e}", path.display()))
}

/// Constructs the path to the registry cache file within the workspace.
pub(crate) fn cache_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("skills").join(".registry-cache.json")
}

/// Determines if the cached registry is still within its TTL.
pub(crate) fn is_cache_fresh(cached: &CachedRegistry) -> bool {
    let Ok(fetched) = chrono::DateTime::parse_from_rfc3339(&cached.fetched_at) else {
        return false;
    };
    let now = chrono::Utc::now();
    (now - fetched.to_utc()).num_seconds() < CACHE_TTL_SECS
}

/// Reads the cached skill registry from disk if it exists.
pub(crate) fn read_cache(workspace_dir: &Path) -> Option<CachedRegistry> {
    let path = cache_path(workspace_dir);
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Persists the skill registry to the local cache file.
pub(crate) fn write_cache(
    workspace_dir: &Path,
    registry: &RemoteSkillRegistry,
) -> Result<(), String> {
    let cached = CachedRegistry {
        fetched_at: chrono::Utc::now().to_rfc3339(),
        registry: registry.clone(),
    };
    let path = cache_path(workspace_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create cache dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&cached).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("failed to write cache: {e}"))?;
    Ok(())
}

/// Automatically tags each skill entry in the registry with its appropriate category
/// based on the source list (core or third-party).
pub(crate) fn tag_categories(registry: &mut RemoteSkillRegistry) {
    for entry in &mut registry.skills.core {
        entry.category = SkillCategory::Core;
    }
    for entry in &mut registry.skills.third_party {
        entry.category = SkillCategory::ThirdParty;
    }
}
