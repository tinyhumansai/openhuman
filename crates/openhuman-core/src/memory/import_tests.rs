use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in};

#[test]
fn the_state_round_trips_through_the_store() {
    let tmp = tempfile::tempdir().unwrap();
    let store = StateKvCheckpointStore::new(tmp.path());
    assert_eq!(
        store.load(),
        ImportFile::default(),
        "no file reads as fresh"
    );
    let mut file = ImportFile::default();
    file.state.phase = ImportPhase::Done;
    file.state.imported = 7;
    file.listed_unconfirmed = true;
    store.save(&file).unwrap();
    assert_eq!(store.load(), file);
    assert!(listed_unconfirmed(tmp.path()));
}

#[test]
fn a_damaged_state_file_reads_as_a_fresh_start() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("memory/import_state.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{ not json").unwrap();
    assert_eq!(read_file(tmp.path()), ImportFile::default());
}

#[test]
fn a_running_state_with_no_live_run_reports_interrupted() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let mut file = ImportFile::default();
    file.state.phase = ImportPhase::Running;
    StateKvCheckpointStore::new(&config.workspace_dir)
        .save(&file)
        .unwrap();
    let state = status(&config);
    assert_eq!(state.phase, ImportPhase::Error);
    assert!(state.error.unwrap().contains("interrupted"));
    assert!(
        !in_progress(&config),
        "a stale persisted Running phase is not a live import task"
    );
}

#[test]
fn a_credits_pause_blocks_organizing_until_resumed() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let mut file = ImportFile::default();
    file.state.phase = ImportPhase::Error;
    file.paused_for_credits = true;
    StateKvCheckpointStore::new(&config.workspace_dir)
        .save(&file)
        .unwrap();
    assert!(in_progress(&config));
}

#[tokio::test]
async fn starting_without_consent_is_refused_before_anything_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let error = start(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        false,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("consent"), "{error}");
    assert!(!is_running(&config.workspace_dir));
    assert_eq!(status(&config).phase, ImportPhase::Idle);
}

#[tokio::test]
async fn retrying_with_nothing_failed_is_refused_and_releases_the_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    // Either no engine is bound here or there is nothing to retry; either
    // way the call fails and the workspace must not stay claimed.
    assert!(
        retry_failed(&crate::core::runtime::DispatchCtx::ambient(), &config)
            .await
            .is_err()
    );
    assert!(!is_running(&config.workspace_dir));
}

#[tokio::test]
async fn nothing_resumes_when_no_import_was_started() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let resumed = resume_interrupted_with(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Arc::new(|| false),
        Arc::new(|_: &Config| true),
    )
    .await;
    assert!(!resumed);
}

#[tokio::test]
async fn a_paused_scheduler_leaves_an_interrupted_import_for_later() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let mut file = ImportFile::default();
    file.state.phase = ImportPhase::Running;
    StateKvCheckpointStore::new(&config.workspace_dir)
        .save(&file)
        .unwrap();
    let resumed = resume_interrupted_with(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Arc::new(|| true),
        Arc::new(|_: &Config| true),
    )
    .await;
    assert!(!resumed);
    assert_eq!(
        read_file(&config.workspace_dir).state.phase,
        ImportPhase::Running
    );
}

// Saves layout-migration state, which needs a backend that can claim imports
// (see `memory/layout_migration/ops.rs`).
#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn a_live_import_blocks_migration_and_does_not_make_a_stale_migration_look_live() {
    use crate::memory::layout_migration::{self, state, test_host::FakeHost, Phase, Trigger};

    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let host = Arc::new(FakeHost::with(2, &config).await);
    let workspace = config.workspace_dir.clone();
    let _reservation = crate::memory::run_slots::reserve(
        &crate::memory::run_slots::IMPORT,
        &workspace,
        &[
            &crate::memory::run_slots::BACKFILL,
            &crate::memory::run_slots::MIGRATION,
        ],
    )
    .expect("the test owns a live import slot");

    let mut stale = state::load(&workspace).unwrap();
    stale.phase = Phase::Copying;
    state::save(&workspace, &stale).unwrap();

    let migration_status = layout_migration::status(&config).unwrap();
    assert!(!migration_status.running);
    assert!(migration_status.interrupted);
    assert!(
        !layout_migration::start(
            &crate::core::runtime::DispatchCtx::ambient(),
            config.clone(),
            host,
            Trigger::Auto,
            Arc::new(|| false),
        ),
        "an active import prevents migration from reserving the workspace"
    );
    assert!(!layout_migration::ops::is_running(&config));
}

