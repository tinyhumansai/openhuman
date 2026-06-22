use super::*;
use crate::openhuman::agent::harness::session::transcript::{
    read_transcript, write_transcript, TranscriptMeta,
};
use crate::openhuman::inference::provider::ChatMessage;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Simulate a v3 user config: narrow allowed_commands, narrow auto_approve,
/// and the old hard-coded `max_actions_per_hour = 20`.
fn simulate_v3_autonomy(config: &mut Config) {
    config.schema_version = 3;
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
    config.autonomy.auto_approve = vec![
        "file_read".into(),
        "memory_search".into(),
        "memory_list".into(),
        "get_time".into(),
        "list_dir".into(),
    ];
    config.autonomy.max_actions_per_hour = 20;
}

fn tainted_prompt() -> String {
    "## Identity\n\nYou are an assistant.\n\n\
     ### PROFILE.md\n\n\
     style/calm tooling/rust\n\n\
     ### Tools\n\n- shell\n"
        .to_string()
}

fn meta() -> TranscriptMeta {
    TranscriptMeta {
        agent_name: "main".into(),
        dispatcher: "native".into(),
        created: "2026-05-01T00:00:00Z".into(),
        updated: "2026-05-01T00:00:00Z".into(),
        turn_count: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: None,
    }
}

fn config_in(tmp: &TempDir) -> Config {
    Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        ..Default::default()
    }
}

fn seed_tainted_transcript(workspace_dir: &Path) -> std::path::PathBuf {
    let raw_dir = workspace_dir.join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();
    let path = raw_dir.join("1700000000_main.jsonl");
    let messages = vec![
        ChatMessage::system(tainted_prompt()),
        ChatMessage::user("hello"),
    ];
    write_transcript(&path, &messages, &meta(), None).unwrap();
    path
}

#[tokio::test]
async fn run_pending_skips_when_version_current() {
    let tmp = TempDir::new().unwrap();
    let path = seed_tainted_transcript(&tmp.path().join("workspace"));
    let before = fs::read(&path).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = CURRENT_SCHEMA_VERSION;
    run_pending(&mut config).await;

    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    let after = fs::read(&path).unwrap();
    assert_eq!(before, after, "transcript must be untouched");
}

#[tokio::test]
async fn run_pending_runs_phase_out_when_version_zero() {
    let tmp = TempDir::new().unwrap();
    let path = seed_tainted_transcript(&tmp.path().join("workspace"));

    let mut config = config_in(&tmp);
    assert_eq!(config.schema_version, 0);
    run_pending(&mut config).await;

    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    let session = read_transcript(&path).unwrap();
    assert!(
        !session.messages[0].content.contains("### PROFILE.md"),
        "PROFILE.md block must be stripped, got:\n{}",
        session.messages[0].content
    );

    let on_disk = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(
        on_disk.contains("schema_version = 7"),
        "saved config.toml must record schema_version=7, got:\n{on_disk}"
    );
}

#[tokio::test]
async fn run_pending_bumps_version_on_fresh_install() {
    let tmp = TempDir::new().unwrap();
    // No session_raw/ at all — pure fresh install.
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    run_pending(&mut config).await;

    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    let on_disk = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(on_disk.contains("schema_version = 7"));
}

#[tokio::test]
async fn run_pending_migrates_fastembed_to_managed_without_local_ollama() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 6;
    config.memory.embedding_provider = "fastembed".to_string();
    config.memory.embedding_model = "BGESmallENV15".to_string();
    config.memory.embedding_dimensions = 384;
    // Point the local-Ollama probe at a guaranteed-dead address so the rewrite
    // target is deterministic (managed) regardless of whether the host happens
    // to run Ollama on the default port.
    config.local_ai.base_url = Some("http://127.0.0.1:1".to_string());

    run_pending(&mut config).await;

    assert_eq!(config.schema_version, 7);
    assert_eq!(
        config.memory.embedding_provider, "managed",
        "no reachable local Ollama ⇒ managed cloud target"
    );
    assert_eq!(config.memory.embedding_dimensions, 1024);
    let on_disk = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(on_disk.contains("schema_version = 7"));
}

