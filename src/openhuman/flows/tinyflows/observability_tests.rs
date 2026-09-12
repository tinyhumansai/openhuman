use super::*;
use serde_json::Value;
use tinyflows::observability::RunStatus;

#[test]
fn callbacks_do_not_panic() {
    let observer = TracingRunObserver {
        run_label: "test".to_string(),
    };
    observer.on_run_start("run-1");
    observer.on_step_finish(&ExecutionStep {
        node_id: "n".to_string(),
        status: StepStatus::Success,
        output: Value::Null,
        duration_ms: 5,
        diagnostics: Vec::new(),
        transcript: Vec::new(),
    });
    observer.on_run_finish(&Run {
        id: "run-1".to_string(),
        status: RunStatus::Completed,
        steps: Vec::new(),
    });
}

#[test]
fn step_status_maps_to_stable_strings() {
    assert_eq!(step_status_str(&StepStatus::Success), "success");
    assert_eq!(step_status_str(&StepStatus::Error), "error");
}

/// The canvas's `.flow-node-running` pulse is bound to a `running` status
/// that only `on_step_start` can produce. Before it was implemented the
/// socket carried nothing but `success`/`failed`, so the pulse was
/// unreachable and the "live" overlay was really a completion trail.
/// Assert the trait method is actually overridden — a default no-op here
/// silently returns the UI to that state with every other test still green.
#[test]
fn on_step_start_is_implemented_so_the_running_status_can_reach_the_canvas() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // A probe observer whose `on_step_start` records that the *trait's*
    // dispatch reached an override rather than the blanket default.
    struct Probe {
        started: Arc<AtomicBool>,
    }
    impl RunObserver for Probe {
        fn on_step_start(&self, _node_id: &str) {
            self.started.store(true, Ordering::SeqCst);
        }
    }
    let started = Arc::new(AtomicBool::new(false));
    let probe: Box<dyn RunObserver> = Box::new(Probe {
        started: started.clone(),
    });
    probe.on_step_start("n1");
    assert!(
        started.load(Ordering::SeqCst),
        "the engine dispatches on_step_start through the trait object — if this ever \
         stops holding, FlowRunObserver's override cannot fire either"
    );

    // And the real observer must override it, not inherit the no-op.
    let src = include_str!("observability.rs");
    assert!(
        src.contains("fn on_step_start(&self, node_id: &str)"),
        "FlowRunObserver must implement on_step_start — without it no `running` \
         status is ever published and the canvas pulse is dead code"
    );
    assert!(
        src.contains("status: \"running\".to_string()"),
        "on_step_start must publish FlowRunProgress with status=running"
    );
}

// The end-to-end proof that `FlowRunObserver::on_step_finish` persists each
// step into the `flow_runs` row lives in `flows::ops_tests`
// (`observer_persists_each_step_incrementally` and the run-driven
// `flows_run_persists_live_steps_with_status_and_timing`), where the flows
// `store` internals are in scope for seeding/asserting rows.
