use super::*;
use crate::openhuman::memory::conversations::{
    ensure_thread, list_threads, CreateConversationThread,
};
use tempfile::TempDir;

fn make_thread(id: &str, labels: Vec<String>) -> CreateConversationThread {
    CreateConversationThread {
        id: id.to_string(),
        title: format!("Test thread {id}"),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        parent_thread_id: None,
        labels: Some(labels),
        personality_id: None,
    }
}

fn write_transcript(path: &Path, agent: &str, thread_id: &str) {
    let body = format!(
        "{{\"_meta\":{{\"agent\":\"{agent}\",\"dispatcher\":\"native\",\"created\":\"2026-05-01T00:00:00Z\",\"updated\":\"2026-05-01T00:00:00Z\",\"turn_count\":1,\"input_tokens\":0,\"output_tokens\":0,\"cached_input_tokens\":0,\"charged_amount_usd\":0.0,\"thread_id\":\"{thread_id}\"}}}}\n{{\"role\":\"user\",\"content\":\"hi\"}}\n"
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn migration_updates_thread_labels_and_transcripts() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path();

    ensure_thread(
        workspace.to_path_buf(),
        make_thread(
            "welcome-thread",
            vec!["onboarding".into(), "personal".into()],
        ),
    )
    .unwrap();

    let raw = workspace.join("session_raw/1715000000_welcome_thread-abc.jsonl");
    write_transcript(&raw, "welcome_thread-abc", "thread-abc");
    let md = workspace.join("sessions/2026_05_01/1715000000_welcome_thread-abc.md");
    fs::create_dir_all(md.parent().unwrap()).unwrap();
    fs::write(&md, "# Session transcript — welcome_thread-abc\n").unwrap();

    let result = migrate_welcome_agent_artifacts(workspace).unwrap();

    assert_eq!(result.threads_updated, 1);
    assert_eq!(result.transcripts_updated, 1);
    assert_eq!(result.transcript_files_renamed, 1);
    assert_eq!(result.markdown_files_renamed, 1);

    let threads = list_threads(workspace.to_path_buf()).unwrap();
    let thread = threads
        .iter()
        .find(|thread| thread.id == "welcome-thread")
        .unwrap();
    assert_eq!(thread.labels, vec!["personal"]);

    let renamed = workspace.join("session_raw/1715000000_orchestrator_thread-abc.jsonl");
    assert!(renamed.exists(), "renamed transcript should exist");
    let contents = fs::read_to_string(&renamed).unwrap();
    assert!(
        contents.contains("\"agent\":\"orchestrator_thread-abc\""),
        "transcript metadata should be rewritten: {contents}"
    );
    assert!(
        workspace
            .join("sessions/2026_05_01/1715000000_orchestrator_thread-abc.md")
            .exists(),
        "markdown companion should be renamed"
    );
    assert!(workspace.join(MIGRATION_MARKER).exists());
}

#[test]
fn migration_is_idempotent_after_marker() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path();
    fs::create_dir_all(workspace.join("state/migrations")).unwrap();
    fs::write(workspace.join(MIGRATION_MARKER), b"done\n").unwrap();

    let result = migrate_welcome_agent_artifacts(workspace).unwrap();
    assert!(result.already_done);
    assert_eq!(result.threads_updated, 0);
    assert_eq!(result.transcripts_updated, 0);
}

#[test]
fn migration_returns_error_without_marker_when_destination_exists() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path();

    let raw = workspace.join("session_raw/1715000000_welcome_thread-abc.jsonl");
    write_transcript(&raw, "welcome_thread-abc", "thread-abc");
    let dest = workspace.join("session_raw/1715000000_orchestrator_thread-abc.jsonl");
    write_transcript(&dest, "orchestrator_thread-abc", "thread-abc");

    let err = migrate_welcome_agent_artifacts(workspace).unwrap_err();

    assert!(
        err.contains("partial migration"),
        "expected partial-migration error, got: {err}"
    );
    let contents = fs::read_to_string(&raw).unwrap();
    assert!(
        contents.contains("\"agent\":\"welcome_thread-abc\""),
        "blocked rename should leave legacy metadata untouched: {contents}"
    );
    assert!(
        !workspace.join(MIGRATION_MARKER).exists(),
        "partial migration must not write marker"
    );
}
