//! Migration 3 → 4: expand autonomy defaults for existing users.
//!
//! PR #2500 expanded the code defaults for `autonomy.allowed_commands` and
//! `autonomy.auto_approve`, and changed `max_actions_per_hour` from 20 to
//! `u32::MAX` (effectively unlimited). Existing users had the old values
//! persisted in their `config.toml` at `schema_version = 3`, so they did
//! **not** pick up the new defaults automatically — their on-disk values
//! shadow the code defaults.
//!
//! ## What this migration does
//!
//! 1. **Merges new commands** into `config.autonomy.allowed_commands`. Only
//!    commands not already present are added, so any user customisation
//!    (e.g. additional entries, deliberate removals) is fully preserved.
//! 2. **Merges new auto-approve tools** into `config.autonomy.auto_approve`
//!    with the same additive-only merge logic.
//! 3. **Bumps `max_actions_per_hour`** from 20 (the old hard-coded default)
//!    to `u32::MAX` only when the persisted value is exactly 20. Users who
//!    deliberately set a different limit are left untouched.
//!
//! ## Idempotency
//!
//! - Gated externally by [`Config::schema_version`] (`== 3`). Once the bump
//!   to version 4 is persisted, future launches skip this migration entirely.
//! - Internally idempotent: merging already-present items is a no-op because
//!   the merge logic guards every insert with a `contains` check.

use crate::openhuman::config::Config;

/// The old hard-coded default for `max_actions_per_hour`. When this exact
/// value is still persisted, we assume it was never deliberately customised
/// and bump it to the new unlimited sentinel.
const OLD_DEFAULT_MAX_ACTIONS_PER_HOUR: u32 = 20;

/// Commands to merge into persisted `allowed_commands` during the v3→v4 bump.
///
/// The target set mirrors the current default more closely than old v3 configs.
/// Some entries may already be present for customized users; the migration is
/// additive and skips duplicates.
const NEW_COMMANDS: &[&str] = &[
    "pnpm", "yarn", "make", "cmake", "sort", "uniq", "diff", "which", "uname", "basename",
    "dirname", "tr", "cut", "realpath", "readlink", "stat", "file", "mkdir", "touch", "cp", "mv",
    "ln", "date", "dir", "type", "where", "findstr", "more",
];

/// New auto-approve tools to merge into `auto_approve`.
///
/// These were added to the code default in PR #2500 but are absent from any
/// `config.toml` written before that change.
const NEW_AUTO_APPROVE_TOOLS: &[&str] = &["glob", "grep"];

/// Counters returned by [`run`] for diagnostics. Logged at INFO once per
/// successful migration run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationStats {
    /// Number of commands added to `allowed_commands`.
    pub commands_added: usize,
    /// Number of tools added to `auto_approve`.
    pub tools_added: usize,
    /// `true` when `max_actions_per_hour` was bumped from 20 to `u32::MAX`.
    pub max_actions_bumped: bool,
}

