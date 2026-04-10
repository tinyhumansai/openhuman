//! Path resolution for the skills runtime.
//!
//! Skill runtime state (OAuth tokens, webhook routes, ops snapshots, per-skill
//! data dirs) must live **under the active user's scoped directory** —
//! `~/.openhuman/users/{user_id}/skills_data` — so that switching users on the
//! same machine cleanly isolates OAuth credentials, sync cursors, and cached
//! results.
//!
//! Before this helper was introduced, both `core::jsonrpc::bootstrap_skill_runtime`
//! and `core::skills_cli::bootstrap_skills_runtime` reconstructed the base path
//! manually from `$OPENHUMAN_WORKSPACE` or `~/.openhuman`, bypassing
//! `Config::load_or_init` and therefore ignoring `active_user.toml`. That left
//! every skill's data pooled at `~/.openhuman/skills_data` regardless of which
//! user was logged in — the single unscoped slice of state in an otherwise
//! user-scoped layout.
//!
//! This module centralises resolution so every bootstrap site produces the
//! same scoped paths.

use std::path::PathBuf;

/// Paths required to initialise the QuickJS skills runtime.
#[derive(Debug, Clone)]
pub struct SkillsRuntimePaths {
    /// Where per-skill persistent state lives (OAuth tokens, ops snapshots,
    /// `skill-preferences.json`, `webhook_routes.json`, per-skill subdirs).
    pub skills_data_dir: PathBuf,
    /// The workspace directory the engine uses for user-installed skills
    /// from the registry and for agent definitions (`agents/*.toml`).
    pub workspace_dir: PathBuf,
}

/// Resolve skills runtime paths via `Config::load_or_init` so they honour
/// `active_user.toml` and the full per-user scoping pipeline.
///
/// If config loading fails (e.g. the process is booting before any config
/// file has been written) this falls back to the legacy unscoped layout so
/// the runtime can still come up — but emits a warning so the degraded mode
/// is visible in logs.
pub async fn resolve_runtime_paths() -> SkillsRuntimePaths {
    match crate::openhuman::config::Config::load_or_init().await {
        Ok(config) => {
            // `config_path` is `{openhuman_dir}/config.toml`. Its parent is the
            // openhuman_dir itself — which, when an active user is set, is
            // `{root}/users/{user_id}` (see `resolve_runtime_config_dirs` in
            // `config/schema/load.rs`). Sibling-of-config matches the existing
            // on-disk convention for skills_data (it used to sit next to
            // config.toml at `~/.openhuman/skills_data`).
            let openhuman_dir = config
                .config_path
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| config.workspace_dir.clone());

            let skills_data_dir = openhuman_dir.join("skills_data");
            let workspace_dir = config.workspace_dir.clone();

            log::debug!(
                "[skills:paths] resolved via Config openhuman_dir={} workspace_dir={} skills_data_dir={}",
                openhuman_dir.display(),
                workspace_dir.display(),
                skills_data_dir.display(),
            );

            warn_if_legacy_unscoped_data_exists(&skills_data_dir);

            SkillsRuntimePaths {
                skills_data_dir,
                workspace_dir,
            }
        }
        Err(err) => {
            log::warn!(
                "[skills:paths] Config::load_or_init failed ({err}) — falling back to legacy unscoped layout"
            );
            legacy_unscoped_paths()
        }
    }
}

/// Warn (once per bootstrap) if the legacy, unscoped
/// `~/.openhuman/skills_data` directory still exists while we are now
/// resolving to a different (user-scoped) location. This tells the user why
/// their skills appear to have "forgotten" their OAuth credentials after
/// upgrading and where to find the stale data.
///
/// We deliberately **do not** auto-migrate: the legacy dir was shared
/// across every user of the machine, so silently moving it into whichever
/// user happened to log in first could leak another user's tokens.
fn warn_if_legacy_unscoped_data_exists(resolved_skills_data_dir: &std::path::Path) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let legacy = home.join(".openhuman").join("skills_data");
    if !legacy.exists() {
        return;
    }
    if legacy == resolved_skills_data_dir {
        // We resolved to the legacy path itself (unauthenticated layout) —
        // nothing to warn about.
        return;
    }
    log::warn!(
        "[skills:paths] legacy unscoped skills_data still present at {} but runtime now uses {}. \
         Skills will start fresh. Manually move the subdirectories you want to preserve \
         (e.g. per-skill OAuth tokens, webhook_routes.json) into the new path.",
        legacy.display(),
        resolved_skills_data_dir.display(),
    );
}

/// Legacy fallback — the original resolution that was inlined in both
/// bootstrap sites. Used only when `Config::load_or_init` fails during
/// runtime bootstrap.
fn legacy_unscoped_paths() -> SkillsRuntimePaths {
    let base_dir = std::env::var("OPENHUMAN_WORKSPACE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".openhuman")
        });
    SkillsRuntimePaths {
        skills_data_dir: base_dir.join("skills_data"),
        workspace_dir: base_dir.join("workspace"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAII guard that restores `OPENHUMAN_WORKSPACE` on drop so tests
    /// remain panic-safe and don't pollute sibling tests that read the
    /// same env var.
    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn legacy_fallback_uses_env_workspace_when_set() {
        let _guard = EnvGuard::set("OPENHUMAN_WORKSPACE", "/tmp/openhuman-test-paths");
        let paths = legacy_unscoped_paths();
        assert_eq!(
            paths.skills_data_dir,
            PathBuf::from("/tmp/openhuman-test-paths/skills_data")
        );
        assert_eq!(
            paths.workspace_dir,
            PathBuf::from("/tmp/openhuman-test-paths/workspace")
        );
    }
}
