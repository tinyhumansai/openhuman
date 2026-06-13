//! Race and invariant tests for the task-board scheduler kernel.
//!
//! These tests exercise concurrent claim races, run-pointer consistency,
//! status-transition validity, double-completion rejection, and randomised
//! operation sequences over small task graphs. Each test is deterministic
//! (seeded PRNG where randomness is needed) and runs under the standard
//! `cargo test` harness / `pnpm debug rust` runner.

use std::collections::HashSet;
use std::sync::{Arc, Barrier};
use tempfile::tempdir;

use crate::openhuman::agent::task_board::TaskCardStatus;
use crate::openhuman::todos::ops::{
    add, claim_card, list, update_status, BoardLocation, CardPatch,
};
use crate::openhuman::todos::runs::{
    complete_run, create_run, get_run, list_runs, reclaim_stale, update_heartbeat, RunLimits,
    RunOutcome,
};

fn thread_loc(dir: &std::path::Path, id: &str) -> BoardLocation {
    BoardLocation::Thread {
        workspace_dir: dir.to_path_buf(),
        thread_id: id.to_string(),
    }
}

fn add_card(loc: &BoardLocation, title: &str) -> String {
    add(loc, title, CardPatch::default())
        .unwrap()
        .cards
        .last()
        .unwrap()
        .id
        .clone()
}

// ── Invariant checkers ─────────────────────────────────────────────────

fn assert_board_invariants(loc: &BoardLocation) {
    let snap = list(loc).unwrap();
    let cards = &snap.cards;

    // At most one InProgress card.
    let in_progress: Vec<_> = cards
        .iter()
        .filter(|c| c.status == TaskCardStatus::InProgress)
        .collect();
    assert!(
        in_progress.len() <= 1,
        "invariant violated: {} cards InProgress (max 1): {:?}",
        in_progress.len(),
        in_progress.iter().map(|c| &c.id).collect::<Vec<_>>()
    );

    // All card IDs are unique.
    let mut ids = HashSet::new();
    for card in cards {
        assert!(
            ids.insert(&card.id),
            "invariant violated: duplicate card id '{}'",
            card.id
        );
    }

    // Order indices are contiguous 0..n.
    let mut orders: Vec<u32> = cards.iter().map(|c| c.order).collect();
    orders.sort();
    let expected: Vec<u32> = (0..cards.len() as u32).collect();
    assert_eq!(
        orders, expected,
        "invariant violated: order indices not contiguous"
    );
}

fn assert_run_pointer_invariants(loc: &BoardLocation) {
    let snap = list(loc).unwrap();
    let runs = list_runs(loc, None).unwrap_or_default();

    // Every active run's card_id references a card that exists on the board.
    let card_ids: HashSet<&str> = snap.cards.iter().map(|c| c.id.as_str()).collect();
    for run in &runs {
        if run.is_active() {
            assert!(
                card_ids.contains(run.card_id.as_str()),
                "invariant violated: active run '{}' references missing card '{}'",
                run.run_id,
                run.card_id
            );
        }
    }

    // Every InProgress card has at least one active run.
    for card in &snap.cards {
        if card.status == TaskCardStatus::InProgress {
            let active_runs: Vec<_> = runs
                .iter()
                .filter(|r| r.card_id == card.id && r.is_active())
                .collect();
            assert!(
                !active_runs.is_empty(),
                "invariant violated: InProgress card '{}' has no active run",
                card.id
            );
        }
    }

    // No card has more than one active run.
    let mut active_by_card: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for run in &runs {
        if run.is_active() {
            *active_by_card.entry(&run.card_id).or_default() += 1;
        }
    }
    for (card_id, count) in &active_by_card {
        assert!(
            *count <= 1,
            "invariant violated: card '{}' has {} active runs (max 1)",
            card_id,
            count
        );
    }
}

/// Wait just long enough for `check_staleness` (which truncates to whole
/// seconds via `num_seconds()`) to see the run as older than 0s.
fn sleep_for_staleness() {
    std::thread::sleep(std::time::Duration::from_millis(1100));
}