#[tokio::test]
async fn a_credit_paused_import_waits_for_billing_and_records_a_resume_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let mut file = ImportFile::default();
    file.state.phase = ImportPhase::Error;
    file.paused_for_credits = true;
    StateKvCheckpointStore::new(&config.workspace_dir)
        .save(&file)
        .unwrap();

    assert!(
        !resume_interrupted_with(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            Arc::new(|| false),
            Arc::new(|_: &Config| false),
        )
        .await
    );
    assert_eq!(read_file(&config.workspace_dir), file);

    assert!(
        !resume_interrupted_with(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            Arc::new(|| false),
            Arc::new(|_: &Config| true),
        )
        .await,
        "a permitted resume still reports failure when no engine can be bound"
    );
    let stopped = read_file(&config.workspace_dir);
    assert_eq!(stopped.state.phase, ImportPhase::Error);
    assert!(stopped
        .state
        .error
        .as_deref()
        .unwrap()
        .contains("could not resume"));
    assert!(!stopped.paused_for_credits);
    assert!(!is_running(&config.workspace_dir));
}

#[test]
fn a_store_with_no_legacy_files_counts_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(tinymemory_integrations::import::count_legacy(tmp.path()).is_none());
}

#[tokio::test]
async fn resume_with_live_import_slot_does_not_claim_a_new_run() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let mut file = ImportFile::default();
    file.state.phase = ImportPhase::Running;
    let store = StateKvCheckpointStore::new(&config.workspace_dir);
    store.save(&file).unwrap();
    let guard = crate::memory::run_slots::reserve(
        &crate::memory::run_slots::IMPORT,
        &config.workspace_dir,
        &[
            &crate::memory::run_slots::BACKFILL,
            &crate::memory::run_slots::MIGRATION,
        ],
    )
    .expect("the live import owns the slot");

    let resumed = resume_interrupted_with(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Arc::new(|| false),
        Arc::new(|_: &Config| true),
    )
    .await;

    assert_eq!(store.load(), file, "a live run leaves its checkpoint alone");
    assert!(!resumed, "no new run was scheduled");
    drop(guard);
}

#[tokio::test]
async fn starting_import_or_retry_while_a_peer_driver_runs_gives_mode_specific_guidance() {
    #[derive(Clone, Copy)]
    enum Peer {
        Migration,
        Backfill,
    }

    for peer in [Peer::Migration, Peer::Backfill] {
        for retry in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let config = config_in(&tmp);
            bind_reference(&config);
            let workspace = config.workspace_dir.clone();
            let _peer_guard = match peer {
                Peer::Migration => crate::memory::run_slots::reserve(
                    &crate::memory::run_slots::MIGRATION,
                    &workspace,
                    &[
                        &crate::memory::run_slots::IMPORT,
                        &crate::memory::run_slots::BACKFILL,
                    ],
                ),
                Peer::Backfill => crate::memory::run_slots::reserve(
                    &crate::memory::run_slots::BACKFILL,
                    &workspace,
                    &[
                        &crate::memory::run_slots::IMPORT,
                        &crate::memory::run_slots::MIGRATION,
                    ],
                ),
            }
            .expect("peer owns its driver-specific slot");

            let error = if retry {
                retry_failed(&crate::core::runtime::DispatchCtx::ambient(), &config)
                    .await
                    .unwrap_err()
            } else {
                start(&crate::core::runtime::DispatchCtx::ambient(), &config, true)
                    .await
                    .unwrap_err()
            };
            assert_eq!(error.code(), "INVALID_REQUEST");
            let action = if retry { "retry" } else { "import" };
            let reason = match peer {
                Peer::Migration => {
                    format!("memory is being organized; {action} once that finishes")
                }
                Peer::Backfill => {
                    format!("a conversation backfill is running; {action} once it finishes")
                }
            };
            assert!(error.to_string().contains(&reason), "{error}");
            assert!(!is_running(&workspace));
            assert_eq!(read_file(&workspace), ImportFile::default());
        }
    }
}

const HOST_LEGACY_MEMORY_DDL: &str = r#"
CREATE TABLE memory_docs (
  document_id TEXT PRIMARY KEY, namespace TEXT NOT NULL, key TEXT NOT NULL, title TEXT NOT NULL,
  content TEXT NOT NULL, source_type TEXT NOT NULL, priority TEXT NOT NULL, tags_json TEXT NOT NULL,
  metadata_json TEXT NOT NULL, category TEXT NOT NULL, session_id TEXT, created_at REAL NOT NULL,
  updated_at REAL NOT NULL, markdown_rel_path TEXT NOT NULL, taint TEXT NOT NULL DEFAULT 'internal',
  logical_namespace TEXT, UNIQUE(namespace, key));
CREATE TABLE kv_global (key TEXT PRIMARY KEY, value_json TEXT NOT NULL, updated_at REAL NOT NULL);
CREATE TABLE kv_namespace (namespace TEXT NOT NULL, key TEXT NOT NULL, value_json TEXT NOT NULL, updated_at REAL NOT NULL,
  PRIMARY KEY(namespace, key));
