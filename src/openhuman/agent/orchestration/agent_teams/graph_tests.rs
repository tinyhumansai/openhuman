use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn completed_outcome_routes_to_complete() {
    let completed = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let c = completed.clone();
    let f = failed.clone();
    run_member_execution_graph(
        "test:complete",
        || async {
            Ok(MemberOutcome::Completed {
                output: "ok".into(),
            })
        },
        move |out| {
            let c = c.clone();
            async move {
                assert_eq!(out, "ok");
                c.store(true, Ordering::SeqCst);
                Ok(())
            }
        },
        move |_reason| {
            let f = f.clone();
            async move {
                f.store(true, Ordering::SeqCst);
                Ok(())
            }
        },
    )
    .await
    .expect("graph runs");
    assert!(completed.load(Ordering::SeqCst), "complete path ran");
    assert!(!failed.load(Ordering::SeqCst), "fail path did not run");
}

#[tokio::test]
async fn failed_outcome_routes_to_fail() {
    let completed = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let c = completed.clone();
    let f = failed.clone();
    run_member_execution_graph(
        "test:fail",
        || async {
            Ok(MemberOutcome::Failed {
                reason: "boom".into(),
            })
        },
        move |_out| {
            let c = c.clone();
            async move {
                c.store(true, Ordering::SeqCst);
                Ok(())
            }
        },
        move |reason| {
            let f = f.clone();
            async move {
                assert_eq!(reason, "boom");
                f.store(true, Ordering::SeqCst);
                Ok(())
            }
        },
    )
    .await
    .expect("graph runs");
    assert!(failed.load(Ordering::SeqCst), "fail path ran");
    assert!(
        !completed.load(Ordering::SeqCst),
        "complete path did not run"
    );
}

#[tokio::test]
async fn engine_error_from_worker_propagates() {
    let result = run_member_execution_graph(
        "test:err",
        || async { Err(anyhow::anyhow!("spawn failed")) },
        |_out| async { Ok(()) },
        |_reason| async { Ok(()) },
    )
    .await;
    assert!(result.is_err(), "worker engine error propagates out");
}
