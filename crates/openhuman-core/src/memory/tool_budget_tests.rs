use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tinymemory_tools::{DenialReason, Refusal};

fn budget() -> RunBudget {
    RunBudget::new(Arc::new(TokioClock::default()))
}

#[tokio::test(start_paused = true)]
async fn a_slow_memory_call_returns_and_disables_memory_only_for_its_turn() {
    let budget = budget();
    let result = budget
        .run(Some("turn-one"), async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            "late"
        })
        .await;
    assert_eq!(result, Err(Refusal::TimedOut));
    assert_eq!(
        budget.run(Some("turn-one"), async { "must not run" }).await,
        Err(Refusal::Denied(DenialReason::CircuitOpen))
    );
    assert_eq!(
        budget.run(Some("turn-two"), async { "healthy" }).await,
        Ok("healthy")
    );
}

#[tokio::test]
async fn successful_memory_churn_is_bounded_without_blocking_the_next_turn() {
    let budget = budget();
    let calls = AtomicUsize::new(0);
    for i in 0..20 {
        let result = budget
            .run(Some("turn-one"), async {
                calls.fetch_add(1, Ordering::SeqCst);
                "saved"
            })
            .await;
        if i < 8 {
            assert!(result.is_ok());
        } else {
            assert_eq!(result, Err(Refusal::Denied(DenialReason::CallLimit)));
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 8);
    assert_eq!(
        budget.run(Some("turn-two"), async { "healthy" }).await,
        Ok("healthy")
    );
}

#[tokio::test(start_paused = true)]
async fn successful_memory_calls_share_one_latency_budget() {
    let budget = budget();
    let started = tokio::time::Instant::now();
    for i in 0..3 {
        let result = budget
            .run(Some("turn"), async {
                tokio::time::sleep(Duration::from_secs(12)).await;
                "saved"
            })
            .await;
        assert_eq!(result.is_ok(), i < 2);
        if i == 2 {
            assert_eq!(result, Err(Refusal::TimedOut));
        }
    }
    assert_eq!(started.elapsed(), Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn an_ordinary_tool_error_refunds_unused_time_without_opening_the_circuit() {
    let budget = budget();
    let error = budget
        .run(Some("turn"), async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            Err::<(), _>("invalid memory arguments")
        })
        .await;
    assert_eq!(error, Ok(Err("invalid memory arguments")));
    assert_eq!(
        budget
            .run(Some("turn"), async {
                tokio::time::sleep(Duration::from_secs(12)).await;
                "healthy"
            })
            .await,
        Ok("healthy")
    );
    assert_eq!(
        budget
            .run(Some("turn"), async {
                tokio::time::sleep(Duration::from_secs(15)).await;
                "remaining per-run allowance"
            })
            .await,
        Ok("remaining per-run allowance")
    );
    assert_eq!(
        budget.run(Some("turn"), async { "out of time" }).await,
        Err(Refusal::Denied(DenialReason::LatencyBudget))
    );
}

#[tokio::test(start_paused = true)]
async fn callers_without_a_run_id_still_have_a_deadline() {
    let result = budget()
        .run(None, async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            "late"
        })
        .await;
    assert_eq!(result, Err(Refusal::TimedOut));
}

#[tokio::test(start_paused = true)]
async fn parallel_memory_reads_reserve_from_the_same_time_budget() {
    let budget = budget();
    let calls = AtomicUsize::new(0);
    let read = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(12)).await;
        "read"
    };
    let (first, second, third) = tokio::join!(
        budget.run(Some("turn"), read()),
        budget.run(Some("turn"), read()),
        budget.run(Some("turn"), read()),
    );
    assert!(first.is_ok());
    assert!(second.is_ok());
    assert_eq!(third, Err(Refusal::Denied(DenialReason::LatencyBudget)));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        budget
            .run(Some("turn"), async {
                tokio::time::sleep(Duration::from_secs(6)).await;
                "last allowance"
            })
            .await,
        Ok("last allowance")
    );
}

#[tokio::test]
async fn completed_run_tracking_is_bounded() {
    let budget = budget();
    for i in 0..200 {
        assert!(budget
            .run(Some(&format!("turn-{i}")), async { "ok" })
            .await
            .is_ok());
    }
    // The first idle record was evicted, so it receives a fresh call allowance.
    for _ in 0..8 {
        assert!(budget.run(Some("turn-0"), async { "ok" }).await.is_ok());
    }
    assert_eq!(
        budget.run(Some("turn-0"), async { "too many" }).await,
        Err(Refusal::Denied(DenialReason::CallLimit))
    );
}

#[tokio::test(start_paused = true)]
async fn an_outstanding_memory_call_survives_other_runs_and_keeps_its_timeout_circuit() {
    let budget = budget();
    let call = budget.run(Some("active"), std::future::pending::<()>());
    tokio::pin!(call);
    assert!(futures::poll!(call.as_mut()).is_pending());
    for i in 0..128 {
        assert!(budget
            .run(Some(&format!("other-{i}")), async { "ok" })
            .await
            .is_ok());
    }
    assert_eq!(call.await, Err(Refusal::TimedOut));
    assert_eq!(
        budget
            .run(Some("active"), async { "must not execute" })
            .await,
        Err(Refusal::Denied(DenialReason::CircuitOpen))
    );
}

#[tokio::test(start_paused = true)]
async fn full_tracking_refuses_new_work_until_an_active_reservation_is_released() {
    let budget = budget();
    let run_ids: Vec<String> = (0..128).map(|i| format!("active-{i}")).collect();
    let mut pending = Vec::new();
    for run_id in &run_ids {
        let mut call = Box::pin(budget.run(Some(run_id), std::future::pending::<()>()));
        assert!(futures::poll!(call.as_mut()).is_pending());
        pending.push(call);
    }
    assert_eq!(
        budget.run(Some("new"), async { "must not execute" }).await,
        Err(Refusal::Denied(DenialReason::TrackingFull))
    );
    drop(pending);
    assert_eq!(
        budget.run(Some("new"), async { "healthy" }).await,
        Ok("healthy")
    );
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_call_returns_its_unused_time_reservation() {
    let budget = budget();
    let mut call = Box::pin(budget.run(Some("cancelled"), std::future::pending::<()>()));
    assert!(futures::poll!(call.as_mut()).is_pending());
    drop(call);
    for _ in 0..2 {
        assert_eq!(
            budget
                .run(Some("cancelled"), async {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    "full reservation returned"
                })
                .await,
            Ok("full reservation returned")
        );
    }
    assert_eq!(
        budget.run(Some("cancelled"), async { "out of time" }).await,
        Err(Refusal::Denied(DenialReason::LatencyBudget))
    );
}
