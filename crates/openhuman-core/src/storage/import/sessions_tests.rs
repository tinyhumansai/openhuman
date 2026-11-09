#![cfg(feature = "storage-sqlite")]

use super::*;
use crate::storage::rusqlite;
use crate::storage::test_support::{sqlite_backend, SqliteBackend};
use crate::util::classic_transcript_fixture::write_classic;
use sha2::Digest as _;
use std::collections::BTreeMap;
use std::ops::Deref;
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

struct TestBackend(SqliteBackend);

impl Deref for TestBackend {
    type Target = Arc<dyn StorageBackend>;

    fn deref(&self) -> &Self::Target {
        &self.0.backend
    }
}

fn backend() -> TestBackend {
    TestBackend(sqlite_backend().unwrap())
}

fn session_provider(backend: &Arc<dyn StorageBackend>) -> DriverSessionStores {
    let backend = Arc::clone(backend);
    let factory: tinyagents_session::port::AgentStorageFactory = Arc::new(move |agent_id| {
        let scope = crate::storage::scope_for_agent(agent_id);
        let storage = backend.for_scope(&scope)?;
        Ok(tinyagents_storage_kit::ScopedStorage::new(
            storage,
            (*scope).clone(),
        ))
    });
    DriverSessionStores::new(factory).unwrap()
}

#[test]
fn host_jsonl_reader_preserves_meta_usage_typed_rows_and_tool_state() {
    use tinyagents_session::transcript::DisplayRecord;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("thread.orchestrator.jsonl");
    std::fs::write(
        &path,
        concat!(
            "{\"_meta\":{\"agent\":\"Orchestrator\",\"agent_id\":\"orchestrator\",\"dispatcher\":\"native\",\"created\":\"2026-01-01T00:00:00Z\",\"updated\":\"2026-01-02T00:00:00Z\",\"turn_count\":2,\"input_tokens\":7,\"output_tokens\":3,\"cached_input_tokens\":1,\"charged_amount_usd\":0.25,\"thread_id\":\"thread\"}}\n",
            "{\"kind\":\"tools\",\"tools\":[{\"name\":\"lookup\"}]}\n",
            "{\"role\":\"assistant\",\"content\":\"answer\",\"provider\":\"local\",\"model\":\"m1\",\"usage\":{\"input\":7,\"output\":3,\"cached_input\":1,\"cost_usd\":0.25},\"ts\":\"2026-01-02T00:00:00Z\",\"request_id\":\"r1\",\"iteration\":2}\n",
            "{\"role\":\"tool\",\"content\":\"{\\\"tool_call_id\\\":\\\"call-1\\\",\\\"content\\\":\\\"old envelope\\\"}\"}\n",
            "{\"role\":\"assistant\",\"content\":\"incomplete usage\",\"usage\":{\"input\":7,\"output\":3,\"cached_input\":1,\"cost_usd\":0.25}}\n"
        ),
    )
    .unwrap();

    let (meta, tools, records) = super::decode_transcript(&path).unwrap();
    assert_eq!(meta.agent_name, "Orchestrator");
    assert_eq!(meta.input_tokens, 7);
    assert_eq!(tools.unwrap()[0]["name"], "lookup");
    let DisplayRecord::Message(answer) = &records[0] else {
        panic!("expected message record");
    };
    assert_eq!(answer.request_id.as_deref(), Some("r1"));
    assert_eq!(answer.message.turn_usage.as_ref().unwrap().iteration, 2);
    let DisplayRecord::Message(row) = &records[1] else {
        panic!("expected typed row");
    };
    assert_eq!(row.message.tool_call_id.as_deref(), Some("call-1"));
    let DisplayRecord::Message(incomplete) = &records[2] else {
        panic!("expected the assistant row with incomplete usage metadata");
    };
    assert!(incomplete.turn_usage.is_none());
}

