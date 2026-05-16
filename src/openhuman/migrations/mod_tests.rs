use super::*;
use crate::openhuman::agent::harness::session::transcript::{
    read_transcript, write_transcript, TranscriptMeta,
};
use crate::openhuman::providers::ChatMessage;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

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

    assert_eq!(config.schema_version, 2);
    let session = read_transcript(&path).unwrap();
    assert!(
        !session.messages[0].content.contains("### PROFILE.md"),
        "PROFILE.md block must be stripped, got:\n{}",
        session.messages[0].content
    );

    let on_disk = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(
        on_disk.contains("schema_version = 2"),
        "saved config.toml must record schema_version=2, got:\n{on_disk}"
    );
}

#[tokio::test]
async fn run_pending_bumps_version_on_fresh_install() {
    let tmp = TempDir::new().unwrap();
    // No session_raw/ at all — pure fresh install.
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    run_pending(&mut config).await;

    assert_eq!(config.schema_version, 2);
    let on_disk = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(on_disk.contains("schema_version = 2"));
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
    assert_eq!(config.schema_version, 2);

    // Mutate the config file timestamp marker by reading + comparing
    // before vs after the second invocation.
    let before = fs::metadata(&config.config_path).unwrap().modified().ok();
    std::thread::sleep(std::time::Duration::from_millis(20));
    run_pending(&mut config).await;
    let after = fs::metadata(&config.config_path).unwrap().modified().ok();

    assert_eq!(config.schema_version, 2);
    assert_eq!(
        before, after,
        "config.toml must not be re-saved on second run"
    );
}