#[tokio::test]
async fn run_pending_rolls_back_schema_version_when_save_fails() {
    let tmp = TempDir::new().unwrap();
    seed_tainted_transcript(&tmp.path().join("workspace"));

    let mut config = config_in(&tmp);
    // Point config.save() at a path whose parent directory cannot be
    // created (a regular file occupies that name), forcing save() to
    // error after the migration body has succeeded.
    let blocker = tmp.path().join("blocker");
    fs::write(&blocker, "not a directory").unwrap();
    config.config_path = blocker.join("nested").join("config.toml");

    assert_eq!(config.schema_version, 0);
    run_pending(&mut config).await;

    assert_eq!(
        config.schema_version, 0,
        "save failed → in-memory schema_version must be rolled back to 0"
    );
}

#[tokio::test]
async fn run_pending_is_a_no_op_on_second_invocation() {
    let tmp = TempDir::new().unwrap();
    seed_tainted_transcript(&tmp.path().join("workspace"));

    let mut config = config_in(&tmp);
    run_pending(&mut config).await;
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);

    // Mutate the config file timestamp marker by reading + comparing
    // before vs after the second invocation.
    let before = fs::metadata(&config.config_path).unwrap().modified().ok();
    std::thread::sleep(std::time::Duration::from_millis(20));
    run_pending(&mut config).await;
    let after = fs::metadata(&config.config_path).unwrap().modified().ok();

    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        before, after,
        "config.toml must not be re-saved on second run"
    );
}

// ── v3 → v4: expand_autonomy_defaults integration test ──────────────────────

/// Verify that `run_pending` applies the v3→v4 autonomy expansion when
/// starting from a simulated old-default config at schema_version=3.
#[tokio::test]
async fn run_pending_expands_autonomy_defaults_from_v3() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    simulate_v3_autonomy(&mut config);

    assert_eq!(config.schema_version, 3);
    assert_eq!(config.autonomy.max_actions_per_hour, 20);
    assert!(!config.autonomy.allowed_commands.iter().any(|c| c == "pnpm"));
    assert!(!config.autonomy.auto_approve.iter().any(|t| t == "glob"));

    run_pending(&mut config).await;

    assert_eq!(
        config.schema_version, CURRENT_SCHEMA_VERSION,
        "schema_version must be bumped to current"
    );

    // New commands must be present after the migration.
    for cmd in &["pnpm", "yarn", "make", "sort", "diff", "mkdir", "cp"] {
        assert!(
            config.autonomy.allowed_commands.iter().any(|c| c == *cmd),
            "expected {:?} in allowed_commands after v3→v4 migration",
            cmd
        );
    }
    // Original commands must be preserved.
    for cmd in &["git", "npm", "cargo", "ls", "cat"] {
        assert!(
            config.autonomy.allowed_commands.iter().any(|c| c == *cmd),
            "expected original command {:?} preserved",
            cmd
        );
    }

    // New read-only auto-approve tools must be present.
    for tool in &["glob", "grep"] {
        assert!(
            config.autonomy.auto_approve.iter().any(|t| t == *tool),
            "expected {:?} in auto_approve after v3→v4 migration",
            tool
        );
    }
    // Write tools must keep Supervised mode's ask-before-edit contract.
    for tool in &["file_write", "edit_file"] {
        assert!(
            !config.autonomy.auto_approve.iter().any(|t| t == *tool),
            "expected {:?} to require approval after v3→v4 migration",
            tool
        );
    }
    // Original auto-approve tools must be preserved.
    for tool in &["file_read", "memory_search", "memory_list"] {
        assert!(
            config.autonomy.auto_approve.iter().any(|t| t == *tool),
            "expected original tool {:?} preserved",
            tool
        );
    }

    // max_actions_per_hour must be bumped from the old default of 20.
    assert_eq!(
        config.autonomy.max_actions_per_hour,
        u32::MAX,
        "max_actions_per_hour must be bumped to u32::MAX"
    );

    // On-disk config must reflect the new schema_version.
    let on_disk = fs::read_to_string(&config.config_path).unwrap();
    assert!(
        on_disk.contains("schema_version = 7"),
        "saved config.toml must record schema_version=7, got:\n{on_disk}"
    );
}

