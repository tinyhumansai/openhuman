//! Persisted, self-contained Claude Code provider settings.
//!
//! The only Claude-Code-specific knob exposed to the user — full access
//! (`bypassPermissions` + full native toolset) vs the default `acceptEdits`
//! posture — lives in its own small JSON file rather than in the central
//! [`crate::openhuman::config::Config`]. Keeping it module-local means the
//! toggle is easy to reason about and trivial to remove, and it avoids
//! threading a Claude-Code-only flag through the shared config/RPC plumbing.
//!
//! **Where the file actually is:** the directory
//! [`super::workspace_dir_from_config`] returns — the parent of
//! `config.config_path`, i.e. the OpenHuman **config directory** (`~/.openhuman`
//! by default), so the file sits next to `config.toml`. Despite the parameter
//! name below, that is *not* [`crate::openhuman::config::Config::workspace_dir`]
//! (the internal state dir, `~/.openhuman/workspace`) and *not* the user's
//! project root (`config.action_dir`).
//!
//! Read at turn time by [`super::driver`]; written by the
//! `inference.claude_code_set_full_access` RPC. The
//! `OPENHUMAN_CLAUDE_CODE_PERMISSION_MODE` env var overrides this at the driver
//! layer (debugging / power users).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name holding the persisted toggle, written into the directory the
/// caller passes (the OpenHuman config dir — see the module docs).
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

/// Load settings from `workspace_dir` — the Claude Code provider's own notion
/// of a workspace, which is the OpenHuman config dir (see the module docs), not
/// [`crate::openhuman::config::Config::workspace_dir`]. A missing or
/// unreadable/corrupt file yields defaults (full access OFF) — fail safe, never
/// fail open.
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

/// Load settings for the directory implied by `config` — the parent of
/// `config.config_path`, resolved via [`super::workspace_dir_from_config`].
/// Keeps path resolution + file IO out of the RPC handler so `schemas.rs` stays
/// a thin delegator.
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
#[path = "settings_tests.rs"]
mod tests;