CREATE TABLE episodic_log (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, timestamp REAL NOT NULL,
  role TEXT NOT NULL, content TEXT NOT NULL, lesson TEXT, tool_calls_json TEXT, cost_microdollars INTEGER DEFAULT 0);
CREATE TABLE user_profile (facet_id TEXT PRIMARY KEY, facet_type TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
  confidence REAL NOT NULL DEFAULT 0.5, evidence_count INTEGER NOT NULL DEFAULT 1, source_segment_ids TEXT,
  first_seen_at REAL NOT NULL, last_seen_at REAL NOT NULL, state TEXT NOT NULL DEFAULT 'active',
  stability REAL NOT NULL DEFAULT 0.0, user_state TEXT NOT NULL DEFAULT 'auto', evidence_refs_json TEXT,
  class TEXT, cue_families_json TEXT, UNIQUE(facet_type, key));
"#;

#[cfg(feature = "legacy-import")]
fn legacy_document(config: &Config, id: &str, title: &str, body: &str) {
    let db = config.workspace_dir.join("memory/memory.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let conn = crate::storage::rusqlite::Connection::open(db).unwrap();
    conn.execute_batch(HOST_LEGACY_MEMORY_DDL).unwrap();
    conn.execute(
        "INSERT INTO memory_docs (document_id, namespace, key, title, content, source_type, priority,
           tags_json, metadata_json, category, session_id, created_at, updated_at, markdown_rel_path,
           taint, logical_namespace)
         VALUES (?1, 'document_notes', ?1, ?2, ?3, 'chat', 'normal', '[]', '{}', 'core', NULL,
                 1700000000.0, 1700000000.0, '', 'internal', NULL)",
        crate::storage::rusqlite::params![id, title, body],
    )
    .unwrap();
}

async fn wait_for_host_import(config: &Config) -> ImportState {
    for _ in 0..100_000 {
        let state = status(config);
        if state.phase == ImportPhase::Done && !is_running(&config.workspace_dir) {
            return state;
        }
        tokio::task::yield_now().await;
    }
    panic!("the Host import did not finish and release its slot");
}

async fn assert_reference_contains(
    engine: &tinymemory_api::conformance::ReferenceEngine,
    text: &str,
) {
    use tinymemory_api::{ItemKind, MetaFilter};

    let items = crate::memory::test_fixtures::stored(
        engine,
        MetaFilter {
            kinds: vec![ItemKind::Document],
            ..MetaFilter::default()
        },
    )
    .await;
    assert!(
        items.iter().any(|item| item.text.contains(text)),
        "{items:?}"
    );
}

#[cfg(feature = "legacy-import")]
#[tokio::test]
async fn a_real_legacy_document_imports_through_the_host_adapter() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    legacy_document(
        &config,
        "host-import-doc",
        "Host import",
        "real legacy body",
    );

    let started = start(&crate::core::runtime::DispatchCtx::ambient(), &config, true)
        .await
        .unwrap();
    assert_eq!(started.phase, ImportPhase::Running);
    assert_eq!(started.total, 1);

    let done = wait_for_host_import(&config).await;
    assert_eq!(done.imported, 1);
    assert_eq!(done.failed, 0);
    let file = StateKvCheckpointStore::new(&config.workspace_dir).load();
    assert_eq!(
        file.checkpoint.documents.as_deref(),
        Some("host-import-doc")
    );
    assert!(file.failed.is_empty());
    assert_reference_contains(&engine, "real legacy body").await;
}

#[cfg(feature = "legacy-import")]
#[tokio::test]
async fn an_interrupted_import_resumes_through_the_host_adapter() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    legacy_document(
        &config,
        "host-resume-doc",
        "Host resume",
        "resumed legacy body",
    );
    let mut interrupted = ImportFile::default();
    interrupted.state.phase = ImportPhase::Running;
    interrupted.state.total = 1;
    StateKvCheckpointStore::new(&config.workspace_dir)
        .save(&interrupted)
        .unwrap();

    assert!(
        resume_interrupted_with(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            Arc::new(|| false),
            Arc::new(|_: &Config| true),
        )
        .await
    );

    let done = wait_for_host_import(&config).await;
    assert_eq!(done.imported, 1);
    assert_eq!(done.failed, 0);
    let file = StateKvCheckpointStore::new(&config.workspace_dir).load();
    assert_eq!(
        file.checkpoint.documents.as_deref(),
        Some("host-resume-doc")
    );
    assert_reference_contains(&engine, "resumed legacy body").await;
}