// ── 1. Double-claim invariant (stress) ────────────────────────────────

#[test]
fn stress_concurrent_claims_n_threads() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "stress-claim-1");
    let card_id = add_card(&loc, "race target");

    let n = 8;
    let barrier = Arc::new(Barrier::new(n));
    let results: Vec<_> = (0..n)
        .map(|_| {
            let loc = loc.clone();
            let id = card_id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                claim_card(
                    &loc,
                    &id,
                    &[TaskCardStatus::Todo, TaskCardStatus::Ready],
                    TaskCardStatus::InProgress,
                )
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();

    let wins = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(wins, 1, "exactly one of {n} concurrent claimers must win");

    let snap = list(&loc).unwrap();
    assert_eq!(snap.cards[0].status, TaskCardStatus::InProgress);
    assert_board_invariants(&loc);
}

#[test]
fn stress_concurrent_claims_multiple_cards() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "stress-claim-multi");

    let id_a = add_card(&loc, "card A");
    let id_b = add_card(&loc, "card B");

    // 4 threads race to claim card A.
    let barrier = Arc::new(Barrier::new(4));
    let results_a: Vec<_> = (0..4)
        .map(|_| {
            let loc = loc.clone();
            let id = id_a.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                claim_card(
                    &loc,
                    &id,
                    &[TaskCardStatus::Todo],
                    TaskCardStatus::InProgress,
                )
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();

    assert_eq!(
        results_a.iter().filter(|r| r.is_ok()).count(),
        1,
        "exactly one claimer wins card A"
    );

    // Card B cannot go InProgress because enforce_single_in_progress blocks it.
    let claim_b = claim_card(
        &loc,
        &id_b,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    );
    assert!(
        claim_b.is_err(),
        "claiming card B as InProgress must fail while card A is InProgress"
    );

    assert_board_invariants(&loc);
}

// ── 2. Run pointer invariant ──────────────────────────────────────────

#[test]
fn run_pointer_active_run_references_existing_card() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "run-ptr-1");
    let card_id = add_card(&loc, "task with run");

    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();

    let run = create_run(&loc, "run-1", &card_id, "dispatcher").unwrap();
    assert!(run.is_active());
    assert_run_pointer_invariants(&loc);
}

#[test]
fn run_pointer_completed_run_consistency() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "run-ptr-2");
    let card_id = add_card(&loc, "task to complete");

    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();

    create_run(&loc, "run-2", &card_id, "dispatcher").unwrap();
    complete_run(
        &loc,
        "run-2",
        RunOutcome::Success,
        None,
        vec!["evidence-1".into()],
    )
    .unwrap();

    update_status(&loc, &card_id, TaskCardStatus::Done).unwrap();

    let snap = list(&loc).unwrap();
    assert_eq!(snap.cards[0].status, TaskCardStatus::Done);

    // No active runs remain.
    let runs = list_runs(&loc, Some(&card_id)).unwrap();
    assert!(
        runs.iter().all(|r| !r.is_active()),
        "all runs should be completed after card is Done"
    );
    assert_board_invariants(&loc);
}

#[test]
fn run_pointer_no_orphaned_active_runs_after_reclaim() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "run-ptr-3");
    let card_id = add_card(&loc, "stale task");

    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();

    create_run(&loc, "run-stale", &card_id, "dispatcher").unwrap();

    // check_staleness uses strict `>`, so we need the run to age past 0s.
    sleep_for_staleness();

    let limits = RunLimits {
        heartbeat_stale_secs: 0,
        claim_ttl_secs: 0,
        max_reclaim_count: 3,
    };
    let result = reclaim_stale(&loc, &limits).unwrap();
    assert_eq!(result.reclaimed_count, 1);

    // After reclaim, the card should be back to Todo and no active runs.
    let snap = list(&loc).unwrap();
    assert_eq!(snap.cards[0].status, TaskCardStatus::Todo);

    let runs = list_runs(&loc, Some(&card_id)).unwrap();
    assert!(
        runs.iter().all(|r| !r.is_active()),
        "no active runs should remain after reclaim"
    );
    assert_board_invariants(&loc);
}

