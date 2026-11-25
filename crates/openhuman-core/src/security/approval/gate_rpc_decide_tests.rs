//! The `approval_decide` RPC must not block the runtime it is called on.
//!
//! The gate's `decide` blocks on the engine, and the native engine's decision
//! waits for the parked call's own check to release its subject's audit stream
//! (`native/engine_impl.rs`). That check is still finishing on the task that
//! parked it, which is polled only by the runtime it was spawned on. When the
//! RPC is served on that same runtime (every in-process router on a
//! current-thread runtime), deciding inline stalls the only thread that could
//! let the check finish, and the decision never returns.

use super::*;
use crate::security::approval::rpc::decide_off_the_caller_runtime;

#[test]
fn an_rpc_decision_does_not_stall_the_runtime_the_parked_call_needs() {
    let (done, finished) = std::sync::mpsc::channel();
    // Its own thread and current-thread runtime, so a stalled decision fails
    // this test on the deadline below instead of hanging the binary.
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let outcome = runtime.block_on(async {
            let (gate, _dir) = test_gate();
            let gate = Arc::new(gate);
            let parked = {
                let gate = gate.clone();
                tokio::spawn(async move {
                    turn_origin::with_origin(
                        web_origin(),
                        gate.intercept(
                            &ApprovalTurnContext::chat(chat_ctx()),
                            "rpc_decide_tool",
                            "decide over rpc",
                            serde_json::json!({}),
                        ),
                    )
                    .await
                })
            };
            let pending = loop {
                if let Some(row) = gate.list_pending().unwrap().into_iter().next() {
                    break row;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            };
            // What the engine's decision waits for can only happen on this
            // runtime, as the parked check's release does.
            let released = Arc::new(tokio::sync::Notify::new());
            stub_of(&gate).decide_waits_for(released.clone());
            tokio::spawn(async move { released.notify_one() });
            let decided = decide_off_the_caller_runtime(
                gate.clone(),
                crate::core::runtime::Tenant::default(),
                None,
                pending.request_id.clone(),
                ApprovalDecision::Deny,
            )
            .await
            .unwrap();
            assert_eq!(decided.map(|row| row.request_id), Some(pending.request_id));
            parked.await.unwrap()
        });
        let _ = done.send(outcome);
    });
    let outcome = finished
        .recv_timeout(Duration::from_secs(30))
        .expect("the decision stalled the runtime its parked call needs");
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
}
