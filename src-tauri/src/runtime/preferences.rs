//! Persistent skill enable/disable preferences.
//!
//! Stores user preferences in `{app_data_dir}/skill-preferences.json`.
//! These preferences override the manifest's `auto_start` field,
//! allowing users to enable or disable skills across restarts.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A single skill's preference record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPreference {
    pub enabled: bool,
}

/// Persistent store for skill enable/disable preferences.
pub struct PreferencesStore {
    path: PathBuf,
    cache: RwLock<HashMap<String, SkillPreference>>,
}

impl PreferencesStore {
    /// Create a new preferences store at the given directory.
    /// The file `skill-preferences.json` is created inside `data_dir`.
    pub fn new(data_dir: &PathBuf) -> Self {
        let path = data_dir.join("skill-preferences.json");
        let store = Self {
            path,
            cache: RwLock::new(HashMap::new()),
        };
        store.load();
        store
    }

    /// Load preferences from disk into memory. Silently ignores missing/corrupt files.
    fn load(&self) {
        if let Ok(content) = std::fs::read_to_string(&self.path) {
            if let Ok(prefs) = serde_json::from_str::<HashMap<String, SkillPreference>>(&content) {
                *self.cache.write() = prefs;
                return;
            }
        }
        // Start empty if file doesn't exist or is corrupt
        *self.cache.write() = HashMap::new();
    }

    /// Persist the current in-memory preferences to disk.
    fn save(&self) {
        let cache = self.cache.read();
        if let Ok(json) = serde_json::to_string_pretty(&*cache) {
            // Ensure parent directory exists
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&self.path, json) {
                log::error!("[preferences] Failed to save preferences: {e}");
            }
        }
    }

    /// Check if a skill has a user preference set. Returns `None` if no preference exists.
    pub fn is_enabled(&self, skill_id: &str) -> Option<bool> {
        self.cache.read().get(skill_id).map(|p| p.enabled)
    }

    /// Set the enabled preference for a skill. Persists immediately.
    pub fn set_enabled(&self, skill_id: &str, enabled: bool) {
        self.cache
            .write()
            .insert(skill_id.to_string(), SkillPreference { enabled });
        self.save();
    }

    /// Get all preferences as a map.
    pub fn get_all(&self) -> HashMap<String, SkillPreference> {
        self.cache.read().clone()
    }

    /// Resolve whether a skill should start, considering user preference and manifest default.
    /// User preference always wins; falls back to `manifest_auto_start` if no preference set.
    pub fn resolve_should_start(&self, skill_id: &str, manifest_auto_start: bool) -> bool {
        match self.is_enabled(skill_id) {
            Some(enabled) => enabled,
            None => manifest_auto_start,
        }
    }
}