// ── 3. Status invariant ───────────────────────────────────────────────

#[test]
fn status_claim_rejects_wrong_expected_status() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "status-inv-1");
    let card_id = add_card(&loc, "wrong status");

    // Card is Todo; claiming with expected=[InProgress] must fail.
    let err = claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::InProgress],
        TaskCardStatus::Done,
    );
    assert!(err.is_err(), "claim with wrong expected status must fail");

    // Card should still be Todo.
    let snap = list(&loc).unwrap();
    assert_eq!(snap.cards[0].status, TaskCardStatus::Todo);
    assert_board_invariants(&loc);
}

#[test]
fn status_all_valid_statuses_accepted() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "status-inv-2");

    let statuses = [
        TaskCardStatus::Todo,
        TaskCardStatus::AwaitingApproval,
        TaskCardStatus::Ready,
        TaskCardStatus::Blocked,
        TaskCardStatus::Rejected,
        TaskCardStatus::Done,
    ];

    for status in &statuses {
        let card_id = add_card(&loc, &format!("card-{:?}", status));
        update_status(&loc, &card_id, status.clone()).unwrap();
        let snap = list(&loc).unwrap();
        let card = snap.cards.iter().find(|c| c.id == card_id).unwrap();
        assert_eq!(&card.status, status);
    }
    assert_board_invariants(&loc);
}

#[test]
fn status_claim_metadata_consistent_with_status() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "status-inv-3");
    let card_id = add_card(&loc, "claim consistency");

    // Claim Todo → InProgress via claim_card.
    let claimed = claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();
    assert_eq!(claimed.status, TaskCardStatus::InProgress);
    assert!(
        !claimed.updated_at.is_empty(),
        "updated_at must be set after claim"
    );

    // The board snapshot agrees with the returned card's status.
    // (normalise_board may re-stamp updated_at, so only compare status.)
    let snap = list(&loc).unwrap();
    let board_card = snap.cards.iter().find(|c| c.id == card_id).unwrap();
    assert_eq!(board_card.status, TaskCardStatus::InProgress);
    assert!(!board_card.updated_at.is_empty());
    assert_board_invariants(&loc);
}

#[test]
fn status_blocked_card_has_blocker_after_reclaim() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "status-inv-4");
    let card_id = add_card(&loc, "reclaim-to-blocked");

    let limits = RunLimits {
        heartbeat_stale_secs: 0,
        claim_ttl_secs: 0,
        max_reclaim_count: 2,
    };

    // First reclaim (count=1 < max=2): card goes back to Todo.
    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();
    create_run(&loc, "run-r1", &card_id, "d").unwrap();
    sleep_for_staleness();
    let r1 = reclaim_stale(&loc, &limits).unwrap();
    assert_eq!(r1.reclaimed_count, 1);

    // Second reclaim (count=2 >= max=2): card goes to Blocked.
    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();
    create_run(&loc, "run-r2", &card_id, "d").unwrap();
    sleep_for_staleness();
    let r2 = reclaim_stale(&loc, &limits).unwrap();
    assert_eq!(r2.blocked_count, 1);

    let snap = list(&loc).unwrap();
    let card = snap.cards.iter().find(|c| c.id == card_id).unwrap();
    assert_eq!(card.status, TaskCardStatus::Blocked);
    assert!(
        card.blocker.is_some(),
        "blocked card must have a blocker message"
    );
    assert_board_invariants(&loc);
}

// ── 4. Completion invariant ───────────────────────────────────────────

