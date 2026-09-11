use super::*;
use crate::openhuman::agent::task_board::TaskCardStatus;
use crate::openhuman::threads::todos::ops::{self, CardPatch};
use tempfile::tempdir;

fn thread_loc(dir: &Path, id: &str) -> BoardLocation {
    BoardLocation::Thread {
        workspace_dir: dir.to_path_buf(),
        thread_id: id.to_string(),
    }
}

/// Lowercase-hex encode a thread id, matching [`super::legacy_thread_id`]'s
/// decoder so test-built keys round-trip through the migration.
fn hex_key(id: &str) -> String {
    id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

#[tokio::test]
async fn create_and_list_run() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "run-test-1");

    let run = create_run(&loc, "run-1", "card-1", "default")
        .await
        .unwrap();
    assert_eq!(run.run_id, "run-1");
    assert_eq!(run.card_id, "card-1");
    assert_eq!(run.claimed_by, "default");
    assert!(run.is_active());
    assert!(!run.claim_token.is_empty());

    let all = list_runs(&loc, None).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(list_runs(&loc, Some("card-1")).await.unwrap().len(), 1);
    assert!(list_runs(&loc, Some("card-other"))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn timestamps_reach_the_wire_as_rfc3339() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "wire-test");
    let run = create_run(&loc, "run-1", "card-1", "default")
        .await
        .unwrap();

    // The RPC surface has always spoken RFC 3339; the crate stores millis.
    assert!(chrono::DateTime::parse_from_rfc3339(&run.started_at).is_ok());
    assert!(chrono::DateTime::parse_from_rfc3339(&run.last_heartbeat_at).is_ok());

    let done = complete_run(&loc, "run-1", RunOutcome::Success, None, vec![])
        .await
        .unwrap();
    let completed_at = done.completed_at.expect("completed stamp");
    assert!(chrono::DateTime::parse_from_rfc3339(&completed_at).is_ok());
}

#[tokio::test]
async fn heartbeat_updates_timestamp() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "hb-test-1");

    create_run(&loc, "run-hb", "card-1", "default")
        .await
        .unwrap();
    let before = get_run(&loc, "run-hb").await.unwrap().unwrap();

    update_heartbeat(&loc, "run-hb").await.unwrap();

    let after = get_run(&loc, "run-hb").await.unwrap().unwrap();
    assert!(after.last_heartbeat_at >= before.last_heartbeat_at);
}

#[tokio::test]
async fn heartbeat_fails_for_completed_run() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "hb-test-2");

    create_run(&loc, "run-done", "card-1", "default")
        .await
        .unwrap();
    complete_run(&loc, "run-done", RunOutcome::Success, None, vec![])
        .await
        .unwrap();

    assert!(update_heartbeat(&loc, "run-done").await.is_err());
}

#[tokio::test]
async fn complete_run_sets_outcome() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "complete-test");

    create_run(&loc, "run-ok", "card-1", "default")
        .await
        .unwrap();
    let done = complete_run(
        &loc,
        "run-ok",
        RunOutcome::Success,
        None,
        vec!["evidence".to_string()],
    )
    .await
    .unwrap();

    assert!(!done.is_active());
    assert_eq!(done.outcome, Some(RunOutcome::Success));
    assert_eq!(done.evidence, vec!["evidence".to_string()]);
}

#[tokio::test]
async fn complete_run_with_failure() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "fail-test");

    create_run(&loc, "run-bad", "card-1", "default")
        .await
        .unwrap();
    let done = complete_run(
        &loc,
        "run-bad",
        RunOutcome::Failed,
        Some("boom".to_string()),
        vec![],
    )
    .await
    .unwrap();

    assert_eq!(done.outcome, Some(RunOutcome::Failed));
    assert_eq!(done.error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn get_run_returns_none_for_missing() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "missing-test");
    assert!(get_run(&loc, "nope").await.unwrap().is_none());
}

/// Age a run past every limit by rewriting its stamps in the crate store.
async fn wedge(loc: &BoardLocation, run_id: &str) {
    let (store, thread_id) = target(loc);
    let mut runs = crate_runs::list_runs(&store, thread_id, None)
        .await
        .unwrap();
    for run in runs.iter_mut().filter(|run| run.run_id == run_id) {
        run.started_at = "0".to_string();
        run.last_heartbeat_at = "0".to_string();
    }
    let key = hex_key(&thread_id);
    store
        .put(
            crate_runs::RUNS_NAMESPACE,
            &key,
            serde_json::to_value(&runs).unwrap(),
        )
        .await
        .unwrap();
}

async fn seed_in_progress_card(loc: &BoardLocation, title: &str) -> String {
    let snapshot = ops::add(loc, title, CardPatch::default()).await.unwrap();
    let card_id = snapshot.cards.last().unwrap().id.clone();
    ops::edit(
        loc,
        &card_id,
        CardPatch {
            status: Some(TaskCardStatus::InProgress),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    card_id
}

#[tokio::test]
async fn reclaim_stale_moves_card_to_todo() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "reclaim-test");
    let card_id = seed_in_progress_card(&loc, "wedged work").await;

    create_run(&loc, "run-stale", &card_id, "default")
        .await
        .unwrap();
    wedge(&loc, "run-stale").await;

    let result = reclaim_stale(&loc, &RunLimits::default()).await.unwrap();
    assert_eq!(result.reclaimed_count, 1);
    assert_eq!(result.blocked_count, 0);
    assert_eq!(result.details[0].new_card_status, "todo");

    let snapshot = ops::list(&loc).await.unwrap();
    assert_eq!(snapshot.cards[0].status, TaskCardStatus::Todo);
}

