use super::*;
use crate::storage::{MemoryStorage, Scope, StorageBackend};
use std::sync::mpsc;
use std::sync::Arc;
use tinystoragedrivers::CollectionSpec;

const COLLECTION: &str = "claims_test";

fn spec() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(COLLECTION)]
}

fn repo() -> Repo {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    Repo::on(&backend, &Scope::local(), "claim-test", spec).unwrap()
}

fn claim_exists(repo: &Repo, marker: &str) -> bool {
    let id = claim_id(marker);
    repo.run(move |docs| async move { docs.get(COLLECTION, &id).await })
        .unwrap()
        .is_some()
}

#[test]
fn claim_runs_the_body_and_releases() {
    let repo = repo();
    let ran = with_claim(&repo, COLLECTION, "m", || Ok(7)).unwrap();
    assert_eq!(ran, Some(7));
    assert!(!claim_exists(&repo, "m"));
}

#[test]
fn claim_is_released_when_the_body_fails() {
    let repo = repo();
    let result: Result<Option<()>> =
        with_claim(&repo, COLLECTION, "m", || Err(anyhow::anyhow!("boom")));
    assert!(result.is_err());
    assert!(!claim_exists(&repo, "m"));
    assert_eq!(
        with_claim(&repo, COLLECTION, "m", || Ok(1)).unwrap(),
        Some(1)
    );
}

#[test]
fn a_second_process_skips_while_the_first_is_importing() {
    let repo = repo();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (finish_tx, finish_rx) = mpsc::channel::<()>();
    let holder_repo = repo.clone();
    let holder = std::thread::spawn(move || {
        with_claim(&holder_repo, COLLECTION, "m", || {
            started_tx.send(()).unwrap();
            finish_rx.recv().unwrap();
            Ok("first")
        })
        .unwrap()
    });
    started_rx.recv().unwrap();

    let mut second_ran = false;
    let now = chrono::Utc::now().timestamp_millis();
    let second = with_claim_at(&repo, COLLECTION, "m", now, CLAIM_TTL_MS, || {
        second_ran = true;
        Ok("second")
    })
    .unwrap();
    assert_eq!(second, None);
    assert!(!second_ran);

    finish_tx.send(()).unwrap();
    assert_eq!(holder.join().unwrap(), Some("first"));
    assert_eq!(
        with_claim(&repo, COLLECTION, "m", || Ok("third")).unwrap(),
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
                with_claim(&repo, COLLECTION, "m", || {
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
    let id = claim_id("m");
    let seeded = id.clone();
    repo.run(move |docs| async move {
        docs.put(
            COLLECTION,
            &seeded,
            serde_json::json!({ "claimed_at_ms": 1_000 }),
            Precondition::Absent,
        )
        .await
        .map(|_| ())
    })
    .unwrap();

    // Fresh: still held.
    let held = with_claim_at(&repo, COLLECTION, "m", 1_000 + 10, 100, || Ok(())).unwrap();
    assert_eq!(held, None);
    // Past the lease: taken over, run, released.
    let taken = with_claim_at(&repo, COLLECTION, "m", 1_000 + 200, 100, || Ok(())).unwrap();
    assert_eq!(taken, Some(()));
    assert!(!claim_exists(&repo, "m"));
}

#[test]
fn a_waiting_process_runs_once_the_holder_releases() {
    let repo = repo();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let holder_repo = repo.clone();
    let holder = std::thread::spawn(move || {
        with_claim(&holder_repo, COLLECTION, "m", || {
            started_tx.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(400));
            Ok("first")
        })
        .unwrap()
    });
    started_rx.recv().unwrap();
    let began = std::time::Instant::now();
    let second = with_claim(&repo, COLLECTION, "m", || Ok("second")).unwrap();
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
    let rival_ran = with_claim_at(&repo, COLLECTION, "m", now, 90, || {
        let mut took_over = false;
        for _ in 0..8 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let at = chrono::Utc::now().timestamp_millis();
            took_over |= with_claim_at(&rival_repo, COLLECTION, "m", at, 90, || Ok(()))
                .unwrap()
                .is_some();
        }
        Ok(took_over)
    })
    .unwrap();
    assert_eq!(rival_ran, Some(false));
}
