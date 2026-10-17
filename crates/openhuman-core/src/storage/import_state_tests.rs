use super::*;
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use std::sync::mpsc;
use std::sync::Arc;
use tinystoragedrivers::{CollectionSpec, Leases};

const OWNER: &str = "claims_test";

fn spec() -> Vec<CollectionSpec> {
    Vec::new()
}

fn repo() -> Repo {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    Repo::on(&backend, &Scope::local(), "claim-test", spec).unwrap()
}

fn claim_held(repo: &Repo, step: &str) -> bool {
    let key = lease_key(OWNER, step);
    repo.run(move |docs| async move {
        declare(&docs).await?;
        Leases::new(&docs).current(CLAIM_OWNER, &key).await
    })
    .unwrap()
    .is_some()
}

#[test]
fn claim_runs_the_body_and_releases() {
    let repo = repo();
    let ran = with_claim(&repo, OWNER, "m", || Ok(7)).unwrap();
    assert_eq!(ran, Some(7));
    assert!(!claim_held(&repo, "m"));
}

#[test]
fn claim_is_released_when_the_body_fails() {
    let repo = repo();
    let result: Result<Option<()>> = with_claim(&repo, OWNER, "m", || Err(anyhow::anyhow!("boom")));
    assert!(result.is_err());
    assert!(!claim_held(&repo, "m"));
    assert_eq!(with_claim(&repo, OWNER, "m", || Ok(1)).unwrap(), Some(1));
}

#[test]
fn a_second_process_skips_while_the_first_is_importing() {
    let repo = repo();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (finish_tx, finish_rx) = mpsc::channel::<()>();
    let holder_repo = repo.clone();
    let holder = std::thread::spawn(move || {
        with_claim(&holder_repo, OWNER, "m", || {
            started_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            Ok("first")
        })
        .unwrap()
    });
    started_rx.recv().unwrap();

    let mut second_ran = false;
    let now = chrono::Utc::now().timestamp_millis();
    let second = with_claim_at(&repo, OWNER, "m", now, CLAIM_TTL_MS, || {
        second_ran = true;
        Ok("second")
    })
    .unwrap();
    assert_eq!(second, None);
    assert!(!second_ran);

    finish_tx.send(()).unwrap();
    assert_eq!(holder.join().unwrap(), Some("first"));
    assert_eq!(
        with_claim(&repo, OWNER, "m", || Ok("third")).unwrap(),
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
                with_claim(&repo, OWNER, "m", || {
                    use std::sync::atomic::Ordering::SeqCst;
                    if active.fetch_add(1, SeqCst) != 0 {
                        overlap.store(true, SeqCst);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(30));
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
        .map(|h| h.join().unwrap())
        .filter(|r| *r)
        .count();
    assert!(ran >= 1, "somebody imports");
    assert!(
        !overlap.load(std::sync::atomic::Ordering::SeqCst),
        "two bodies held the claim at once"
    );
}

#[test]
fn a_stale_claim_is_taken_over() {
    let repo = repo();
    let key = lease_key(OWNER, "m");
    repo.run(move |docs| async move {
        declare(&docs).await?;
        let clock: tinystoragedrivers::Clock = Arc::new(|| 1_000);
        Leases::new(&docs)
            .with_clock(clock)
            .acquire(CLAIM_OWNER, &key, "dead-process", 100)
            .await
            .map(|_| ())
    })
    .unwrap();

    // Fresh: still held.
    let held = with_claim_at(&repo, OWNER, "m", 1_000 + 10, 100, || Ok(())).unwrap();
    assert_eq!(held, None);
    // Past the lease: taken over, run, released.
    let taken = with_claim_at(&repo, OWNER, "m", 1_000 + 200, 100, || Ok(())).unwrap();
    assert_eq!(taken, Some(()));
}

#[test]
fn completion_is_a_migrations_entry_in_the_scope() {
    let repo = repo();
    assert!(!is_done(&repo, "threads", "legacy-conversations").unwrap());
    set_done(&repo, "threads", "legacy-conversations").unwrap();
    assert!(is_done(&repo, "threads", "legacy-conversations").unwrap());
    assert!(!is_done(&repo, "threads", "other").unwrap());
    assert!(!is_done(&repo, "subagents", "legacy-conversations").unwrap());
}

#[test]
fn a_legacy_marker_row_is_honoured_and_removed() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let scoped = backend.for_scope(&Scope::local()).unwrap();
    let owned = scoped.clone();
    crate::storage::block_on(async move {
        let scoped = owned;
        let docs = scoped.documents();
        docs.ensure_collection(&CollectionSpec::new("storage_import"))
            .await?;
        docs.put(
            "storage_import",
            "approval@abcd",
            serde_json::json!({ "version": 1, "domain": "approval" }),
            tinystoragedrivers::Precondition::Absent,
        )
        .await?;
        docs.put(
            "storage_import",
            "approval@abcd#claim",
            serde_json::json!({ "claimed_at_ms": 1 }),
            tinystoragedrivers::Precondition::Absent,
        )
        .await?;
        let moved = migrate_legacy_markers(&scoped).await?;
        assert_eq!(moved, 1);
        assert!(
            done(
                &scoped,
                "storage",
                &step_id("storage_import", "approval@abcd")
            )
            .await?
        );
        assert!(!done(&scoped, "storage", &step_id("storage_import", "other@x")).await?);
        assert!(
            docs.query_all("storage_import", &tinystoragedrivers::Query::all())
                .await
                .map_or(true, |rows| rows.is_empty()),
            "the marker rows are gone"
        );
        // A second sweep has nothing to do and keeps the entry.
        assert_eq!(migrate_legacy_markers(&scoped).await?, 0);
        assert!(
            done(
                &scoped,
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
fn a_waiting_process_runs_once_the_holder_releases() {
    let repo = repo();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let holder_repo = repo.clone();
    let holder = std::thread::spawn(move || {
        with_claim(&holder_repo, OWNER, "m", || {
            started_tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(400));
            Ok("first")
        })
        .unwrap()
    });
    started_rx.recv().unwrap();
    let began = std::time::Instant::now();
    let second = with_claim(&repo, OWNER, "m", || Ok("second")).unwrap();
    assert_eq!(second, Some("second"));
    assert!(began.elapsed() >= std::time::Duration::from_millis(200));
    assert_eq!(holder.join().unwrap(), Some("first"));
}

#[test]
fn a_long_import_keeps_its_claim_alive() {
    let repo = repo();
    let now = chrono::Utc::now().timestamp_millis();
    // 90ms lease, body runs 400ms: without renewal a rival would take over.
    let rival_repo = repo.clone();
    let rival_ran = with_claim_at(&repo, OWNER, "m", now, 90, || {
        let mut took_over = false;
        for _ in 0..8 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let at = chrono::Utc::now().timestamp_millis();
            took_over |= with_claim_at(&rival_repo, OWNER, "m", at, 90, || Ok(()))
                .unwrap()
                .is_some();
        }
        Ok(took_over)
    })
    .unwrap();
    assert_eq!(rival_ran, Some(false));
}
