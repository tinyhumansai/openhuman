//! Persistence for the skill registry: cached catalog entries.
//!
//! The cache lives at `~/.openhuman/skill-registry/cache.json` with a 1-hour
//! TTL. Past the TTL the cache is kept (not deleted) so callers can serve it
//! stale-while-revalidate; see [`load_cached_catalog_state`]. Set
//! `OPENHUMAN_SKILL_REGISTRY_CACHE_DIR` to relocate the cache (used by tests).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::types::CatalogEntry;

const CACHE_DIR: &str = "skill-registry";
const CACHE_FILE: &str = "cache.json";
const CACHE_TTL_SECS: u64 = 3600;
const CACHE_DIR_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_CACHE_DIR";

#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogCache {
    pub entries: Vec<CatalogEntry>,
    pub fetched_at_epoch: u64,
}

/// A cached catalog tagged by freshness relative to `CACHE_TTL_SECS`.
pub enum CachedCatalog {
    /// Within the TTL — safe to serve directly.
    Fresh(Vec<CatalogEntry>),
    /// Past the TTL — usable for stale-while-revalidate while a refresh runs.
    Stale(Vec<CatalogEntry>),
}

fn registry_dir() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os(CACHE_DIR_ENV) {
        let trimmed = raw.to_string_lossy().trim().to_string();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    dirs::home_dir().map(|h| h.join(".openhuman").join(CACHE_DIR))
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Load the cached catalog regardless of age, tagged `Fresh`/`Stale` by TTL.
/// Returns `None` only when no readable cache exists.
pub fn load_cached_catalog_state() -> Option<CachedCatalog> {
    let dir = registry_dir()?;
    let path = dir.join(CACHE_FILE);
    let data = std::fs::read_to_string(&path).ok()?;
    let cache: CatalogCache = serde_json::from_str(&data).ok()?;

    let age_secs = now_epoch().saturating_sub(cache.fetched_at_epoch);
    if age_secs > CACHE_TTL_SECS {
        tracing::debug!(age_secs, "[skill_registry] cache stale (past TTL)");
        Some(CachedCatalog::Stale(cache.entries))
    } else {
        tracing::debug!(
            count = cache.entries.len(),
            age_secs,
            "[skill_registry] cache fresh"
        );
        Some(CachedCatalog::Fresh(cache.entries))
    }
}

/// Load the cached catalog only when within the freshness TTL.
pub fn load_cached_catalog() -> Option<Vec<CatalogEntry>> {
    match load_cached_catalog_state()? {
        CachedCatalog::Fresh(entries) => Some(entries),
        CachedCatalog::Stale(_) => None,
    }
}

pub fn save_catalog_cache(entries: &[CatalogEntry]) {
    let Some(dir) = registry_dir() else { return };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, "[skill_registry] failed to create cache dir");
        return;
    }
    let cache = CatalogCache {
        entries: entries.to_vec(),
        fetched_at_epoch: now_epoch(),
    };
    let path = dir.join(CACHE_FILE);
    match serde_json::to_string_pretty(&cache) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!(error = %e, "[skill_registry] failed to write cache");
            }
        }
        Err(e) => tracing::warn!(error = %e, "[skill_registry] failed to serialize cache"),
    }
}

pub fn clear_cache() {
    let Some(dir) = registry_dir() else { return };
    let path = dir.join(CACHE_FILE);
    let _ = std::fs::remove_file(&path);
    tracing::debug!("[skill_registry] cache cleared");
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
