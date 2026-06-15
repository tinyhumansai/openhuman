//! Persisted, self-contained Claude Code provider settings.
//!
//! The only Claude-Code-specific knob exposed to the user — full access
//! (`bypassPermissions` + full native toolset) vs the default `acceptEdits`
//! posture — lives in a small JSON file under the user's workspace rather than
//! in the central [`crate::openhuman::config::Config`]. Keeping it module-local
//! means the toggle is easy to reason about and trivial to remove, and it
//! avoids threading a Claude-Code-only flag through the shared config/RPC
//! plumbing.
//!
//! Read at turn time by [`super::driver`]; written by the
//! `inference.claude_code_set_full_access` RPC. The
//! `OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE` env var overrides this at the driver
//! layer (debugging / power users).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name (under `workspace_dir`) holding the persisted toggle.
const SETTINGS_FILE: &str = "claude_code_settings.json";

/// Persisted Claude Code provider settings. Defaults are the safe posture.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeCodeSettings {
    /// When true, Claude Code runs with `--permission-mode bypassPermissions`
    /// plus its complete native toolset (Bash/network/subagents). Default
    /// false → `acceptEdits` (auto-apply file edits, gate everything else).
    #[serde(default)]
    pub full_access: bool,
}

fn settings_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(SETTINGS_FILE)
}

/// Load settings from `workspace_dir`. A missing or unreadable/corrupt file
/// yields defaults (full access OFF) — fail safe, never fail open.
pub fn load(workspace_dir: &Path) -> ClaudeCodeSettings {
    let path = settings_path(workspace_dir);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            log::warn!(
                "[claude-code][settings] corrupt {} ({e}); using safe defaults",
                path.display()
            );
            ClaudeCodeSettings::default()
        }),
        Err(e) => {
            log::debug!(
                "[claude-code][settings] no settings at {} ({e}); using defaults",
                path.display()
            );
            ClaudeCodeSettings::default()
        }
    }
}

/// Persist `settings` to `workspace_dir`, creating the directory if needed.
pub fn save(workspace_dir: &Path, settings: &ClaudeCodeSettings) -> std::io::Result<()> {
    let path = settings_path(workspace_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    log::debug!(
        "[claude-code][settings] saved full_access={} → {}",
        settings.full_access,
        path.display()
    );
    Ok(())
}

/// Load settings for the workspace implied by `config` (resolved via
/// [`super::workspace_dir_from_config`]). Keeps workspace resolution + file IO
/// out of the RPC handler so `schemas.rs` stays a thin delegator.
pub fn load_for_config(config: &crate::openhuman::config::Config) -> ClaudeCodeSettings {
    load(&super::workspace_dir_from_config(config))
}

/// Persist the full-access toggle for the workspace implied by `config` and
/// return the saved settings.
pub fn save_full_access_for_config(
    config: &crate::openhuman::config::Config,
    full_access: bool,
) -> std::io::Result<ClaudeCodeSettings> {
    let settings = ClaudeCodeSettings { full_access };
    save(&super::workspace_dir_from_config(config), &settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_returns_safe_defaults() {
        let dir = std::env::temp_dir().join("oh_cc_settings_missing_test");
        let _ = std::fs::remove_dir_all(&dir);
        let s = load(&dir);
        assert!(!s.full_access, "missing settings must default to OFF");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join("oh_cc_settings_roundtrip_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        save(&dir, &ClaudeCodeSettings { full_access: true }).unwrap();
        assert!(
            load(&dir).full_access,
            "saved full_access=true must persist"
        );
        save(&dir, &ClaudeCodeSettings { full_access: false }).unwrap();
        assert!(
            !load(&dir).full_access,
            "toggling back to false must persist"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_corrupt_file_returns_safe_defaults() {
        let dir = std::env::temp_dir().join("oh_cc_settings_corrupt_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(settings_path(&dir), b"{not json").unwrap();
        assert!(
            !load(&dir).full_access,
            "corrupt settings must fail safe to OFF"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
