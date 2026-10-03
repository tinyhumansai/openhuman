use super::*;
use crate::memory::error::{INVALID_REQUEST, MEMORY_OFF};
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use rusqlite::{params, Connection};
use tinymemory::MetaFilter;

/// An early v1 `memory.db` (no optional columns), enough for the importer.
const LEGACY_DDL: &str = "
CREATE TABLE memory_docs (
  document_id TEXT PRIMARY KEY, namespace TEXT NOT NULL, key TEXT NOT NULL, title TEXT NOT NULL,
  content TEXT NOT NULL, source_type TEXT NOT NULL, priority TEXT NOT NULL, tags_json TEXT NOT NULL,
  metadata_json TEXT NOT NULL, category TEXT NOT NULL, session_id TEXT, created_at REAL NOT NULL,
  updated_at REAL NOT NULL, markdown_rel_path TEXT NOT NULL, UNIQUE(namespace, key));
CREATE TABLE episodic_log (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, timestamp REAL NOT NULL,
  role TEXT NOT NULL, content TEXT NOT NULL, lesson TEXT);
CREATE TABLE user_profile (facet_id TEXT PRIMARY KEY, facet_type TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
  confidence REAL NOT NULL DEFAULT 0.5, evidence_count INTEGER NOT NULL DEFAULT 1, source_segment_ids TEXT,
  first_seen_at REAL NOT NULL, last_seen_at REAL NOT NULL);
";

fn memory_doc(conn: &Connection, id: &str, namespace: &str, title: &str, content: &str) {
    conn.execute(
        "INSERT INTO memory_docs (document_id, namespace, key, title, content, source_type, priority,
           tags_json, metadata_json, category, created_at, updated_at, markdown_rel_path)
         VALUES (?1, ?2, ?1, ?3, ?4, 'chat', 'normal', '[]', '{}', 'core', 1700000000.0, 1700000000.0, '')",
        params![id, namespace, title, content],
    )
    .unwrap();
}

/// A v1 workspace: two documents, one learning, one conversation, one facet.
fn legacy_workspace(workspace_dir: &Path) {
    std::fs::create_dir_all(workspace_dir.join("memory")).unwrap();
    let conn = Connection::open(workspace_dir.join("memory").join("memory.db")).unwrap();
    conn.execute_batch(LEGACY_DDL).unwrap();
    memory_doc(&conn, "d1", "notes", "Plan", "Ship memory v2 on Friday");
    memory_doc(&conn, "d2", "notes", "Ideas", "Try oolong tea");
    memory_doc(&conn, "d3", "learning:style", "Style", "Keep answers terse");
    for (n, (role, text)) in [("user", "hi there"), ("assistant", "hello")]
        .iter()
        .enumerate()
    {
        conn.execute(
            "INSERT INTO episodic_log (session_id, timestamp, role, content) VALUES ('s1', ?1, ?2, ?3)",
            params![1_700_000_000.0 + n as f64, role, text],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO user_profile (facet_id, facet_type, key, value, confidence, first_seen_at, last_seen_at)
         VALUES ('f1', 'preference', 'tone', 'terse', 0.9, 1700000000.0, 1700000000.0)",
        [],
    )
    .unwrap();
}

async fn wait_until_settled(config: &Config) -> ImportState {
    for _ in 0..400 {
        let state = status(config);
        if state.phase != ImportPhase::Running {
            return state;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("import never settled");
}

#[tokio::test]
async fn scan_reports_no_store_for_a_fresh_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let view = scan(&config).await.unwrap();
    assert!(!view.found);
    assert!(view.counts.is_none());
    assert!(count_legacy(&config.workspace_dir).is_none());
}

#[tokio::test]
async fn scan_counts_what_a_legacy_store_holds() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    legacy_workspace(&config.workspace_dir);
    let view = scan(&config).await.unwrap();
    assert!(view.found);
    let counts = view.counts.expect("counts");
    assert_eq!(counts.documents, 2);
    assert_eq!(counts.conversations, 1);
    assert_eq!(counts.learnings, 2, "a learning doc and a profile facet");
}

#[test]
fn status_of_an_untouched_workspace_is_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(status(&config), ImportState::default());
}

#[tokio::test]
async fn start_is_refused_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    legacy_workspace(&config.workspace_dir);
    bind_reference(&config);
    let error = start(&config, false).await.unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);
    assert!(error.to_string().contains("consent"));
    assert_eq!(status(&config).phase, ImportPhase::Idle, "nothing started");
}