#[tokio::test]
async fn reclaim_blocks_after_max_reclaims() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "reclaim-max-test");
    let card_id = seed_in_progress_card(&loc, "poison work").await;
    // Two reclaims are tolerated; the one that reaches the limit parks it.
    let limits = RunLimits {
        max_reclaim_count: 2,
        ..RunLimits::default()
    };

    for (attempt, expected) in ["todo", "blocked"].iter().enumerate() {
        if attempt > 0 {
            ops::edit(
                &loc,
                &card_id,
                CardPatch {
                    status: Some(TaskCardStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let run_id = format!("run-{attempt}");
        create_run(&loc, &run_id, &card_id, "default")
            .await
            .unwrap();
        wedge(&loc, &run_id).await;
        let result = reclaim_stale(&loc, &limits).await.unwrap();
        assert_eq!(&result.details[0].new_card_status, expected);
    }

    let snapshot = ops::list(&loc).await.unwrap();
    assert_eq!(snapshot.cards[0].status, TaskCardStatus::Blocked);
    assert!(snapshot.cards[0]
        .blocker
        .as_deref()
        .unwrap_or_default()
        .contains("exceeding limit of 2"));
}

#[tokio::test]
async fn reclaim_skips_healthy_runs() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "reclaim-healthy-test");
    let card_id = seed_in_progress_card(&loc, "live work").await;
    create_run(&loc, "run-live", &card_id, "default")
        .await
        .unwrap();

    let result = reclaim_stale(&loc, &RunLimits::default()).await.unwrap();
    assert_eq!(result.reclaimed_count, 0);
    assert_eq!(result.blocked_count, 0);

    let snapshot = ops::list(&loc).await.unwrap();
    assert_eq!(snapshot.cards[0].status, TaskCardStatus::InProgress);
}

#[tokio::test]
async fn scratch_location_returns_empty_runs() {
    // Serialize against the process-global scratch store shared with
    // `todos::ops` / agent-tool tests (see `scratch_test_lock`).
    let _guard = ops::scratch_test_lock();
    let runs = list_runs(&BoardLocation::Scratch, None).await.unwrap();
    assert!(runs.is_empty());
}

#[tokio::test]
async fn legacy_ledger_is_imported_once_and_never_replaces_crate_runs() {
    let workspace = tempdir().unwrap();
    let legacy_dir = workspace.path().join(TASK_BOARD_DIR);
    tokio::fs::create_dir_all(&legacy_dir).await.unwrap();

    let thread_id = "legacy-thread";
    let hex = hex_key(thread_id);
    let legacy = vec![TaskRun {
        run_id: "legacy-run".to_string(),
        card_id: "card-1".to_string(),
        claimed_by: "default".to_string(),
        claim_token: "token".to_string(),
        started_at: "0".to_string(),
        last_heartbeat_at: "0".to_string(),
        completed_at: None,
        outcome: None,
        error: None,
        evidence: Vec::new(),
    }];
    tokio::fs::write(
        legacy_dir.join(format!("{hex}.runs.json")),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .await
    .unwrap();
    // A board file alongside it must not be mistaken for a run ledger.
    tokio::fs::write(legacy_dir.join(format!("{hex}.json")), b"{}")
        .await
        .unwrap();

    let first = migrate_legacy_task_runs(workspace.path()).await.unwrap();
    assert_eq!(
        first,
        TaskRunMigrationReport {
            total: 1,
            copied: 1,
            skipped: 0,
        }
    );

    let loc = thread_loc(workspace.path(), thread_id);
    assert_eq!(list_runs(&loc, None).await.unwrap().len(), 1);

    // Second pass: the crate log is authoritative and is left alone.
    let second = migrate_legacy_task_runs(workspace.path()).await.unwrap();
    assert_eq!(second.copied, 0);
    assert_eq!(second.skipped, 1);
    assert_eq!(list_runs(&loc, None).await.unwrap().len(), 1);
}

#[test]
fn legacy_file_names_decode_only_run_ledgers() {
    let hex: String = "thread-1"
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        legacy_thread_id(Path::new(&format!("/w/{hex}.runs.json"))).as_deref(),
        Some("thread-1")
    );
    assert!(legacy_thread_id(Path::new(&format!("/w/{hex}.json"))).is_none());
    assert!(legacy_thread_id(Path::new("/w/notes.txt")).is_none());
    assert!(legacy_thread_id(Path::new("/w/zz.runs.json")).is_none());
}

#[test]
fn legacy_file_names_reject_malformed_stems_without_panicking() {
    // Multi-byte UTF-8 in the stem slices an odd index if decoded by byte
    // pairs; it must be rejected, not panic.
    assert!(legacy_thread_id(Path::new("/w/aéb.runs.json")).is_none());
    // from_str_radix would accept "+f" as 15; strict hex decoding rejects the sign.
    assert!(legacy_thread_id(Path::new("/w/+f+f.runs.json")).is_none());
    assert!(legacy_thread_id(Path::new("/w/gg.runs.json")).is_none());
    assert!(legacy_thread_id(Path::new("/w/GG.runs.json")).is_none());
    // Uppercase hex is rejected even though to_digit(16) would accept it.
    assert!(legacy_thread_id(Path::new("/w/4A.runs.json")).is_none());
    assert!(legacy_thread_id(Path::new("/w/4a.runs.json")).is_some());
    // A mixed pair with a valid second nibble but invalid first is rejected.
    assert!(legacy_thread_id(Path::new("/w/0g.runs.json")).is_none());
    // Non-hex ASCII letters are rejected.
    assert!(legacy_thread_id(Path::new("/w/zz.runs.json")).is_none());
}
