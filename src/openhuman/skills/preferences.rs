//! Persistent skill preferences management.
//!
//! This module manages user-defined preferences for skills, such as whether a skill
//! is enabled and whether its initial setup has been completed. Preferences are
//! persisted to a JSON file on disk, ensuring they survive application restarts.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Represents the user's persistent preferences for a single skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPreference {
    /// Whether the skill is currently enabled by the user.
    pub enabled: bool,
    /// Whether the skill's required setup process (e.g., OAuth) has been completed.
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

    /// Retrieves the preference for a skill, returning a default object if no preference exists.
    fn get_or_default(&self, skill_id: &str) -> SkillPreference {
        self.cache
            .read()
            .get(skill_id)
            .cloned()
            .unwrap_or(SkillPreference {
                enabled: false,
                setup_complete: false,
            })
    }

    /// Updates the preference for a skill using the provided closure and persists the change.
    fn update<F: FnOnce(&mut SkillPreference)>(&self, skill_id: &str, f: F) {
        let mut cache = self.cache.write();
        let pref = cache
            .entry(skill_id.to_string())
            .or_insert(SkillPreference {
                enabled: false,
                setup_complete: false,
            });
        f(pref);
        drop(cache); // Explicitly release lock before saving to avoid potential deadlocks
        self.save();
    }

    /// Returns whether a skill is explicitly enabled by the user.
    ///
    /// Returns `None` if no preference has been set for this skill.
    pub fn is_enabled(&self, skill_id: &str) -> Option<bool> {
        self.cache.read().get(skill_id).map(|p| p.enabled)
    }

    /// Sets the enabled preference for a skill and persists it to disk.
    pub fn set_enabled(&self, skill_id: &str, enabled: bool) {
        self.update(skill_id, |p| p.enabled = enabled);
    }

    /// Checks if a skill's setup process has been recorded as complete.
    pub fn is_setup_complete(&self, skill_id: &str) -> bool {
        self.get_or_default(skill_id).setup_complete
    }

    /// Marks a skill's setup as complete (or incomplete) and persists the state.
    ///
    /// If marked as complete, the skill is also automatically marked as `enabled`.
    pub fn set_setup_complete(&self, skill_id: &str, complete: bool) {
        self.update(skill_id, |p| {
            p.setup_complete = complete;
            if complete {
                p.enabled = true;
            }
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
    /// 2. If an explicit `enabled` preference exists, use it.
    /// 3. Otherwise, fall back to the `auto_start` value from the skill's manifest.
    pub fn resolve_should_start(&self, skill_id: &str, manifest_auto_start: bool) -> bool {
        let pref = self.cache.read().get(skill_id).cloned();
        match pref {
            Some(p) if p.setup_complete => true,
            Some(p) => p.enabled,
            None => manifest_auto_start,
        }
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
    fn enable_disable_roundtrip() {
        let (store, _dir) = temp_store();
        assert_eq!(store.is_enabled("my-skill"), None);
        store.set_enabled("my-skill", true);
        assert_eq!(store.is_enabled("my-skill"), Some(true));
        store.set_enabled("my-skill", false);
        assert_eq!(store.is_enabled("my-skill"), Some(false));
    }

    #[test]
    fn setup_complete_also_enables() {
        let (store, _dir) = temp_store();
        store.set_setup_complete("s1", true);
        assert!(store.is_setup_complete("s1"));
        assert_eq!(store.is_enabled("s1"), Some(true));
    }

    #[test]
    fn persistence_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        {
            let store = PreferencesStore::new(&path);
            store.set_enabled("x", true);
            store.set_setup_complete("x", true);
        }
        // Reload from disk
        let store2 = PreferencesStore::new(&path);
        assert_eq!(store2.is_enabled("x"), Some(true));
        assert!(store2.is_setup_complete("x"));
    }

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferencesStore::new(&dir.path().to_path_buf());
        assert_eq!(store.is_enabled("nonexistent"), None);
        assert!(!store.is_setup_complete("nonexistent"));
    }

    #[test]
    fn corrupt_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("skill-preferences.json");
        std::fs::write(&file, "not valid json {{{}").unwrap();
        let store = PreferencesStore::new(&dir.path().to_path_buf());
        assert_eq!(store.is_enabled("any"), None);
    }

    #[test]
    fn resolve_should_start_setup_complete_overrides() {
        let (store, _dir) = temp_store();
        store.set_setup_complete("s1", true);
        // Even if manifest says false, setup_complete wins
        assert!(store.resolve_should_start("s1", false));
    }

    #[test]
    fn resolve_should_start_falls_back_to_enabled() {
        let (store, _dir) = temp_store();
        store.set_enabled("s1", true);
        assert!(store.resolve_should_start("s1", false));
        store.set_enabled("s1", false);
        assert!(!store.resolve_should_start("s1", true));
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
        store.set_enabled("a", true);
        store.set_enabled("b", false);
        let all = store.get_all();
        assert_eq!(all.len(), 2);
        assert!(all["a"].enabled);
        assert!(!all["b"].enabled);
    }
}