#[tokio::test]
async fn start_needs_memory_on_and_a_legacy_store() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    legacy_workspace(&config.workspace_dir);
    assert_eq!(start(&config, true).await.unwrap_err().code(), MEMORY_OFF);

    let tmp2 = tempfile::tempdir().unwrap();
    let empty = config_in(&tmp2);
    bind_reference(&empty);
    for _ in 0..2 {
        // The claim is released, so asking again gives the same answer rather
        // than a stale "already running" status.
        assert_eq!(
            start(&empty, true).await.unwrap_err().code(),
            INVALID_REQUEST
        );
    }
}

#[tokio::test]
async fn a_full_import_stores_every_item_and_finishes_done() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    legacy_workspace(&config.workspace_dir);
    let engine = bind_reference(&config);

    let started = start(&config, true).await.unwrap();
    assert_eq!(started.phase, ImportPhase::Running);
    assert_eq!(started.total, 5);

    let done = wait_until_settled(&config).await;
    assert_eq!(done.phase, ImportPhase::Done, "{done:?}");
    assert_eq!(done.imported, 5);
    assert_eq!(done.total, 5);
    assert!(done.error.is_none());
    let items = stored(&engine, MetaFilter::default()).await;
    assert_eq!(items.len(), 5);
    assert!(items
        .iter()
        .all(|item| item.meta.source.kind == tinymemory::SourceKind::Import));
    assert!(file_path(&config.workspace_dir).exists());
}

#[tokio::test]
async fn an_interrupted_import_resumes_from_its_checkpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    legacy_workspace(&config.workspace_dir);
    let engine = bind_reference(&config);

    // A previous run stored d1 and stopped on an error.
    write_file(
        &config.workspace_dir,
        &ImportFile {
            state: ImportState {
                phase: ImportPhase::Error,
                imported: 1,
                total: 5,
                error: Some("unauthorized: sign in".into()),
            },
            checkpoint: Checkpoint {
                documents: Some("d1".into()),
                ..Checkpoint::default()
            },
        },
    );
    assert_eq!(status(&config).phase, ImportPhase::Error);

    start(&config, true).await.unwrap();
    let done = wait_until_settled(&config).await;
    assert_eq!(done.phase, ImportPhase::Done, "{done:?}");
    assert_eq!(done.imported, 5, "the earlier item counts toward the total");
    let items = stored(&engine, MetaFilter::default()).await;
    assert_eq!(items.len(), 4, "d1 is not sent again");
    assert!(
        !items
            .iter()
            .any(|item| item.text.contains("Ship memory v2")),
        "the checkpointed document was skipped"
    );
}

#[test]
fn a_running_state_with_no_live_import_reads_as_interrupted() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    write_file(
        &config.workspace_dir,
        &ImportFile {
            state: ImportState {
                phase: ImportPhase::Running,
                imported: 3,
                total: 9,
                error: None,
            },
            checkpoint: Checkpoint::default(),
        },
    );
    let state = status(&config);
    assert_eq!(state.phase, ImportPhase::Error);
    assert_eq!(state.imported, 3);
    assert!(state.error.as_deref().unwrap().contains("start it again"));
}

#[tokio::test]
async fn a_finished_import_starts_over_when_run_again() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    legacy_workspace(&config.workspace_dir);
    bind_reference(&config);
    start(&config, true).await.unwrap();
    assert_eq!(wait_until_settled(&config).await.phase, ImportPhase::Done);

    let again = start(&config, true).await.unwrap();
    assert_eq!(again.imported, 0, "Done does not resume; it restarts");
    assert_eq!(wait_until_settled(&config).await.imported, 5);
}

#[tokio::test]
async fn a_second_start_while_running_returns_the_current_status() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    legacy_workspace(&config.workspace_dir);
    bind_reference(&config);
    RUNNING.lock().unwrap().insert(config.workspace_dir.clone());
    let state = start(&config, true).await.unwrap();
    assert_eq!(state.phase, ImportPhase::Idle, "no second run was launched");
    RUNNING.lock().unwrap().remove(&config.workspace_dir);
}

#[test]
fn a_corrupt_import_file_reads_as_idle() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("memory")).unwrap();
    std::fs::write(file_path(tmp.path()), "garbage").unwrap();
    assert_eq!(read_file(tmp.path()).state, ImportState::default());
}