#[test]
fn completion_run_cannot_be_completed_twice() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "complete-inv-1");
    let card_id = add_card(&loc, "double complete");

    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();

    create_run(&loc, "run-dc", &card_id, "dispatcher").unwrap();
    complete_run(&loc, "run-dc", RunOutcome::Success, None, vec![]).unwrap();

    // Second completion must fail (run is no longer active).
    let err = complete_run(
        &loc,
        "run-dc",
        RunOutcome::Failed,
        Some("retry".into()),
        vec![],
    );
    assert!(
        err.is_err(),
        "completing an already-completed run must fail"
    );
}

#[test]
fn completion_distinct_runs_same_card_only_one_active() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "complete-inv-2");
    let card_id = add_card(&loc, "multi-run card");

    // First claim + run cycle.
    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();
    create_run(&loc, "run-a", &card_id, "d1").unwrap();
    complete_run(
        &loc,
        "run-a",
        RunOutcome::Failed,
        Some("oops".into()),
        vec![],
    )
    .unwrap();

    // Move card back to Todo for re-dispatch.
    update_status(&loc, &card_id, TaskCardStatus::Todo).unwrap();

    // Second claim + run cycle.
    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();
    create_run(&loc, "run-b", &card_id, "d2").unwrap();

    // run-a is already completed, run-b is active.
    let run_a = get_run(&loc, "run-a").unwrap().unwrap();
    let run_b = get_run(&loc, "run-b").unwrap().unwrap();
    assert!(!run_a.is_active());
    assert!(run_b.is_active());

    assert_run_pointer_invariants(&loc);
}

#[test]
fn completion_heartbeat_after_completion_fails() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "complete-inv-3");
    let card_id = add_card(&loc, "hb after complete");

    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();
    create_run(&loc, "run-hb", &card_id, "d").unwrap();
    complete_run(&loc, "run-hb", RunOutcome::Success, None, vec![]).unwrap();

    let err = update_heartbeat(&loc, "run-hb");
    assert!(err.is_err(), "heartbeat on a completed run must fail");
}

// ── 5. Randomised operation sequence ──────────────────────────────────

#[test]
fn randomised_operation_sequence_maintains_invariants() {
    // Deterministic PRNG via simple LCG.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn range(&mut self, max: u64) -> u64 {
            if max == 0 {
                return 0;
            }
            self.next() % max
        }
    }

    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "rand-seq-1");
    let mut rng = Lcg(42);
    let mut card_ids: Vec<String> = Vec::new();
    let mut run_counter = 0u32;

    for _step in 0..200 {
        let op = rng.range(6);
        match op {
            // Add a new card.
            0 => {
                let title = format!("task-{}", card_ids.len());
                let id = add_card(&loc, &title);
                card_ids.push(id);
            }
            // Claim a random card Todo → InProgress.
            1 if !card_ids.is_empty() => {
                let idx = rng.range(card_ids.len() as u64) as usize;
                let _ = claim_card(
                    &loc,
                    &card_ids[idx],
                    &[TaskCardStatus::Todo, TaskCardStatus::Ready],
                    TaskCardStatus::InProgress,
                );
            }
            // Complete the in-progress card's run (if any) then mark Done.
            2 => {
                let snap = list(&loc).unwrap();
                if let Some(ip) = snap
                    .cards
                    .iter()
                    .find(|c| c.status == TaskCardStatus::InProgress)
                {
                    let runs = list_runs(&loc, Some(&ip.id)).unwrap_or_default();
                    if let Some(active) = runs.iter().find(|r| r.is_active()) {
                        let outcome = if rng.range(2) == 0 {
                            RunOutcome::Success
                        } else {
                            RunOutcome::Failed
                        };
                        let _ = complete_run(&loc, &active.run_id, outcome, None, vec![]);
                    }
                    let _ = update_status(&loc, &ip.id, TaskCardStatus::Done);
                }
            }
            // Create a run for an InProgress card that lacks one.
            3 => {
                let snap = list(&loc).unwrap();
                if let Some(ip) = snap
                    .cards
                    .iter()
                    .find(|c| c.status == TaskCardStatus::InProgress)
                {
                    let runs = list_runs(&loc, Some(&ip.id)).unwrap_or_default();
                    if !runs.iter().any(|r| r.is_active()) {
                        run_counter += 1;
                        let _ = create_run(&loc, &format!("rnd-run-{run_counter}"), &ip.id, "d");
                    }
                }
            }
            // Move a random card back to Todo (simulating retry).
            4 if !card_ids.is_empty() => {
                let idx = rng.range(card_ids.len() as u64) as usize;
                let snap = list(&loc).unwrap();
                if let Some(card) = snap.cards.iter().find(|c| c.id == card_ids[idx]) {
                    if matches!(
                        card.status,
                        TaskCardStatus::Done | TaskCardStatus::Blocked | TaskCardStatus::Rejected
                    ) {
                        let _ = update_status(&loc, &card_ids[idx], TaskCardStatus::Todo);
                    }
                }
            }
            // Heartbeat a random active run.
            5 => {
                let runs = list_runs(&loc, None).unwrap_or_default();
                let active: Vec<_> = runs.iter().filter(|r| r.is_active()).collect();
                if !active.is_empty() {
                    let idx = rng.range(active.len() as u64) as usize;
                    let _ = update_heartbeat(&loc, &active[idx].run_id);
                }
            }
            _ => {}
        }

        // After every operation, all invariants must hold.
        assert_board_invariants(&loc);
    }
}

