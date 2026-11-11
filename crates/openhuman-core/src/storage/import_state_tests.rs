use super::*;
use crate::storage::{Scope, StorageBackend};
use std::sync::mpsc;
use std::sync::Arc;
use tinystoragedrivers::CollectionSpec;
use tinystoragedrivers_testkit::MemoryBackend;

const OWNER: &str = "claims_test";

fn spec() -> Vec<CollectionSpec> {
    Vec::new()
}

fn repo() -> Repo {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryBackend::new());
    Repo::on(&backend, &Scope::local(), "claim-test", spec).unwrap()
}

#[cfg(feature = "storage-sqlite")]
fn sqlite_repo() -> (tempfile::TempDir, Arc<dyn StorageBackend>, Repo) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("claims.sqlite");
    let url = format!("sqlite:{}", database.display());
    let backend =
        crate::storage::block_on(async move { crate::storage::open(&url).await }).unwrap();
    let repo = Repo::on(&backend, &Scope::local(), "claim-test", spec).unwrap();
    (dir, backend, repo)
}

#[test]
fn state_keys_keep_owner_and_step_boundaries_unambiguous() {
    assert_ne!(
        record_key("a/b", "c", "done"),
        record_key("a", "b/c", "done")
    );
    assert_eq!(collection_specs()[0].name, "storage.import_state");
}

fn load_claim(repo: &Repo, owner: &str, step: &str) -> Option<Claim> {
    let scoped = repo.scoped().clone();
    let (owner, step) = (owner.to_owned(), step.to_owned());
    repo.run(move |docs| async move {
        ensure(&scoped).await?;
        let id = record_key(&owner, &step, "claim");
        match docs.get(COLLECTION, &id).await? {
            Some(row) => decode(row, "claim", &owner, &step).map(Some),
            None => Ok(None),
        }
    })
    .unwrap()
}

#[test]
fn claim_runs_the_body_and_releases_while_retaining_its_epoch() {
    let repo = repo();
    let ran = with_claim(&repo, OWNER, "m", |_| Ok(7)).unwrap();
    assert_eq!(ran, Some(7));
    let released = load_claim(&repo, OWNER, "m").unwrap();
    assert_eq!(released.epoch, 1);
    assert!(released.holder.is_none());
    assert_eq!(with_claim(&repo, OWNER, "m", |_| Ok(8)).unwrap(), Some(8));
    assert_eq!(load_claim(&repo, OWNER, "m").unwrap().epoch, 2);
}

#[test]
fn claim_is_released_when_the_body_fails() {
    let repo = repo();
    let result: Result<Option<()>> =
        with_claim(&repo, OWNER, "m", |_| Err(anyhow::anyhow!("boom")));
    assert!(result.is_err());
    assert!(load_claim(&repo, OWNER, "m").unwrap().holder.is_none());
    assert_eq!(with_claim(&repo, OWNER, "m", |_| Ok(1)).unwrap(), Some(1));
}

#[test]
fn a_second_process_skips_while_the_first_is_importing() {
    let repo = repo();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (finish_tx, finish_rx) = mpsc::channel::<()>();
    let holder_repo = repo.clone();
    let holder = std::thread::spawn(move || {
        with_claim(&holder_repo, OWNER, "m", |_| {
            started_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            Ok("first")
        })
        .unwrap()
    });
    started_rx.recv().unwrap();

    let mut second_ran = false;
    let now = chrono::Utc::now().timestamp_millis();
    let second = with_claim_at(&repo, OWNER, "m", now, CLAIM_TTL_MS, |_| {
        second_ran = true;
        Ok("second")
    })
    .unwrap();
    assert_eq!(second, None);
    assert!(!second_ran);

    finish_tx.send(()).unwrap();
    assert_eq!(holder.join().unwrap(), Some("first"));
    assert_eq!(
        with_claim(&repo, OWNER, "m", |_| Ok("third")).unwrap(),
        Some("third")
    );
}

#[test]
fn many_racing_processes_run_the_body_once_at_a_time() {
    let repo = repo();
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let overlap = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (repo, active, overlap, barrier) = (
                repo.clone(),
                Arc::clone(&active),
                Arc::clone(&overlap),
                Arc::clone(&barrier),
            );
            std::thread::spawn(move || {
                barrier.wait();
                with_claim(&repo, OWNER, "m", |_| {
                    use std::sync::atomic::Ordering::SeqCst;
                    if active.fetch_add(1, SeqCst) != 0 {
                        overlap.store(true, SeqCst);
                    }
                    std::thread::sleep(Duration::from_millis(30));
                    active.fetch_sub(1, SeqCst);
                    Ok(())
                })
                .unwrap()
                .is_some()
            })
        })
        .collect();
    let ran = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|ran| *ran)
        .count();
    assert!(ran >= 1, "somebody imports");
    assert!(!overlap.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn a_stale_claim_is_taken_over_with_a_new_epoch() {
    let repo = repo();
    let first = acquire(&repo, OWNER, "m", 1_000, 100).unwrap().unwrap();
    assert!(acquire(&repo, OWNER, "m", 1_010, 100).unwrap().is_none());
    let second = acquire(&repo, OWNER, "m", 1_200, 100).unwrap().unwrap();
    assert_eq!(second.epoch, first.epoch + 1);
    release(&repo, &second).unwrap();
}

#[test]
fn completion_is_an_immutable_host_owned_record_in_the_scope() {
    let repo = repo();
    assert!(!is_done(&repo, "threads", "legacy-conversations").unwrap());
    set_done(&repo, "threads", "legacy-conversations").unwrap();
    assert!(is_done(&repo, "threads", "legacy-conversations").unwrap());
    assert!(!is_done(&repo, "threads", "other").unwrap());
    assert!(!is_done(&repo, "subagents", "legacy-conversations").unwrap());
}

#[test]
fn a_legacy_marker_row_is_honoured_and_removed() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryBackend::new());
    let scoped = backend.for_scope(&Scope::local()).unwrap();
    let owned = scoped.clone();
    crate::storage::block_on(async move {
        let docs = owned.documents();
        docs.ensure_collection(&CollectionSpec::new("storage_import"))
            .await?;
        docs.put(
            "storage_import",
            "approval@abcd",
            serde_json::json!({ "version": 1, "domain": "approval" }),
            Precondition::Absent,
        )
        .await?;
        docs.put(
            "storage_import",
            "approval@abcd#claim",
            serde_json::json!({ "claimed_at_ms": 1 }),
            Precondition::Absent,
        )
        .await?;
        let moved = migrate_legacy_markers(&owned).await?;
        assert_eq!(moved, 1);
        assert!(
            done(
                &owned,
                "storage",
                &step_id("storage_import", "approval@abcd")
            )
            .await?
        );
        assert!(!done(&owned, "storage", &step_id("storage_import", "other@x")).await?);
        assert!(docs
            .query_all("storage_import", &tinystoragedrivers::Query::all())
            .await?
            .is_empty());
        assert_eq!(migrate_legacy_markers(&owned).await?, 0);
        assert!(
            done(
                &owned,
                "storage",
                &step_id("storage_import", "approval@abcd")
            )
            .await?
        );
        Ok::<_, StorageError>(())
    })
    .unwrap();
}

