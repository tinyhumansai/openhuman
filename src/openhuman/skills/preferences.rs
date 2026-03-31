//! Persistent skill preferences (enable/disable, setup completion).
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
    #[serde(default)]
    pub setup_complete: bool,
}

/// Persistent store for skill preferences.
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
        *self.cache.write() = HashMap::new();
    }

    /// Persist the current in-memory preferences to disk.
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

    fn update<F: FnOnce(&mut SkillPreference)>(&self, skill_id: &str, f: F) {
        let mut cache = self.cache.write();
        let pref = cache
            .entry(skill_id.to_string())
            .or_insert(SkillPreference {
                enabled: false,
                setup_complete: false,
            });
        f(pref);
        drop(cache);
        self.save();
    }

    /// Check if a skill has a user preference set. Returns `None` if no preference exists.
    pub fn is_enabled(&self, skill_id: &str) -> Option<bool> {
        self.cache.read().get(skill_id).map(|p| p.enabled)
    }

    /// Set the enabled preference for a skill. Persists immediately.
    pub fn set_enabled(&self, skill_id: &str, enabled: bool) {
        self.update(skill_id, |p| p.enabled = enabled);
    }

    /// Get whether a skill's setup has been completed.
    pub fn is_setup_complete(&self, skill_id: &str) -> bool {
        self.get_or_default(skill_id).setup_complete
    }

    /// Set the setup completion flag for a skill. Persists immediately.
    /// When marking setup as complete, also sets `enabled = true` so the skill
    /// auto-starts on subsequent app launches.
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

    /// Get all preferences as a map.
    pub fn get_all(&self) -> HashMap<String, SkillPreference> {
        self.cache.read().clone()
    }

    /// Resolve whether a skill should start, considering user preference,
    /// setup completion, and manifest default.
    ///
    /// A skill with `setup_complete = true` always starts — the user explicitly
    /// went through setup/OAuth, so the intent is to have it running.
    /// Otherwise fall back to the explicit `enabled` preference, then the manifest default.
    pub fn resolve_should_start(&self, skill_id: &str, manifest_auto_start: bool) -> bool {
        let pref = self.cache.read().get(skill_id).cloned();
        match pref {
            Some(p) if p.setup_complete => true,
            Some(p) => p.enabled,
            None => manifest_auto_start,
        }
    }
}