// ── 6. Concurrent claim + run lifecycle stress ────────────────────────

#[test]
fn stress_claim_create_run_complete_cycle() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "stress-lifecycle");
    let card_id = add_card(&loc, "lifecycle target");

    for round in 0..10 {
        // N threads race to claim.
        let n = 4;
        let barrier = Arc::new(Barrier::new(n));
        let results: Vec<_> = (0..n)
            .map(|_| {
                let loc = loc.clone();
                let id = card_id.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    claim_card(
                        &loc,
                        &id,
                        &[TaskCardStatus::Todo, TaskCardStatus::Ready],
                        TaskCardStatus::InProgress,
                    )
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();

        let wins = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(wins, 1, "round {round}: exactly one claimer wins");

        assert_board_invariants(&loc);

        // Winner creates a run, completes it, moves card back.
        let run_id = format!("run-cycle-{round}");
        create_run(&loc, &run_id, &card_id, "d").unwrap();
        assert_run_pointer_invariants(&loc);

        complete_run(&loc, &run_id, RunOutcome::Success, None, vec![]).unwrap();
        update_status(&loc, &card_id, TaskCardStatus::Todo).unwrap();
        assert_board_invariants(&loc);
    }
}

// ── 7. Claim after reclaim race ───────────────────────────────────────

#[test]
fn reclaim_then_concurrent_re_claim() {
    let dir = tempdir().unwrap();
    let loc = thread_loc(dir.path(), "reclaim-race-1");
    let card_id = add_card(&loc, "reclaim-race");

    // Claim and create a run, then let it age and reclaim.
    claim_card(
        &loc,
        &card_id,
        &[TaskCardStatus::Todo],
        TaskCardStatus::InProgress,
    )
    .unwrap();
    create_run(&loc, "run-pre-reclaim", &card_id, "d").unwrap();

    sleep_for_staleness();

    let limits = RunLimits {
        heartbeat_stale_secs: 0,
        claim_ttl_secs: 0,
        max_reclaim_count: 10,
    };
    let result = reclaim_stale(&loc, &limits).unwrap();
    assert_eq!(
        result.reclaimed_count, 1,
        "run must be reclaimed after aging"
    );

    // Card is back to Todo. Now N threads race to re-claim.
    let n = 6;
    let barrier = Arc::new(Barrier::new(n));
    let results: Vec<_> = (0..n)
        .map(|_| {
            let loc = loc.clone();
            let id = card_id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                claim_card(
                    &loc,
                    &id,
                    &[TaskCardStatus::Todo],
                    TaskCardStatus::InProgress,
                )
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();

    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "exactly one re-claimer wins after reclaim"
    );
    assert_board_invariants(&loc);
}