#[test]
fn try_claim_returns_at_once_while_another_holder_is_live() {
    let repo = repo();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder_repo = repo.clone();
    let holder = std::thread::spawn(move || {
        with_claim(&holder_repo, OWNER, "m", |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap()
    });
    started_rx.recv().unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let rival_repo = repo.clone();
    std::thread::spawn(move || {
        let _ = done_tx.send(try_claim(&rival_repo, OWNER, "m", |_| Ok(())).unwrap());
    });
    let attempt = done_rx.recv_timeout(Duration::from_secs(2));
    release_tx.send(()).unwrap();
    assert!(
        matches!(attempt, Ok(None)),
        "a live rival claim must not make the caller wait: {attempt:?}"
    );
    assert_eq!(holder.join().unwrap(), Some(()));
}

#[test]
fn a_waiting_process_runs_once_the_holder_releases() {
    let repo = repo();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let holder_repo = repo.clone();
    let holder = std::thread::spawn(move || {
        with_claim(&holder_repo, OWNER, "m", |_| {
            started_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(400));
            Ok("first")
        })
        .unwrap()
    });
    started_rx.recv().unwrap();
    let began = Instant::now();
    let second = with_claim(&repo, OWNER, "m", |_| Ok("second")).unwrap();
    assert_eq!(second, Some("second"));
    assert!(began.elapsed() >= Duration::from_millis(200));
    assert_eq!(holder.join().unwrap(), Some("first"));
}

#[test]
#[cfg(feature = "storage-sqlite")]
fn a_long_import_keeps_its_claim_alive() {
    let (_dir, _backend, repo) = sqlite_repo();
    let rival_repo = repo.clone();
    let rival_ran = with_claim_ttl(&repo, OWNER, "m", 1_000, |_| {
        let mut took_over = false;
        for _ in 0..26 {
            std::thread::sleep(Duration::from_millis(50));
            let at = chrono::Utc::now().timestamp_millis();
            took_over |= with_claim_at(&rival_repo, OWNER, "m", at, 1_000, |_| Ok(()))
                .unwrap()
                .is_some();
        }
        Ok(took_over)
    })
    .unwrap();
    assert_eq!(rival_ran, Some(false));
}

#[cfg(feature = "storage-sqlite")]
#[test]
fn takeover_and_lost_heartbeat_refuse_the_stale_payload_write() {
    let (_dir, _backend, repo) = sqlite_repo();
    let (condition_tx, condition_rx) = mpsc::channel();
    let (takeover_tx, takeover_rx) = mpsc::channel();
    let worker_repo = repo.clone();
    let worker = std::thread::spawn(move || {
        with_claim_ttl(&worker_repo, OWNER, "payload", 120, |condition| {
            condition_tx.send(condition.clone()).unwrap();
            takeover_rx.recv().unwrap();
            std::thread::sleep(Duration::from_millis(150));
            let stale_repo = worker_repo.conditioned(condition)?;
            let attempted = stale_repo.run(|docs| async move {
                let spec = CollectionSpec::new("claim_payload");
                docs.ensure_collection(&spec).await?;
                docs.put(
                    "claim_payload",
                    "stale",
                    serde_json::json!({ "value": 1 }),
                    Precondition::Absent,
                )
                .await
                .map(|_| ())
            });
            assert!(
                attempted.is_err(),
                "the expired claim cannot write its payload"
            );
            Ok(())
        })
    });

    let _old_condition = condition_rx.recv().unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let replacement = acquire(&repo, OWNER, "payload", now.saturating_add(10_000), 5_000)
        .unwrap()
        .expect("takeover replaces the old claim");
    takeover_tx.send(()).unwrap();
    assert!(
        worker.join().unwrap().is_err(),
        "lost heartbeat fails the import"
    );
    release(&repo, &replacement).unwrap();

    let payload = repo
        .run(|docs| async move { docs.get("claim_payload", "stale").await })
        .unwrap();
    assert!(payload.is_none(), "the stale payload was never published");
}