// ── v4 → v5: remove_write_auto_approve integration test ─────────────────────

/// Verify that workspaces already migrated to schema_version=4 have write tools
/// removed from `auto_approve` so Supervised mode prompts before file edits.
#[tokio::test]
async fn run_pending_v4_to_v5_removes_write_tools_from_auto_approve() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 4;
    config.autonomy.auto_approve = vec![
        "file_read".into(),
        "file_write".into(),
        "edit_file".into(),
        "glob".into(),
    ];

    run_pending(&mut config).await;

    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        config.autonomy.auto_approve,
        vec!["file_read".to_string(), "glob".to_string()]
    );

    let on_disk = fs::read_to_string(&config.config_path).unwrap();
    assert!(
        on_disk.contains("schema_version = 7"),
        "saved config.toml must record schema_version=7, got:\n{on_disk}"
    );
}

// ── v5 → v6: repair_http_request_limits integration test ────────────────────

/// A workspace persisted at schema_version=5 with stale-zero `[http_request]`
/// limits (the exact bug this PR fixes) must be repaired to the schema
/// defaults and bumped to v6 by the full `run_pending` wiring — not just the
/// migration's own unit tests.
#[tokio::test]
async fn run_pending_v5_to_v6_repairs_http_request_limits() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 5;
    config.http_request.timeout_secs = 0;
    config.http_request.max_response_size = 0;

    run_pending(&mut config).await;

    let defaults = crate::openhuman::config::HttpRequestConfig::default();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(config.http_request.timeout_secs, defaults.timeout_secs);
    assert_eq!(
        config.http_request.max_response_size,
        defaults.max_response_size
    );
    assert_ne!(config.http_request.timeout_secs, 0);
    assert_ne!(config.http_request.max_response_size, 0);

    // The version bump must be persisted to disk too.
    let on_disk = fs::read_to_string(&config.config_path).unwrap();
    assert!(
        on_disk.contains("schema_version = 7"),
        "saved config.toml must record schema_version=7, got:\n{on_disk}"
    );
}

/// Companion to the repair test above: the same single 5 -> 6 transition must
/// also run `reconcile_orphaned_providers`. A config at schema_version=5 with an
/// orphaned `chat_provider` (slug not in `cloud_providers`) and a dangling
/// `primary_cloud` must have both reset to managed and be bumped to v6 through
/// the full `run_pending` wiring.
#[tokio::test]
async fn run_pending_v5_to_v6_reconciles_orphaned_providers() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 5;
    // `openai` is not present in cloud_providers (left empty) → orphaned.
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.primary_cloud = Some("p_missing".to_string());

    run_pending(&mut config).await;

    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        config.chat_provider, None,
        "orphaned chat_provider must be reset to managed"
    );
    assert_eq!(
        config.primary_cloud, None,
        "dangling primary_cloud must be cleared"
    );

    let on_disk = fs::read_to_string(&config.config_path).unwrap();
    assert!(
        on_disk.contains("schema_version = 7"),
        "saved config.toml must record schema_version=7, got:\n{on_disk}"
    );
}

/// Verify that a user at v3 with a deliberately customised
/// `max_actions_per_hour` does NOT have it reset by the migration.
#[tokio::test]
async fn run_pending_v3_to_v4_preserves_custom_max_actions() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    simulate_v3_autonomy(&mut config);
    // User has deliberately configured a specific ceiling.
    config.autonomy.max_actions_per_hour = 50;

    run_pending(&mut config).await;

    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        config.autonomy.max_actions_per_hour, 50,
        "user-customised max_actions_per_hour must not be overwritten"
    );
}
