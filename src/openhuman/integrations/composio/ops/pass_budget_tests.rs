//! The budgeted drain the single-call sync entry points run (openhuman#6025).
//!
//! Scripted passes stand in for the connector: each test says what the
//! module would return per call and asserts which pass budgets were asked for
//! and what the aggregate reports.

use std::cell::RefCell;
use std::collections::VecDeque;

use super::super::providers_ops::{SyncPassOutcome, SYNC_PASS_MAX_ITEMS};
use super::{run_passes_within_budget, SINGLE_CALL_ITEM_BUDGET};

fn pass(written: u32, more_pending: bool) -> SyncPassOutcome {
    SyncPassOutcome {
        records_read: written as usize,
        written,
        already_ingested: false,
        more_pending,
        message: None,
    }
}

/// Drive the loop with a scripted connector; returns the outcome and the
/// per-pass budgets it asked for, in order.
async fn drive(
    budget: u32,
    script: Vec<Result<SyncPassOutcome, String>>,
) -> (Result<SyncPassOutcome, String>, Vec<usize>) {
    let script = RefCell::new(VecDeque::from(script));
    let asked = RefCell::new(Vec::new());
    let out = run_passes_within_budget(budget, |pass_budget| {
        asked.borrow_mut().push(pass_budget);
        let next = script
            .borrow_mut()
            .pop_front()
            .expect("the loop asked for more passes than the script holds");
        async move { next }
    })
    .await;
    (out, asked.into_inner())
}

#[tokio::test]
async fn stops_when_the_connector_reports_the_end() {
    // 320 records: one full pass, then a short one that says "done".
    let (out, asked) = drive(
        SINGLE_CALL_ITEM_BUDGET,
        vec![Ok(pass(200, true)), Ok(pass(120, false))],
    )
    .await;
    let out = out.expect("both passes succeed");
    assert_eq!(asked, vec![SYNC_PASS_MAX_ITEMS, SYNC_PASS_MAX_ITEMS]);
    assert_eq!(out.written, 320);
    assert_eq!(out.records_read, 320);
    assert!(!out.more_pending, "the last pass's word wins");
}

#[tokio::test]
async fn stops_when_the_budget_is_spent_and_says_more_is_pending() {
    // An account bigger than the budget: 200 + 200 + the 100 remainder, then
    // stop even though the connector still has more — that is the ceiling the
    // single-call entry points always had, now in deadline-sized calls.
    let (out, asked) = drive(
        SINGLE_CALL_ITEM_BUDGET,
        vec![
            Ok(pass(200, true)),
            Ok(pass(200, true)),
            Ok(pass(100, true)),
        ],
    )
    .await;
    let out = out.expect("all passes succeed");
    assert_eq!(asked, vec![SYNC_PASS_MAX_ITEMS, SYNC_PASS_MAX_ITEMS, 100]);
    assert_eq!(out.written, u32::try_from(SINGLE_CALL_ITEM_BUDGET).unwrap());
    assert!(
        out.more_pending,
        "the caller must learn the account is not drained"
    );
}

#[tokio::test]
async fn a_small_account_is_still_one_pass() {
    let (out, asked) = drive(SINGLE_CALL_ITEM_BUDGET, vec![Ok(pass(37, false))]).await;
    let out = out.expect("one pass succeeds");
    assert_eq!(asked, vec![SYNC_PASS_MAX_ITEMS]);
    assert_eq!(out.written, 37);
    assert!(!out.more_pending);
}

#[tokio::test]
async fn a_pass_error_ends_the_run_with_that_error() {
    let (out, asked) = drive(
        SINGLE_CALL_ITEM_BUDGET,
        vec![Ok(pass(200, true)), Err("module went away".to_string())],
    )
    .await;
    assert_eq!(out.unwrap_err(), "module went away");
    assert_eq!(asked.len(), 2, "no pass is attempted after the failure");
}

#[tokio::test]
async fn already_ingested_holds_only_when_every_pass_was_a_no_op() {
    let noop = SyncPassOutcome {
        records_read: 200,
        written: 0,
        already_ingested: true,
        more_pending: true,
        message: None,
    };
    let last = SyncPassOutcome {
        more_pending: false,
        ..noop.clone()
    };
    let (out, _) = drive(SINGLE_CALL_ITEM_BUDGET, vec![Ok(noop.clone()), Ok(last)]).await;
    let out = out.expect("passes succeed");
    assert!(out.already_ingested, "two no-op passes are a no-op run");
    assert_eq!(out.records_read, 400);
    assert_eq!(out.written, 0);

    let (out, _) = drive(SINGLE_CALL_ITEM_BUDGET, vec![Ok(noop), Ok(pass(5, false))]).await;
    assert!(
        !out.expect("passes succeed").already_ingested,
        "one write makes it a run that wrote"
    );
}

#[tokio::test]
async fn the_last_pass_note_wins_present_or_absent() {
    let noted = SyncPassOutcome {
        message: Some("  daily request budget spent  ".to_string()),
        ..pass(200, true)
    };
    let noted = SyncPassOutcome {
        more_pending: false,
        ..noted
    };
    let (out, _) = drive(
        SINGLE_CALL_ITEM_BUDGET,
        vec![Ok(pass(200, true)), Ok(noted)],
    )
    .await;
    assert_eq!(
        out.expect("passes succeed").message.as_deref(),
        Some("  daily request budget spent  "),
        "the note rides through untouched; the stage detail trims it"
    );

    let noted = SyncPassOutcome {
        message: Some("stopped short".to_string()),
        ..pass(200, true)
    };
    let (out, _) = drive(
        SINGLE_CALL_ITEM_BUDGET,
        vec![Ok(noted), Ok(pass(10, false))],
    )
    .await;
    assert_eq!(
        out.expect("passes succeed").message,
        None,
        "a pass that completes cleanly must not inherit an earlier pass's note"
    );
}

#[tokio::test]
async fn the_budget_counts_records_read_not_records_written() {
    // Everything the module returns is already ingested, and it keeps saying
    // more is pending. The old single call fetched 500 and stopped; so does
    // this — charging the budget by writes would ask forever.
    let noop = SyncPassOutcome {
        records_read: 200,
        written: 0,
        already_ingested: true,
        more_pending: true,
        message: None,
    };
    let (out, asked) = drive(
        SINGLE_CALL_ITEM_BUDGET,
        vec![
            Ok(noop.clone()),
            Ok(noop.clone()),
            Ok(SyncPassOutcome {
                records_read: 100,
                ..noop
            }),
        ],
    )
    .await;
    let out = out.expect("passes succeed");
    assert_eq!(asked, vec![SYNC_PASS_MAX_ITEMS, SYNC_PASS_MAX_ITEMS, 100]);
    assert_eq!(out.records_read, 500);
    assert_eq!(out.written, 0);
    assert!(out.more_pending);
}

#[tokio::test]
async fn a_pass_that_reads_nothing_ends_the_run() {
    // "More pending" with an empty page cannot be acted on: asking again
    // would fetch the same nothing. One pass, then stop.
    let empty = SyncPassOutcome {
        records_read: 0,
        written: 0,
        already_ingested: false,
        more_pending: true,
        message: None,
    };
    let (out, asked) = drive(SINGLE_CALL_ITEM_BUDGET, vec![Ok(empty)]).await;
    let out = out.expect("the pass succeeds");
    assert_eq!(asked.len(), 1);
    assert_eq!(out.records_read, 0);
    assert!(out.more_pending, "the module's own word is still reported");
}