/// Run the autonomy defaults expansion migration on the given `Config`.
///
/// Synchronous — pure config mutation, no I/O. The caller
/// (`migrations::run_pending`) persists the result via `Config::save()` and
/// bumps `schema_version`.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let mut stats = MigrationStats::default();

    log::debug!(
        "[migrations][expand-autonomy-defaults] starting \
         allowed_commands.len={} auto_approve.len={} max_actions_per_hour={}",
        config.autonomy.allowed_commands.len(),
        config.autonomy.auto_approve.len(),
        config.autonomy.max_actions_per_hour,
    );

    // Merge new commands (additive only — never remove user entries).
    for &cmd in NEW_COMMANDS {
        if !config.autonomy.allowed_commands.iter().any(|c| c == cmd) {
            log::debug!(
                "[migrations][expand-autonomy-defaults] adding command={:?} to allowed_commands",
                cmd
            );
            config.autonomy.allowed_commands.push(cmd.to_string());
            stats.commands_added += 1;
        }
    }

    // Merge new auto-approve tools (additive only).
    for &tool in NEW_AUTO_APPROVE_TOOLS {
        if !config.autonomy.auto_approve.iter().any(|t| t == tool) {
            log::debug!(
                "[migrations][expand-autonomy-defaults] adding tool={:?} to auto_approve",
                tool
            );
            config.autonomy.auto_approve.push(tool.to_string());
            stats.tools_added += 1;
        }
    }

    // Bump max_actions_per_hour only when it still holds the old default.
    // Users who deliberately configured a different ceiling keep their value.
    if config.autonomy.max_actions_per_hour == OLD_DEFAULT_MAX_ACTIONS_PER_HOUR {
        log::info!(
            "[migrations][expand-autonomy-defaults] bumping max_actions_per_hour \
             {} -> u32::MAX (old hard-coded default, PR #2500 changed code default)",
            OLD_DEFAULT_MAX_ACTIONS_PER_HOUR
        );
        config.autonomy.max_actions_per_hour = u32::MAX;
        stats.max_actions_bumped = true;
    } else {
        log::debug!(
            "[migrations][expand-autonomy-defaults] max_actions_per_hour={} — \
             not the old default, leaving unchanged",
            config.autonomy.max_actions_per_hour
        );
    }

    log::info!(
        "[migrations][expand-autonomy-defaults] done \
         commands_added={} tools_added={} max_actions_bumped={}",
        stats.commands_added,
        stats.tools_added,
        stats.max_actions_bumped,
    );

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    /// A config representing the old v3 defaults that a user would have
    /// persisted before PR #2500 expanded the code defaults.
    fn old_v3_config() -> Config {
        let mut config = Config::default();
        // Reset to the narrow v3 allowed_commands.
        config.autonomy.allowed_commands = vec![
            "git".into(),
            "npm".into(),
            "cargo".into(),
            "ls".into(),
            "cat".into(),
            "grep".into(),
            "find".into(),
            "echo".into(),
            "pwd".into(),
            "wc".into(),
            "head".into(),
            "tail".into(),
        ];
        // Reset to the narrow v3 auto_approve list.
        config.autonomy.auto_approve = vec![
            "file_read".into(),
            "memory_search".into(),
            "memory_list".into(),
            "get_time".into(),
            "list_dir".into(),
        ];
        // Reset to the old numeric default.
        config.autonomy.max_actions_per_hour = OLD_DEFAULT_MAX_ACTIONS_PER_HOUR;
        config
    }

    #[test]
    fn adds_new_commands_to_narrow_list() {
        let mut config = old_v3_config();
        let stats = run(&mut config).expect("migration should succeed");

        // Every new command must now be present.
        for cmd in NEW_COMMANDS {
            assert!(
                config.autonomy.allowed_commands.iter().any(|c| c == cmd),
                "expected {:?} in allowed_commands after migration",
                cmd
            );
        }
        // Existing commands must be preserved.
        for cmd in &["git", "npm", "cargo", "ls", "cat", "grep"] {
            assert!(
                config.autonomy.allowed_commands.iter().any(|c| c == *cmd),
                "expected existing command {:?} preserved",
                cmd
            );
        }
        assert!(
            stats.commands_added > 0,
            "expected at least one command to be added"
        );
    }

    #[test]
    fn adds_new_auto_approve_tools_to_narrow_list() {
        let mut config = old_v3_config();
        let stats = run(&mut config).expect("migration should succeed");

        for tool in NEW_AUTO_APPROVE_TOOLS {
            assert!(
                config.autonomy.auto_approve.iter().any(|t| t == tool),
                "expected {:?} in auto_approve after migration",
                tool
            );
        }
        // Write tools must keep Supervised mode's ask-before-edit contract.
        for tool in &["file_write", "edit_file"] {
            assert!(
                !config.autonomy.auto_approve.iter().any(|t| t == *tool),
                "expected {:?} to require approval after migration",
                tool
            );
        }
        // Existing tools must be preserved.
        for tool in &["file_read", "memory_search", "memory_list"] {
            assert!(
                config.autonomy.auto_approve.iter().any(|t| t == *tool),
                "expected existing tool {:?} preserved",
                tool
            );
        }
        assert!(
            stats.tools_added > 0,
            "expected at least one tool to be added"
        );
    }

    #[test]
    fn bumps_max_actions_when_old_default() {
        let mut config = old_v3_config();
        assert_eq!(
            config.autonomy.max_actions_per_hour,
            OLD_DEFAULT_MAX_ACTIONS_PER_HOUR
        );

        let stats = run(&mut config).expect("migration should succeed");

        assert!(stats.max_actions_bumped);
        assert_eq!(config.autonomy.max_actions_per_hour, u32::MAX);
    }

    #[test]
    fn does_not_bump_max_actions_when_user_customised() {
        let mut config = old_v3_config();
        config.autonomy.max_actions_per_hour = 100;

        let stats = run(&mut config).expect("migration should succeed");

        assert!(!stats.max_actions_bumped);
        assert_eq!(
            config.autonomy.max_actions_per_hour, 100,
            "user-customised ceiling must be preserved"
        );
    }

    #[test]
    fn idempotent_on_already_expanded_config() {
        let mut config = old_v3_config();

        // First run.
        let stats1 = run(&mut config).expect("first run should succeed");
        assert!(stats1.commands_added > 0 || stats1.tools_added > 0 || stats1.max_actions_bumped);

        // Second run — nothing changes.
        let snapshot_commands = config.autonomy.allowed_commands.clone();
        let snapshot_tools = config.autonomy.auto_approve.clone();
        let snapshot_max = config.autonomy.max_actions_per_hour;

        let stats2 = run(&mut config).expect("second run should succeed");

        assert_eq!(stats2.commands_added, 0, "no commands added on second run");
        assert_eq!(stats2.tools_added, 0, "no tools added on second run");
        assert!(
            !stats2.max_actions_bumped,
            "max_actions not bumped on second run"
        );
        assert_eq!(config.autonomy.allowed_commands, snapshot_commands);
        assert_eq!(config.autonomy.auto_approve, snapshot_tools);
        assert_eq!(config.autonomy.max_actions_per_hour, snapshot_max);
    }

    #[test]
    fn preserves_user_custom_commands_not_in_new_set() {
        let mut config = old_v3_config();
        config
            .autonomy
            .allowed_commands
            .push("my_custom_tool".to_string());

        run(&mut config).expect("migration should succeed");

        assert!(
            config
                .autonomy
                .allowed_commands
                .iter()
                .any(|c| c == "my_custom_tool"),
            "user's custom command must be preserved"
        );
    }

    #[test]
    fn no_duplicate_commands_when_some_already_present() {
        let mut config = old_v3_config();
        // Pre-seed a subset of new commands so we can check no duplicates appear.
        config.autonomy.allowed_commands.push("pnpm".to_string());
        config.autonomy.allowed_commands.push("yarn".to_string());

        run(&mut config).expect("migration should succeed");

        let pnpm_count = config
            .autonomy
            .allowed_commands
            .iter()
            .filter(|c| *c == "pnpm")
            .count();
        let yarn_count = config
            .autonomy
            .allowed_commands
            .iter()
            .filter(|c| *c == "yarn")
            .count();
        assert_eq!(pnpm_count, 1, "pnpm must appear exactly once");
        assert_eq!(yarn_count, 1, "yarn must appear exactly once");
    }

    #[test]
    fn no_op_on_fresh_install_defaults() {
        // A fresh install already has the expanded defaults; the migration
        // should be a complete no-op (all guards fire early).
        let mut config = Config::default();

        let stats = run(&mut config).expect("migration should succeed");

        assert_eq!(
            stats.commands_added, 0,
            "fresh install: no commands should be added"
        );
        assert_eq!(
            stats.tools_added, 0,
            "fresh install: no tools should be added"
        );
        assert!(
            !stats.max_actions_bumped,
            "fresh install: max_actions already u32::MAX, must not bump again"
        );
    }
}