#[test]
fn transcript_replay_keeps_background_provenance_for_messages_and_compaction() {
    use tinyagents_session::transcript::{BackgroundOrigin, DisplayRecord};

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("thread.orchestrator.jsonl");
    let meta = serde_json::to_value(meta("thread", "orchestrator")).unwrap();
    let background_row = |content: &str, key: &str| {
        serde_json::json!({
            "role": "assistant",
            "content": content,
            "request_id": key,
            "background": {
                "idempotency_key": key,
                "provenance": { "kind": "cron", "job": "daily" }
            }
        })
    };
    let first = background_row("scheduled result", "run-1");
    let compacted = background_row("compacted scheduled result", "run-2");
    let contents = [
        serde_json::json!({"_meta": meta}),
        first,
        serde_json::json!({
            "role": "tool",
            "content": "tool failed",
            "tool_call_id": "call-1",
            "failure": true,
            "failure_detail": "provider refused"
        }),
        serde_json::json!({"kind": "compaction", "replacement": [compacted]}),
    ];
    std::fs::write(
        &path,
        contents
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();

    let (meta, tools, records) = super::decode_transcript(&path).unwrap();
    let backend = backend();
    let stores = session_provider(&backend)
        .try_for_agent("orchestrator")
        .unwrap();
    let history = stores
        .transcripts
        .open_stem("thread.orchestrator", meta.clone())
        .unwrap();
    super::replay_transcript(history.as_ref(), &meta, tools.as_ref(), records).unwrap();

    let display = history.read_display().unwrap().unwrap();
    let DisplayRecord::Message(first) = &display.records[0] else {
        panic!("expected the imported background message");
    };
    assert_eq!(
        first.background,
        Some(BackgroundOrigin {
            idempotency_key: "run-1".into(),
            provenance: serde_json::json!({ "kind": "cron", "job": "daily" }),
        })
    );
    let DisplayRecord::Message(tool_result) = &display.records[1] else {
        panic!("expected the imported failed tool result");
    };
    assert_eq!(
        tool_result.message.tool_failure,
        Some(tinyagents_session::transcript::ToolFailure {
            failed: true,
            detail: Some("provider refused".into()),
        })
    );
    let DisplayRecord::Compaction(compaction) = &display.records[2] else {
        panic!("expected the imported compaction record");
    };
    assert_eq!(
        compaction.replacement[0].background,
        Some(BackgroundOrigin {
            idempotency_key: "run-2".into(),
            provenance: serde_json::json!({ "kind": "cron", "job": "daily" }),
        })
    );
    let logical = history.messages().unwrap();
    assert_eq!(logical[0].request_id.as_deref(), Some("run-2"));
    assert!(logical[0].preserve_request_id);
}

#[test]
fn profile_phase_out_rewrites_imported_bytes_without_mutating_legacy_source() {
    use tinyagents_session::transcript::DisplayRecord;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("thread.orchestrator.jsonl");
    let prompt = "## Identity\n\nhello\n\n### PROFILE.md\n\nprivate\n\n### Tools\n\nshell\n";
    write_classic(
        &path,
        &[
            TranscriptMessage::system(prompt),
            TranscriptMessage::system("a later system message mentioning PROFILE.md stays intact"),
            TranscriptMessage::user("question"),
        ],
        &TranscriptMeta {
            agent_name: "Orchestrator".into(),
            agent_id: Some("orchestrator".into()),
            agent_type: None,
            dispatcher: "native".into(),
            provider: None,
            model: None,
            created: "2026-01-01T00:00:00Z".into(),
            updated: "2026-01-01T00:00:00Z".into(),
            turn_count: 1,
            prefix_message_count: None,
            prefix_reset: false,
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            charged_amount_usd: 0.0,
            thread_id: Some("thread".into()),
            task_id: None,
            session_id: None,
            parent_session_id: None,
        },
    );
    let source_before = std::fs::read(&path).unwrap();

    let (_, _, records) = decode_transcript(&path).unwrap();

    let DisplayRecord::Message(system) = &records[0] else {
        panic!("expected imported system message");
    };
    assert!(!system.message.content.contains("PROFILE.md"));
    assert!(!system.message.content.contains("private"));
    assert!(system.message.content.contains("### Tools"));
    let DisplayRecord::Message(later_system) = &records[1] else {
        panic!("expected the second system message");
    };
    assert!(later_system.message.content.contains("PROFILE.md"));
    assert_eq!(std::fs::read(&path).unwrap(), source_before);
}

#[test]
fn profile_phase_out_sanitizes_imported_sqlite_system_rows_only() {
    let timestamp = chrono::DateTime::from_timestamp_millis(1_700_000_000_000).unwrap();
    let mut messages = vec![
        tinyagents_session::types::SessionMessage {
            id: 7,
            session_id: "session".into(),
            role: "system".into(),
            content: "## Identity\n\n### PROFILE.md\n\nprivate\n\n### Tools\n\nshell".into(),
            reasoning_content: None,
            model: None,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            created_at: timestamp,
        },
        tinyagents_session::types::SessionMessage {
            id: 8,
            session_id: "session".into(),
            role: "user".into(),
            content: "please keep this PROFILE.md mention".into(),
            reasoning_content: None,
            model: None,
            input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            created_at: timestamp,
        },
    ];

    sanitize_legacy_system_messages(&mut messages);

    assert_eq!(messages[0].id, 7);
    assert!(!messages[0].content.contains("PROFILE.md"));
    assert!(messages[0].content.contains("### Tools"));
    assert_eq!(messages[1].id, 8);
    assert!(messages[1].content.contains("PROFILE.md"));
}

#[cfg(unix)]
#[test]
fn session_file_discovery_never_follows_symlinks_outside_legacy_roots() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().unwrap();
    let workspace = dir.path().join("workspace");
    let raw = workspace.join("session_raw");
    let turns = workspace.join("turn_states");
    std::fs::create_dir_all(&raw).unwrap();
    std::fs::create_dir_all(&turns).unwrap();
    let external = dir.path().join("outside.jsonl");
    std::fs::write(&external, "{}\n").unwrap();
    symlink(&external, raw.join("external.jsonl")).unwrap();
    let outside_dir = dir.path().join("outside-dir");
    std::fs::create_dir(&outside_dir).unwrap();
    std::fs::write(outside_dir.join("thread.jsonl"), "{}\n").unwrap();
    let linked_raw = workspace.join("linked-session-raw");
    symlink(&outside_dir, &linked_raw).unwrap();

    let external_turn = dir.path().join("outside.json");
    std::fs::write(&external_turn, "{}").unwrap();
    let turn_dir = turns.join("thread");
    std::fs::create_dir(&turn_dir).unwrap();
    symlink(&external_turn, turn_dir.join("external.json")).unwrap();
    let linked_turns = workspace.join("linked-turn-states");
    symlink(&outside_dir, &linked_turns).unwrap();

    assert!(transcript_files(&raw).unwrap().is_empty());
    assert!(turn_state_files(&turns).unwrap().is_empty());
    assert!(transcript_files(&linked_raw).is_err());
    assert!(turn_state_files(&linked_turns).is_err());
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
        prefix_reset: false,
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
         VALUES ('sess-1', 'orchestrator', 'Orchestrator', 'thread-1', '2026-01-01T00:00:00Z');
         CREATE TABLE session_messages (
            id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL,
            content TEXT NOT NULL, reasoning_content TEXT, model TEXT,
            input_tokens INTEGER, output_tokens INTEGER, cost_usd REAL,
            created_at TEXT NOT NULL);
         INSERT INTO session_messages (id, session_id, role, content, created_at)
         VALUES (37, 'sess-1', 'user', 'preserved row', '2026-01-02T03:04:05Z');
         CREATE TABLE session_tool_calls (
            id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, message_id INTEGER,
            tool_name TEXT NOT NULL, tool_input TEXT, tool_output TEXT,
            status TEXT NOT NULL, duration_ms INTEGER, created_at TEXT NOT NULL);
         INSERT INTO session_tool_calls (id, session_id, message_id, tool_name, status, created_at)
         VALUES (9, 'sess-1', 37, 'lookup', 'completed', '2026-01-02T03:05:06Z');",
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

    let provider = session_provider(&backend);
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
    let ledger = provider.driver_run_ledger(DEFAULT_AGENT).unwrap();
    let (messages, calls) = crate::storage::block_on_anyhow(async move {
        use tinyagents_session::run_ledger::history::SessionHistoryStore;
        Ok::<_, anyhow::Error>((
            ledger.list_messages("sess-1", None).await?,
            ledger.list_tool_calls("sess-1", None).await?,
        ))
    })
    .unwrap();
    assert_eq!(messages[0].id, 37);
    assert_eq!(
        messages[0].created_at.to_rfc3339(),
        "2026-01-02T03:04:05+00:00"
    );
    assert_eq!(calls[0].id, 9);
    assert_eq!(calls[0].message_id, Some(37));
    assert_eq!(
        calls[0].created_at.to_rfc3339(),
        "2026-01-02T03:05:06+00:00"
    );
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

    let provider = session_provider(&backend);
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

#[test]
fn database_import_totals_source_families_not_derived_fts_rows() {
    let counts = |read, imported| ImportCounts {
        read,
        imported,
        already_present: read - imported,
    };
    let counts = [
        counts(1, 1),
        counts(2, 1),
        counts(3, 2),
        counts(4, 3),
        counts(5, 4),
        counts(6, 5),
        counts(7, 6),
        counts(8, 7),
        counts(9, 8),
        counts(10, 9),
        counts(11, 10),
    ];
    assert_eq!(database_totals(counts), (66, 56));
}
