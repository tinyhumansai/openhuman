use super::*;
use crate::storage::rusqlite;
use crate::storage::MemoryStorage;
use crate::util::classic_transcript_fixture::write_classic;
use sha2::Digest as _;
use std::collections::BTreeMap;
use tempfile::TempDir;
use tinyagents_session::transcript::{TranscriptMessage, TranscriptMeta};
use tinyagents_session::turn_state::TurnState;

fn workspace() -> (TempDir, Config) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    (dir, config)
}

fn backend() -> Arc<dyn StorageBackend> {
    Arc::new(MemoryStorage::new())
}

/// Every file under `root` with a digest of its bytes.
fn tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let digest = sha2::Sha256::digest(std::fs::read(&path).unwrap());
                out.insert(
                    path.strip_prefix(root).unwrap().display().to_string(),
                    hex::encode(digest),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn meta(thread: &str, agent: &str) -> TranscriptMeta {
    TranscriptMeta {
        agent_name: agent.into(),
        agent_id: Some(agent.into()),
        agent_type: None,
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-01-01T00:00:00Z".into(),
        updated: "2026-01-01T00:00:00Z".into(),
        turn_count: 1,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some(thread.into()),
        task_id: None,
        session_id: None,
        parent_session_id: None,
    }
}

/// A classic workspace: one transcript of the orchestrator, one in-flight turn
/// snapshot, and a `sessions.db` holding one session.
fn legacy_workspace(config: &Config) {
    let stem = session_stem(&SessionRef::scoped("thread-1", "orchestrator"));
    let raw = config.workspace_dir.join("session_raw");
    std::fs::create_dir_all(&raw).unwrap();
    let message = TranscriptMessage::new("user", "hello from the old layout");
    write_classic(
        &raw.join(format!("{stem}.jsonl")),
        &[message],
        &meta("thread-1", "orchestrator"),
    );

    // The classic file layout: turn_states/<hex(thread)>/<hex(request)>.json.
    let snapshot = TurnState::started("thread-1", "req-1", 4, "2026-01-01T00:00:00Z");
    let thread_dir = config
        .workspace_dir
        .join("memory")
        .join("conversations")
        .join("turn_states")
        .join(hex::encode("thread-1"));
    std::fs::create_dir_all(&thread_dir).unwrap();
    std::fs::write(
        thread_dir.join(format!("{}.json", hex::encode("req-1"))),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();

    let db = config.workspace_dir.join(SESSIONS_DB);
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions (
            id TEXT PRIMARY KEY, agent_definition_id TEXT NOT NULL,
            agent_definition_name TEXT NOT NULL, session_key TEXT NOT NULL,
            parent_session_id TEXT, thread_id TEXT, source_channel TEXT,
            status TEXT NOT NULL DEFAULT 'running', model TEXT,
            turn_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0, cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0.0, transcript_path TEXT,
            started_at TEXT NOT NULL, ended_at TEXT);
         INSERT INTO sessions (id, agent_definition_id, agent_definition_name, session_key, started_at)
         VALUES ('sess-1', 'orchestrator', 'Orchestrator', 'thread-1', '2026-01-01T00:00:00Z');",
    )
    .unwrap();
}

#[test]
fn a_classic_workspace_moves_into_the_backend_and_its_files_are_untouched() {
    let (dir, config) = workspace();
    legacy_workspace(&config);
    let before = tree(dir.path());
    let backend = backend();

    let reports = run_with(&backend, &config);
    assert_eq!(reports.len(), 1);
    let outcome = reports.into_iter().next().unwrap().outcome.unwrap();
    assert!(matches!(outcome, Outcome::Imported { .. }), "{outcome:?}");

    let provider = DriverSessionStores::new(Arc::clone(&backend), Scope::local()).unwrap();
    // The transcript landed in the orchestrator's scope, where its session
    // resumes it.
    let orchestrator = provider.try_for_agent("orchestrator").unwrap();
    assert!(
        orchestrator
            .transcripts
            .root_for_thread("thread-1")
            .is_some(),
        "the transcript is in the agent its stem names"
    );
    // The in-flight turn snapshot is the default agent's.
    let default = provider.try_for_agent(DEFAULT_AGENT).unwrap();
    assert!(default.turn_states.get("thread-1").unwrap().is_some());
    assert_eq!(
        tree(dir.path()),
        before,
        "no legacy file was modified, created or removed"
    );
}

#[test]
fn a_finished_import_does_not_run_again() {
    let (_dir, config) = workspace();
    legacy_workspace(&config);
    let backend = backend();
    run_with(&backend, &config).remove(0).outcome.unwrap();

    let again = run_with(&backend, &config).remove(0).outcome.unwrap();

    assert_eq!(again, Outcome::AlreadyDone);
}

#[test]
fn a_workspace_with_no_legacy_sessions_imports_nothing() {
    let (dir, config) = workspace();
    let backend = backend();

    let outcome = run_with(&backend, &config).remove(0).outcome.unwrap();

    assert_eq!(outcome, Outcome::NoSource);
    assert!(tree(dir.path()).is_empty(), "nothing was created");
}

#[test]
fn a_transcript_naming_an_unknown_agent_lands_in_the_default_agent() {
    let (_dir, config) = workspace();
    let stem = session_stem(&SessionRef::scoped("thread-9", "no-such-agent"));
    let raw = config.workspace_dir.join("session_raw");
    std::fs::create_dir_all(&raw).unwrap();
    write_classic(
        &raw.join(format!("{stem}.jsonl")),
        &[TranscriptMessage::new("user", "who am I")],
        &meta("thread-9", "no-such-agent"),
    );
    let backend = backend();

    run_with(&backend, &config).remove(0).outcome.unwrap();

    let provider = DriverSessionStores::new(Arc::clone(&backend), Scope::local()).unwrap();
    let default = provider.try_for_agent(DEFAULT_AGENT).unwrap();
    assert!(default.transcripts.root_for_thread("thread-9").is_some());
}

#[test]
fn a_stem_names_the_agent_it_was_written_by() {
    let known = known_agents();
    let stem = session_stem(&SessionRef::scoped("thread-1", "orchestrator"));
    assert_eq!(agent_of(&stem, &known).as_deref(), Some("orchestrator"));
    // A sub-agent transcript belongs to its root's agent.
    assert_eq!(
        agent_of(&format!("{stem}__child.x"), &known).as_deref(),
        Some("orchestrator")
    );
    assert_eq!(agent_of("plain", &known), None);
}