#[cfg(feature = "legacy-import")]
#[tokio::test]
async fn retry_failed_reimports_a_real_legacy_row_through_the_host_adapter() {
    use tinymemory_integrations::import::FailedItem;

    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    legacy_document(
        &config,
        "host-retry-doc",
        "Host retry",
        "retried legacy body",
    );
    let mut retry_state = ImportFile::default();
    retry_state.state.phase = ImportPhase::Done;
    retry_state.state.total = 1;
    retry_state.state.failed = 1;
    retry_state.failed.push(FailedItem {
        id: "memory_docs:host-retry-doc".to_string(),
        reason: "the prior engine refused this row".to_string(),
    });
    StateKvCheckpointStore::new(&config.workspace_dir)
        .save(&retry_state)
        .unwrap();

    let started = retry_failed(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .await
        .unwrap();
    assert_eq!(started.phase, ImportPhase::Running);
    let done = wait_for_host_import(&config).await;
    assert_eq!(done.failed, 0);
    assert_eq!(done.imported, 1);
    let file = StateKvCheckpointStore::new(&config.workspace_dir).load();
    assert!(file.failed.is_empty());
    assert!(!file.retrying);
    assert_reference_contains(&engine, "retried legacy body").await;
}

#[cfg(feature = "legacy-import")]
#[tokio::test]
async fn an_interrupted_retry_resumes_its_real_legacy_row() {
    use tinymemory_integrations::import::FailedItem;

    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    legacy_document(
        &config,
        "host-retry-resume-doc",
        "Host retry resume",
        "resumed retry body",
    );
    let mut interrupted = ImportFile::default();
    interrupted.state.phase = ImportPhase::Running;
    interrupted.state.total = 1;
    interrupted.state.failed = 1;
    interrupted.retrying = true;
    interrupted.failed.push(FailedItem {
        id: "memory_docs:host-retry-resume-doc".to_string(),
        reason: "the prior retry was interrupted".to_string(),
    });
    StateKvCheckpointStore::new(&config.workspace_dir)
        .save(&interrupted)
        .unwrap();

    assert!(
        resume_interrupted_with(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            Arc::new(|| false),
            Arc::new(|_: &Config| true),
        )
        .await
    );

    let done = wait_for_host_import(&config).await;
    assert_eq!(done.failed, 0);
    assert_eq!(done.imported, 1);
    let file = StateKvCheckpointStore::new(&config.workspace_dir).load();
    assert!(file.failed.is_empty());
    assert!(!file.retrying);
    assert_reference_contains(&engine, "resumed retry body").await;
}

#[cfg(feature = "legacy-import")]
#[tokio::test]
async fn peer_slot_contention_preserves_interrupted_import_for_later_resume() {
    #[derive(Clone, Copy)]
    enum Peer {
        Migration,
        Backfill,
    }

    for peer in [Peer::Migration, Peer::Backfill] {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_in(&tmp);
        let engine = bind_reference(&config);
        legacy_document(
            &config,
            "resume-after-peer-slot",
            "Resume after peer slot",
            "legacy body resumed after peer slot",
        );
        let workspace = config.workspace_dir.clone();
        let peer_guard = match peer {
            Peer::Migration => crate::memory::run_slots::reserve(
                &crate::memory::run_slots::MIGRATION,
                &workspace,
                &[
                    &crate::memory::run_slots::IMPORT,
                    &crate::memory::run_slots::BACKFILL,
                ],
            ),
            Peer::Backfill => crate::memory::run_slots::reserve(
                &crate::memory::run_slots::BACKFILL,
                &workspace,
                &[
                    &crate::memory::run_slots::IMPORT,
                    &crate::memory::run_slots::MIGRATION,
                ],
            ),
        }
        .expect("peer owns its real driver slot");
        let mut interrupted = ImportFile::default();
        interrupted.state.phase = ImportPhase::Running;
        interrupted.state.total = 1;
        let store = StateKvCheckpointStore::new(&workspace);
        store.save(&interrupted).unwrap();

        assert!(
            !resume_interrupted_with(
                &crate::core::runtime::DispatchCtx::ambient(),
                &config,
                Arc::new(|| false),
                Arc::new(|_: &Config| true),
            )
            .await,
            "peer contention must defer this resume"
        );
        assert_eq!(
            store.load(),
            interrupted,
            "peer contention must leave the resumable checkpoint unchanged"
        );

        drop(peer_guard);
        assert!(
            resume_interrupted_with(
                &crate::core::runtime::DispatchCtx::ambient(),
                &config,
                Arc::new(|| false),
                Arc::new(|_: &Config| true),
            )
            .await,
            "the import resumes once the peer slot is released"
        );
        let done = wait_for_host_import(&config).await;
        assert_eq!(done.imported, 1);
        assert_eq!(done.failed, 0);
        let final_file = store.load();
        assert_eq!(final_file.state.phase, ImportPhase::Done);
        assert_eq!(
            final_file.checkpoint.documents.as_deref(),
            Some("resume-after-peer-slot")
        );
        assert_reference_contains(&engine, "legacy body resumed after peer slot").await;
    }
}
