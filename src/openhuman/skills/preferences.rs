//! Persistent skill preferences management.
//!
//! This module manages user-defined preferences for skills. The only thing it
//! tracks today is whether a skill's setup process (e.g. OAuth) has been
//! completed. Setup completion is the single source of truth for "should this
//! skill be running": there is no separate `enabled` toggle. Preferences are
//! persisted to a JSON file on disk so they survive application restarts.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Represents the user's persistent preferences for a single skill.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillPreference {
    /// Whether the skill's required setup process (e.g., OAuth) has been completed.
    /// A skill with `setup_complete = true` is automatically started by the runtime.
    #[serde(default)]
    pub setup_complete: bool,
}

/// A thread-safe store for managing and persisting skill preferences.
pub struct PreferencesStore {
    /// The file path where preferences are saved.
    path: PathBuf,
    /// An in-memory cache of the preferences, protected by a read-write lock.
    cache: RwLock<HashMap<String, SkillPreference>>,
}

impl PreferencesStore {
    /// Creates a new `PreferencesStore` located in the specified data directory.
    ///
    /// Automatically attempts to load existing preferences from `skill-preferences.json`.
    pub fn new(data_dir: &PathBuf) -> Self {
        let path = data_dir.join("skill-preferences.json");
        let store = Self {
            path,
            cache: RwLock::new(HashMap::new()),
        };
        store.load();
        store
    }

    /// Loads preferences from the persistent file on disk into the in-memory cache.
    ///
    /// If the file is missing or contains invalid JSON, the cache is initialized as empty.
    /// Legacy entries that still carry an `enabled` field are silently ignored — only
    /// `setup_complete` is read.
    fn load(&self) {
        if let Ok(content) = std::fs::read_to_string(&self.path) {
            if let Ok(prefs) = serde_json::from_str::<HashMap<String, SkillPreference>>(&content) {
                *self.cache.write() = prefs;
                return;
            }
        }
        *self.cache.write() = HashMap::new();
    }

    /// Persists the current in-memory preferences cache to the disk as pretty-printed JSON.
    fn save(&self) {
        let cache = self.cache.read();
        if let Ok(json) = serde_json::to_string_pretty(&*cache) {
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&self.path, json) {
                log::error!("[preferences] Failed to save preferences: {e}");
            }
        }
    }

    /// Updates the preference for a skill using the provided closure and persists the change.
    fn update<F: FnOnce(&mut SkillPreference)>(&self, skill_id: &str, f: F) {
        let mut cache = self.cache.write();
        let pref = cache
            .entry(skill_id.to_string())
            .or_insert(SkillPreference::default());
        f(pref);
        drop(cache); // Explicitly release lock before saving to avoid potential deadlocks
        self.save();
    }

    /// Checks if a skill's setup process has been recorded as complete.
    pub fn is_setup_complete(&self, skill_id: &str) -> bool {
        self.cache
            .read()
            .get(skill_id)
            .map(|p| p.setup_complete)
            .unwrap_or(false)
    }

    /// Marks a skill's setup as complete (or incomplete) and persists the state.
    pub fn set_setup_complete(&self, skill_id: &str, complete: bool) {
        self.update(skill_id, |p| {
            p.setup_complete = complete;
        });
        log::info!(
            "[preferences] setup_complete for '{}' set to {}",
            skill_id,
            complete
        );
    }

    /// Returns a snapshot of all current skill preferences.
    pub fn get_all(&self) -> HashMap<String, SkillPreference> {
        self.cache.read().clone()
    }

    /// Resolves whether a skill should be started, factoring in user preferences and manifest defaults.
    ///
    /// Priority order:
    /// 1. If `setup_complete` is true, the skill should start.
    /// 2. Otherwise, fall back to the `auto_start` value from the skill's manifest.
    pub fn resolve_should_start(&self, skill_id: &str, manifest_auto_start: bool) -> bool {
        if self.is_setup_complete(skill_id) {
            return true;
        }
        manifest_auto_start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (PreferencesStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferencesStore::new(&dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn setup_complete_roundtrip() {
        let (store, _dir) = temp_store();
        assert!(!store.is_setup_complete("s1"));
        store.set_setup_complete("s1", true);
        assert!(store.is_setup_complete("s1"));
        store.set_setup_complete("s1", false);
        assert!(!store.is_setup_complete("s1"));
    }

    #[test]
    fn persistence_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let store = PreferencesStore::new(&path);
            store.set_setup_complete("x", true);
        }
        // Reload from disk
        let store2 = PreferencesStore::new(&path);
        assert!(store2.is_setup_complete("x"));
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferencesStore::new(&dir.path().to_path_buf());
        assert!(!store.is_setup_complete("nonexistent"));
    }

    #[test]
    fn corrupt_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("skill-preferences.json");
        std::fs::write(&file, "not valid json {{{}").unwrap();
        let store = PreferencesStore::new(&dir.path().to_path_buf());
        assert!(!store.is_setup_complete("any"));
    }

    #[test]
    fn resolve_should_start_setup_complete_overrides_manifest() {
        let (store, _dir) = temp_store();
        store.set_setup_complete("s1", true);
        // Even if manifest says false, setup_complete wins
        assert!(store.resolve_should_start("s1", false));
    }

    #[test]
    fn resolve_should_start_falls_back_to_manifest() {
        let (store, _dir) = temp_store();
        assert!(store.resolve_should_start("unknown", true));
        assert!(!store.resolve_should_start("unknown", false));
    }

    #[test]
    fn get_all_returns_complete_map() {
        let (store, _dir) = temp_store();
        store.set_setup_complete("a", true);
        store.set_setup_complete("b", false);
        let all = store.get_all();
        assert_eq!(all.len(), 2);
        assert!(all["a"].setup_complete);
        assert!(!all["b"].setup_complete);
    }
}
