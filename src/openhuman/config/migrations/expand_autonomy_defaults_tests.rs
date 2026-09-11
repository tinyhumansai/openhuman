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
